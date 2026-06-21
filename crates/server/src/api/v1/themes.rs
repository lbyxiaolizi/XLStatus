//! Theme catalog and selection API.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::middleware::{AuthKind, AuthSession};
use crate::db::DatabaseBackend;
use axum::{
    extract::{DefaultBodyLimit, Path, State},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const CUSTOM_THEMES_KEY: &str = "theme_custom_catalog";
const SELECTED_PUBLIC_THEME_KEY: &str = "theme_selected_public";
const SELECTED_DASHBOARD_THEME_KEY: &str = "theme_selected_dashboard";
const THEME_API_MAX_BODY_BYTES: usize = 64 * 1024;
const THEME_MAX_CUSTOM_THEMES: usize = 32;
const THEME_MAX_CUSTOM_CATALOG_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub target: String,
    #[serde(default)]
    pub variables: HashMap<String, String>,
    #[serde(default)]
    pub light_variables: HashMap<String, String>,
    #[serde(default)]
    pub dark_variables: HashMap<String, String>,
    pub custom_css: Option<String>,
    pub light_custom_css: Option<String>,
    pub dark_custom_css: Option<String>,
    pub builtin: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ThemeListResponse {
    pub themes: Vec<ThemeDefinition>,
    pub selected_public_theme_id: Option<String>,
    pub selected_dashboard_theme_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ImportThemeRequest {
    pub theme: ThemeDefinitionInput,
}

#[derive(Debug, Deserialize)]
pub struct UpdateThemeRequest {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub target: Option<String>,
    pub variables: Option<HashMap<String, String>>,
    pub light_variables: Option<HashMap<String, String>>,
    pub dark_variables: Option<HashMap<String, String>>,
    pub custom_css: Option<Option<String>>,
    pub light_custom_css: Option<Option<String>>,
    pub dark_custom_css: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub struct SelectThemeRequest {
    pub target: String,
}

#[derive(Debug, Deserialize)]
pub struct ThemeDefinitionInput {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub target: Option<String>,
    #[serde(default)]
    pub variables: HashMap<String, String>,
    #[serde(default)]
    pub light_variables: HashMap<String, String>,
    #[serde(default)]
    pub dark_variables: HashMap<String, String>,
}

pub fn theme_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(THEME_API_MAX_BODY_BYTES)
}

pub async fn list_themes(
    State(state): State<AppState>,
    _auth: AuthSession,
) -> Result<Json<ApiResponse<ThemeListResponse>>, AppError> {
    Ok(Json(ApiResponse::success(
        theme_list_response(&state.db).await?,
    )))
}

pub async fn import_theme(
    State(state): State<AppState>,
    auth: AuthSession,
    Json(req): Json<ImportThemeRequest>,
) -> Result<Json<ApiResponse<ThemeDefinition>>, AppError> {
    require_admin_cookie_session(&auth)?;
    let mut theme = normalize_theme_input(req.theme, false)?;
    let now = Utc::now().to_rfc3339();
    theme.created_at = Some(now.clone());
    theme.updated_at = Some(now);
    upsert_custom_theme(&state.db, theme.clone()).await?;
    Ok(Json(ApiResponse::success(theme)))
}

pub async fn update_theme(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    Json(req): Json<UpdateThemeRequest>,
) -> Result<Json<ApiResponse<ThemeDefinition>>, AppError> {
    require_admin_cookie_session(&auth)?;
    if builtin_theme(&id).is_some() {
        return Err(AppError::BadRequest(
            "builtin themes cannot be edited; import a custom copy instead".into(),
        ));
    }
    let mut themes = custom_themes(&state.db).await?;
    let Some(theme) = themes.iter_mut().find(|theme| theme.id == id) else {
        return Err(AppError::NotFound("theme not found".into()));
    };
    if let Some(name) = req.name {
        theme.name = normalize_name(&name, "name")?;
    }
    if let Some(description) = req.description {
        theme.description = normalize_optional_text(description, 500, "description")?;
    }
    if let Some(target) = req.target {
        theme.target = normalize_target(&target)?;
    }
    if let Some(variables) = req.variables {
        theme.variables = normalize_theme_variables(variables)?;
    }
    if let Some(variables) = req.light_variables {
        theme.light_variables = normalize_theme_variables(variables)?;
    }
    if let Some(variables) = req.dark_variables {
        theme.dark_variables = normalize_theme_variables(variables)?;
    }
    if req.custom_css.is_some() {
        theme.custom_css = None;
    }
    if req.light_custom_css.is_some() {
        theme.light_custom_css = None;
    }
    if req.dark_custom_css.is_some() {
        theme.dark_custom_css = None;
    }
    theme.updated_at = Some(Utc::now().to_rfc3339());
    let updated = theme.clone();
    set_custom_themes(&state.db, &themes).await?;
    Ok(Json(ApiResponse::success(updated)))
}

pub async fn select_theme(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
    Json(req): Json<SelectThemeRequest>,
) -> Result<Json<ApiResponse<ThemeListResponse>>, AppError> {
    require_admin_cookie_session(&auth)?;
    let target = normalize_select_target(&req.target)?;
    let all = all_themes(&state.db).await?;
    let theme = all
        .iter()
        .find(|theme| theme.id == id)
        .ok_or(AppError::NotFound("theme not found".into()))?;
    if target == "both" && theme.target != "both" {
        return Err(AppError::BadRequest(format!(
            "theme {} does not support both public and dashboard",
            theme.id
        )));
    }
    if target != "both" && theme.target != "both" && theme.target != target {
        return Err(AppError::BadRequest(format!(
            "theme {} does not support {target}",
            theme.id
        )));
    }
    if target == "public" || target == "both" {
        set_string_setting(&state.db, SELECTED_PUBLIC_THEME_KEY, &id).await?;
    }
    if target == "dashboard" || target == "both" {
        set_string_setting(&state.db, SELECTED_DASHBOARD_THEME_KEY, &id).await?;
    }
    Ok(Json(ApiResponse::success(
        theme_list_response(&state.db).await?,
    )))
}

pub async fn delete_theme(
    State(state): State<AppState>,
    auth: AuthSession,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<serde_json::Value>>, AppError> {
    require_admin_cookie_session(&auth)?;
    if builtin_theme(&id).is_some() {
        return Err(AppError::BadRequest(
            "builtin themes cannot be deleted".into(),
        ));
    }
    let mut themes = custom_themes(&state.db).await?;
    let before = themes.len();
    themes.retain(|theme| theme.id != id);
    if themes.len() == before {
        return Err(AppError::NotFound("theme not found".into()));
    }
    set_custom_themes(&state.db, &themes).await?;
    if selected_public_theme_id(&state.db).await?.as_deref() == Some(&id) {
        set_string_setting(&state.db, SELECTED_PUBLIC_THEME_KEY, "").await?;
    }
    if selected_dashboard_theme_id(&state.db).await?.as_deref() == Some(&id) {
        set_string_setting(&state.db, SELECTED_DASHBOARD_THEME_KEY, "").await?;
    }
    Ok(Json(ApiResponse::success(
        serde_json::json!({ "id": id, "deleted": true }),
    )))
}

pub async fn selected_public_theme(
    db: &DatabaseBackend,
) -> Result<Option<ThemeDefinition>, AppError> {
    let Some(id) = selected_public_theme_id(db).await? else {
        return Ok(None);
    };
    Ok(all_themes(db)
        .await?
        .into_iter()
        .find(|theme| theme.id == id))
}

async fn theme_list_response(db: &DatabaseBackend) -> Result<ThemeListResponse, AppError> {
    Ok(ThemeListResponse {
        themes: all_themes(db).await?,
        selected_public_theme_id: selected_public_theme_id(db).await?,
        selected_dashboard_theme_id: selected_dashboard_theme_id(db).await?,
    })
}

async fn all_themes(db: &DatabaseBackend) -> Result<Vec<ThemeDefinition>, AppError> {
    let mut themes = builtin_themes();
    themes.extend(custom_themes(db).await?);
    Ok(themes)
}

fn builtin_themes() -> Vec<ThemeDefinition> {
    vec![
        builtin_theme_def(
            "bold-pink",
            "BOLD Pink",
            "Default XLStatus high-contrast theme",
            "both",
            [
                ("--bg-page", "#f8f8f8"),
                ("--bg-card", "#ffffff"),
                ("--text-main", "#1a1a1a"),
                ("--text-muted", "#4b5563"),
                ("--border-color", "#000000"),
                ("--accent-color", "#db2777"),
                ("--accent-bg", "#fce7f3"),
                ("--btn-bg", "#000000"),
                ("--btn-text", "#ffffff"),
                ("--dot-color", "#e5e7eb"),
            ],
            [
                ("--bg-page", "#161318"),
                ("--bg-card", "#231923"),
                ("--text-main", "#fff7fb"),
                ("--text-muted", "#d8b4c4"),
                ("--border-color", "#f472b6"),
                ("--accent-color", "#f472b6"),
                ("--accent-bg", "#4a1934"),
                ("--btn-bg", "#f472b6"),
                ("--btn-text", "#180b12"),
                ("--dot-color", "#5b2b43"),
            ],
        ),
        builtin_theme_def(
            "midnight-green",
            "Midnight Green",
            "Dark operations console",
            "both",
            [
                ("--bg-page", "#f4fbf7"),
                ("--bg-card", "#ffffff"),
                ("--text-main", "#08231b"),
                ("--text-muted", "#3f6658"),
                ("--border-color", "#064e3b"),
                ("--accent-color", "#059669"),
                ("--accent-bg", "#d1fae5"),
                ("--btn-bg", "#064e3b"),
                ("--btn-text", "#ffffff"),
                ("--dot-color", "#a7f3d0"),
            ],
            [
                ("--bg-page", "#121212"),
                ("--bg-card", "#1e1e1e"),
                ("--text-main", "#e5e5e5"),
                ("--text-muted", "#a3a3a3"),
                ("--border-color", "#10b981"),
                ("--accent-color", "#10b981"),
                ("--accent-bg", "#064e3b"),
                ("--btn-bg", "#10b981"),
                ("--btn-text", "#000000"),
                ("--dot-color", "#333333"),
            ],
        ),
        builtin_theme_def(
            "clear-blue",
            "Clear Blue",
            "Quiet status-page theme",
            "public",
            [
                ("--bg-page", "#f8fafc"),
                ("--bg-card", "#ffffff"),
                ("--text-main", "#0f172a"),
                ("--text-muted", "#475569"),
                ("--border-color", "#0f172a"),
                ("--accent-color", "#2563eb"),
                ("--accent-bg", "#dbeafe"),
                ("--btn-bg", "#1d4ed8"),
                ("--btn-text", "#ffffff"),
                ("--dot-color", "#cbd5e1"),
            ],
            [
                ("--bg-page", "#0b1220"),
                ("--bg-card", "#111827"),
                ("--text-main", "#e5f0ff"),
                ("--text-muted", "#9fb5d1"),
                ("--border-color", "#60a5fa"),
                ("--accent-color", "#60a5fa"),
                ("--accent-bg", "#172554"),
                ("--btn-bg", "#93c5fd"),
                ("--btn-text", "#082f49"),
                ("--dot-color", "#1e3a8a"),
            ],
        ),
    ]
}

fn builtin_theme(id: &str) -> Option<ThemeDefinition> {
    builtin_themes().into_iter().find(|theme| theme.id == id)
}

fn builtin_theme_def<const N: usize>(
    id: &str,
    name: &str,
    description: &str,
    target: &str,
    light_variables: [(&str, &str); N],
    dark_variables: [(&str, &str); N],
) -> ThemeDefinition {
    let light_variables = theme_map(light_variables);
    let dark_variables = theme_map(dark_variables);
    ThemeDefinition {
        id: id.to_string(),
        name: name.to_string(),
        description: Some(description.to_string()),
        target: target.to_string(),
        variables: light_variables.clone(),
        light_variables,
        dark_variables,
        custom_css: None,
        light_custom_css: None,
        dark_custom_css: None,
        builtin: true,
        created_at: None,
        updated_at: None,
    }
}

fn theme_map<const N: usize>(variables: [(&str, &str); N]) -> HashMap<String, String> {
    variables
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn normalize_theme_input(
    input: ThemeDefinitionInput,
    builtin: bool,
) -> Result<ThemeDefinition, AppError> {
    let ThemeDefinitionInput {
        id,
        name,
        description,
        target,
        variables,
        light_variables,
        dark_variables,
    } = input;
    let variables = normalize_theme_variables(variables)?;
    let light_variables = if light_variables.is_empty() {
        variables.clone()
    } else {
        normalize_theme_variables(light_variables)?
    };
    let dark_variables = normalize_theme_variables(dark_variables)?;

    Ok(ThemeDefinition {
        id: normalize_theme_id(&id)?,
        name: normalize_name(&name, "name")?,
        description: normalize_optional_text(description, 500, "description")?,
        target: normalize_target(target.as_deref().unwrap_or("both"))?,
        variables,
        light_variables,
        dark_variables,
        custom_css: None,
        light_custom_css: None,
        dark_custom_css: None,
        builtin,
        created_at: None,
        updated_at: None,
    })
}

fn normalize_theme_id(value: &str) -> Result<String, AppError> {
    let value = value.trim().to_ascii_lowercase();
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_');
    if !valid {
        return Err(AppError::BadRequest(
            "theme id must use lowercase letters, numbers, dashes, or underscores".into(),
        ));
    }
    if builtin_theme(&value).is_some() {
        return Err(AppError::BadRequest(
            "custom theme id conflicts with a builtin theme".into(),
        ));
    }
    Ok(value)
}

fn normalize_target(value: &str) -> Result<String, AppError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "public" => Ok("public".into()),
        "dashboard" | "admin" => Ok("dashboard".into()),
        "both" => Ok("both".into()),
        _ => Err(AppError::BadRequest(
            "theme target must be public, dashboard, or both".into(),
        )),
    }
}

fn normalize_select_target(value: &str) -> Result<String, AppError> {
    normalize_target(value)
}

fn normalize_name(value: &str, field: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{field} is required")));
    }
    if value.len() > 120 {
        return Err(AppError::BadRequest(format!("{field} is too long")));
    }
    Ok(value.to_string())
}

fn normalize_optional_text(
    value: Option<String>,
    max_len: usize,
    field: &str,
) -> Result<Option<String>, AppError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > max_len {
        return Err(AppError::BadRequest(format!("{field} is too long")));
    }
    Ok(Some(value.to_string()))
}

