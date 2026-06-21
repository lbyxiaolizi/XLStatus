//! Notification channel and group management API.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::middleware::AuthSession;
use crate::auth::rbac::has_scope;
use crate::db::DatabaseBackend;
use crate::notifications::sender::{
    NotificationChannel, NotificationMessage, NotificationSender, NotificationSeverity,
};
use crate::security::validate_outbound_url;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    100
}

#[derive(Debug, Deserialize)]
pub struct CreateNotificationRequest {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub request_method: Option<String>,
    #[serde(default)]
    pub request_type: Option<String>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub headers_json: Option<String>,
    #[serde(default)]
    pub body_template: Option<String>,
    #[serde(default)]
    pub verify_tls: Option<bool>,
    #[serde(default)]
    pub format_metric_units: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNotificationRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub request_method: Option<String>,
    #[serde(default)]
    pub request_type: Option<String>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub headers_json: Option<String>,
    #[serde(default)]
    pub body_template: Option<String>,
    #[serde(default)]
    pub verify_tls: Option<bool>,
    #[serde(default)]
    pub format_metric_units: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateNotificationGroupRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNotificationGroupRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct AddNotificationGroupMemberRequest {
    pub notification_id: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct NotificationView {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub url: String,
    pub request_method: String,
    pub request_type: String,
    pub headers_json: Option<String>,
    pub headers: HashMap<String, String>,
    pub body_template: String,
    pub verify_tls: bool,
    pub format_metric_units: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct NotificationListResponse {
    pub notifications: Vec<NotificationView>,
    pub total: i64,
}

#[derive(Debug, Serialize, Clone)]
pub struct NotificationGroupMemberView {
    pub id: String,
    pub name: String,
    pub request_type: String,
    pub url: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct NotificationGroupView {
    pub id: String,
    pub owner_user_id: String,
    pub name: String,
    pub members: Vec<NotificationGroupMemberView>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct NotificationGroupListResponse {
    pub groups: Vec<NotificationGroupView>,
    pub total: i64,
}

#[derive(Debug, Serialize)]
pub struct NotificationProviderView {
    pub id: String,
    pub name: String,
    pub request_type: String,
    pub request_method: String,
    pub body_template: String,
}

#[derive(Debug, Serialize)]
pub struct NotificationProviderListResponse {
    pub providers: Vec<NotificationProviderView>,
}

#[derive(Debug, Deserialize)]
struct NotificationProviderPreset {
    id: String,
    name: String,
    request_type: String,
    request_method: String,
    body_template: String,
}

#[derive(Debug, Clone)]
struct NotificationInput {
    name: String,
    url: String,
    request_method: String,
    request_type: String,
    headers_json: Option<String>,
    body_template: String,
    verify_tls: bool,
    format_metric_units: bool,
}

pub async fn list_notifications(
    State(state): State<AppState>,
    auth: AuthSession,
    Query(query): Query<ListQuery>,
) -> Result<Json<ApiResponse<NotificationListResponse>>, AppError> {
    require_scope(&auth, "notification:read")?;
    let owner = auth.user_id.0;
    let limit = query.limit.clamp(1, 500);
    let offset = query.offset.max(0);
    let (notifications, total) =
        list_notifications_for_owner(&state.db, owner, limit, offset).await?;
    Ok(Json(ApiResponse::success(NotificationListResponse {
        notifications,
        total,
    })))
}

pub async fn create_notification(
    State(state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<CreateNotificationRequest>,
) -> Result<Json<ApiResponse<NotificationView>>, AppError> {
    require_scope(&auth, "notification:write")?;
    let input = build_create_notification_input(req).await?;
    let id = Uuid::now_v7().to_string();
    let owner = auth.user_id.0;
    let now = Utc::now();
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            let now_text = now.to_rfc3339();
            sqlx::query(
                r#"
                INSERT INTO notifications (
                    id, owner_user_id, name, url, request_method, request_type,
                    headers_json, body_template, verify_tls, format_metric_units,
                    created_at, updated_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(owner.to_string())
            .bind(&input.name)
            .bind(&input.url)
            .bind(&input.request_method)
            .bind(&input.request_type)
            .bind(&input.headers_json)
            .bind(&input.body_template)
            .bind(input.verify_tls)
            .bind(input.format_metric_units)
            .bind(&now_text)
            .bind(&now_text)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query(
                r#"
                INSERT INTO notifications (
                    id, owner_user_id, name, url, request_method, request_type,
                    headers_json, body_template, verify_tls, format_metric_units,
                    created_at, updated_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                "#,
            )
            .bind(&id)
            .bind(owner)
            .bind(&input.name)
            .bind(&input.url)
            .bind(&input.request_method)
            .bind(&input.request_type)
            .bind(&input.headers_json)
            .bind(&input.body_template)
            .bind(input.verify_tls)
            .bind(input.format_metric_units)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
    }

    Ok(Json(ApiResponse::success(
        load_notification_for_owner(&state.db, &id, owner).await?,
    )))
}

pub async fn update_notification(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    Json(req): Json<UpdateNotificationRequest>,
) -> Result<Json<ApiResponse<NotificationView>>, AppError> {
    require_scope(&auth, "notification:write")?;
    let owner = auth.user_id.0;
    let existing = load_notification_for_owner(&state.db, &id, owner).await?;
    let input = build_update_notification_input(req, &existing).await?;
    let now = Utc::now();
    let affected = match &state.db {
        DatabaseBackend::Sqlite(pool) => sqlx::query(
            r#"
                UPDATE notifications
                SET name = ?, url = ?, request_method = ?, request_type = ?,
                    headers_json = ?, body_template = ?, verify_tls = ?,
                    format_metric_units = ?, updated_at = ?
                WHERE id = ? AND owner_user_id = ?
                "#,
        )
        .bind(&input.name)
        .bind(&input.url)
        .bind(&input.request_method)
        .bind(&input.request_type)
        .bind(&input.headers_json)
        .bind(&input.body_template)
        .bind(input.verify_tls)
        .bind(input.format_metric_units)
        .bind(now.to_rfc3339())
        .bind(&id)
        .bind(owner.to_string())
        .execute(pool)
        .await
        .map_err(db_err)?
        .rows_affected(),
        DatabaseBackend::Postgres(pool) => sqlx::query(
            r#"
            UPDATE notifications
            SET name = $1, url = $2, request_method = $3, request_type = $4,
                headers_json = $5, body_template = $6, verify_tls = $7,
                format_metric_units = $8, updated_at = $9
            WHERE id = $10 AND owner_user_id = $11
            "#,
        )
        .bind(&input.name)
        .bind(&input.url)
        .bind(&input.request_method)
        .bind(&input.request_type)
        .bind(&input.headers_json)
        .bind(&input.body_template)
        .bind(input.verify_tls)
        .bind(input.format_metric_units)
        .bind(now)
        .bind(&id)
        .bind(owner)
        .execute(pool)
        .await
        .map_err(db_err)?
        .rows_affected(),
    };

    if affected == 0 {
        return Err(AppError::NotFound("notification not found".into()));
    }
    Ok(Json(ApiResponse::success(
        load_notification_for_owner(&state.db, &id, owner).await?,
    )))
}

pub async fn delete_notification(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_scope(&auth, "notification:delete")?;
    let owner = auth.user_id.0;
    let affected = match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query("DELETE FROM notifications WHERE id = ? AND owner_user_id = ?")
                .bind(&id)
                .bind(owner.to_string())
                .execute(pool)
                .await
                .map_err(db_err)?
                .rows_affected()
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query("DELETE FROM notifications WHERE id = $1 AND owner_user_id = $2")
                .bind(&id)
                .bind(owner)
                .execute(pool)
                .await
                .map_err(db_err)?
                .rows_affected()
        }
    };
    if affected == 0 {
        return Err(AppError::NotFound("notification not found".into()));
    }
    Ok(Json(ApiResponse::success(
        serde_json::json!({"id": id, "deleted": true}),
    )))
}

pub async fn test_notification(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_scope(&auth, "notification:write")?;
    let notification = load_notification_for_owner(&state.db, &id, auth.user_id.0).await?;
    let channel = notification_to_channel(&notification)?;
    let mut metadata = HashMap::new();
    metadata.insert("notification_id".to_string(), notification.id.clone());
    metadata.insert("notification_name".to_string(), notification.name.clone());
    NotificationSender::new()
        .send(
            &channel,
            &NotificationMessage {
                title: "XLStatus 通知测试".to_string(),
                message: "这是一条来自 XLStatus 的测试通知。".to_string(),
                severity: NotificationSeverity::Info,
                timestamp: Utc::now().to_rfc3339(),
                metadata,
            },
        )
        .await
        .map_err(|e| AppError::BadRequest(format!("notification test failed: {e}")))?;
    Ok(Json(ApiResponse::success(
        serde_json::json!({"id": id, "sent": true}),
    )))
}

pub async fn list_notification_groups(
    State(state): State<AppState>,
    auth: AuthSession,
    Query(query): Query<ListQuery>,
) -> Result<Json<ApiResponse<NotificationGroupListResponse>>, AppError> {
    require_scope(&auth, "notification:read")?;
    let owner = auth.user_id.0;
    let limit = query.limit.clamp(1, 500);
    let offset = query.offset.max(0);
    let (groups, total) = list_groups_for_owner(&state.db, owner, limit, offset).await?;
    Ok(Json(ApiResponse::success(NotificationGroupListResponse {
        groups,
        total,
    })))
}

pub async fn create_notification_group(
    State(state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<CreateNotificationGroupRequest>,
) -> Result<Json<ApiResponse<NotificationGroupView>>, AppError> {
    require_scope(&auth, "notification:write")?;
    let name = require_name(req.name, "name")?;
    let id = Uuid::now_v7().to_string();
    let owner = auth.user_id.0;
    let now = Utc::now();
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            let now_text = now.to_rfc3339();
            sqlx::query(
                "INSERT INTO notification_groups (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(owner.to_string())
            .bind(&name)
            .bind(&now_text)
            .bind(&now_text)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO notification_groups (id, owner_user_id, name, created_at, updated_at) VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(&id)
            .bind(owner)
            .bind(&name)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
    }
    Ok(Json(ApiResponse::success(
        load_group_for_owner(&state.db, &id, owner).await?,
    )))
}

pub async fn update_notification_group(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    Json(req): Json<UpdateNotificationGroupRequest>,
) -> Result<Json<ApiResponse<NotificationGroupView>>, AppError> {
    require_scope(&auth, "notification:write")?;
    let owner = auth.user_id.0;
    let name = require_name(req.name, "name")?;
    let now = Utc::now();
    let affected = match &state.db {
        DatabaseBackend::Sqlite(pool) => sqlx::query(
            "UPDATE notification_groups SET name = ?, updated_at = ? WHERE id = ? AND owner_user_id = ?",
        )
        .bind(&name)
        .bind(now.to_rfc3339())
        .bind(&id)
        .bind(owner.to_string())
        .execute(pool)
        .await
        .map_err(db_err)?
        .rows_affected(),
        DatabaseBackend::Postgres(pool) => sqlx::query(
            "UPDATE notification_groups SET name = $1, updated_at = $2 WHERE id = $3 AND owner_user_id = $4",
        )
        .bind(&name)
        .bind(now)
        .bind(&id)
        .bind(owner)
        .execute(pool)
        .await
        .map_err(db_err)?
        .rows_affected(),
    };
    if affected == 0 {
        return Err(AppError::NotFound("notification group not found".into()));
    }
    Ok(Json(ApiResponse::success(
        load_group_for_owner(&state.db, &id, owner).await?,
    )))
}

pub async fn delete_notification_group(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_scope(&auth, "notification:delete")?;
    let owner = auth.user_id.0;
    let affected = match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query("DELETE FROM notification_groups WHERE id = ? AND owner_user_id = ?")
                .bind(&id)
                .bind(owner.to_string())
                .execute(pool)
                .await
                .map_err(db_err)?
                .rows_affected()
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query("DELETE FROM notification_groups WHERE id = $1 AND owner_user_id = $2")
                .bind(&id)
                .bind(owner)
                .execute(pool)
                .await
                .map_err(db_err)?
                .rows_affected()
        }
    };
    if affected == 0 {
        return Err(AppError::NotFound("notification group not found".into()));
    }
    Ok(Json(ApiResponse::success(
        serde_json::json!({"id": id, "deleted": true}),
    )))
}

pub async fn add_notification_group_member(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    Json(req): Json<AddNotificationGroupMemberRequest>,
) -> Result<Json<ApiResponse<NotificationGroupView>>, AppError> {
    require_scope(&auth, "notification:write")?;
    let owner = auth.user_id.0;
    ensure_group_exists(&state.db, &id, owner).await?;
    ensure_notification_exists(&state.db, &req.notification_id, owner).await?;
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO notification_group_members (group_id, notification_id) VALUES (?, ?) ON CONFLICT DO NOTHING",
            )
            .bind(&id)
            .bind(&req.notification_id)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO notification_group_members (group_id, notification_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(&id)
            .bind(&req.notification_id)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
    }
    Ok(Json(ApiResponse::success(
        load_group_for_owner(&state.db, &id, owner).await?,
    )))
}

pub async fn delete_notification_group_member(
    State(state): State<AppState>,
    auth: AuthSession,
    Path((id, notification_id)): Path<(String, String)>,
) -> Result<Json<ApiResponse<NotificationGroupView>>, AppError> {
    require_scope(&auth, "notification:write")?;
    let owner = auth.user_id.0;
    ensure_group_exists(&state.db, &id, owner).await?;
    let affected = match &state.db {
        DatabaseBackend::Sqlite(pool) => sqlx::query(
            "DELETE FROM notification_group_members WHERE group_id = ? AND notification_id = ?",
        )
        .bind(&id)
        .bind(&notification_id)
        .execute(pool)
        .await
        .map_err(db_err)?
        .rows_affected(),
        DatabaseBackend::Postgres(pool) => sqlx::query(
            "DELETE FROM notification_group_members WHERE group_id = $1 AND notification_id = $2",
        )
        .bind(&id)
        .bind(&notification_id)
        .execute(pool)
        .await
        .map_err(db_err)?
        .rows_affected(),
    };
    if affected == 0 {
        return Err(AppError::NotFound(
            "notification group member not found".into(),
        ));
    }
    Ok(Json(ApiResponse::success(
        load_group_for_owner(&state.db, &id, owner).await?,
    )))
}

pub async fn list_notification_providers(
    auth: AuthSession,
) -> Result<Json<ApiResponse<NotificationProviderListResponse>>, AppError> {
    require_scope(&auth, "notification:read")?;
    let mut providers = vec![
        notification_provider(
            "generic-json",
            "通用 JSON Webhook",
            "json",
            "POST",
            r#"{"title":"{{title}}","message":"{{message}}","severity":"{{severity}}","timestamp":"{{timestamp}}"}"#,
        ),
        notification_provider(
            "generic-form",
            "通用表单 Webhook",
            "form",
            "POST",
            "title={{title}}&message={{message}}&severity={{severity}}&timestamp={{timestamp}}",
        ),
        notification_provider(
            "telegram",
            "Telegram Bot",
            "json",
            "POST",
            r#"{"text":"[{{severity}}] {{title}}\n{{message}}\n{{timestamp}}","parse_mode":"HTML"}"#,
        ),
        notification_provider(
            "bark",
            "Bark",
            "json",
            "POST",
            r#"{"title":"{{title}}","body":"{{message}}","group":"XLStatus","level":"active"}"#,
        ),
        notification_provider(
            "email-webhook",
            "Email Webhook",
            "json",
            "POST",
            r#"{"subject":"{{title}}","text":"{{message}}\n\n{{timestamp}}","severity":"{{severity}}"}"#,
        ),
        notification_provider(
            "serverchan",
            "ServerChan",
            "form",
            "POST",
            "title={{title}}&desp={{message}}%0A%0A{{timestamp}}",
        ),
        notification_provider(
            "discord",
            "Discord Webhook",
            "json",
            "POST",
            r#"{"content":"**{{title}}**\n{{message}}\n`{{severity}}` · {{timestamp}}"}"#,
        ),
        notification_provider(
            "slack",
            "Slack Webhook",
            "json",
            "POST",
            r#"{"text":"*{{title}}*\n{{message}}\n{{severity}} · {{timestamp}}"}"#,
        ),
        notification_provider("custom", "自定义 Body", "custom", "POST", "{{message}}"),
    ];
    providers.extend(notification_provider_presets_from_env());
    Ok(Json(ApiResponse::success(
        NotificationProviderListResponse { providers },
    )))
}

async fn build_create_notification_input(
    req: CreateNotificationRequest,
) -> Result<NotificationInput, AppError> {
    let name = require_name(req.name, "name")?;
    let url = require_url(req.url).await?;
    let request_method = normalize_method(req.request_method.as_deref().unwrap_or("POST"))?;
    let request_type = normalize_request_type(req.request_type.as_deref().unwrap_or("json"))?;
    let headers_json = normalize_headers(req.headers, req.headers_json.as_deref())?;
    Ok(NotificationInput {
        name,
        url,
        request_method,
        request_type,
        headers_json,
        body_template: req.body_template.unwrap_or_default(),
        verify_tls: req.verify_tls.unwrap_or(true),
        format_metric_units: req.format_metric_units.unwrap_or(true),
    })
}

async fn build_update_notification_input(
    req: UpdateNotificationRequest,
    existing: &NotificationView,
) -> Result<NotificationInput, AppError> {
    let name = match req.name {
        Some(name) => require_name(name, "name")?,
        None => existing.name.clone(),
    };
    let url_was_updated = req.url.is_some();
    let url = match req.url {
        Some(url) => require_url(url).await?,
        None => existing.url.clone(),
    };
    if !url_was_updated {
        validate_notification_url(&url).await?;
    }
    let request_method = normalize_method(
        req.request_method
            .as_deref()
            .unwrap_or(&existing.request_method),
    )?;
    let request_type = normalize_request_type(
        req.request_type
            .as_deref()
            .unwrap_or(&existing.request_type),
    )?;
    let headers_json = if req.headers.is_some() || req.headers_json.is_some() {
        normalize_headers(req.headers, req.headers_json.as_deref())?
    } else {
        existing.headers_json.clone()
    };
    Ok(NotificationInput {
        name,
        url,
        request_method,
        request_type,
        headers_json,
        body_template: req
            .body_template
            .unwrap_or_else(|| existing.body_template.clone()),
        verify_tls: req.verify_tls.unwrap_or(existing.verify_tls),
        format_metric_units: req
            .format_metric_units
            .unwrap_or(existing.format_metric_units),
    })
}

async fn list_notifications_for_owner(
    db: &DatabaseBackend,
    owner: Uuid,
    limit: i64,
    offset: i64,
) -> Result<(Vec<NotificationView>, i64), AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT id, owner_user_id, name, url, request_method, request_type,
                       headers_json, body_template, verify_tls, format_metric_units,
                       created_at, updated_at
                FROM notifications
                WHERE owner_user_id = ?
                ORDER BY created_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(owner.to_string())
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
            let total: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM notifications WHERE owner_user_id = ?")
                    .bind(owner.to_string())
                    .fetch_one(pool)
                    .await
                    .map_err(db_err)?;
            let notifications = rows
                .into_iter()
                .map(row_to_notification_sqlite)
                .collect::<Result<Vec<_>, _>>()?;
            Ok((notifications, total.0))
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT id, owner_user_id, name, url, request_method, request_type,
                       headers_json, body_template, verify_tls, format_metric_units,
                       created_at, updated_at
                FROM notifications
                WHERE owner_user_id = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(owner)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
            let total: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM notifications WHERE owner_user_id = $1")
                    .bind(owner)
                    .fetch_one(pool)
                    .await
                    .map_err(db_err)?;
            let notifications = rows
                .into_iter()
                .map(row_to_notification_postgres)
                .collect::<Result<Vec<_>, _>>()?;
            Ok((notifications, total.0))
        }
    }
}

async fn load_notification_for_owner(
    db: &DatabaseBackend,
    id: &str,
    owner: Uuid,
) -> Result<NotificationView, AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let row = sqlx::query(
                r#"
                SELECT id, owner_user_id, name, url, request_method, request_type,
                       headers_json, body_template, verify_tls, format_metric_units,
                       created_at, updated_at
                FROM notifications
                WHERE id = ? AND owner_user_id = ?
                "#,
            )
            .bind(id)
            .bind(owner.to_string())
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            row.map(row_to_notification_sqlite)
                .transpose()?
                .ok_or_else(|| AppError::NotFound("notification not found".into()))
        }
        DatabaseBackend::Postgres(pool) => {
            let row = sqlx::query(
                r#"
                SELECT id, owner_user_id, name, url, request_method, request_type,
                       headers_json, body_template, verify_tls, format_metric_units,
                       created_at, updated_at
                FROM notifications
                WHERE id = $1 AND owner_user_id = $2
                "#,
            )
            .bind(id)
            .bind(owner)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            row.map(row_to_notification_postgres)
                .transpose()?
                .ok_or_else(|| AppError::NotFound("notification not found".into()))
        }
    }
}

