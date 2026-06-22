#![allow(dead_code)]
#![allow(unused_imports)]

use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::notifications::ensure_notification_group_owned_by;
use crate::auth::middleware::{AuthSession, AuthUser};
use crate::auth::rbac::{can_access_servers, has_scope};
use crate::db::repository::tasks::{TaskRepository, TaskRunRepository};
use crate::db::Db;
use crate::tasks::{
    dispatch_task_to_agents, parse_task_schedule, validate_task_definition, TASK_API_MAX_BODY_BYTES,
};
use xlstatus_shared::tasks::*;

const TASK_LIST_MAX_LIMIT: i64 = 500;
const TASK_LIST_SCAN_BATCH: i64 = 500;

#[derive(Debug, Deserialize)]
pub struct CreateTaskRequest {
    pub name: String,
    pub task_type: TaskType,
    pub schedule: Option<String>,
    pub command: Option<String>,
    pub payload_json: Option<String>,
    pub cover_mode: CoverMode,
    pub server_selector_json: String,
    pub push_successful: bool,
    pub notification_group_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTaskRequest {
    pub name: Option<String>,
    pub schedule: Option<String>,
    pub command: Option<String>,
    pub payload_json: Option<String>,
    pub cover_mode: Option<CoverMode>,
    pub server_selector_json: Option<String>,
    pub push_successful: Option<bool>,
    pub notification_group_id: Option<Option<String>>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Debug, Serialize)]
pub struct TaskResponse {
    pub task: Task,
}

#[derive(Debug, Serialize)]
pub struct TaskListResponse {
    pub tasks: Vec<Task>,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct TaskRunsResponse {
    pub runs: Vec<TaskRun>,
    pub total: usize,
}

/// Create a new task
pub async fn create_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<CreateTaskRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "task:write")?;
    validate_task_selector_for_write_or_403(&db, &auth_user, &req.server_selector_json).await?;
    let notification_group_id = normalize_optional_id(req.notification_group_id);
    ensure_task_notification_group_owned(&db, &auth_user, notification_group_id.as_deref()).await?;
    // Validate schedule if present
    if let Some(ref schedule) = req.schedule {
        if parse_task_schedule(schedule).is_err() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("Invalid cron schedule".to_string()),
                }),
            ));
        }
    }

    let now = Utc::now().to_rfc3339();
    let task = Task {
        id: uuid::Uuid::now_v7().to_string(),
        owner_user_id: auth_user.user.id.0.to_string(),
        name: req.name,
        task_type: req.task_type,
        schedule: req.schedule,
        command: req.command,
        payload_json: req.payload_json,
        cover_mode: req.cover_mode,
        server_selector_json: req.server_selector_json,
        push_successful: req.push_successful,
        notification_group_id,
        last_executed_at: None,
        last_result: None,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };
    validate_task_definition_or_403(&task)?;

    TaskRepository::create(&db, &task).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to create task: {}", e)),
            }),
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(ApiResponse {
            success: true,
            data: Some(TaskResponse { task }),
            error: None,
        }),
    ))
}

/// List tasks
pub async fn list_tasks(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ListQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "task:read")?;
    let limit = normalize_list_limit(query.limit);
    let offset = normalize_list_offset(query.offset);
    let tasks = list_visible_tasks(&db, &auth_user, limit, offset).await?;
    let total = tasks.len();

    Ok(Json(ApiResponse {
        success: true,
        data: Some(TaskListResponse { tasks, total }),
        error: None,
    }))
}

/// Get a specific task
pub async fn get_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "task:read")?;
    let task = TaskRepository::get_by_id(&db, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to get task: {}", e)),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("Task not found".to_string()),
                }),
            )
        })?;

    // Check ownership
    if task.owner_user_id != auth_user.user.id.0.to_string() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("Not authorized".to_string()),
            }),
        ));
    }
    ensure_task_visible_to_auth(&auth_user, &task)?;

    Ok(Json(ApiResponse {
        success: true,
        data: Some(TaskResponse { task }),
        error: None,
    }))
}