fn normalize_theme_variables(
    variables: HashMap<String, String>,
) -> Result<HashMap<String, String>, AppError> {
    if variables.len() > 60 {
        return Err(AppError::BadRequest(
            "theme variables must contain at most 60 entries".into(),
        ));
    }
    let mut out = HashMap::new();
    for (key, value) in variables {
        let key = key.trim();
        let value = value.trim();
        let valid_key = key.starts_with("--")
            && key.len() <= 80
            && key
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'));
        if !valid_key {
            return Err(AppError::BadRequest(format!(
                "invalid CSS variable name: {key}"
            )));
        }
        if value.len() > 240 || value.contains(';') || value.contains('{') || value.contains('}') {
            return Err(AppError::BadRequest(format!(
                "invalid CSS variable value for {key}"
            )));
        }
        out.insert(key.to_string(), value.to_string());
    }
    Ok(out)
}

async fn custom_themes(db: &DatabaseBackend) -> Result<Vec<ThemeDefinition>, AppError> {
    let Some(raw) = get_string_setting(db, CUSTOM_THEMES_KEY).await? else {
        return Ok(Vec::new());
    };
    let mut themes = serde_json::from_str::<Vec<ThemeDefinition>>(&raw)
        .map_err(|e| AppError::BadRequest(format!("stored theme catalog is invalid: {e}")))?;
    for theme in &mut themes {
        clear_custom_theme_css(theme);
    }
    Ok(themes)
}

