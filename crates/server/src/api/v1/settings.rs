//! System settings API.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::notifications::ensure_notification_group_owned_by;
use crate::auth::middleware::{AuthKind, AuthSession};
use crate::db::{AgentRepository, DatabaseBackend};
use crate::secrets::{decrypt_secret_if_needed, encrypt_secret, is_encrypted_secret};
use axum::{
    extract::{DefaultBodyLimit, State},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use xlstatus_shared::AgentId;

const PUBLIC_SITE_ENABLED: &str = "public_site_enabled";
const PUBLIC_SITE_NAME: &str = "public_site_name";
const PUBLIC_LOGO_URL: &str = "public_logo_url";
const PUBLIC_FAVICON_URL: &str = "public_favicon_url";
const PUBLIC_THEME_COLOR: &str = "public_theme_color";
const PUBLIC_BACKGROUND_URL: &str = "public_background_url";
const PUBLIC_CUSTOM_HEAD: &str = "public_custom_head";
const PUBLIC_CUSTOM_BODY: &str = "public_custom_body";
const PUBLIC_SERVER_DETAILS_ENABLED: &str = "public_server_details_enabled";
const GEOIP_PROVIDER: &str = "geoip_provider";
const GEOIP_IPINFO_TOKEN: &str = "geoip_ipinfo_token";
const GEOIP_IP_CHANGE_ENABLED: &str = "geoip_ip_change_enabled";
const GEOIP_IP_CHANGE_NOTIFICATION_GROUP_ID: &str = "geoip_ip_change_notification_group_id";
const GEOIP_IP_CHANGE_SERVER_IDS: &str = "geoip_ip_change_server_ids";
const GEOIP_IP_CHANGE_SEVERITY: &str = "geoip_ip_change_severity";
const DDNS_RESOLVER_URL: &str = "ddns_resolver_url";
const TSDB_RETENTION_DAYS: &str = "tsdb_retention_days";
const CLOUDFLARED_TOKEN: &str = "cloudflared_token";
const SETTINGS_MAX_BODY_BYTES: usize = 64 * 1024;
const SETTINGS_MAX_URL_BYTES: usize = 2048;
const SETTINGS_MAX_GEOIP_TOKEN_BYTES: usize = 4096;
const SETTINGS_MAX_CLOUDFLARED_TOKEN_BYTES: usize = 8192;
const SETTINGS_MAX_DISABLED_CUSTOM_HTML_BYTES: usize = 1024;
const SETTINGS_MAX_GEOIP_IP_CHANGE_SERVERS: usize = 64;

#[derive(Debug, Clone, Serialize)]
pub struct PublicSiteBranding {
    pub site_name: String,
    pub logo_url: Option<String>,
    pub favicon_url: Option<String>,
    pub theme_color: Option<String>,
    pub background_url: Option<String>,
    pub custom_head: Option<String>,
    pub custom_body: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SystemSettingsResponse {
    pub public_site_enabled: bool,
    pub public_site_name: String,
    pub public_logo_url: Option<String>,
    pub public_favicon_url: Option<String>,
    pub public_theme_color: Option<String>,
    pub public_background_url: Option<String>,
    pub public_custom_head: Option<String>,
    pub public_custom_body: Option<String>,
    pub public_server_details_enabled: bool,
    pub geoip_provider: String,
    pub geoip_ipinfo_token_configured: bool,
    pub geoip_ip_change_enabled: bool,
    pub geoip_ip_change_notification_group_id: Option<String>,
    pub geoip_ip_change_server_ids: Vec<String>,
    pub geoip_ip_change_severity: String,
    pub ddns_resolver_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateSystemSettingsRequest {
    pub public_site_enabled: Option<bool>,
    pub public_site_name: Option<String>,
    pub public_logo_url: Option<Option<String>>,
    pub public_favicon_url: Option<Option<String>>,
    pub public_theme_color: Option<Option<String>>,
    pub public_background_url: Option<Option<String>>,
    pub public_custom_head: Option<Option<String>>,
    pub public_custom_body: Option<Option<String>>,
    pub public_server_details_enabled: Option<bool>,
    pub geoip_provider: Option<String>,
    pub geoip_ipinfo_token: Option<String>,
    pub geoip_ip_change_enabled: Option<bool>,
    pub geoip_ip_change_notification_group_id: Option<Option<String>>,
    pub geoip_ip_change_server_ids: Option<Vec<String>>,
    pub geoip_ip_change_severity: Option<String>,
    pub ddns_resolver_url: Option<String>,
}

pub fn settings_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(SETTINGS_MAX_BODY_BYTES)
}

pub async fn get_settings(
    State(state): State<AppState>,
    auth: AuthSession,
) -> Result<Json<ApiResponse<SystemSettingsResponse>>, AppError> {
    require_admin_cookie_session(&auth)?;
    Ok(Json(ApiResponse::success(
        system_settings_response(&state.db).await?,
    )))
}

pub async fn update_settings(
    State(state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<UpdateSystemSettingsRequest>,
) -> Result<Json<ApiResponse<SystemSettingsResponse>>, AppError> {
    require_admin_cookie_session(&auth)?;
    if let Some(enabled) = req.public_site_enabled {
        set_bool_setting(&state.db, PUBLIC_SITE_ENABLED, enabled).await?;
    }
    if let Some(site_name) = req.public_site_name {
        set_string_setting(
            &state.db,
            PUBLIC_SITE_NAME,
            &normalize_short_text(&site_name, 80, "public_site_name")?,
        )
        .await?;
    }
    if let Some(logo_url) = req.public_logo_url {
        set_string_setting(
            &state.db,
            PUBLIC_LOGO_URL,
            &normalize_optional_public_asset_url(logo_url, "public_logo_url")?,
        )
        .await?;
    }
    if let Some(favicon_url) = req.public_favicon_url {
        set_string_setting(
            &state.db,
            PUBLIC_FAVICON_URL,
            &normalize_optional_public_asset_url(favicon_url, "public_favicon_url")?,
        )
        .await?;
    }
    if let Some(theme_color) = req.public_theme_color {
        set_string_setting(
            &state.db,
            PUBLIC_THEME_COLOR,
            &normalize_optional_theme_color(theme_color)?,
        )
        .await?;
    }
    if let Some(background_url) = req.public_background_url {
        set_string_setting(
            &state.db,
            PUBLIC_BACKGROUND_URL,
            &normalize_optional_public_background_url(background_url)?,
        )
        .await?;
    }
    if let Some(custom_head) = req.public_custom_head {
        set_string_setting(
            &state.db,
            PUBLIC_CUSTOM_HEAD,
            &normalize_disabled_custom_html(custom_head, "public_custom_head")?,
        )
        .await?;
    }
    if let Some(custom_body) = req.public_custom_body {
        set_string_setting(
            &state.db,
            PUBLIC_CUSTOM_BODY,
            &normalize_disabled_custom_html(custom_body, "public_custom_body")?,
        )
        .await?;
    }
    if let Some(enabled) = req.public_server_details_enabled {
        set_bool_setting(&state.db, PUBLIC_SERVER_DETAILS_ENABLED, enabled).await?;
    }
    if let Some(provider) = req.geoip_provider {
        set_string_setting(
            &state.db,
            GEOIP_PROVIDER,
            &normalize_geoip_provider(&provider)?,
        )
        .await?;
    }
    if let Some(token) = req.geoip_ipinfo_token {
        let token = normalize_optional_secret_text(
            Some(token),
            SETTINGS_MAX_GEOIP_TOKEN_BYTES,
            "geoip_ipinfo_token",
        )?;
        set_secret_string_setting(&state.db, GEOIP_IPINFO_TOKEN, &token).await?;
    }
    if let Some(enabled) = req.geoip_ip_change_enabled {
        set_bool_setting(&state.db, GEOIP_IP_CHANGE_ENABLED, enabled).await?;
    }
    if let Some(group_id) = req.geoip_ip_change_notification_group_id {
        let group_id =
            normalize_optional_uuid_text(group_id, "geoip_ip_change_notification_group_id")?;
        ensure_notification_group_owned_by(&state.db, auth.user_id.0, group_id.as_deref()).await?;
        set_string_setting(
            &state.db,
            GEOIP_IP_CHANGE_NOTIFICATION_GROUP_ID,
            group_id.as_deref().unwrap_or(""),
        )
        .await?;
    }
    if let Some(server_ids) = req.geoip_ip_change_server_ids {
        let server_ids = normalize_geoip_ip_change_server_ids(server_ids)?;
        ensure_geoip_ip_change_servers_active(&state.db, &server_ids).await?;
        set_string_list_setting(&state.db, GEOIP_IP_CHANGE_SERVER_IDS, server_ids).await?;
    }
    if let Some(severity) = req.geoip_ip_change_severity {
        set_string_setting(
            &state.db,
            GEOIP_IP_CHANGE_SEVERITY,
            &normalize_notification_severity(&severity)?,
        )
        .await?;
    }
    if let Some(resolver_url) = req.ddns_resolver_url {
        let resolver_url = normalize_optional_url_setting(
            Some(resolver_url),
            SETTINGS_MAX_URL_BYTES,
            "ddns_resolver_url",
        )?;
        set_string_setting(&state.db, DDNS_RESOLVER_URL, &resolver_url).await?;
    }
    Ok(Json(ApiResponse::success(
        system_settings_response(&state.db).await?,
    )))
}

async fn system_settings_response(
    db: &DatabaseBackend,
) -> Result<SystemSettingsResponse, AppError> {
    let branding = public_site_branding(db).await?;
    Ok(SystemSettingsResponse {
        public_site_enabled: public_site_enabled(db).await?,
        public_site_name: branding.site_name,
        public_logo_url: branding.logo_url,
        public_favicon_url: branding.favicon_url,
        public_theme_color: branding.theme_color,
        public_background_url: branding.background_url,
        public_custom_head: get_string_setting(db, PUBLIC_CUSTOM_HEAD).await?,
        public_custom_body: get_string_setting(db, PUBLIC_CUSTOM_BODY).await?,
        public_server_details_enabled: public_server_details_enabled(db).await?,
        geoip_provider: geoip_provider(db).await?,
        geoip_ipinfo_token_configured: geoip_ipinfo_token(db).await?.is_some(),
        geoip_ip_change_enabled: geoip_ip_change_enabled(db).await?,
        geoip_ip_change_notification_group_id: geoip_ip_change_notification_group_id(db).await?,
        geoip_ip_change_server_ids: geoip_ip_change_server_ids(db).await?,
        geoip_ip_change_severity: geoip_ip_change_severity(db).await?,
        ddns_resolver_url: ddns_resolver_url(db).await?,
    })
}

pub async fn public_site_enabled(db: &DatabaseBackend) -> Result<bool, AppError> {
    Ok(default_public_site_enabled(
        get_bool_setting(db, PUBLIC_SITE_ENABLED).await?,
    ))
}

pub async fn public_server_details_enabled(db: &DatabaseBackend) -> Result<bool, AppError> {
    Ok(get_bool_setting(db, PUBLIC_SERVER_DETAILS_ENABLED)
        .await?
        .unwrap_or(false))
}

pub async fn public_site_branding(db: &DatabaseBackend) -> Result<PublicSiteBranding, AppError> {
    Ok(PublicSiteBranding {
        site_name: get_string_setting(db, PUBLIC_SITE_NAME)
            .await?
            .unwrap_or_else(|| "XLStatus".to_string()),
        logo_url: normalize_optional_public_asset_url(
            get_string_setting(db, PUBLIC_LOGO_URL).await?,
            PUBLIC_LOGO_URL,
        )
        .ok()
        .filter(|value| !value.is_empty()),
        favicon_url: normalize_optional_public_asset_url(
            get_string_setting(db, PUBLIC_FAVICON_URL).await?,
            PUBLIC_FAVICON_URL,
        )
        .ok()
        .filter(|value| !value.is_empty()),
        theme_color: get_string_setting(db, PUBLIC_THEME_COLOR).await?,
        background_url: normalize_optional_public_background_url(
            get_string_setting(db, PUBLIC_BACKGROUND_URL).await?,
        )
        .ok()
        .filter(|value| !value.is_empty()),
        custom_head: None,
        custom_body: None,
    })
}

pub async fn geoip_provider(db: &DatabaseBackend) -> Result<String, AppError> {
    Ok(get_string_setting(db, GEOIP_PROVIDER)
        .await?
        .as_deref()
        .map(normalize_geoip_provider)
        .transpose()?
        .unwrap_or_else(|| "mmdb".to_string()))
}

pub async fn geoip_ipinfo_token(db: &DatabaseBackend) -> Result<Option<String>, AppError> {
    Ok(get_secret_string_setting(db, GEOIP_IPINFO_TOKEN).await?)
}

pub async fn geoip_ip_change_enabled(db: &DatabaseBackend) -> Result<bool, AppError> {
    Ok(get_bool_setting(db, GEOIP_IP_CHANGE_ENABLED)
        .await?
        .unwrap_or(true))
}

pub async fn geoip_ip_change_notification_group_id(
    db: &DatabaseBackend,
) -> Result<Option<String>, AppError> {
    Ok(get_string_setting(db, GEOIP_IP_CHANGE_NOTIFICATION_GROUP_ID).await?)
}

pub async fn geoip_ip_change_server_ids(db: &DatabaseBackend) -> Result<Vec<String>, AppError> {
    Ok(get_string_list_setting(db, GEOIP_IP_CHANGE_SERVER_IDS).await?)
}

pub async fn geoip_ip_change_severity(db: &DatabaseBackend) -> Result<String, AppError> {
    Ok(get_string_setting(db, GEOIP_IP_CHANGE_SEVERITY)
        .await?
        .as_deref()
        .map(normalize_notification_severity)
        .transpose()?
        .unwrap_or_else(|| "info".to_string()))
}

pub async fn ddns_resolver_url(db: &DatabaseBackend) -> Result<Option<String>, AppError> {
    Ok(get_string_setting(db, DDNS_RESOLVER_URL).await?)
}

pub async fn tsdb_retention_days(db: &DatabaseBackend) -> Result<i64, AppError> {
    Ok(get_i64_setting(db, TSDB_RETENTION_DAYS)
        .await?
        .unwrap_or(30)
        .clamp(1, 3650))
}

pub async fn set_tsdb_retention_days(db: &DatabaseBackend, days: i64) -> Result<(), AppError> {
    if !(1..=3650).contains(&days) {
        return Err(AppError::BadRequest(
            "tsdb_retention_days must be between 1 and 3650".into(),
        ));
    }
    set_i64_setting(db, TSDB_RETENTION_DAYS, days).await
}

pub async fn cloudflared_token(db: &DatabaseBackend) -> Result<Option<String>, AppError> {
    get_secret_string_setting(db, CLOUDFLARED_TOKEN).await
}

pub async fn cloudflared_token_configured(db: &DatabaseBackend) -> Result<bool, AppError> {
    Ok(cloudflared_token(db).await?.is_some())
}

pub async fn set_cloudflared_token(
    db: &DatabaseBackend,
    token: Option<String>,
) -> Result<(), AppError> {
    let token = normalize_optional_secret_text(
        token,
        SETTINGS_MAX_CLOUDFLARED_TOKEN_BYTES,
        "cloudflared_token",
    )?;
    set_secret_string_setting(db, CLOUDFLARED_TOKEN, &token).await
}

async fn get_bool_setting(db: &DatabaseBackend, key: &str) -> Result<Option<bool>, AppError> {
    let raw = match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value_json FROM system_settings WHERE key = ?")
                    .bind(key)
                    .fetch_optional(pool)
                    .await?;
            row.map(|(value,)| value)
        }
        DatabaseBackend::Postgres(pool) => {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value_json FROM system_settings WHERE key = $1")
                    .bind(key)
                    .fetch_optional(pool)
                    .await?;
            row.map(|(value,)| value)
        }
    };
    raw.map(|value| serde_json::from_str::<bool>(&value))
        .transpose()
        .map_err(|e| AppError::BadRequest(format!("invalid setting value for {key}: {e}")))
}

async fn get_i64_setting(db: &DatabaseBackend, key: &str) -> Result<Option<i64>, AppError> {
    let raw = match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value_json FROM system_settings WHERE key = ?")
                    .bind(key)
                    .fetch_optional(pool)
                    .await?;
            row.map(|(value,)| value)
        }
        DatabaseBackend::Postgres(pool) => {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value_json FROM system_settings WHERE key = $1")
                    .bind(key)
                    .fetch_optional(pool)
                    .await?;
            row.map(|(value,)| value)
        }
    };
    raw.map(|value| serde_json::from_str::<i64>(&value))
        .transpose()
        .map_err(|e| AppError::BadRequest(format!("invalid setting value for {key}: {e}")))
}

