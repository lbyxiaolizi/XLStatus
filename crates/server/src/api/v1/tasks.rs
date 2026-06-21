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
    validate_task_selector_or_403(&auth_user, &req.server_selector_json)?;
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
    let tasks = TaskRepository::list_by_user(
        &db,
        &auth_user.user.id.0.to_string(),
        query.limit,
        query.offset,
    )
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                success: false,
                data: None,
                error: Some(format!("Failed to list tasks: {}", e)),
            }),
        )
    })?;

    let tasks = tasks
        .into_iter()
        .filter(|task| task_visible_to_auth(&auth_user, task))
        .collect::<Vec<_>>();
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
    validate_task_selector_or_403(&auth_user, &task.server_selector_json)?;
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

    let mut runs = TaskRunRepository::list_by_task(&db, &task_id, query.limit, query.offset)
        .await
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
    filter_task_runs_for_auth(&auth_user, &mut runs);

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
    let selector: ServerSelector = match serde_json::from_str(selector_json) {
        Ok(s) => s,
        Err(_) => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "server_selector_json is not valid JSON",
            ));
        }
    };

    let mut scoped_server_ids = selector.server_ids.clone();
    scoped_server_ids.extend(selector.exclude_server_ids.clone());
    let session = auth_user.auth_session();

    if auth_user.server_ids.is_some() {
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
        if !can_access_servers(&session, &scoped_server_ids) {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "server_selector_json contains servers outside PAT allowlist",
            ));
        }
    } else if !scoped_server_ids.is_empty() && !can_access_servers(&session, &scoped_server_ids) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "server_selector_json contains servers outside PAT allowlist",
        ));
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

fn filter_task_runs_for_auth(auth_user: &AuthUser, runs: &mut Vec<TaskRun>) {
    let Some(allowed) = auth_user.server_ids.as_ref() else {
        return;
    };
    runs.retain(|run| allowed.iter().any(|server_id| server_id == &run.server_id));
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
    use crate::db::User;
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

    #[test]
    fn scoped_pat_filters_task_runs_to_allowlist() {
        let auth = pat_with_servers(vec!["server-a"]);
        let mut runs = vec![task_run("server-a"), task_run("server-b")];

        filter_task_runs_for_auth(&auth, &mut runs);

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].server_id, "server-a");
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

    fn task(cover_mode: CoverMode, selector: serde_json::Value) -> Task {
        Task {
            id: "task".into(),
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

    fn task_run(server_id: &str) -> TaskRun {
        TaskRun {
            id: format!("run-{server_id}"),
            task_id: "task".into(),
            server_id: server_id.into(),
            status: TaskStatus::Success,
            delay_ms: Some(1),
            output: None,
            output_truncated: false,
            error: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }
}