/// Update a task
pub async fn update_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateTaskRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "task:write")?;
    let mut task = TaskRepository::get_by_id(&db, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to get task: {}", e)),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("Task not found".to_string()),
                }),
            )
        })?;

    // Check ownership
    if task.owner_user_id != auth_user.user.id.0.to_string() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("Not authorized".to_string()),
            }),
        ));
    }
    ensure_task_visible_to_auth(&auth_user, &task)?;

    // Apply updates
    if let Some(name) = req.name {
        task.name = name;
    }
    if let Some(schedule) = req.schedule {
        // Validate schedule
        if parse_task_schedule(&schedule).is_err() {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("Invalid cron schedule".to_string()),
                }),
            ));
        }
        task.schedule = Some(schedule);
    }
    if let Some(command) = req.command {
        task.command = Some(command);
    }
    if let Some(payload_json) = req.payload_json {
        task.payload_json = Some(payload_json);
    }
    if let Some(cover_mode) = req.cover_mode {
        task.cover_mode = cover_mode;
    }
    if let Some(server_selector_json) = req.server_selector_json {
        task.server_selector_json = server_selector_json;
    }
    if let Some(push_successful) = req.push_successful {
        task.push_successful = push_successful;
    }
    if let Some(notification_group_id) = req.notification_group_id {
        task.notification_group_id = normalize_optional_id(notification_group_id);
    }
    if let Some(enabled) = req.enabled {
        task.enabled = enabled;
    }
    validate_task_selector_for_write_or_403(&db, &auth_user, &task.server_selector_json).await?;
    ensure_task_notification_group_owned(&db, &auth_user, task.notification_group_id.as_deref())
        .await?;

    task.updated_at = Utc::now().to_rfc3339();
    validate_task_definition_or_403(&task)?;

    TaskRepository::update(&db, &task).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to update task: {}", e)),
            }),
        )
    })?;

    Ok(Json(ApiResponse {
        success: true,
        data: Some(TaskResponse { task }),
        error: None,
    }))
}

/// Delete a task
pub async fn delete_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "task:delete")?;
    let task = TaskRepository::get_by_id(&db, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to get task: {}", e)),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("Task not found".to_string()),
                }),
            )
        })?;

    // Check ownership
    if task.owner_user_id != auth_user.user.id.0.to_string() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("Not authorized".to_string()),
            }),
        ));
    }
    ensure_task_visible_to_auth(&auth_user, &task)?;

    TaskRepository::delete(&db, &task_id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to delete task: {}", e)),
            }),
        )
    })?;

    Ok((
        StatusCode::NO_CONTENT,
        Json(ApiResponse::<()> {
            success: true,
            data: None,
            error: None,
        }),
    ))
}

/// Manually run a task.
///
/// M5: parse the task's server selector, dispatch a
/// `ServerMessage::Task` to every live agent that matches, persist a
/// `task_runs` row per agent, and wait up to ~5s for each
/// `TaskResult` reply. Agents that are offline are recorded as
/// `offline`. The response summarizes the per-agent outcomes.
pub async fn run_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "task:exec")?;
    let task = TaskRepository::get_by_id(&db, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to get task: {}", e)),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("Task not found".to_string()),
                }),
            )
        })?;

    // Check ownership
    if task.owner_user_id != auth_user.user.id.0.to_string() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("Not authorized".to_string()),
            }),
        ));
    }

    ensure_task_visible_to_auth(&auth_user, &task)?;

    let report = dispatch_task_to_agents(
        &db,
        &state.session_registry,
        crate::current_task_response_registry(),
        &task,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(e.to_string()),
            }),
        )
    })?;

    Ok(Json(ApiResponse {
        success: true,
        data: Some(report),
        error: None,
    }))
}