async fn get_string_setting(db: &DatabaseBackend, key: &str) -> Result<Option<String>, AppError> {
    let raw = match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value_json FROM system_settings WHERE key = ?")
                    .bind(key)
                    .fetch_optional(pool)
                    .await?;
            row.map(|(value,)| value)
        }
        DatabaseBackend::Postgres(pool) => {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value_json FROM system_settings WHERE key = $1")
                    .bind(key)
                    .fetch_optional(pool)
                    .await?;
            row.map(|(value,)| value)
        }
    };
    raw.map(|value| serde_json::from_str::<String>(&value))
        .transpose()
        .map(|value| {
            value
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
        })
        .map_err(|e| AppError::BadRequest(format!("invalid setting value for {key}: {e}")))
}

async fn get_string_list_setting(db: &DatabaseBackend, key: &str) -> Result<Vec<String>, AppError> {
    let raw = match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value_json FROM system_settings WHERE key = ?")
                    .bind(key)
                    .fetch_optional(pool)
                    .await?;
            row.map(|(value,)| value)
        }
        DatabaseBackend::Postgres(pool) => {
            let row: Option<(String,)> =
                sqlx::query_as("SELECT value_json FROM system_settings WHERE key = $1")
                    .bind(key)
                    .fetch_optional(pool)
                    .await?;
            row.map(|(value,)| value)
        }
    };
    raw.map(|value| serde_json::from_str::<Vec<String>>(&value))
        .transpose()
        .map(|value| normalize_string_list(value.unwrap_or_default()))
        .map_err(|e| AppError::BadRequest(format!("invalid setting value for {key}: {e}")))
}