async fn list_groups_for_owner(
    db: &DatabaseBackend,
    owner: Uuid,
    limit: i64,
    offset: i64,
) -> Result<(Vec<NotificationGroupView>, i64), AppError> {
    let mut groups = match db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT id, owner_user_id, name, created_at, updated_at
                FROM notification_groups
                WHERE owner_user_id = ?
                ORDER BY created_at DESC
                LIMIT ? OFFSET ?
                "#,
            )
            .bind(owner.to_string())
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
            rows.into_iter()
                .map(row_to_group_sqlite)
                .collect::<Result<Vec<_>, _>>()?
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT id, owner_user_id, name, created_at, updated_at
                FROM notification_groups
                WHERE owner_user_id = $1
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#,
            )
            .bind(owner)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
            rows.into_iter()
                .map(row_to_group_postgres)
                .collect::<Result<Vec<_>, _>>()?
        }
    };

    for group in &mut groups {
        group.members = list_group_members(db, &group.id, owner).await?;
    }

    let total = match db {
        DatabaseBackend::Sqlite(pool) => {
            let total: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM notification_groups WHERE owner_user_id = ?")
                    .bind(owner.to_string())
                    .fetch_one(pool)
                    .await
                    .map_err(db_err)?;
            total.0
        }
        DatabaseBackend::Postgres(pool) => {
            let total: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM notification_groups WHERE owner_user_id = $1")
                    .bind(owner)
                    .fetch_one(pool)
                    .await
                    .map_err(db_err)?;
            total.0
        }
    };
    Ok((groups, total))
}