/// Get task execution history
pub async fn get_task_runs(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let db = state.db.clone();

    require_scope_or_403(&auth_user, "task:read")?;
    let task = TaskRepository::get_by_id(&db, &task_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some(format!("Failed to get task: {}", e)),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ApiResponse {
                    success: false,
                    data: None,
                    error: Some("Task not found".to_string()),
                }),
            )
        })?;

    // Check ownership
    if task.owner_user_id != auth_user.user.id.0.to_string() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some("Not authorized".to_string()),
            }),
        ));
    }
    ensure_task_visible_to_auth(&auth_user, &task)?;

    let limit = normalize_list_limit(query.limit);
    let offset = normalize_list_offset(query.offset);
    let runs = if let Some(server_ids) = auth_user.server_ids.as_ref() {
        TaskRunRepository::list_by_task_for_server_ids(&db, &task_id, server_ids, limit, offset)
            .await
    } else {
        TaskRunRepository::list_by_task(&db, &task_id, limit, offset).await
    }
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to list task runs: {}", e)),
            }),
        )
    })?;

    let total = runs.len();

    Ok(Json(ApiResponse {
        success: true,
        data: Some(TaskRunsResponse { runs, total }),
        error: None,
    }))
}

pub fn task_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(TASK_API_MAX_BODY_BYTES)
}

pub(crate) async fn ensure_task_ids_visible_to_auth_session(
    db: &Db,
    auth: &AuthSession,
    task_ids: &[String],
) -> Result<(), AppError> {
    let owner_user_id = auth.user_id.0.to_string();
    for task_id in task_ids {
        let task = TaskRepository::get_by_id(db, task_id)
            .await
            .map_err(AppError::Database)?
            .ok_or_else(|| {
                AppError::BadRequest(format!(
                    "task {task_id} does not exist or is not owned by current user"
                ))
            })?;
        if task.owner_user_id != owner_user_id {
            return Err(AppError::BadRequest(format!(
                "task {task_id} does not exist or is not owned by current user"
            )));
        }
        validate_task_selector_for_session(auth, &task.server_selector_json)
            .map_err(|_| AppError::Forbidden("trigger task not in scope".into()))?;
    }
    Ok(())
}

async fn list_visible_tasks(
    db: &Db,
    auth_user: &AuthUser,
    limit: i64,
    offset: i64,
) -> Result<Vec<Task>, (StatusCode, Json<ApiResponse<()>>)> {
    let owner = auth_user.user.id.0.to_string();
    if auth_user.server_ids.is_none() {
        return TaskRepository::list_by_user(db, &owner, limit, offset)
            .await
            .map_err(list_tasks_error);
    }
    if matches!(auth_user.server_ids.as_ref(), Some(ids) if ids.is_empty()) {
        return Ok(Vec::new());
    }

    let mut db_offset = 0_i64;
    let mut visible_seen = 0_i64;
    let mut out = Vec::new();
    loop {
        let batch = TaskRepository::list_by_user(db, &owner, TASK_LIST_SCAN_BATCH, db_offset)
            .await
            .map_err(list_tasks_error)?;
        if batch.is_empty() {
            break;
        }
        db_offset += batch.len() as i64;
        for task in batch {
            if !task_visible_to_auth(auth_user, &task) {
                continue;
            }
            if visible_seen < offset {
                visible_seen += 1;
                continue;
            }
            if out.len() >= limit as usize {
                return Ok(out);
            }
            visible_seen += 1;
            out.push(task);
            if out.len() >= limit as usize {
                return Ok(out);
            }
        }
    }
    Ok(out)
}

fn list_tasks_error(e: anyhow::Error) -> (StatusCode, Json<ApiResponse<()>>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiResponse {
            success: false,
            data: None,
            error: Some(format!("Failed to list tasks: {}", e)),
        }),
    )
}

fn normalize_list_limit(limit: i64) -> i64 {
    limit.clamp(1, TASK_LIST_MAX_LIMIT)
}

fn normalize_list_offset(offset: i64) -> i64 {
    offset.max(0)
}

fn require_scope_or_403(
    auth_user: &AuthUser,
    required_scope: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    if has_scope(&auth_user.auth_session(), required_scope) {
        Ok(())
    } else {
        Err(api_error(
            StatusCode::FORBIDDEN,
            format!("missing required scope: {}", required_scope),
        ))
    }
}