async fn set_bool_setting(db: &DatabaseBackend, key: &str, value: bool) -> Result<(), AppError> {
    let value_json =
        serde_json::to_string(&value).map_err(|e| AppError::BadRequest(e.to_string()))?;
    set_raw_setting(db, key, &value_json).await
}

async fn set_i64_setting(db: &DatabaseBackend, key: &str, value: i64) -> Result<(), AppError> {
    let value_json =
        serde_json::to_string(&value).map_err(|e| AppError::BadRequest(e.to_string()))?;
    set_raw_setting(db, key, &value_json).await
}

async fn set_string_list_setting(
    db: &DatabaseBackend,
    key: &str,
    value: Vec<String>,
) -> Result<(), AppError> {
    let value_json =
        serde_json::to_string(&value).map_err(|e| AppError::BadRequest(e.to_string()))?;
    set_raw_setting(db, key, &value_json).await
}

async fn set_string_setting(db: &DatabaseBackend, key: &str, value: &str) -> Result<(), AppError> {
    let value_json =
        serde_json::to_string(value).map_err(|e| AppError::BadRequest(e.to_string()))?;
    set_raw_setting(db, key, &value_json).await
}

async fn get_secret_string_setting(
    db: &DatabaseBackend,
    key: &str,
) -> Result<Option<String>, AppError> {
    get_string_setting(db, key)
        .await?
        .map(|value| decrypt_secret_if_needed(&value))
        .transpose()
        .map_err(AppError::from)
}

