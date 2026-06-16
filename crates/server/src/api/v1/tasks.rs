use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::types::ApiResponse;
use crate::auth::middleware::AuthUser;
use crate::db::repository::tasks::{TaskRepository, TaskRunRepository};
use crate::db::Db;
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
    pub notification_group_id: Option<String>,
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
    State(db): State<Db>,
    auth_user: AuthUser,
    Json(req): Json<CreateTaskRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    // Validate schedule if present
    if let Some(ref schedule) = req.schedule {
        if schedule.parse::<cron::Schedule>().is_err() {
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
        notification_group_id: req.notification_group_id,
        last_executed_at: None,
        last_result: None,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };

    TaskRepository::create(&db, &task)
        .await
        .map_err(|e| {
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
    State(db): State<Db>,
    auth_user: AuthUser,
    Query(query): Query<ListQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
    let tasks = TaskRepository::list_by_user(&db, &auth_user.user.id.0.to_string(), query.limit, query.offset)
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

    let total = tasks.len();

    Ok(Json(ApiResponse {
        success: true,
        data: Some(TaskListResponse { tasks, total }),
        error: None,
        
    }))
}

/// Get a specific task
pub async fn get_task(
    State(db): State<Db>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
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

    Ok(Json(ApiResponse {
        success: true,
        data: Some(TaskResponse { task }),
        error: None,
        
    }))
}

/// Update a task
pub async fn update_task(
    State(db): State<Db>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
    Json(req): Json<UpdateTaskRequest>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
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

    // Apply updates
    if let Some(name) = req.name {
        task.name = name;
    }
    if let Some(schedule) = req.schedule {
        // Validate schedule
        if schedule.parse::<cron::Schedule>().is_err() {
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
        task.notification_group_id = Some(notification_group_id);
    }
    if let Some(enabled) = req.enabled {
        task.enabled = enabled;
    }

    task.updated_at = Utc::now().to_rfc3339();

    TaskRepository::update(&db, &task)
        .await
        .map_err(|e| {
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
    State(db): State<Db>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
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

    TaskRepository::delete(&db, &task_id)
        .await
        .map_err(|e| {
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

/// Manually run a task
pub async fn run_task(
    State(db): State<Db>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
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

    // TODO: Trigger task execution via scheduler
    // For now, just return success

    Ok(Json(ApiResponse {
        success: true,
        data: Some(serde_json::json!({
            "message": "Task execution triggered"
        })),
        error: None,
        
    }))
}

/// Get task execution history
pub async fn get_task_runs(
    State(db): State<Db>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
    Query(query): Query<ListQuery>,
) -> Result<impl IntoResponse, (StatusCode, Json<ApiResponse<()>>)> {
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

    let runs = TaskRunRepository::list_by_task(&db, &task_id, query.limit, query.offset)
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

    let total = runs.len();

    Ok(Json(ApiResponse {
        success: true,
        data: Some(TaskRunsResponse { runs, total }),
        error: None,
        
    }))
}