fn validate_task_selector_or_403(
    auth_user: &AuthUser,
    selector_json: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    validate_task_selector_for_session(&auth_user.auth_session(), selector_json)
}

async fn validate_task_selector_for_write_or_403(
    db: &Db,
    auth_user: &AuthUser,
    selector_json: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    validate_task_selector_or_403(auth_user, selector_json)?;
    let selector = parse_task_selector_or_403(selector_json)?;
    let explicit_server_ids = explicit_selector_server_ids(&selector);
    ensure_explicit_selector_servers_active(db, auth_user, &explicit_server_ids).await
}

fn validate_task_selector_for_session(
    session: &AuthSession,
    selector_json: &str,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    let selector = parse_task_selector_or_403(selector_json)?;

    let mut scoped_server_ids = selector.server_ids.clone();
    scoped_server_ids.extend(selector.exclude_server_ids.clone());

    if session.server_ids.is_some() {
        let has_dynamic_selector = selector.source_server
            || !selector.group_ids.is_empty()
            || !selector.tag_names.is_empty()
            || !selector.tags.is_empty();
        if has_dynamic_selector || selector.server_ids.is_empty() {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "PAT-scoped tasks must target explicit servers in the allowlist",
            ));
        }
        if !can_access_servers(session, &scoped_server_ids) {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "server_selector_json contains servers outside PAT allowlist",
            ));
        }
    } else if !scoped_server_ids.is_empty() && !can_access_servers(session, &scoped_server_ids) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "server_selector_json contains servers outside PAT allowlist",
        ));
    }

    Ok(())
}

fn parse_task_selector_or_403(
    selector_json: &str,
) -> Result<ServerSelector, (StatusCode, Json<ApiResponse<()>>)> {
    serde_json::from_str(selector_json).map_err(|_| {
        api_error(
            StatusCode::BAD_REQUEST,
            "server_selector_json is not valid JSON",
        )
    })
}

fn explicit_selector_server_ids(selector: &ServerSelector) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for server_id in selector
        .server_ids
        .iter()
        .chain(selector.exclude_server_ids.iter())
    {
        if seen.insert(server_id) {
            out.push(server_id.clone());
        }
    }
    out
}

async fn ensure_explicit_selector_servers_active(
    db: &Db,
    auth_user: &AuthUser,
    server_ids: &[String],
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    if server_ids.is_empty() {
        return Ok(());
    }

    for server_id in server_ids {
        let parsed = uuid::Uuid::parse_str(server_id).map_err(|_| {
            api_error(
                StatusCode::BAD_REQUEST,
                format!("server_selector_json contains invalid server_id: {server_id}"),
            )
        })?;
        let exists = match db {
            Db::Sqlite(pool) => {
                let row: (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM agents WHERE id = ? AND owner_user_id = ? AND revoked_at IS NULL",
                )
                .bind(parsed.to_string())
                .bind(auth_user.user.id.0.to_string())
                .fetch_one(pool)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to validate task selector server: {}", e);
                    api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                })?;
                row.0 > 0
            }
            Db::Postgres(pool) => {
                let row: (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM agents WHERE id = $1 AND owner_user_id = $2 AND revoked_at IS NULL",
                )
                .bind(parsed)
                .bind(auth_user.user.id.0)
                .fetch_one(pool)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to validate task selector server: {}", e);
                    api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
                })?;
                row.0 > 0
            }
        };
        if !exists {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                format!("server_selector_json contains unknown or revoked server: {server_id}"),
            ));
        }
    }
    Ok(())
}

fn ensure_task_visible_to_auth(
    auth_user: &AuthUser,
    task: &Task,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    validate_task_selector_or_403(auth_user, &task.server_selector_json)
}

fn validate_task_definition_or_403(task: &Task) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    validate_task_definition(task)
        .map_err(|err| api_error(StatusCode::BAD_REQUEST, err.to_string()))
}