async fn load_group_for_owner(
    db: &DatabaseBackend,
    id: &str,
    owner: Uuid,
) -> Result<NotificationGroupView, AppError> {
    let mut group = match db {
        DatabaseBackend::Sqlite(pool) => {
            let row = sqlx::query(
                "SELECT id, owner_user_id, name, created_at, updated_at FROM notification_groups WHERE id = ? AND owner_user_id = ?",
            )
            .bind(id)
            .bind(owner.to_string())
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            row.map(row_to_group_sqlite)
                .transpose()?
                .ok_or_else(|| AppError::NotFound("notification group not found".into()))?
        }
        DatabaseBackend::Postgres(pool) => {
            let row = sqlx::query(
                "SELECT id, owner_user_id, name, created_at, updated_at FROM notification_groups WHERE id = $1 AND owner_user_id = $2",
            )
            .bind(id)
            .bind(owner)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            row.map(row_to_group_postgres)
                .transpose()?
                .ok_or_else(|| AppError::NotFound("notification group not found".into()))?
        }
    };
    group.members = list_group_members(db, id, owner).await?;
    Ok(group)
}

async fn list_group_members(
    db: &DatabaseBackend,
    group_id: &str,
    owner: Uuid,
) -> Result<Vec<NotificationGroupMemberView>, AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT n.id, n.name, n.request_type, n.url
                FROM notifications n
                JOIN notification_group_members ngm ON ngm.notification_id = n.id
                JOIN notification_groups ng ON ng.id = ngm.group_id
                WHERE ngm.group_id = ? AND ng.owner_user_id = ? AND n.owner_user_id = ?
                ORDER BY n.name ASC
                "#,
            )
            .bind(group_id)
            .bind(owner.to_string())
            .bind(owner.to_string())
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
            rows.into_iter()
                .map(row_to_group_member)
                .collect::<Result<Vec<_>, _>>()
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT n.id, n.name, n.request_type, n.url
                FROM notifications n
                JOIN notification_group_members ngm ON ngm.notification_id = n.id
                JOIN notification_groups ng ON ng.id = ngm.group_id
                WHERE ngm.group_id = $1 AND ng.owner_user_id = $2 AND n.owner_user_id = $2
                ORDER BY n.name ASC
                "#,
            )
            .bind(group_id)
            .bind(owner)
            .fetch_all(pool)
            .await
            .map_err(db_err)?;
            rows.into_iter()
                .map(row_to_group_member)
                .collect::<Result<Vec<_>, _>>()
        }
    }
}

