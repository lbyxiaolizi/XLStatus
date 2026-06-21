//! System settings API.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::api::v1::notifications::ensure_notification_group_owned_by;
use crate::auth::middleware::{AuthKind, AuthSession};
use crate::db::{AgentRepository, DatabaseBackend};
use crate::secrets::{decrypt_secret_if_needed, encrypt_secret, is_encrypted_secret};
use axum::{extract::State, Json};
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
        set_secret_string_setting(&state.db, GEOIP_IPINFO_TOKEN, token.trim()).await?;
    }
    if let Some(enabled) = req.geoip_ip_change_enabled {
        set_bool_setting(&state.db, GEOIP_IP_CHANGE_ENABLED, enabled).await?;
    }
    if let Some(group_id) = req.geoip_ip_change_notification_group_id {
        let group_id = normalize_optional_id(group_id);
        ensure_notification_group_owned_by(&state.db, auth.user_id.0, group_id.as_deref()).await?;
        set_string_setting(
            &state.db,
            GEOIP_IP_CHANGE_NOTIFICATION_GROUP_ID,
            group_id.as_deref().unwrap_or(""),
        )
        .await?;
    }
    if let Some(server_ids) = req.geoip_ip_change_server_ids {
        let server_ids = normalize_string_list(server_ids);
        ensure_geoip_ip_change_servers_exist(&state.db, &server_ids).await?;
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
        logo_url: get_string_setting(db, PUBLIC_LOGO_URL).await?,
        favicon_url: get_string_setting(db, PUBLIC_FAVICON_URL).await?,
        theme_color: get_string_setting(db, PUBLIC_THEME_COLOR).await?,
        background_url: get_string_setting(db, PUBLIC_BACKGROUND_URL).await?,
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
    set_secret_string_setting(db, CLOUDFLARED_TOKEN, token.unwrap_or_default().trim()).await
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

fn default_public_site_enabled(value: Option<bool>) -> bool {
    value.unwrap_or(false)
}

fn normalize_disabled_custom_html(value: Option<String>, field: &str) -> Result<String, AppError> {
    if value.unwrap_or_default().trim().is_empty() {
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

fn normalize_optional_id(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

async fn ensure_geoip_ip_change_servers_exist(
    db: &DatabaseBackend,
    server_ids: &[String],
) -> Result<(), AppError> {
    let repo = AgentRepository::new(db.clone());
    for server_id in server_ids {
        let parsed = uuid::Uuid::parse_str(server_id)
            .map_err(|_| AppError::BadRequest(format!("invalid server id: {server_id}")))?;
        if repo.find_by_id(AgentId(parsed)).await?.is_none() {
            return Err(AppError::BadRequest(format!(
                "geoip_ip_change_server_ids contains unknown server: {server_id}"
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
        default_public_site_enabled, normalize_disabled_custom_html, require_admin_cookie_session,
    };
    use crate::auth::middleware::{AuthKind, AuthSession};
    use xlstatus_shared::{UserId, UserRole};

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
}
