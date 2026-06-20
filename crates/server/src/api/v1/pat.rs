use crate::api::types::*;
use crate::auth::generate_pat;
use crate::auth::middleware::AuthUser;
use crate::auth::rbac;
use crate::db::{CreatePATInput, PATRepository};
use axum::{extract::Path, extract::State, http::HeaderMap, Json};
use serde::{Deserialize, Serialize};

use super::auth::AppError;

#[derive(Debug, Deserialize)]
pub struct CreatePATRequest {
    pub name: String,
    pub scopes: Vec<String>,
    pub server_ids: Option<Vec<String>>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreatePATResponse {
    pub id: String,
    pub name: String,
    pub token: String, // Only returned on creation
    pub scopes: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct PATInfo {
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub server_ids: Option<Vec<String>>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

fn validate_scopes(scopes: &[String], is_admin: bool) -> Result<(), AppError> {
    rbac::validate_pat_scopes(scopes, is_admin).map_err(|msg| {
        if msg.contains("admin:*") {
            AppError::Forbidden(msg)
        } else {
            AppError::BadRequest(msg)
        }
    })
}

fn validate_servers(ids: Option<&[String]>) -> Result<(), AppError> {
    rbac::validate_server_ids(ids).map_err(AppError::BadRequest)
}

pub async fn create_pat(
    State(state): State<super::auth::AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Json(req): Json<CreatePATRequest>,
) -> Result<Json<ApiResponse<CreatePATResponse>>, AppError> {
    auth_user
        .require_cookie_session()
        .map_err(|_| AppError::Forbidden("PAT cannot manage API tokens".to_string()))?;
    super::auth::require_sensitive_totp(&state.db, auth_user.user.id, &headers).await?;
    validate_scopes(&req.scopes, auth_user.user.role.is_admin())?;
    validate_servers(req.server_ids.as_deref())?;

    let pat_repo = PATRepository::new(state.db.clone());

    // Generate token
    let (token, token_hash) = generate_pat();

    // Parse expires_at
    let expires_at = req
        .expires_at
        .as_ref()
        .map(|s| chrono::DateTime::parse_from_rfc3339(s))
        .transpose()
        .map_err(|e| AppError::BadRequest(format!("Invalid expires_at: {}", e)))?
        .map(|dt| dt.with_timezone(&chrono::Utc));

    // Create PAT
    let pat = pat_repo
        .create(
            CreatePATInput {
                user_id: auth_user.user.id,
                name: req.name,
                scopes: req.scopes,
                server_ids: req.server_ids,
                expires_at,
            },
            token_hash,
        )
        .await?;

    Ok(Json(ApiResponse::success(CreatePATResponse {
        id: pat.id,
        name: pat.name,
        token, // Only returned on creation
        scopes: pat.scopes,
        created_at: pat.created_at.to_rfc3339(),
    })))
}

pub async fn list_pats(
    State(state): State<super::auth::AppState>,
    auth_user: AuthUser,
) -> Result<Json<ApiResponse<Vec<PATInfo>>>, AppError> {
    auth_user
        .require_cookie_session()
        .map_err(|_| AppError::Forbidden("PAT cannot manage API tokens".to_string()))?;

    let pat_repo = PATRepository::new(state.db.clone());
    let pats = pat_repo.list_by_user(auth_user.user.id).await?;

    let pat_infos = pats
        .into_iter()
        .map(|pat| PATInfo {
            id: pat.id,
            name: pat.name,
            scopes: pat.scopes,
            server_ids: pat.server_ids,
            expires_at: pat.expires_at.map(|dt| dt.to_rfc3339()),
            last_used_at: pat.last_used_at.map(|dt| dt.to_rfc3339()),
            created_at: pat.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(ApiResponse::success(pat_infos)))
}

pub async fn revoke_pat(
    State(state): State<super::auth::AppState>,
    Path(id): Path<String>,
    auth_user: AuthUser,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<()>>, AppError> {
    auth_user
        .require_cookie_session()
        .map_err(|_| AppError::Forbidden("PAT cannot manage API tokens".to_string()))?;
    super::auth::require_sensitive_totp(&state.db, auth_user.user.id, &headers).await?;

    let pat_repo = PATRepository::new(state.db.clone());
    let revoked = pat_repo.revoke(&id, auth_user.user.id).await?;

    if !revoked {
        return Err(AppError::BadRequest(
            "Token not found or already revoked".to_string(),
        ));
    }

    Ok(Json(ApiResponse::success(())))
}