async fn set_secret_string_setting(
    db: &DatabaseBackend,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    let value = value.trim();
    let stored = if value.is_empty() || is_encrypted_secret(value) {
        value.to_string()
    } else {
        encrypt_secret(value)?
    };
    set_string_setting(db, key, &stored).await
}

async fn set_raw_setting(
    db: &DatabaseBackend,
    key: &str,
    value_json: &str,
) -> Result<(), AppError> {
    let now = Utc::now();
    match db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query(
                r#"
                INSERT INTO system_settings (key, value_json, updated_at)
                VALUES (?, ?, ?)
                ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at
                "#,
            )
            .bind(key)
            .bind(value_json)
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query(
                r#"
                INSERT INTO system_settings (key, value_json, updated_at)
                VALUES ($1, $2, $3)
                ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at
                "#,
            )
            .bind(key)
            .bind(value_json)
            .bind(now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

fn normalize_geoip_provider(value: &str) -> Result<String, AppError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "empty" => Ok("empty".into()),
        "geojs" => Ok("geojs".into()),
        "ip-api" | "ipapi" => Ok("ip-api".into()),
        "ipinfo" => Ok("ipinfo".into()),
        "mmdb" => Ok("mmdb".into()),
        _ => Err(AppError::BadRequest(
            "geoip_provider must be empty, geojs, ip-api, ipinfo, or mmdb".into(),
        )),
    }
}