async fn upsert_custom_theme(db: &DatabaseBackend, theme: ThemeDefinition) -> Result<(), AppError> {
    let mut themes = custom_themes(db).await?;
    let mut theme = theme;
    clear_custom_theme_css(&mut theme);
    let replacing_existing = themes.iter().any(|item| item.id == theme.id);
    if !replacing_existing && themes.len() >= THEME_MAX_CUSTOM_THEMES {
        return Err(AppError::BadRequest(format!(
            "custom theme catalog can contain at most {THEME_MAX_CUSTOM_THEMES} themes"
        )));
    }
    themes.retain(|item| item.id != theme.id);
    themes.push(theme);
    themes.sort_by(|a, b| a.name.cmp(&b.name));
    set_custom_themes(db, &themes).await
}

fn clear_custom_theme_css(theme: &mut ThemeDefinition) {
    theme.custom_css = None;
    theme.light_custom_css = None;
    theme.dark_custom_css = None;
}

async fn set_custom_themes(
    db: &DatabaseBackend,
    themes: &[ThemeDefinition],
) -> Result<(), AppError> {
    let value = custom_theme_catalog_json(themes)?;
    set_string_setting(db, CUSTOM_THEMES_KEY, &value).await
}

fn custom_theme_catalog_json(themes: &[ThemeDefinition]) -> Result<String, AppError> {
    if themes.len() > THEME_MAX_CUSTOM_THEMES {
        return Err(AppError::BadRequest(format!(
            "custom theme catalog can contain at most {THEME_MAX_CUSTOM_THEMES} themes"
        )));
    }
    let value = serde_json::to_string(themes).map_err(|e| AppError::BadRequest(e.to_string()))?;
    if value.len() > THEME_MAX_CUSTOM_CATALOG_BYTES {
        return Err(AppError::BadRequest(
            "custom theme catalog is too large".into(),
        ));
    }
    Ok(value)
}

