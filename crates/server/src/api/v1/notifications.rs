//! Notification channel and group management API.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{require_sensitive_totp, AppError, AppState};
use crate::auth::middleware::{AuthKind, AuthSession};
use crate::auth::rbac::has_scope;
use crate::db::DatabaseBackend;
use crate::notifications::policy::parse_notification_headers_json;
use crate::notifications::sender::{
    ensure_headers_allowed, NotificationChannel, NotificationMessage, NotificationSender,
    NotificationSeverity, NOTIFICATION_MAX_BODY_TEMPLATE_BYTES, NOTIFICATION_MAX_GROUP_CHANNELS,
    NOTIFICATION_MAX_HEADERS_JSON_BYTES, NOTIFICATION_MAX_NAME_BYTES, NOTIFICATION_MAX_URL_BYTES,
};
use crate::security::validate_webhook_outbound_url;
use axum::{
    extract::{DefaultBodyLimit, Path, Query, State},
    http::HeaderMap,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;
use std::time::{Duration as StdDuration, Instant};
use uuid::Uuid;

const NOTIFICATION_TEST_COOLDOWN_SECS: u64 = 30;
const NOTIFICATION_UUID_TEXT_LEN: usize = 36;

static NOTIFICATION_TEST_RATE_STATE: once_cell::sync::Lazy<
    std::sync::Mutex<HashMap<String, Instant>>,
> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(HashMap::new()));

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

const REDACTED_NOTIFICATION_SECRET: &str = "[redacted]";
pub(crate) const NOTIFICATION_API_MAX_BODY_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone)]
struct NotificationRecord {
    id: String,
    owner_user_id: String,
    name: String,
    url: String,
    request_method: String,
    request_type: String,
    headers_json: Option<String>,
    body_template: String,
    verify_tls: bool,
    format_metric_units: bool,
    created_at: String,
    updated_at: String,
}

impl NotificationRecord {
    fn to_view(&self) -> Result<NotificationView, AppError> {
        let (headers_json, headers) = redacted_headers_for_view(self.headers_json.as_deref());
        Ok(NotificationView {
            id: self.id.clone(),
            owner_user_id: self.owner_user_id.clone(),
            name: notification_view_name(&self.name),
            url: redact_notification_url_for_view(&self.url),
            request_method: notification_view_method(&self.request_method),
            request_type: notification_view_request_type(&self.request_type),
            headers_json,
            headers,
            body_template: redact_body_template(&self.body_template),
            verify_tls: self.verify_tls,
            format_metric_units: self.format_metric_units,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
        })
    }
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
    let (records, total) = list_notifications_for_owner(&state.db, owner, limit, offset).await?;
    let notifications = records
        .into_iter()
        .map(|record| record.to_view())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(ApiResponse::success(NotificationListResponse {
        notifications,
        total,
    })))
}

pub async fn create_notification(
    State(state): State<AppState>,
    auth: AuthSession,
    headers: HeaderMap,
    Json(req): Json<CreateNotificationRequest>,
) -> Result<Json<ApiResponse<NotificationView>>, AppError> {
    require_notification_sensitive_scope(&state.db, &auth, "notification:write", &headers).await?;
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
        load_notification_record_for_owner(&state.db, &id, owner)
            .await?
            .to_view()?,
    )))
}

pub async fn update_notification(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<UpdateNotificationRequest>,
) -> Result<Json<ApiResponse<NotificationView>>, AppError> {
    require_notification_sensitive_scope(&state.db, &auth, "notification:write", &headers).await?;
    let id = require_uuid_text(id, "notification_id")?;
    let owner = auth.user_id.0;
    let existing = load_notification_record_for_owner(&state.db, &id, owner).await?;
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
        load_notification_record_for_owner(&state.db, &id, owner)
            .await?
            .to_view()?,
    )))
}

pub async fn delete_notification(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_notification_sensitive_scope(&state.db, &auth, "notification:delete", &headers).await?;
    let id = require_uuid_text(id, "notification_id")?;
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
    headers: HeaderMap,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_notification_sensitive_scope(&state.db, &auth, "notification:write", &headers).await?;
    let id = require_uuid_text(id, "notification_id")?;
    let notification = load_notification_record_for_owner(&state.db, &id, auth.user_id.0).await?;
    check_notification_test_rate_limit(auth.user_id.0, &notification.id)?;
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

pub fn notification_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(NOTIFICATION_API_MAX_BODY_BYTES)
}