async fn ensure_group_exists(db: &DatabaseBackend, id: &str, owner: Uuid) -> Result<(), AppError> {
    let exists = match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM notification_groups WHERE id = ? AND owner_user_id = ?",
            )
            .bind(id)
            .bind(owner.to_string())
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            row.is_some()
        }
        DatabaseBackend::Postgres(pool) => {
            let row: Option<(i64,)> = sqlx::query_as(
                "SELECT 1::BIGINT FROM notification_groups WHERE id = $1 AND owner_user_id = $2",
            )
            .bind(id)
            .bind(owner)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            row.is_some()
        }
    };
    if exists {
        Ok(())
    } else {
        Err(AppError::NotFound("notification group not found".into()))
    }
}

pub(crate) async fn ensure_notification_group_owned_by(
    db: &DatabaseBackend,
    owner: Uuid,
    group_id: Option<&str>,
) -> Result<(), AppError> {
    let Some(group_id) = group_id.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    let exists = match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: Option<(i64,)> = sqlx::query_as(
                "SELECT 1 FROM notification_groups WHERE id = ? AND owner_user_id = ?",
            )
            .bind(group_id)
            .bind(owner.to_string())
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            row.is_some()
        }
        DatabaseBackend::Postgres(pool) => {
            let parsed_group_id = Uuid::parse_str(group_id)
                .map_err(|e| AppError::BadRequest(format!("invalid notification_group_id: {e}")))?;
            let row: Option<(i64,)> = sqlx::query_as(
                "SELECT 1::BIGINT FROM notification_groups WHERE id = $1 AND owner_user_id = $2",
            )
            .bind(parsed_group_id)
            .bind(owner)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            row.is_some()
        }
    };
    if exists {
        Ok(())
    } else {
        Err(AppError::BadRequest(
            "notification_group_id does not exist or is not owned by current user".into(),
        ))
    }
}

