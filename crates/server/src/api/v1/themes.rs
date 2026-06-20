//! Theme catalog and selection API.

use crate::api::types::ApiResponse;
use crate::api::v1::auth::{AppError, AppState};
use crate::auth::middleware::AuthSession;
use crate::db::DatabaseBackend;
use axum::{
    extract::{Path, State},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const CUSTOM_THEMES_KEY: &str = "theme_custom_catalog";
const SELECTED_PUBLIC_THEME_KEY: &str = "theme_selected_public";
const SELECTED_DASHBOARD_THEME_KEY: &str = "theme_selected_dashboard";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeDefinition {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub target: String,
    pub variables: HashMap<String, String>,
    pub custom_css: Option<String>,
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
    pub custom_css: Option<Option<String>>,
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
    pub custom_css: Option<String>,
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
    require_admin(&auth)?;
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
    require_admin(&auth)?;
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
    if let Some(custom_css) = req.custom_css {
        theme.custom_css = normalize_optional_text(custom_css, 10_000, "custom_css")?;
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
    require_admin(&auth)?;
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
    require_admin(&auth)?;
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
        ),
        builtin_theme_def(
            "midnight-green",
            "Midnight Green",
            "Dark operations console",
            "both",
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
    variables: [(&str, &str); N],
) -> ThemeDefinition {
    ThemeDefinition {
        id: id.to_string(),
        name: name.to_string(),
        description: Some(description.to_string()),
        target: target.to_string(),
        variables: variables
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect(),
        custom_css: None,
        builtin: true,
        created_at: None,
        updated_at: None,
    }
}

fn normalize_theme_input(
    input: ThemeDefinitionInput,
    builtin: bool,
) -> Result<ThemeDefinition, AppError> {
    Ok(ThemeDefinition {
        id: normalize_theme_id(&input.id)?,
        name: normalize_name(&input.name, "name")?,
        description: normalize_optional_text(input.description, 500, "description")?,
        target: normalize_target(input.target.as_deref().unwrap_or("both"))?,
        variables: normalize_theme_variables(input.variables)?,
        custom_css: normalize_optional_text(input.custom_css, 10_000, "custom_css")?,
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
    serde_json::from_str::<Vec<ThemeDefinition>>(&raw)
        .map_err(|e| AppError::BadRequest(format!("stored theme catalog is invalid: {e}")))
}

async fn upsert_custom_theme(db: &DatabaseBackend, theme: ThemeDefinition) -> Result<(), AppError> {
    let mut themes = custom_themes(db).await?;
    themes.retain(|item| item.id != theme.id);
    themes.push(theme);
    themes.sort_by(|a, b| a.name.cmp(&b.name));
    set_custom_themes(db, &themes).await
}

async fn set_custom_themes(
    db: &DatabaseBackend,
    themes: &[ThemeDefinition],
) -> Result<(), AppError> {
    let value = serde_json::to_string(themes).map_err(|e| AppError::BadRequest(e.to_string()))?;
    set_string_setting(db, CUSTOM_THEMES_KEY, &value).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_theme_variable_values() {
        let err = normalize_theme_variables(HashMap::from([(
            "--accent-color".to_string(),
            "red;body{}".to_string(),
        )]))
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }
}