fn check_notification_test_rate_limit(
    user_id: Uuid,
    notification_id: &str,
) -> Result<(), AppError> {
    let key = format!("{user_id}:{notification_id}");
    check_notification_test_rate_limit_in(&NOTIFICATION_TEST_RATE_STATE, key, Instant::now())
}

fn check_notification_test_rate_limit_in(
    state: &std::sync::Mutex<HashMap<String, Instant>>,
    key: String,
    now: Instant,
) -> Result<(), AppError> {
    let mut state = state
        .lock()
        .map_err(|_| AppError::Database(anyhow::anyhow!("notification test limiter poisoned")))?;
    state.retain(|_, last| now.duration_since(*last) < notification_test_cooldown());
    if let Some(last) = state.get(&key) {
        if now.duration_since(*last) < notification_test_cooldown() {
            return Err(AppError::TooManyRequests(format!(
                "notification test can be sent once every {NOTIFICATION_TEST_COOLDOWN_SECS} seconds"
            )));
        }
    }
    state.insert(key, now);
    Ok(())
}

fn notification_test_cooldown() -> StdDuration {
    StdDuration::from_secs(NOTIFICATION_TEST_COOLDOWN_SECS)
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
    headers: HeaderMap,
    Json(req): Json<CreateNotificationGroupRequest>,
) -> Result<Json<ApiResponse<NotificationGroupView>>, AppError> {
    require_notification_sensitive_scope(&state.db, &auth, "notification:write", &headers).await?;
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
    headers: HeaderMap,
    Json(req): Json<UpdateNotificationGroupRequest>,
) -> Result<Json<ApiResponse<NotificationGroupView>>, AppError> {
    require_notification_sensitive_scope(&state.db, &auth, "notification:write", &headers).await?;
    let id = require_uuid_text(id, "group_id")?;
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
    headers: HeaderMap,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_notification_sensitive_scope(&state.db, &auth, "notification:delete", &headers).await?;
    let id = require_uuid_text(id, "group_id")?;
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
    headers: HeaderMap,
    Json(req): Json<AddNotificationGroupMemberRequest>,
) -> Result<Json<ApiResponse<NotificationGroupView>>, AppError> {
    require_notification_sensitive_scope(&state.db, &auth, "notification:write", &headers).await?;
    let owner = auth.user_id.0;
    let group_id = require_uuid_text(id, "group_id")?;
    ensure_group_exists(&state.db, &group_id, owner).await?;
    let notification_id = require_uuid_text(req.notification_id, "notification_id")?;
    ensure_notification_exists(&state.db, &notification_id, owner).await?;
    if notification_group_member_exists(&state.db, &group_id, &notification_id, owner).await? {
        return Ok(Json(ApiResponse::success(
            load_group_for_owner(&state.db, &group_id, owner).await?,
        )));
    }
    ensure_group_member_count_allowed(&state.db, &group_id, owner).await?;
    match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO notification_group_members (group_id, notification_id) VALUES (?, ?) ON CONFLICT DO NOTHING",
            )
            .bind(&group_id)
            .bind(&notification_id)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO notification_group_members (group_id, notification_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(&group_id)
            .bind(&notification_id)
            .execute(pool)
            .await
            .map_err(db_err)?;
        }
    }
    Ok(Json(ApiResponse::success(
        load_group_for_owner(&state.db, &group_id, owner).await?,
    )))
}