async fn selected_public_theme_id(db: &DatabaseBackend) -> Result<Option<String>, AppError> {
    get_string_setting(db, SELECTED_PUBLIC_THEME_KEY).await
}

async fn selected_dashboard_theme_id(db: &DatabaseBackend) -> Result<Option<String>, AppError> {
    get_string_setting(db, SELECTED_DASHBOARD_THEME_KEY).await
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

async fn set_string_setting(db: &DatabaseBackend, key: &str, value: &str) -> Result<(), AppError> {
    let value_json =
        serde_json::to_string(value).map_err(|e| AppError::BadRequest(e.to_string()))?;
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
            .bind(&value_json)
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
            .bind(&value_json)
            .bind(now)
            .execute(pool)
            .await?;
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
        return Err(AppError::Forbidden("Cookie session required".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use xlstatus_shared::{UserId, UserRole};

    #[test]
    fn rejects_unsafe_theme_variable_values() {
        let err = normalize_theme_variables(HashMap::from([(
            "--accent-color".to_string(),
            "red;body{}".to_string(),
        )]))
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn theme_resource_limits_are_explicit() {
        let _ = theme_body_limit();
        assert_eq!(THEME_API_MAX_BODY_BYTES, 64 * 1024);
        assert_eq!(THEME_MAX_CUSTOM_THEMES, 32);
        assert_eq!(THEME_MAX_CUSTOM_CATALOG_BYTES, 256 * 1024);
    }

    #[test]
    fn rejects_too_many_theme_variables() {
        let variables = (0..=60)
            .map(|idx| (format!("--color-{idx}"), "#ffffff".to_string()))
            .collect();
        let err = normalize_theme_variables(variables).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn custom_theme_catalog_rejects_too_many_themes() {
        let themes: Vec<_> = (0..=THEME_MAX_CUSTOM_THEMES)
            .map(|idx| sample_custom_theme(&format!("custom-{idx}")))
            .collect();
        let err = custom_theme_catalog_json(&themes).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn custom_theme_catalog_rejects_oversized_json() {
        let mut theme = sample_custom_theme("custom");
        theme.description = Some("x".repeat(THEME_MAX_CUSTOM_CATALOG_BYTES));
        let err = custom_theme_catalog_json(&[theme]).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn theme_admin_mutations_reject_admin_pat() {
        let auth = auth_session(AuthKind::PersonalAccessToken);
        let err = require_admin_cookie_session(&auth).unwrap_err();
        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[test]
    fn theme_admin_mutations_allow_admin_cookie_session() {
        let auth = auth_session(AuthKind::Session);
        assert!(require_admin_cookie_session(&auth).is_ok());
    }

    #[test]
    fn stored_custom_theme_css_is_cleared() {
        let mut theme = ThemeDefinition {
            id: "custom".into(),
            name: "Custom".into(),
            description: None,
            target: "both".into(),
            variables: HashMap::from([("--accent-color".into(), "#16a34a".into())]),
            light_variables: HashMap::new(),
            dark_variables: HashMap::new(),
            custom_css: Some("body{background:url(https://example.com/x)}".into()),
            light_custom_css: Some("*{display:none}".into()),
            dark_custom_css: Some("@import url(https://example.com/x.css);".into()),
            builtin: false,
            created_at: None,
            updated_at: None,
        };
        clear_custom_theme_css(&mut theme);
        assert!(theme.custom_css.is_none());
        assert!(theme.light_custom_css.is_none());
        assert!(theme.dark_custom_css.is_none());
    }

    fn sample_custom_theme(id: &str) -> ThemeDefinition {
        ThemeDefinition {
            id: id.into(),
            name: id.into(),
            description: None,
            target: "both".into(),
            variables: HashMap::from([("--accent-color".into(), "#16a34a".into())]),
            light_variables: HashMap::new(),
            dark_variables: HashMap::new(),
            custom_css: None,
            light_custom_css: None,
            dark_custom_css: None,
            builtin: false,
            created_at: None,
            updated_at: None,
        }
    }

    fn auth_session(auth_kind: AuthKind) -> AuthSession {
        AuthSession {
            session_id: "session".into(),
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