async fn ensure_notification_exists(
    db: &DatabaseBackend,
    id: &str,
    owner: Uuid,
) -> Result<(), AppError> {
    let exists = match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: Option<(i64,)> =
                sqlx::query_as("SELECT 1 FROM notifications WHERE id = ? AND owner_user_id = ?")
                    .bind(id)
                    .bind(owner.to_string())
                    .fetch_optional(pool)
                    .await
                    .map_err(db_err)?;
            row.is_some()
        }
        DatabaseBackend::Postgres(pool) => {
            let row: Option<(i64,)> = sqlx::query_as(
                "SELECT 1::BIGINT FROM notifications WHERE id = $1 AND owner_user_id = $2",
            )
            .bind(id)
            .bind(owner)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            row.is_some()
        }
    };
    if exists {
        Ok(())
    } else {
        Err(AppError::NotFound("notification not found".into()))
    }
}

fn row_to_notification_sqlite(row: sqlx::sqlite::SqliteRow) -> Result<NotificationView, AppError> {
    let headers_json: Option<String> = row.try_get("headers_json").map_err(db_err)?;
    let headers = parse_headers_json(headers_json.as_deref())?;
    Ok(NotificationView {
        id: row.try_get("id").map_err(db_err)?,
        owner_user_id: row.try_get("owner_user_id").map_err(db_err)?,
        name: row.try_get("name").map_err(db_err)?,
        url: row.try_get("url").map_err(db_err)?,
        request_method: row.try_get("request_method").map_err(db_err)?,
        request_type: row.try_get("request_type").map_err(db_err)?,
        headers_json,
        headers,
        body_template: row
            .try_get::<Option<String>, _>("body_template")
            .map_err(db_err)?
            .unwrap_or_default(),
        verify_tls: row.try_get("verify_tls").map_err(db_err)?,
        format_metric_units: row.try_get("format_metric_units").map_err(db_err)?,
        created_at: row.try_get("created_at").map_err(db_err)?,
        updated_at: row.try_get("updated_at").map_err(db_err)?,
    })
}