pub async fn delete_notification_group_member(
    State(state): State<AppState>,
    auth: AuthSession,
    Path((id, notification_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<NotificationGroupView>>, AppError> {
    require_notification_sensitive_scope(&state.db, &auth, "notification:write", &headers).await?;
    let owner = auth.user_id.0;
    let group_id = require_uuid_text(id, "group_id")?;
    let notification_id = require_uuid_text(notification_id, "notification_id")?;
    delete_notification_group_member_for_owner(&state.db, &group_id, &notification_id, owner)
        .await?;
    Ok(Json(ApiResponse::success(
        load_group_for_owner(&state.db, &group_id, owner).await?,
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
    let body_template = normalize_body_template(req.body_template)?;
    Ok(NotificationInput {
        name,
        url,
        request_method,
        request_type,
        headers_json,
        body_template,
        verify_tls: req.verify_tls.unwrap_or(true),
        format_metric_units: req.format_metric_units.unwrap_or(true),
    })
}

async fn build_update_notification_input(
    req: UpdateNotificationRequest,
    existing: &NotificationRecord,
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
    let body_template = normalize_body_template(
        req.body_template
            .or_else(|| Some(existing.body_template.clone())),
    )?;
    Ok(NotificationInput {
        name,
        url,
        request_method,
        request_type,
        headers_json,
        body_template,
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
) -> Result<(Vec<NotificationRecord>, i64), AppError> {
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
                .map(row_to_notification_record_sqlite)
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
                .map(row_to_notification_record_postgres)
                .collect::<Result<Vec<_>, _>>()?;
            Ok((notifications, total.0))
        }
    }
}

async fn load_notification_record_for_owner(
    db: &DatabaseBackend,
    id: &str,
    owner: Uuid,
) -> Result<NotificationRecord, AppError> {
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
            row.map(row_to_notification_record_sqlite)
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
            row.map(row_to_notification_record_postgres)
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
                LIMIT ?
                "#,
            )
            .bind(group_id)
            .bind(owner.to_string())
            .bind(owner.to_string())
            .bind(NOTIFICATION_MAX_GROUP_CHANNELS as i64)
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
                LIMIT $3
                "#,
            )
            .bind(group_id)
            .bind(owner)
            .bind(NOTIFICATION_MAX_GROUP_CHANNELS as i64)
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
            let row: Option<(i64,)> = sqlx::query_as(
                "SELECT 1::BIGINT FROM notification_groups WHERE id = $1 AND owner_user_id = $2",
            )
            .bind(group_id)
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

async fn ensure_group_member_count_allowed(
    db: &DatabaseBackend,
    group_id: &str,
    owner: Uuid,
) -> Result<(), AppError> {
    let count = match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: (i64,) = sqlx::query_as(
                r#"
                SELECT COUNT(*)
                FROM notification_group_members ngm
                JOIN notification_groups ng ON ng.id = ngm.group_id
                WHERE ngm.group_id = ? AND ng.owner_user_id = ?
                "#,
            )
            .bind(group_id)
            .bind(owner.to_string())
            .fetch_one(pool)
            .await
            .map_err(db_err)?;
            row.0 as usize
        }
        DatabaseBackend::Postgres(pool) => {
            let row: (i64,) = sqlx::query_as(
                r#"
                SELECT COUNT(*)::BIGINT
                FROM notification_group_members ngm
                JOIN notification_groups ng ON ng.id = ngm.group_id
                WHERE ngm.group_id = $1 AND ng.owner_user_id = $2
                "#,
            )
            .bind(group_id)
            .bind(owner)
            .fetch_one(pool)
            .await
            .map_err(db_err)?;
            row.0 as usize
        }
    };
    if count >= NOTIFICATION_MAX_GROUP_CHANNELS {
        return Err(AppError::BadRequest(format!(
            "notification group contains too many members; maximum is {NOTIFICATION_MAX_GROUP_CHANNELS}"
        )));
    }
    Ok(())
}

async fn notification_group_member_exists(
    db: &DatabaseBackend,
    group_id: &str,
    notification_id: &str,
    owner: Uuid,
) -> Result<bool, AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: Option<(i64,)> = sqlx::query_as(
                r#"
                SELECT 1
                FROM notification_group_members ngm
                JOIN notification_groups ng ON ng.id = ngm.group_id
                JOIN notifications n ON n.id = ngm.notification_id
                WHERE ngm.group_id = ?
                  AND ngm.notification_id = ?
                  AND ng.owner_user_id = ?
                  AND n.owner_user_id = ?
                "#,
            )
            .bind(group_id)
            .bind(notification_id)
            .bind(owner.to_string())
            .bind(owner.to_string())
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            Ok(row.is_some())
        }
        DatabaseBackend::Postgres(pool) => {
            let row: Option<(i64,)> = sqlx::query_as(
                r#"
                SELECT 1::BIGINT
                FROM notification_group_members ngm
                JOIN notification_groups ng ON ng.id = ngm.group_id
                JOIN notifications n ON n.id = ngm.notification_id
                WHERE ngm.group_id = $1
                  AND ngm.notification_id = $2
                  AND ng.owner_user_id = $3
                  AND n.owner_user_id = $3
                "#,
            )
            .bind(group_id)
            .bind(notification_id)
            .bind(owner)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?;
            Ok(row.is_some())
        }
    }
}

