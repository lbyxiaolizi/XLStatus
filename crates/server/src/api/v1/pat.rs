use crate::api::types::*;
use crate::auth::generate_pat;
use crate::auth::middleware::AuthUser;
use crate::auth::rbac;
use crate::db::{AgentRepository, CreatePATInput, DatabaseBackend, PATRepository};
use axum::{extract::Path, extract::State, http::HeaderMap, Json};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use xlstatus_shared::UserId;

use super::auth::AppError;

const DEFAULT_PAT_TTL_DAYS: i64 = 90;
const MAX_PAT_TTL_DAYS: i64 = 365;

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
    pub expires_at: String,
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

fn validate_servers(
    ids: Option<&[String]>,
    scopes: &[String],
    is_admin: bool,
) -> Result<(), AppError> {
    rbac::validate_server_ids(ids).map_err(AppError::BadRequest)
        .and_then(|_| {
            if !is_admin
                && rbac::pat_scopes_require_server_allowlist(scopes)
                && ids.map(|ids| ids.is_empty()).unwrap_or(true)
            {
                return Err(AppError::Forbidden(
                    "non-admin PATs with server-scoped permissions require a non-empty server allowlist"
                        .to_string(),
                ));
            }
            Ok(())
        })
}

async fn validate_server_allowlist_ownership(
    db: &DatabaseBackend,
    user_id: UserId,
    is_admin: bool,
    ids: Option<&[String]>,
) -> Result<(), AppError> {
    let Some(ids) = ids else {
        return Ok(());
    };
    let repo = AgentRepository::new(db.clone());
    for id in ids {
        let agent_id = uuid::Uuid::parse_str(id)
            .map(xlstatus_shared::AgentId)
            .map_err(|_| AppError::BadRequest(format!("invalid server id in allowlist: {id}")))?;
        let Some(agent) = repo.find_by_id(agent_id).await? else {
            return Err(AppError::BadRequest(format!(
                "server id in allowlist does not exist: {id}"
            )));
        };
        if !is_admin && agent.owner_user_id != user_id {
            return Err(AppError::Forbidden(
                "non-admin PAT server allowlist can only contain owned servers".to_string(),
            ));
        }
    }
    Ok(())
}

fn resolve_pat_expires_at(
    input: Option<&str>,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, AppError> {
    let expires_at = match input.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => DateTime::parse_from_rfc3339(value)
            .map_err(|e| AppError::BadRequest(format!("Invalid expires_at: {e}")))?
            .with_timezone(&Utc),
        None => now + Duration::days(DEFAULT_PAT_TTL_DAYS),
    };

    if expires_at <= now {
        return Err(AppError::BadRequest(
            "expires_at must be in the future".to_string(),
        ));
    }

    let max_expires_at = now + Duration::days(MAX_PAT_TTL_DAYS);
    if expires_at > max_expires_at {
        return Err(AppError::BadRequest(format!(
            "expires_at must be within {MAX_PAT_TTL_DAYS} days"
        )));
    }

    Ok(expires_at)
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
    let is_admin = auth_user.user.role.is_admin();
    validate_scopes(&req.scopes, is_admin)?;
    validate_servers(req.server_ids.as_deref(), &req.scopes, is_admin)?;
    validate_server_allowlist_ownership(
        &state.db,
        auth_user.user.id,
        is_admin,
        req.server_ids.as_deref(),
    )
    .await?;

    let pat_repo = PATRepository::new(state.db.clone());

    // Generate token
    let (token, token_hash) = generate_pat();

    let expires_at = resolve_pat_expires_at(req.expires_at.as_deref(), Utc::now())?;

    // Create PAT
    let pat = pat_repo
        .create(
            CreatePATInput {
                user_id: auth_user.user.id,
                name: req.name,
                scopes: req.scopes,
                server_ids: req.server_ids,
                expires_at: Some(expires_at),
            },
            token_hash,
        )
        .await?;

    Ok(Json(ApiResponse::success(CreatePATResponse {
        id: pat.id,
        name: pat.name,
        token, // Only returned on creation
        scopes: pat.scopes,
        expires_at: pat
            .expires_at
            .expect("new PAT has an expiration")
            .to_rfc3339(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CreateAgentInput, CreateUserInput, UserRepository};
    use chrono::TimeZone;
    use xlstatus_shared::UserRole;

    #[test]
    fn pat_expiration_defaults_to_ninety_days() {
        let now = Utc.with_ymd_and_hms(2026, 6, 21, 12, 0, 0).unwrap();

        let expires_at = resolve_pat_expires_at(None, now).unwrap();

        assert_eq!(expires_at, now + Duration::days(DEFAULT_PAT_TTL_DAYS));
    }

    #[test]
    fn pat_expiration_rejects_past_timestamp() {
        let now = Utc.with_ymd_and_hms(2026, 6, 21, 12, 0, 0).unwrap();

        let err = resolve_pat_expires_at(Some("2026-06-21T11:59:59Z"), now).unwrap_err();

        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn pat_expiration_rejects_too_far_timestamp() {
        let now = Utc.with_ymd_and_hms(2026, 6, 21, 12, 0, 0).unwrap();

        let err = resolve_pat_expires_at(Some("2027-06-22T12:00:01Z"), now).unwrap_err();

        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn non_admin_pat_allowlist_rejects_other_owner_server() {
        let db = test_db().await;
        let owner = seed_user(&db, "owner", UserRole::Member).await;
        let other = seed_user(&db, "other", UserRole::Member).await;
        let other_agent = AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "other".into(),
                public_key: "pk".into(),
                owner_user_id: other,
            })
            .await
            .unwrap();

        let err = validate_server_allowlist_ownership(
            &db,
            owner,
            false,
            Some(&[other_agent.id.0.to_string()]),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn admin_pat_allowlist_accepts_existing_other_owner_server() {
        let db = test_db().await;
        let admin = seed_user(&db, "admin", UserRole::Admin).await;
        let other = seed_user(&db, "other", UserRole::Member).await;
        let other_agent = AgentRepository::new(db.clone())
            .create(CreateAgentInput {
                name: "other".into(),
                public_key: "pk".into(),
                owner_user_id: other,
            })
            .await
            .unwrap();

        assert!(validate_server_allowlist_ownership(
            &db,
            admin,
            true,
            Some(&[other_agent.id.0.to_string()]),
        )
        .await
        .is_ok());
    }

    #[tokio::test]
    async fn pat_allowlist_rejects_missing_server() {
        let db = test_db().await;
        let owner = seed_user(&db, "owner", UserRole::Member).await;

        let err = validate_server_allowlist_ownership(
            &db,
            owner,
            false,
            Some(&["00000000-0000-0000-0000-000000000404".to_string()]),
        )
        .await
        .unwrap_err();

        assert!(matches!(err, AppError::BadRequest(_)));
    }

    async fn test_db() -> DatabaseBackend {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn seed_user(db: &DatabaseBackend, username: &str, role: UserRole) -> UserId {
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: username.into(),
                password: "password123".into(),
                role,
            })
            .await
            .unwrap();
        user.id
    }
}