fn normalize_notification_severity(value: &str) -> Result<String, AppError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "info" => Ok("info".into()),
        "warning" | "warn" => Ok("warning".into()),
        "error" => Ok("error".into()),
        "critical" => Ok("critical".into()),
        _ => Err(AppError::BadRequest(
            "geoip_ip_change_severity must be info, warning, error, or critical".into(),
        )),
    }
}

fn normalize_short_text(value: &str, max_len: usize, field: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} must not be empty")));
    }
    if value.len() > max_len {
        return Err(AppError::BadRequest(format!("{field} is too long")));
    }
    Ok(value.to_string())
}

fn normalize_optional_public_asset_url(
    value: Option<String>,
    field: &str,
) -> Result<String, AppError> {
    normalize_optional_url_setting(value, 500, field)
}

fn normalize_optional_public_background_url(value: Option<String>) -> Result<String, AppError> {
    let normalized = normalize_optional_public_asset_url(value, "public_background_url")?;
    if normalized.contains(['"', '\'', '(', ')', '\\'])
        || normalized.chars().any(|ch| ch.is_control())
    {
        return Err(AppError::BadRequest(
            "public_background_url contains characters unsafe for CSS url()".into(),
        ));
    }
    Ok(normalized)
}

fn default_public_site_enabled(value: Option<bool>) -> bool {
    value.unwrap_or(false)
}

fn normalize_disabled_custom_html(value: Option<String>, field: &str) -> Result<String, AppError> {
    let value = value.unwrap_or_default();
    if value.len() > SETTINGS_MAX_DISABLED_CUSTOM_HTML_BYTES {
        return Err(AppError::BadRequest(format!(
            "{field} must be at most {SETTINGS_MAX_DISABLED_CUSTOM_HTML_BYTES} bytes"
        )));
    }
    if value.trim().is_empty() {
        Ok(String::new())
    } else {
        Err(AppError::BadRequest(format!(
            "{field} is disabled because arbitrary public HTML is not supported"
        )))
    }
}