fn row_to_notification_postgres(row: sqlx::postgres::PgRow) -> Result<NotificationView, AppError> {
    let headers_json: Option<String> = row.try_get("headers_json").map_err(db_err)?;
    let headers = parse_headers_json(headers_json.as_deref())?;
    let owner: Uuid = row.try_get("owner_user_id").map_err(db_err)?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(db_err)?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at").map_err(db_err)?;
    Ok(NotificationView {
        id: row.try_get("id").map_err(db_err)?,
        owner_user_id: owner.to_string(),
        name: row.try_get("name").map_err(db_err)?,
        url: row.try_get("url").map_err(db_err)?,
        request_method: row.try_get("request_method").map_err(db_err)?,
        request_type: row.try_get("request_type").map_err(db_err)?,
        headers_json,
        headers,
        body_template: row
            .try_get::<Option<String>, _>("body_template")
            .map_err(db_err)?
            .unwrap_or_default(),
        verify_tls: row.try_get("verify_tls").map_err(db_err)?,
        format_metric_units: row.try_get("format_metric_units").map_err(db_err)?,
        created_at: created_at.to_rfc3339(),
        updated_at: updated_at.to_rfc3339(),
    })
}

fn row_to_group_sqlite(row: sqlx::sqlite::SqliteRow) -> Result<NotificationGroupView, AppError> {
    Ok(NotificationGroupView {
        id: row.try_get("id").map_err(db_err)?,
        owner_user_id: row.try_get("owner_user_id").map_err(db_err)?,
        name: row.try_get("name").map_err(db_err)?,
        members: Vec::new(),
        created_at: row.try_get("created_at").map_err(db_err)?,
        updated_at: row.try_get("updated_at").map_err(db_err)?,
    })
}