fn task_visible_to_auth(auth_user: &AuthUser, task: &Task) -> bool {
    ensure_task_visible_to_auth(auth_user, task).is_ok()
}

async fn ensure_task_notification_group_owned(
    db: &Db,
    auth_user: &AuthUser,
    group_id: Option<&str>,
) -> Result<(), (StatusCode, Json<ApiResponse<()>>)> {
    ensure_notification_group_owned_by(db, auth_user.user.id.0, group_id)
        .await
        .map_err(app_error_to_api)
}

fn normalize_optional_id(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn app_error_to_api(err: AppError) -> (StatusCode, Json<ApiResponse<()>>) {
    match err {
        AppError::Database(e) => {
            tracing::error!("Database error: {}", e);
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
        AppError::Unauthorized(message) => api_error(StatusCode::UNAUTHORIZED, message),
        AppError::Forbidden(message) => api_error(StatusCode::FORBIDDEN, message),
        AppError::BadRequest(message) => api_error(StatusCode::BAD_REQUEST, message),
        AppError::TooManyRequests(message) => api_error(StatusCode::TOO_MANY_REQUESTS, message),
        AppError::NotFound(message) => api_error(StatusCode::NOT_FOUND, message),
    }
}

fn api_error(
    status: StatusCode,
    message: impl Into<String>,
) -> (StatusCode, Json<ApiResponse<()>>) {
    (
        status,
        Json(ApiResponse {
            success: false,
            data: None,
            error: Some(message.into()),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::middleware::AuthKind;
    use crate::db::{DatabaseBackend, User};
    use chrono::Utc;
    use serde_json::json;
    use xlstatus_shared::{UserId, UserRole};

    #[test]
    fn scoped_pat_rejects_task_all_selector_without_explicit_servers() {
        let auth = pat_with_servers(vec!["server-a"]);
        let selector = json!({}).to_string();

        let err = validate_task_selector_or_403(&auth, &selector).unwrap_err();

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn scoped_pat_rejects_task_dynamic_selector() {
        let auth = pat_with_servers(vec!["server-a"]);
        let selector = json!({ "group_ids": ["group-a"] }).to_string();

        let err = validate_task_selector_or_403(&auth, &selector).unwrap_err();

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn scoped_pat_rejects_task_selector_outside_allowlist() {
        let auth = pat_with_servers(vec!["server-a"]);
        let selector = json!({ "server_ids": ["server-b"] }).to_string();

        let err = validate_task_selector_or_403(&auth, &selector).unwrap_err();

        assert_eq!(err.0, StatusCode::FORBIDDEN);
    }

    #[test]
    fn scoped_pat_accepts_explicit_allowed_task_selector() {
        let auth = pat_with_servers(vec!["server-a"]);
        let selector = json!({ "server_ids": ["server-a"] }).to_string();

        assert!(validate_task_selector_or_403(&auth, &selector).is_ok());
    }

    #[test]
    fn scoped_pat_task_visibility_rejects_existing_broad_task() {
        let auth = pat_with_servers(vec!["server-a"]);
        let task = task(CoverMode::All, json!({}));

        assert!(!task_visible_to_auth(&auth, &task));
    }

    #[tokio::test]
    async fn scoped_pat_task_list_filters_visibility_before_limit() {
        let db = test_db().await;
        let auth = pat_with_servers(vec!["server-a"]);
        let owner = auth.user.id.0.to_string();
        seed_user(&db, &owner).await;
        seed_task_with_selector(
            &db,
            "blocked",
            &owner,
            json!({ "server_ids": ["server-b"] }),
            "2026-01-03T00:00:00Z",
        )
        .await;
        seed_task_with_selector(
            &db,
            "allowed",
            &owner,
            json!({ "server_ids": ["server-a"] }),
            "2026-01-02T00:00:00Z",
        )
        .await;

        let visible = list_visible_tasks(&db, &auth, 1, 0).await.unwrap();

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "allowed");
    }

    #[tokio::test]
    async fn scoped_pat_task_runs_filter_allowlist_before_limit() {
        let db = test_db().await;
        let task_id = "task";
        let owner = uuid::Uuid::from_bytes([1; 16]).to_string();
        seed_user(&db, &owner).await;
        seed_server(&db, "00000000-0000-0000-0000-0000000000aa", &owner).await;
        seed_server(&db, "00000000-0000-0000-0000-0000000000bb", &owner).await;
        seed_task(&db, task_id, &owner).await;
        seed_task_run(
            &db,
            "run-blocked",
            task_id,
            "00000000-0000-0000-0000-0000000000bb",
            "2026-01-03T00:00:00Z",
        )
        .await;
        seed_task_run(
            &db,
            "run-allowed",
            task_id,
            "00000000-0000-0000-0000-0000000000aa",
            "2026-01-02T00:00:00Z",
        )
        .await;

        let runs = TaskRunRepository::list_by_task_for_server_ids(
            &db,
            task_id,
            &["00000000-0000-0000-0000-0000000000aa".to_string()],
            1,
            0,
        )
        .await
        .unwrap();

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, "run-allowed");
        assert_eq!(runs[0].server_id, "00000000-0000-0000-0000-0000000000aa");
    }

    #[test]
    fn task_definition_rejects_oversized_command() {
        let mut task = task(CoverMode::Specific, json!({ "server_ids": ["server-a"] }));
        task.command = Some("x".repeat(crate::tasks::TASK_MAX_COMMAND_BYTES + 1));

        let err = validate_task_definition_or_403(&task).unwrap_err();

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err
            .1
             .0
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("task.command"));
    }

    #[test]
    fn task_definition_rejects_oversized_selector_shape() {
        let server_ids = (0..=crate::tasks::TASK_MAX_SELECTOR_IDS)
            .map(|idx| format!("server-{idx}"))
            .collect::<Vec<_>>();
        let task = task(CoverMode::Specific, json!({ "server_ids": server_ids }));

        let err = validate_task_definition_or_403(&task).unwrap_err();

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err
            .1
             .0
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("too many entries"));
    }

    #[tokio::test]
    async fn task_write_selector_requires_explicit_active_servers() {
        let db = test_db().await;
        let auth = cookie_admin();
        let owner = auth.user.id.0.to_string();
        let active_server = uuid::Uuid::from_bytes([2; 16]).to_string();
        let revoked_server = uuid::Uuid::from_bytes([3; 16]).to_string();
        let other_owner_server = uuid::Uuid::from_bytes([4; 16]).to_string();
        let other_owner = uuid::Uuid::from_bytes([5; 16]).to_string();
        seed_user(&db, &owner).await;
        seed_user(&db, &other_owner).await;
        seed_agent(&db, &active_server, &owner, None).await;
        seed_agent(&db, &revoked_server, &owner, Some("2026-01-02T00:00:00Z")).await;
        seed_agent(&db, &other_owner_server, &other_owner, None).await;

        let active_selector = json!({ "server_ids": [active_server] }).to_string();
        assert!(
            validate_task_selector_for_write_or_403(&db, &auth, &active_selector)
                .await
                .is_ok()
        );

        let revoked_selector = json!({ "server_ids": [revoked_server] }).to_string();
        let err = validate_task_selector_for_write_or_403(&db, &auth, &revoked_selector)
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err
            .1
             .0
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("unknown or revoked server"));

        let other_owner_selector =
            json!({ "exclude_server_ids": [other_owner_server] }).to_string();
        let err = validate_task_selector_for_write_or_403(&db, &auth, &other_owner_selector)
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn scoped_pat_task_write_rejects_revoked_allowlist_server() {
        let db = test_db().await;
        let owner = uuid::Uuid::from_bytes([1; 16]).to_string();
        let revoked_server = uuid::Uuid::from_bytes([6; 16]).to_string();
        seed_user(&db, &owner).await;
        seed_agent(&db, &revoked_server, &owner, Some("2026-01-02T00:00:00Z")).await;
        let auth = pat_with_servers(vec![&revoked_server]);
        let selector = json!({ "server_ids": [revoked_server] }).to_string();

        let err = validate_task_selector_for_write_or_403(&db, &auth, &selector)
            .await
            .unwrap_err();

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err
            .1
             .0
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("unknown or revoked server"));
    }

    fn pat_with_servers(server_ids: Vec<&str>) -> AuthUser {
        let now = Utc::now();
        AuthUser {
            user: User {
                id: UserId(uuid::Uuid::from_bytes([1; 16])),
                username: "owner".into(),
                password_hash: "x".into(),
                role: UserRole::Admin,
                token_version: 0,
                created_at: now,
                updated_at: now,
            },
            session_id: "pat-session".into(),
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::PersonalAccessToken,
            scopes: vec!["task:read".into(), "task:write".into(), "task:exec".into()],
            server_ids: Some(server_ids.into_iter().map(str::to_string).collect()),
            pat_id: Some("pat".into()),
        }
    }

    fn cookie_admin() -> AuthUser {
        let now = Utc::now();
        AuthUser {
            user: User {
                id: UserId(uuid::Uuid::from_bytes([1; 16])),
                username: "owner".into(),
                password_hash: "x".into(),
                role: UserRole::Admin,
                token_version: 0,
                created_at: now,
                updated_at: now,
            },
            session_id: "session".into(),
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::Session,
            scopes: Vec::new(),
            server_ids: None,
            pat_id: None,
        }
    }

    fn task(cover_mode: CoverMode, selector: serde_json::Value) -> Task {
        task_with_id("task", cover_mode, selector)
    }

    fn task_with_id(id: &str, cover_mode: CoverMode, selector: serde_json::Value) -> Task {
        Task {
            id: id.into(),
            owner_user_id: uuid::Uuid::from_bytes([1; 16]).to_string(),
            name: "task".into(),
            task_type: TaskType::Shell,
            schedule: None,
            command: Some("true".into()),
            payload_json: None,
            cover_mode,
            server_selector_json: selector.to_string(),
            push_successful: false,
            notification_group_id: None,
            last_executed_at: None,
            last_result: None,
            enabled: true,
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    async fn test_db() -> DatabaseBackend {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_user(db: &DatabaseBackend, id: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'x', 'admin', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("user-{id}"))
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_server(db: &DatabaseBackend, id: &str, owner_user_id: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO servers (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, 'server', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner_user_id)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_agent(
        db: &DatabaseBackend,
        id: &str,
        owner_user_id: &str,
        revoked_at: Option<&str>,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO agents (id, name, public_key, owner_user_id, revoked_at, created_at, updated_at) VALUES (?, ?, 'pk', ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(format!("agent-{id}"))
        .bind(owner_user_id)
        .bind(revoked_at)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_task(db: &DatabaseBackend, id: &str, owner_user_id: &str) {
        seed_task_with_selector(
            db,
            id,
            owner_user_id,
            json!({ "server_ids": ["00000000-0000-0000-0000-0000000000aa"] }),
            "2026-01-01T00:00:00Z",
        )
        .await;
    }

    async fn seed_task_with_selector(
        db: &DatabaseBackend,
        id: &str,
        owner_user_id: &str,
        selector: serde_json::Value,
        created_at: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO tasks (id, owner_user_id, name, task_type, cover_mode, server_selector_json, created_at, updated_at) VALUES (?, ?, 'task', ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(owner_user_id)
        .bind(serde_json::to_string(&TaskType::Shell).unwrap())
        .bind(serde_json::to_string(&CoverMode::Specific).unwrap())
        .bind(selector.to_string())
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_task_run(
        db: &DatabaseBackend,
        id: &str,
        task_id: &str,
        server_id: &str,
        created_at: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO task_runs (id, task_id, server_id, status, delay_ms, output_truncated, created_at) VALUES (?, ?, ?, 'success', 1, 0, ?)",
        )
        .bind(id)
        .bind(task_id)
        .bind(server_id)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
    }
}
