use crate::api::types::*;
use crate::auth::{generate_pat, hash_token};
use crate::config::Config;
use crate::db::{CreatePATInput, DatabaseBackend, PATRepository};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json, extract::Path,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use xlstatus_shared::UserId;

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

pub async fn create_pat(
    State(state): State<super::auth::AppState>,
    // TODO: extract user from session
    Json(req): Json<CreatePATRequest>,
) -> Result<Json<ApiResponse<CreatePATResponse>>, AppError> {
    // TEMPORARY: Get the first admin user from database for testing
    // In real implementation, this would come from the authenticated session
    let user_repo = crate::db::UserRepository::new(state.db.clone());
    let user = user_repo
        .find_by_username("admin")
        .await?
        .ok_or(AppError::Unauthorized("No user found".to_string()))?;

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
                user_id: user.id,
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
    // TODO: extract user from session
) -> Result<Json<ApiResponse<Vec<PATInfo>>>, AppError> {
    // TEMPORARY: Get the first admin user for testing
    let user_repo = crate::db::UserRepository::new(state.db.clone());
    let user = user_repo
        .find_by_username("admin")
        .await?
        .ok_or(AppError::Unauthorized("No user found".to_string()))?;

    let pat_repo = PATRepository::new(state.db.clone());
    let pats = pat_repo.list_by_user(user.id).await?;

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
    // TODO: extract user from session
) -> Result<Json<ApiResponse<()>>, AppError> {
    // TEMPORARY: Get the first admin user for testing
    let user_repo = crate::db::UserRepository::new(state.db.clone());
    let user = user_repo
        .find_by_username("admin")
        .await?
        .ok_or(AppError::Unauthorized("No user found".to_string()))?;

    let pat_repo = PATRepository::new(state.db.clone());
    let revoked = pat_repo.revoke(&id, user.id).await?;

    if !revoked {
        return Err(AppError::BadRequest("Token not found or already revoked".to_string()));
    }

    Ok(Json(ApiResponse::success(())))
}