fn row_to_group_postgres(row: sqlx::postgres::PgRow) -> Result<NotificationGroupView, AppError> {
    let owner: Uuid = row.try_get("owner_user_id").map_err(db_err)?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(db_err)?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at").map_err(db_err)?;
    Ok(NotificationGroupView {
        id: row.try_get("id").map_err(db_err)?,
        owner_user_id: owner.to_string(),
        name: row.try_get("name").map_err(db_err)?,
        members: Vec::new(),
        created_at: created_at.to_rfc3339(),
        updated_at: updated_at.to_rfc3339(),
    })
}

fn row_to_group_member<R>(row: R) -> Result<NotificationGroupMemberView, AppError>
where
    R: Row,
    String: for<'a> sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
    for<'a> &'a str: sqlx::ColumnIndex<R>,
{
    Ok(NotificationGroupMemberView {
        id: row.try_get("id").map_err(db_err)?,
        name: row.try_get("name").map_err(db_err)?,
        request_type: row.try_get("request_type").map_err(db_err)?,
        url: row.try_get("url").map_err(db_err)?,
    })
}

fn notification_to_channel(view: &NotificationView) -> Result<NotificationChannel, AppError> {
    Ok(NotificationChannel {
        id: view.id.clone(),
        name: view.name.clone(),
        url: view.url.clone(),
        request_method: view.request_method.clone(),
        request_type: view.request_type.clone(),
        headers: parse_headers_json(view.headers_json.as_deref())?,
        body_template: view.body_template.clone(),
        verify_tls: view.verify_tls,
    })
}

fn notification_provider(
    id: &str,
    name: &str,
    request_type: &str,
    request_method: &str,
    body_template: &str,
) -> NotificationProviderView {
    NotificationProviderView {
        id: id.to_string(),
        name: name.to_string(),
        request_type: request_type.to_string(),
        request_method: request_method.to_string(),
        body_template: body_template.to_string(),
    }
}