fn normalize_optional_theme_color(value: Option<String>) -> Result<String, AppError> {
    let value = value.unwrap_or_default();
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    let valid = value.len() == 7
        && value.starts_with('#')
        && value.chars().skip(1).all(|ch| ch.is_ascii_hexdigit());
    if !valid {
        return Err(AppError::BadRequest(
            "public_theme_color must use #RRGGBB format".into(),
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn normalize_optional_uuid_text(
    value: Option<String>,
    field: &str,
) -> Result<Option<String>, AppError> {
    let Some(value) = value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    uuid::Uuid::parse_str(&value)
        .map(|id| Some(id.to_string()))
        .map_err(|e| AppError::BadRequest(format!("invalid {field}: {e}")))
}

fn normalize_geoip_ip_change_server_ids(values: Vec<String>) -> Result<Vec<String>, AppError> {
    if values.len() > SETTINGS_MAX_GEOIP_IP_CHANGE_SERVERS {
        return Err(AppError::BadRequest(format!(
            "geoip_ip_change_server_ids must contain at most {SETTINGS_MAX_GEOIP_IP_CHANGE_SERVERS} items"
        )));
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        let parsed = uuid::Uuid::parse_str(value).map_err(|e| {
            AppError::BadRequest(format!("invalid geoip_ip_change_server_ids item: {e}"))
        })?;
        if seen.insert(parsed) {
            if out.len() >= SETTINGS_MAX_GEOIP_IP_CHANGE_SERVERS {
                return Err(AppError::BadRequest(format!(
                    "geoip_ip_change_server_ids must contain at most {SETTINGS_MAX_GEOIP_IP_CHANGE_SERVERS} unique servers"
                )));
            }
            out.push(parsed.to_string());
        }
    }
    Ok(out)
}

fn normalize_optional_secret_text(
    value: Option<String>,
    max_bytes: usize,
    field: &str,
) -> Result<String, AppError> {
    let value = value.unwrap_or_default();
    let value = value.trim();
    if value.len() > max_bytes {
        return Err(AppError::BadRequest(format!(
            "{field} must be at most {max_bytes} bytes"
        )));
    }
    Ok(value.to_string())
}

fn normalize_optional_url_setting(
    value: Option<String>,
    max_bytes: usize,
    field: &str,
) -> Result<String, AppError> {
    let value = value.unwrap_or_default();
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    if value.len() > max_bytes {
        return Err(AppError::BadRequest(format!(
            "{field} must be at most {max_bytes} bytes"
        )));
    }
    let parsed = reqwest::Url::parse(value)
        .map_err(|e| AppError::BadRequest(format!("invalid {field}: {e}")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::BadRequest(format!(
            "{field} must use http or https"
        )));
    }
    if parsed.host_str().is_none() {
        return Err(AppError::BadRequest(format!("{field} must include a host")));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::BadRequest(format!(
            "{field} must not include credentials"
        )));
    }
    if parsed.fragment().is_some() {
        return Err(AppError::BadRequest(format!(
            "{field} must not include a fragment"
        )));
    }
    Ok(parsed.to_string())
}

async fn ensure_geoip_ip_change_servers_active(
    db: &DatabaseBackend,
    server_ids: &[String],
) -> Result<(), AppError> {
    let repo = AgentRepository::new(db.clone());
    for server_id in server_ids {
        let parsed = uuid::Uuid::parse_str(server_id)
            .map_err(|_| AppError::BadRequest(format!("invalid server id: {server_id}")))?;
        let agent = repo.find_by_id(AgentId(parsed)).await?.ok_or_else(|| {
            AppError::BadRequest(format!(
                "geoip_ip_change_server_ids contains unknown server: {server_id}"
            ))
        })?;
        if agent.revoked_at.is_some() {
            return Err(AppError::BadRequest(format!(
                "geoip_ip_change_server_ids contains revoked server: {server_id}"
            )));
        }
    }
    Ok(())
}

fn require_admin(auth: &AuthSession) -> Result<(), AppError> {
    if auth.role.is_admin() {
        Ok(())
    } else {
        Err(AppError::Forbidden("Admin role required".into()))
    }
}

fn require_admin_cookie_session(auth: &AuthSession) -> Result<(), AppError> {
    require_admin(auth)?;
    if matches!(auth.auth_kind, AuthKind::PersonalAccessToken) {
        return Err(AppError::Forbidden(
            "System settings require an admin cookie session".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        default_public_site_enabled, ensure_geoip_ip_change_servers_active,
        normalize_disabled_custom_html, normalize_geoip_ip_change_server_ids,
        normalize_optional_public_asset_url, normalize_optional_public_background_url,
        normalize_optional_secret_text, normalize_optional_url_setting,
        normalize_optional_uuid_text, public_site_branding, require_admin_cookie_session,
        set_string_setting, settings_body_limit, PUBLIC_BACKGROUND_URL, PUBLIC_FAVICON_URL,
        PUBLIC_LOGO_URL, SETTINGS_MAX_BODY_BYTES, SETTINGS_MAX_CLOUDFLARED_TOKEN_BYTES,
        SETTINGS_MAX_DISABLED_CUSTOM_HTML_BYTES, SETTINGS_MAX_GEOIP_IP_CHANGE_SERVERS,
        SETTINGS_MAX_GEOIP_TOKEN_BYTES, SETTINGS_MAX_URL_BYTES,
    };
    use crate::auth::middleware::{AuthKind, AuthSession};
    use crate::db::DatabaseBackend;
    use xlstatus_shared::{UserId, UserRole};

    #[test]
    fn settings_body_budget_is_explicit() {
        let _ = settings_body_limit();
        assert_eq!(SETTINGS_MAX_BODY_BYTES, 64 * 1024);
        assert_eq!(SETTINGS_MAX_URL_BYTES, 2048);
        assert_eq!(SETTINGS_MAX_GEOIP_TOKEN_BYTES, 4096);
        assert_eq!(SETTINGS_MAX_CLOUDFLARED_TOKEN_BYTES, 8192);
        assert_eq!(SETTINGS_MAX_DISABLED_CUSTOM_HTML_BYTES, 1024);
        assert_eq!(SETTINGS_MAX_GEOIP_IP_CHANGE_SERVERS, 64);
    }

    #[test]
    fn parses_bool_json_setting() {
        assert_eq!(serde_json::from_str::<bool>("true").unwrap(), true);
        assert_eq!(serde_json::from_str::<bool>("false").unwrap(), false);
    }

    #[test]
    fn rejects_non_empty_public_custom_html() {
        assert!(
            normalize_disabled_custom_html(Some("<script>alert(1)</script>".into()), "field")
                .is_err()
        );
        assert_eq!(
            normalize_disabled_custom_html(Some("   ".into()), "field").unwrap(),
            ""
        );
        assert_eq!(normalize_disabled_custom_html(None, "field").unwrap(), "");
        assert!(normalize_disabled_custom_html(
            Some(" ".repeat(SETTINGS_MAX_DISABLED_CUSTOM_HTML_BYTES + 1)),
            "field",
        )
        .is_err());
    }

    #[test]
    fn bounds_secret_and_url_settings() {
        assert_eq!(
            normalize_optional_secret_text(Some(" token ".into()), 16, "token").unwrap(),
            "token"
        );
        assert!(normalize_optional_secret_text(Some("x".repeat(17)), 16, "token").is_err());

        assert_eq!(
            normalize_optional_url_setting(
                Some(" https://dns.google/resolve ".into()),
                SETTINGS_MAX_URL_BYTES,
                "ddns_resolver_url",
            )
            .unwrap(),
            "https://dns.google/resolve"
        );
        assert!(normalize_optional_url_setting(
            Some(format!(
                "https://example.com/{}",
                "a".repeat(SETTINGS_MAX_URL_BYTES)
            )),
            SETTINGS_MAX_URL_BYTES,
            "ddns_resolver_url",
        )
        .is_err());
        assert!(normalize_optional_url_setting(
            Some("ftp://example.com/resolve".into()),
            SETTINGS_MAX_URL_BYTES,
            "ddns_resolver_url",
        )
        .is_err());
        assert!(normalize_optional_url_setting(
            Some("https://user:pass@example.com/resolve".into()),
            SETTINGS_MAX_URL_BYTES,
            "ddns_resolver_url",
        )
        .is_err());
        assert!(normalize_optional_url_setting(
            Some("https://example.com/resolve#frag".into()),
            SETTINGS_MAX_URL_BYTES,
            "ddns_resolver_url",
        )
        .is_err());
    }

    #[test]
    fn public_branding_urls_reject_unsafe_schemes_and_css_tokens() {
        assert_eq!(
            normalize_optional_public_asset_url(
                Some(" https://cdn.example.com/logo.png ".into()),
                "public_logo_url",
            )
            .unwrap(),
            "https://cdn.example.com/logo.png"
        );
        assert!(normalize_optional_public_asset_url(
            Some("javascript:alert(1)".into()),
            "public_logo_url",
        )
        .is_err());
        assert!(normalize_optional_public_asset_url(
            Some("https://user:pass@example.com/logo.png".into()),
            "public_logo_url",
        )
        .is_err());
        assert!(normalize_optional_public_asset_url(
            Some("https://example.com/logo.svg#frag".into()),
            "public_logo_url",
        )
        .is_err());
        assert!(normalize_optional_public_background_url(Some(
            "https://example.com/bg.png\") , url(https://evil.example/pixel".into(),
        ))
        .is_err());
        assert!(normalize_optional_public_background_url(Some(
            "https://example.com/bg(1).png".into(),
        ))
        .is_err());
    }

    #[tokio::test]
    async fn public_branding_filters_historical_unsafe_urls() {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();

        set_string_setting(&db, PUBLIC_LOGO_URL, "javascript:alert(1)")
            .await
            .unwrap();
        set_string_setting(
            &db,
            PUBLIC_FAVICON_URL,
            "https://user:pass@example.com/icon.png",
        )
        .await
        .unwrap();
        set_string_setting(
            &db,
            PUBLIC_BACKGROUND_URL,
            "https://example.com/bg.png\") , url(https://evil.example/pixel",
        )
        .await
        .unwrap();

        let branding = public_site_branding(&db).await.unwrap();

        assert!(branding.logo_url.is_none());
        assert!(branding.favicon_url.is_none());
        assert!(branding.background_url.is_none());
    }

    #[tokio::test]
    async fn geoip_ip_change_server_list_rejects_revoked_servers() {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();

        let owner = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let active_server = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000101").unwrap();
        let revoked_server = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000202").unwrap();
        seed_user(&db, owner).await;
        seed_agent(&db, active_server, owner, "active-server").await;
        seed_agent(&db, revoked_server, owner, "revoked-server").await;
        revoke_agent(&db, revoked_server).await;

        ensure_geoip_ip_change_servers_active(&db, &[active_server.to_string()])
            .await
            .unwrap();

        let err = ensure_geoip_ip_change_servers_active(&db, &[revoked_server.to_string()])
            .await
            .unwrap_err();
        assert!(matches!(err, super::AppError::BadRequest(_)));
    }

    #[test]
    fn normalizes_uuid_settings_and_bounds_server_lists() {
        let id = uuid::Uuid::now_v7();
        assert_eq!(
            normalize_optional_uuid_text(Some(format!(" {id} ")), "group")
                .unwrap()
                .as_deref(),
            Some(id.to_string().as_str())
        );
        assert!(normalize_optional_uuid_text(Some("not-a-uuid".into()), "group").is_err());

        let duplicate = uuid::Uuid::now_v7().to_string();
        let ids = normalize_geoip_ip_change_server_ids(vec![
            duplicate.clone(),
            " ".into(),
            duplicate.clone(),
            uuid::Uuid::now_v7().to_string(),
        ])
        .unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], duplicate);

        let too_many = (0..=SETTINGS_MAX_GEOIP_IP_CHANGE_SERVERS)
            .map(|idx| uuid::Uuid::from_u128(idx as u128 + 1).to_string())
            .collect::<Vec<_>>();
        assert!(normalize_geoip_ip_change_server_ids(too_many).is_err());
        assert!(normalize_geoip_ip_change_server_ids(vec!["not-a-uuid".into()]).is_err());
    }

    #[test]
    fn public_site_is_private_by_default() {
        assert!(!default_public_site_enabled(None));
        assert!(default_public_site_enabled(Some(true)));
        assert!(!default_public_site_enabled(Some(false)));
    }

    #[test]
    fn system_settings_reject_admin_pat_session() {
        let auth = auth_session(AuthKind::PersonalAccessToken);

        assert!(matches!(
            require_admin_cookie_session(&auth),
            Err(super::AppError::Forbidden(_))
        ));
    }

    #[test]
    fn system_settings_allow_admin_cookie_session() {
        let auth = auth_session(AuthKind::Session);

        assert!(require_admin_cookie_session(&auth).is_ok());
    }

    fn auth_session(auth_kind: AuthKind) -> AuthSession {
        AuthSession {
            session_id: "sess".into(),
            user_id: UserId(uuid::Uuid::from_bytes([1; 16])),
            username: "admin".into(),
            role: UserRole::Admin,
            csrf_token: "csrf".into(),
            auth_kind,
            scopes: vec!["admin:*".into()],
            server_ids: None,
            pat_id: None,
        }
    }

    async fn seed_user(db: &DatabaseBackend, id: uuid::Uuid) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?, 'owner', 'x', 'admin', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_agent(db: &DatabaseBackend, id: uuid::Uuid, owner: uuid::Uuid, name: &str) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query(
            "INSERT INTO agents (id, name, public_key, owner_user_id, created_at, updated_at) VALUES (?, ?, 'pk', ?, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(id.to_string())
        .bind(name)
        .bind(owner.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn revoke_agent(db: &DatabaseBackend, id: uuid::Uuid) {
        let DatabaseBackend::Sqlite(pool) = db else {
            unreachable!();
        };
        sqlx::query("UPDATE agents SET revoked_at = '2026-06-22T00:00:00Z' WHERE id = ?")
            .bind(id.to_string())
            .execute(pool)
            .await
            .unwrap();
    }
}
