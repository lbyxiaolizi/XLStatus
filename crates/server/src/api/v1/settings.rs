//! System settings API.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::middleware::AuthSession;
use crate::db::DatabaseBackend;
use axum::{extract::State, Json};
use chrono::Utc;
use serde::{Deserialize, Serialize};

const PUBLIC_SITE_ENABLED: &str = "public_site_enabled";
const PUBLIC_SITE_NAME: &str = "public_site_name";
const PUBLIC_LOGO_URL: &str = "public_logo_url";
const PUBLIC_FAVICON_URL: &str = "public_favicon_url";
const PUBLIC_THEME_COLOR: &str = "public_theme_color";
const PUBLIC_BACKGROUND_URL: &str = "public_background_url";
const PUBLIC_CUSTOM_HEAD: &str = "public_custom_head";
const PUBLIC_CUSTOM_BODY: &str = "public_custom_body";
const GEOIP_PROVIDER: &str = "geoip_provider";
const GEOIP_IPINFO_TOKEN: &str = "geoip_ipinfo_token";
const GEOIP_IP_CHANGE_ENABLED: &str = "geoip_ip_change_enabled";
const GEOIP_IP_CHANGE_NOTIFICATION_GROUP_ID: &str = "geoip_ip_change_notification_group_id";
const GEOIP_IP_CHANGE_SERVER_IDS: &str = "geoip_ip_change_server_ids";
const GEOIP_IP_CHANGE_SEVERITY: &str = "geoip_ip_change_severity";
const DDNS_RESOLVER_URL: &str = "ddns_resolver_url";
const TSDB_RETENTION_DAYS: &str = "tsdb_retention_days";
const CLOUDFLARED_TOKEN: &str = "cloudflared_token";

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
    pub geoip_provider: Option<String>,
    pub geoip_ipinfo_token: Option<String>,
    pub geoip_ip_change_enabled: Option<bool>,
    pub geoip_ip_change_notification_group_id: Option<Option<String>>,
    pub geoip_ip_change_server_ids: Option<Vec<String>>,
    pub geoip_ip_change_severity: Option<String>,
    pub ddns_resolver_url: Option<String>,
}

pub async fn get_settings(
    State(state): State<AppState>,
    auth: AuthSession,
) -> Result<Json<ApiResponse<SystemSettingsResponse>>, AppError> {
    require_admin(&auth)?;
    Ok(Json(ApiResponse::success(
        system_settings_response(&state.db).await?,
    )))
}

pub async fn update_settings(
    State(state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<UpdateSystemSettingsRequest>,
) -> Result<Json<ApiResponse<SystemSettingsResponse>>, AppError> {
    require_admin(&auth)?;
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
            &normalize_optional_text_setting(logo_url, 500, "public_logo_url")?,
        )
        .await?;
    }
    if let Some(favicon_url) = req.public_favicon_url {
        set_string_setting(
            &state.db,
            PUBLIC_FAVICON_URL,
            &normalize_optional_text_setting(favicon_url, 500, "public_favicon_url")?,
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
            &normalize_optional_text_setting(background_url, 500, "public_background_url")?,
        )
        .await?;
    }
    if let Some(custom_head) = req.public_custom_head {
        set_string_setting(
            &state.db,
            PUBLIC_CUSTOM_HEAD,
            &normalize_optional_text_setting(custom_head, 10_000, "public_custom_head")?,
        )
        .await?;
    }
    if let Some(custom_body) = req.public_custom_body {
        set_string_setting(
            &state.db,
            PUBLIC_CUSTOM_BODY,
            &normalize_optional_text_setting(custom_body, 20_000, "public_custom_body")?,
        )
        .await?;
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
        set_string_setting(&state.db, GEOIP_IPINFO_TOKEN, token.trim()).await?;
    }
    if let Some(enabled) = req.geoip_ip_change_enabled {
        set_bool_setting(&state.db, GEOIP_IP_CHANGE_ENABLED, enabled).await?;
    }
    if let Some(group_id) = req.geoip_ip_change_notification_group_id {
        set_string_setting(
            &state.db,
            GEOIP_IP_CHANGE_NOTIFICATION_GROUP_ID,
            group_id.as_deref().unwrap_or("").trim(),
        )
        .await?;
    }
    if let Some(server_ids) = req.geoip_ip_change_server_ids {
        set_string_list_setting(
            &state.db,
            GEOIP_IP_CHANGE_SERVER_IDS,
            normalize_string_list(server_ids),
        )
        .await?;
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
        set_string_setting(&state.db, DDNS_RESOLVER_URL, resolver_url.trim()).await?;
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
        public_custom_head: branding.custom_head,
        public_custom_body: branding.custom_body,
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
    Ok(get_bool_setting(db, PUBLIC_SITE_ENABLED)
        .await?
        .unwrap_or(true))
}

pub async fn public_site_branding(db: &DatabaseBackend) -> Result<PublicSiteBranding, AppError> {
    Ok(PublicSiteBranding {
        site_name: get_string_setting(db, PUBLIC_SITE_NAME)
            .await?
            .unwrap_or_else(|| "XLStatus".to_string()),
        logo_url: get_string_setting(db, PUBLIC_LOGO_URL).await?,
        favicon_url: get_string_setting(db, PUBLIC_FAVICON_URL).await?,
        theme_color: get_string_setting(db, PUBLIC_THEME_COLOR).await?,
        background_url: get_string_setting(db, PUBLIC_BACKGROUND_URL).await?,
        custom_head: get_string_setting(db, PUBLIC_CUSTOM_HEAD).await?,
        custom_body: get_string_setting(db, PUBLIC_CUSTOM_BODY).await?,
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
    Ok(get_string_setting(db, GEOIP_IPINFO_TOKEN).await?)
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
    get_string_setting(db, CLOUDFLARED_TOKEN).await
}

pub async fn cloudflared_token_configured(db: &DatabaseBackend) -> Result<bool, AppError> {
    Ok(cloudflared_token(db).await?.is_some())
}

pub async fn set_cloudflared_token(
    db: &DatabaseBackend,
    token: Option<String>,
) -> Result<(), AppError> {
    set_string_setting(db, CLOUDFLARED_TOKEN, token.unwrap_or_default().trim()).await
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

fn normalize_optional_text_setting(
    value: Option<String>,
    max_len: usize,
    field: &str,
) -> Result<String, AppError> {
    let value = value.unwrap_or_default();
    let value = value.trim();
    if value.len() > max_len {
        return Err(AppError::BadRequest(format!("{field} is too long")));
    }
    Ok(value.to_string())
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

fn require_admin(auth: &AuthSession) -> Result<(), AppError> {
    if auth.role.is_admin() {
        Ok(())
    } else {
        Err(AppError::Forbidden("Admin role required".into()))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_bool_json_setting() {
        assert_eq!(serde_json::from_str::<bool>("true").unwrap(), true);
        assert_eq!(serde_json::from_str::<bool>("false").unwrap(), false);
    }
}