fn notification_provider_presets_from_env() -> Vec<NotificationProviderView> {
    let Ok(raw) = std::env::var("XLSTATUS_NOTIFICATION_PROVIDER_PRESETS") else {
        return Vec::new();
    };
    let presets = match serde_json::from_str::<Vec<NotificationProviderPreset>>(&raw) {
        Ok(presets) => presets,
        Err(err) => {
            tracing::warn!("invalid XLSTATUS_NOTIFICATION_PROVIDER_PRESETS: {}", err);
            return Vec::new();
        }
    };
    presets
        .into_iter()
        .filter_map(|preset| {
            let id = preset.id.trim();
            let name = preset.name.trim();
            if id.is_empty() || name.is_empty() {
                return None;
            }
            let request_type = normalize_request_type(&preset.request_type).ok()?;
            let request_method = normalize_method(&preset.request_method).ok()?;
            Some(NotificationProviderView {
                id: id.to_string(),
                name: name.to_string(),
                request_type,
                request_method,
                body_template: preset.body_template,
            })
        })
        .collect()
}

fn require_name(value: String, field: &str) -> Result<String, AppError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    if value.len() > 255 {
        return Err(AppError::BadRequest(format!("{field} is too long")));
    }
    Ok(value)
}

async fn require_url(value: String) -> Result<String, AppError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(AppError::BadRequest("url is required".into()));
    }
    validate_notification_url(&value).await?;
    Ok(value)
}

async fn validate_notification_url(url: &str) -> Result<(), AppError> {
    let probe_url = replace_template_markers(url);
    validate_outbound_url(&probe_url, "notification webhook")
        .await
        .map(|_| ())
        .map_err(|e| AppError::BadRequest(e.to_string()))
}

fn normalize_method(value: &str) -> Result<String, AppError> {
    let method = value.trim().to_uppercase();
    match method.as_str() {
        "GET" | "POST" | "PUT" | "PATCH" => Ok(method),
        _ => Err(AppError::BadRequest(
            "request_method must be GET, POST, PUT, or PATCH".into(),
        )),
    }
}

fn normalize_request_type(value: &str) -> Result<String, AppError> {
    let request_type = value.trim().to_lowercase();
    match request_type.as_str() {
        "json" | "form" | "custom" => Ok(request_type),
        _ => Err(AppError::BadRequest(
            "request_type must be json, form, or custom".into(),
        )),
    }
}

fn normalize_headers(
    headers: Option<HashMap<String, String>>,
    headers_json: Option<&str>,
) -> Result<Option<String>, AppError> {
    let headers = match headers {
        Some(headers) => headers,
        None => parse_headers_json(headers_json)?,
    };
    let mut cleaned = HashMap::new();
    for (key, value) in headers {
        let key = key.trim();
        if key.is_empty() {
            return Err(AppError::BadRequest("header name must not be empty".into()));
        }
        if key.contains('\n') || key.contains('\r') || value.contains('\n') || value.contains('\r')
        {
            return Err(AppError::BadRequest(
                "headers must not contain newline characters".into(),
            ));
        }
        cleaned.insert(key.to_string(), value);
    }
    if cleaned.is_empty() {
        Ok(None)
    } else {
        serde_json::to_string(&cleaned)
            .map(Some)
            .map_err(|e| AppError::BadRequest(format!("invalid headers: {e}")))
    }
}

fn parse_headers_json(value: Option<&str>) -> Result<HashMap<String, String>, AppError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(HashMap::new());
    };
    serde_json::from_str(value)
        .map_err(|e| AppError::BadRequest(format!("headers_json must be a string map: {e}")))
}

fn replace_template_markers(input: &str) -> String {
    let mut rest = input;
    let mut out = String::with_capacity(input.len());
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        if let Some(end) = after_start.find("}}") {
            out.push_str("test");
            rest = &after_start[end + 2..];
        } else {
            out.push_str(&rest[start..]);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

fn require_scope(auth: &AuthSession, scope: &str) -> Result<(), AppError> {
    if has_scope(auth, scope) {
        Ok(())
    } else {
        Err(AppError::Forbidden(format!("missing scope: {scope}")))
    }
}

fn db_err(err: sqlx::Error) -> AppError {
    AppError::Database(anyhow::anyhow!(err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headers_from_json_object() {
        let headers = parse_headers_json(Some(r#"{"X-Test":"ok"}"#)).unwrap();
        assert_eq!(headers.get("X-Test"), Some(&"ok".to_string()));
    }

    #[test]
    fn rejects_invalid_request_type() {
        assert!(normalize_request_type("xml").is_err());
    }

    #[test]
    fn replaces_unknown_url_template_markers() {
        assert_eq!(
            replace_template_markers("https://example.com/{{title}}/{{metadata.host}}"),
            "https://example.com/test/test"
        );
    }
}