async fn delete_notification_group_member_for_owner(
    db: &DatabaseBackend,
    group_id: &str,
    notification_id: &str,
    owner: Uuid,
) -> Result<(), AppError> {
    ensure_group_exists(db, group_id, owner).await?;
    ensure_notification_exists(db, notification_id, owner).await?;
    let affected = match db {
        DatabaseBackend::Sqlite(pool) => sqlx::query(
            r#"
            DELETE FROM notification_group_members
            WHERE group_id = ?
              AND notification_id = ?
              AND EXISTS (
                  SELECT 1 FROM notification_groups ng
                  WHERE ng.id = notification_group_members.group_id
                    AND ng.owner_user_id = ?
              )
              AND EXISTS (
                  SELECT 1 FROM notifications n
                  WHERE n.id = notification_group_members.notification_id
                    AND n.owner_user_id = ?
              )
            "#,
        )
        .bind(group_id)
        .bind(notification_id)
        .bind(owner.to_string())
        .bind(owner.to_string())
        .execute(pool)
        .await
        .map_err(db_err)?
        .rows_affected(),
        DatabaseBackend::Postgres(pool) => sqlx::query(
            r#"
            DELETE FROM notification_group_members
            WHERE group_id = $1
              AND notification_id = $2
              AND EXISTS (
                  SELECT 1 FROM notification_groups ng
                  WHERE ng.id = notification_group_members.group_id
                    AND ng.owner_user_id = $3
              )
              AND EXISTS (
                  SELECT 1 FROM notifications n
                  WHERE n.id = notification_group_members.notification_id
                    AND n.owner_user_id = $3
              )
            "#,
        )
        .bind(group_id)
        .bind(notification_id)
        .bind(owner)
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
    Ok(())
}

fn row_to_notification_record_sqlite(
    row: sqlx::sqlite::SqliteRow,
) -> Result<NotificationRecord, AppError> {
    let headers_json: Option<String> = row.try_get("headers_json").map_err(db_err)?;
    Ok(NotificationRecord {
        id: row.try_get("id").map_err(db_err)?,
        owner_user_id: row.try_get("owner_user_id").map_err(db_err)?,
        name: row.try_get("name").map_err(db_err)?,
        url: row.try_get("url").map_err(db_err)?,
        request_method: row.try_get("request_method").map_err(db_err)?,
        request_type: row.try_get("request_type").map_err(db_err)?,
        headers_json,
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

fn row_to_notification_record_postgres(
    row: sqlx::postgres::PgRow,
) -> Result<NotificationRecord, AppError> {
    let headers_json: Option<String> = row.try_get("headers_json").map_err(db_err)?;
    let owner: Uuid = row.try_get("owner_user_id").map_err(db_err)?;
    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(db_err)?;
    let updated_at: DateTime<Utc> = row.try_get("updated_at").map_err(db_err)?;
    Ok(NotificationRecord {
        id: row.try_get("id").map_err(db_err)?,
        owner_user_id: owner.to_string(),
        name: row.try_get("name").map_err(db_err)?,
        url: row.try_get("url").map_err(db_err)?,
        request_method: row.try_get("request_method").map_err(db_err)?,
        request_type: row.try_get("request_type").map_err(db_err)?,
        headers_json,
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
    let url: String = row.try_get("url").map_err(db_err)?;
    Ok(NotificationGroupMemberView {
        id: row.try_get("id").map_err(db_err)?,
        name: row.try_get("name").map_err(db_err)?,
        request_type: row.try_get("request_type").map_err(db_err)?,
        url: redact_notification_url(&url),
    })
}

fn notification_to_channel(record: &NotificationRecord) -> Result<NotificationChannel, AppError> {
    Ok(NotificationChannel {
        id: record.id.clone(),
        name: record.name.clone(),
        url: record.url.clone(),
        request_method: record.request_method.clone(),
        request_type: record.request_type.clone(),
        headers: parse_headers_json(record.headers_json.as_deref())?,
        body_template: record.body_template.clone(),
        verify_tls: record.verify_tls,
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
    if value.len() > NOTIFICATION_MAX_NAME_BYTES {
        return Err(AppError::BadRequest(format!(
            "{field} must be at most {NOTIFICATION_MAX_NAME_BYTES} bytes"
        )));
    }
    Ok(value)
}

async fn require_url(value: String) -> Result<String, AppError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(AppError::BadRequest("url is required".into()));
    }
    if value.len() > NOTIFICATION_MAX_URL_BYTES {
        return Err(AppError::BadRequest(format!(
            "url must be at most {NOTIFICATION_MAX_URL_BYTES} bytes"
        )));
    }
    validate_notification_url(&value).await?;
    Ok(value)
}

fn require_uuid_text(value: String, field: &str) -> Result<String, AppError> {
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    if value.len() != NOTIFICATION_UUID_TEXT_LEN {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    let parsed = Uuid::parse_str(&value)
        .map_err(|e| AppError::BadRequest(format!("invalid {field}: {e}")))?;
    if parsed.to_string() != value {
        return Err(AppError::BadRequest(format!(
            "{field} must be a canonical UUID"
        )));
    }
    Ok(value)
}

async fn validate_notification_url(url: &str) -> Result<(), AppError> {
    let probe_url = replace_template_markers(url);
    validate_webhook_outbound_url(&probe_url, "notification webhook")
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
    if let Some(headers_json) = headers_json {
        let trimmed = headers_json.trim();
        if trimmed.len() > NOTIFICATION_MAX_HEADERS_JSON_BYTES {
            return Err(AppError::BadRequest(format!(
                "headers_json must be at most {NOTIFICATION_MAX_HEADERS_JSON_BYTES} bytes"
            )));
        }
    }
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
    ensure_headers_allowed(&cleaned).map_err(|e| AppError::BadRequest(e.to_string()))?;
    if cleaned.is_empty() {
        Ok(None)
    } else {
        serde_json::to_string(&cleaned)
            .map(Some)
            .map_err(|e| AppError::BadRequest(format!("invalid headers: {e}")))
    }
}

fn parse_headers_json(value: Option<&str>) -> Result<HashMap<String, String>, AppError> {
    parse_notification_headers_json(value).map_err(|e| AppError::BadRequest(e.to_string()))
}

fn normalize_body_template(value: Option<String>) -> Result<String, AppError> {
    let value = value.unwrap_or_default();
    if value.len() > NOTIFICATION_MAX_BODY_TEMPLATE_BYTES {
        return Err(AppError::BadRequest(format!(
            "body_template must be at most {NOTIFICATION_MAX_BODY_TEMPLATE_BYTES} bytes"
        )));
    }
    Ok(value)
}

fn redacted_headers_for_view(value: Option<&str>) -> (Option<String>, HashMap<String, String>) {
    let headers = match parse_headers_json(value) {
        Ok(headers) => headers,
        Err(err) => {
            tracing::warn!("historical notification headers omitted from API view: {err:?}");
            return (None, HashMap::new());
        }
    };
    if headers.is_empty() {
        return (None, HashMap::new());
    }
    let redacted = headers
        .into_keys()
        .map(|key| (key, REDACTED_NOTIFICATION_SECRET.to_string()))
        .collect::<HashMap<_, _>>();
    let headers_json = serde_json::to_string(&redacted).ok();
    (headers_json, redacted)
}

fn notification_view_name(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > NOTIFICATION_MAX_NAME_BYTES {
        return "Invalid notification".to_string();
    }
    trimmed.to_string()
}

fn redact_notification_url_for_view(value: &str) -> String {
    if value.len() > NOTIFICATION_MAX_URL_BYTES {
        return REDACTED_NOTIFICATION_SECRET.to_string();
    }
    redact_notification_url(value)
}

fn notification_view_method(value: &str) -> String {
    if value.trim().len() > 8 {
        return "POST".to_string();
    }
    normalize_method(value).unwrap_or_else(|_| "POST".to_string())
}

fn notification_view_request_type(value: &str) -> String {
    if value.trim().len() > 16 {
        return "json".to_string();
    }
    normalize_request_type(value).unwrap_or_else(|_| "json".to_string())
}

fn redact_body_template(value: &str) -> String {
    if value.trim().is_empty() {
        String::new()
    } else {
        REDACTED_NOTIFICATION_SECRET.to_string()
    }
}

fn redact_notification_url(value: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(value) else {
        return REDACTED_NOTIFICATION_SECRET.to_string();
    };
    let Some(host) = parsed.host_str() else {
        return REDACTED_NOTIFICATION_SECRET.to_string();
    };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let authority = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    if parsed.path() == "/" && parsed.query().is_none() && parsed.fragment().is_none() {
        format!("{}://{authority}/", parsed.scheme())
    } else {
        format!(
            "{}://{authority}/{REDACTED_NOTIFICATION_SECRET}",
            parsed.scheme()
        )
    }
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

async fn require_notification_sensitive_scope(
    db: &DatabaseBackend,
    auth: &AuthSession,
    scope: &str,
    headers: &HeaderMap,
) -> Result<(), AppError> {
    require_scope(auth, scope)?;
    if matches!(auth.auth_kind, AuthKind::PersonalAccessToken) {
        return Err(AppError::Forbidden(
            "notification changes require a cookie session".into(),
        ));
    }
    require_sensitive_totp(db, auth.user_id, headers).await
}

fn db_err(err: sqlx::Error) -> AppError {
    AppError::Database(anyhow::anyhow!(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use xlstatus_shared::{UserId, UserRole};

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

    #[test]
    fn notification_api_view_redacts_secret_bearing_fields() {
        let record = NotificationRecord {
            id: "notification-1".to_string(),
            owner_user_id: "user-1".to_string(),
            name: "webhook".to_string(),
            url: "https://hooks.example.com/services/token/path?secret=value".to_string(),
            request_method: "POST".to_string(),
            request_type: "json".to_string(),
            headers_json: Some(
                r#"{"Authorization":"Bearer secret-token","X-Webhook-Secret":"secret"}"#
                    .to_string(),
            ),
            body_template: r#"{"token":"secret","message":"{{message}}"}"#.to_string(),
            verify_tls: true,
            format_metric_units: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let view = record.to_view().unwrap();

        assert_eq!(
            view.url,
            format!("https://hooks.example.com/{REDACTED_NOTIFICATION_SECRET}")
        );
        assert_eq!(
            view.headers.get("Authorization").map(String::as_str),
            Some(REDACTED_NOTIFICATION_SECRET)
        );
        assert_eq!(
            view.headers.get("X-Webhook-Secret").map(String::as_str),
            Some(REDACTED_NOTIFICATION_SECRET)
        );
        assert_eq!(view.body_template, REDACTED_NOTIFICATION_SECRET);
        assert!(!serde_json::to_string(&view)
            .unwrap()
            .contains("secret-token"));
    }

    #[test]
    fn notification_api_view_tolerates_historical_dirty_fields() {
        let record = NotificationRecord {
            id: "notification-1".to_string(),
            owner_user_id: "user-1".to_string(),
            name: "x".repeat(NOTIFICATION_MAX_NAME_BYTES + 1),
            url: "x".repeat(NOTIFICATION_MAX_URL_BYTES + 1),
            request_method: "TRACE".to_string(),
            request_type: "xml".to_string(),
            headers_json: Some("{not-json".to_string()),
            body_template: r#"{"token":"secret"}"#.to_string(),
            verify_tls: true,
            format_metric_units: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let view = record.to_view().unwrap();

        assert_eq!(view.name, "Invalid notification");
        assert_eq!(view.url, REDACTED_NOTIFICATION_SECRET);
        assert_eq!(view.request_method, "POST");
        assert_eq!(view.request_type, "json");
        assert!(view.headers_json.is_none());
        assert!(view.headers.is_empty());
        assert_eq!(view.body_template, REDACTED_NOTIFICATION_SECRET);
    }

    #[tokio::test]
    async fn list_notifications_skips_dirty_headers_in_api_view() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        seed_user(&db, owner, "owner").await;
        seed_notification_with_headers(
            &db,
            "00000000-0000-0000-0000-000000000101",
            owner,
            "dirty",
            Some("{not-json"),
        )
        .await;
        seed_notification_with_headers(
            &db,
            "00000000-0000-0000-0000-000000000102",
            owner,
            "clean",
            Some(r#"{"Authorization":"secret"}"#),
        )
        .await;

        let (records, total) = list_notifications_for_owner(&db, owner, 10, 0)
            .await
            .unwrap();
        let views = records
            .into_iter()
            .map(|record| record.to_view())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(total, 2);
        assert_eq!(views.len(), 2);
        let dirty = views.iter().find(|view| view.name == "dirty").unwrap();
        assert!(dirty.headers_json.is_none());
        assert!(dirty.headers.is_empty());
        let clean = views.iter().find(|view| view.name == "clean").unwrap();
        assert_eq!(
            clean.headers.get("Authorization").map(String::as_str),
            Some(REDACTED_NOTIFICATION_SECRET)
        );
    }

    #[test]
    fn notification_url_redaction_preserves_origin_only() {
        assert_eq!(
            redact_notification_url("https://example.com/webhook/token?secret=value"),
            format!("https://example.com/{REDACTED_NOTIFICATION_SECRET}")
        );
        assert_eq!(
            redact_notification_url("https://example.com/"),
            "https://example.com/"
        );
    }

    #[test]
    fn notification_resource_ids_require_canonical_uuid_text() {
        let canonical = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        assert_eq!(
            require_uuid_text(canonical.to_string(), "notification_id").unwrap(),
            canonical
        );
        assert_eq!(
            require_uuid_text(canonical.to_string(), "group_id").unwrap(),
            canonical
        );

        for value in [
            "",
            "channel-a",
            " aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa ",
            "aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaaaaaa",
            "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
            "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaax",
        ] {
            assert!(
                require_uuid_text(value.to_string(), "notification_id").is_err(),
                "{value:?} should be rejected"
            );
        }
    }

    #[tokio::test]
    async fn notification_input_rejects_oversized_headers_and_body_template() {
        let mut req = CreateNotificationRequest {
            name: "webhook".into(),
            url: "https://example.com/hook".into(),
            request_method: Some("POST".into()),
            request_type: Some("json".into()),
            headers: Some(
                (0..=crate::notifications::sender::NOTIFICATION_MAX_HEADERS)
                    .map(|idx| (format!("X-Test-{idx}"), "value".to_string()))
                    .collect(),
            ),
            headers_json: None,
            body_template: None,
            verify_tls: Some(true),
            format_metric_units: Some(true),
        };
        let err = build_create_notification_input(req).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        req = CreateNotificationRequest {
            name: "webhook".into(),
            url: "https://example.com/hook".into(),
            request_method: Some("POST".into()),
            request_type: Some("json".into()),
            headers: None,
            headers_json: None,
            body_template: Some("x".repeat(NOTIFICATION_MAX_BODY_TEMPLATE_BYTES + 1)),
            verify_tls: Some(true),
            format_metric_units: Some(true),
        };
        let err = build_create_notification_input(req).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn notification_group_member_count_is_bounded() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let group_id = "00000000-0000-0000-0000-000000000101";
        seed_user(&db, owner, "owner").await;
        seed_notification_group(&db, group_id, owner, "group").await;

        for idx in 0..NOTIFICATION_MAX_GROUP_CHANNELS {
            let notification_id = format!("00000000-0000-0000-0000-{:012}", idx + 1);
            seed_notification(&db, &notification_id, owner, &format!("n-{idx}")).await;
            seed_notification_group_member(&db, group_id, &notification_id).await;
        }

        let err = ensure_group_member_count_allowed(&db, group_id, owner)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));

        assert!(notification_group_member_exists(
            &db,
            group_id,
            "00000000-0000-0000-0000-000000000001",
            owner,
        )
        .await
        .unwrap());
        assert!(!notification_group_member_exists(
            &db,
            group_id,
            "00000000-0000-0000-0000-000000000999",
            owner,
        )
        .await
        .unwrap());
    }

    #[tokio::test]
    async fn notification_group_member_delete_requires_member_owner() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let other = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let group_id = "00000000-0000-0000-0000-000000000101";
        let other_notification_id = "00000000-0000-0000-0000-000000000202";

        seed_user(&db, owner, "owner").await;
        seed_user(&db, other, "other").await;
        seed_notification_group(&db, group_id, owner, "group").await;
        seed_notification(&db, other_notification_id, other, "other-channel").await;
        seed_notification_group_member(&db, group_id, other_notification_id).await;

        let err =
            delete_notification_group_member_for_owner(&db, group_id, other_notification_id, owner)
                .await
                .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));

        let raw_member_exists: (i64,) = match &db {
            DatabaseBackend::Sqlite(pool) => sqlx::query_as(
                "SELECT COUNT(*) FROM notification_group_members WHERE group_id = ? AND notification_id = ?",
            )
            .bind(group_id)
            .bind(other_notification_id)
            .fetch_one(pool)
            .await
            .unwrap(),
            DatabaseBackend::Postgres(_) => unreachable!(),
        };
        assert_eq!(raw_member_exists.0, 1);
    }

    #[test]
    fn notification_test_rate_limit_is_per_user_and_notification() {
        assert_eq!(NOTIFICATION_TEST_COOLDOWN_SECS, 30);
        let state = std::sync::Mutex::new(HashMap::new());
        let now = Instant::now();
        let key = "user-1:notification-1".to_string();

        assert!(check_notification_test_rate_limit_in(&state, key.clone(), now).is_ok());
        assert!(matches!(
            check_notification_test_rate_limit_in(
                &state,
                key.clone(),
                now + StdDuration::from_secs(1)
            ),
            Err(AppError::TooManyRequests(_))
        ));
        assert!(check_notification_test_rate_limit_in(
            &state,
            "user-1:notification-2".to_string(),
            now + StdDuration::from_secs(1)
        )
        .is_ok());
        assert!(check_notification_test_rate_limit_in(
            &state,
            key,
            now + notification_test_cooldown()
        )
        .is_ok());
    }

    #[tokio::test]
    async fn notification_sensitive_scope_rejects_pat_session() {
        let db = test_db().await;
        let auth = auth_session(
            Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
            AuthKind::PersonalAccessToken,
            vec!["notification:write".into()],
        );

        let err = require_notification_sensitive_scope(
            &db,
            &auth,
            "notification:write",
            &HeaderMap::new(),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn notification_sensitive_scope_requires_totp_when_enabled() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        seed_user(&db, owner, "owner").await;
        seed_totp_enabled_user(&db, owner).await;
        let auth = auth_session(owner, AuthKind::Session, vec!["notification:write".into()]);

        let err = require_notification_sensitive_scope(
            &db,
            &auth,
            "notification:write",
            &HeaderMap::new(),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn notification_sensitive_scope_allows_cookie_session_without_totp() {
        let db = test_db().await;
        let owner = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        seed_user(&db, owner, "owner").await;
        let auth = auth_session(owner, AuthKind::Session, vec!["notification:write".into()]);

        require_notification_sensitive_scope(&db, &auth, "notification:write", &HeaderMap::new())
            .await
            .unwrap();
    }

    fn auth_session(user_id: Uuid, auth_kind: AuthKind, scopes: Vec<String>) -> AuthSession {
        AuthSession {
            session_id: "session".into(),
            user_id: UserId(user_id),
            username: "owner".into(),
            role: UserRole::Admin,
            csrf_token: "csrf".into(),
            auth_kind,
            scopes,
            server_ids: None,
            pat_id: None,
        }
    }

    async fn test_db() -> DatabaseBackend {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_user(db: &DatabaseBackend, id: Uuid, username: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, ?, 'hash', 'member', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.to_string())
        .bind(username)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_totp_enabled_user(db: &DatabaseBackend, id: Uuid) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query("UPDATE users SET totp_secret = ?, totp_enabled = 1 WHERE id = ?")
            .bind("totp-secret")
            .bind(id.to_string())
            .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_notification_group(db: &DatabaseBackend, id: &str, owner: Uuid, name: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO notification_groups (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner.to_string())
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_notification(db: &DatabaseBackend, id: &str, owner: Uuid, name: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO notifications (id, owner_user_id, name, url, request_method, request_type, verify_tls, format_metric_units, created_at, updated_at) VALUES (?, ?, ?, 'https://example.com/hook', 'POST', 'json', 1, 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner.to_string())
        .bind(name)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_notification_with_headers(
        db: &DatabaseBackend,
        id: &str,
        owner: Uuid,
        name: &str,
        headers_json: Option<&str>,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO notifications (id, owner_user_id, name, url, request_method, request_type, headers_json, verify_tls, format_metric_units, created_at, updated_at) VALUES (?, ?, ?, 'https://example.com/hook', 'POST', 'json', ?, 1, 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(owner.to_string())
        .bind(name)
        .bind(headers_json)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_notification_group_member(
        db: &DatabaseBackend,
        group_id: &str,
        notification_id: &str,
    ) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO notification_group_members (group_id, notification_id) VALUES (?, ?)",
        )
        .bind(group_id)
        .bind(notification_id)
        .execute(pool)
        .await
        .unwrap();
    }
}
