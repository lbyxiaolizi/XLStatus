#![allow(dead_code)]
#![allow(unused)]

use axum::{
    async_trait,
    extract::{FromRequestParts, Request, State},
    http::{header, request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use xlstatus_shared::{UserId, UserRole};

use crate::api::v1::auth::AppState;
use crate::auth::{hash_token, SessionRepository};
use crate::db::{PATRepository, User, UserRepository};

pub const SESSION_COOKIE_NAME: &str = "xlstatus_session";
pub const CSRF_COOKIE_NAME: &str = "xlstatus_csrf";
pub const CSRF_HEADER_NAME: &str = "x-csrf-token";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuthKind {
    Session,
    PersonalAccessToken,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub session_id: String,
    pub user_id: UserId,
    pub username: String,
    pub role: UserRole,
    pub csrf_token: String,
    pub auth_kind: AuthKind,
    pub scopes: Vec<String>,
    pub server_ids: Option<Vec<String>>,
    pub pat_id: Option<String>,
}

/// Extract authenticated user from session cookie
pub struct AuthUser {
    pub user: User,
    pub session_id: String,
    pub csrf_token: String,
    pub auth_kind: AuthKind,
    pub scopes: Vec<String>,
    pub server_ids: Option<Vec<String>>,
    pub pat_id: Option<String>,
}

impl AuthUser {
    pub fn is_pat(&self) -> bool {
        matches!(self.auth_kind, AuthKind::PersonalAccessToken)
    }

    pub fn require_cookie_session(&self) -> Result<(), StatusCode> {
        if self.is_pat() {
            Err(StatusCode::FORBIDDEN)
        } else {
            Ok(())
        }
    }

    pub fn require_pat(&self) -> Result<(), StatusCode> {
        if self.is_pat() {
            Ok(())
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }

    pub fn has_scope(&self, required_scope: &str) -> bool {
        if !self.is_pat() {
            return true;
        }

        let Some((namespace, _)) = required_scope.split_once(':') else {
            return self.scopes.iter().any(|scope| scope == required_scope);
        };

        self.scopes.iter().any(|scope| {
            scope == required_scope || scope == "*" || scope == &format!("{}:*", namespace)
        })
    }

    pub fn require_scope(&self, required_scope: &str) -> Result<(), StatusCode> {
        if self.has_scope(required_scope) {
            Ok(())
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }

    pub fn can_access_server(&self, server_id: &str) -> bool {
        self.server_ids
            .as_ref()
            .map(|ids| ids.iter().any(|id| id == server_id))
            .unwrap_or(true)
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for AuthSession
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthSession>()
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)
    }
}
#[async_trait]
impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // Extract session from extensions (set by middleware)
        let session = parts
            .extensions
            .get::<AuthSession>()
            .ok_or(StatusCode::UNAUTHORIZED)?;
        let user = parts
            .extensions
            .get::<User>()
            .ok_or(StatusCode::UNAUTHORIZED)?;

        Ok(AuthUser {
            user: user.clone(),
            session_id: session.session_id.clone(),
            csrf_token: session.csrf_token.clone(),
            auth_kind: session.auth_kind.clone(),
            scopes: session.scopes.clone(),
            server_ids: session.server_ids.clone(),
            pat_id: session.pat_id.clone(),
        })
    }
}

/// Middleware to extract and validate session
pub async fn session_middleware(
    State(state): State<AppState>,
    cookie_jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Response {
    let bearer_token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .strip_prefix("Bearer ")
                .or_else(|| value.strip_prefix("bearer "))
        })
        .filter(|token| token.starts_with("xlp_"))
        .map(str::to_string);

    if let Some(token) = bearer_token {
        let token_hash = hash_token(&token);
        let pat_repo = PATRepository::new(state.db.clone());
        if let Ok(Some(pat)) = pat_repo.find_by_token_hash(&token_hash).await {
            let is_expired = pat
                .expires_at
                .map(|expires_at| expires_at <= Utc::now())
                .unwrap_or(false);
            if !is_expired {
                let user_repo = UserRepository::new(state.db.clone());
                if let Ok(Some(user)) = user_repo.find_by_id(pat.user_id).await {
                    let auth_session = AuthSession {
                        session_id: pat.id.clone(),
                        user_id: user.id,
                        username: user.username.clone(),
                        role: user.role,
                        csrf_token: String::new(),
                        auth_kind: AuthKind::PersonalAccessToken,
                        scopes: pat.scopes.clone(),
                        server_ids: pat.server_ids.clone(),
                        pat_id: Some(pat.id.clone()),
                    };
                    let _ = pat_repo.mark_used(&pat.id, None).await;
                    request.extensions_mut().insert(auth_session);
                    request.extensions_mut().insert(user);
                }
            }
        }
    } else if let Some(cookie) = cookie_jar.get(SESSION_COOKIE_NAME) {
        let token = cookie.value();
        let token_hash = hash_token(token);

        let session_repo = SessionRepository::new(state.db.clone());
        if let Ok(Some(session)) = session_repo.find_by_token_hash(&token_hash).await {
            let user_repo = UserRepository::new(state.db.clone());
            if let Ok(Some(user)) = user_repo.find_by_id(session.user_id).await {
                let csrf_token = derive_csrf_token(&token_hash);
                let auth_session = AuthSession {
                    session_id: session.id.clone(),
                    user_id: user.id,
                    username: user.username.clone(),
                    role: user.role,
                    csrf_token,
                    auth_kind: AuthKind::Session,
                    scopes: Vec::new(),
                    server_ids: None,
                    pat_id: None,
                };
                request.extensions_mut().insert(auth_session);
                request.extensions_mut().insert(user);
            }
        }
    }

    if matches!(
        request.method().as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE"
    ) {
        match request.extensions().get::<AuthSession>() {
            Some(session) => {
                if matches!(session.auth_kind, AuthKind::Session) {
                    let csrf_header = request
                        .headers()
                        .get(CSRF_HEADER_NAME)
                        .and_then(|h| h.to_str().ok());
                    if csrf_header != Some(&session.csrf_token) {
                        return StatusCode::FORBIDDEN.into_response();
                    }
                }
            }
            None => return StatusCode::UNAUTHORIZED.into_response(),
        }
    }

    next.run(request).await
}

/// Middleware to validate CSRF token for state-changing requests
pub async fn csrf_middleware(request: Request, next: Next) -> Result<Response, StatusCode> {
    let method = request.method().clone();

    // Only check CSRF for state-changing methods
    if matches!(method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE") {
        // Extract session from extensions
        let session = request.extensions().get::<AuthSession>();

        if let Some(session) = session {
            // Extract CSRF token from header
            let csrf_header = request
                .headers()
                .get(CSRF_HEADER_NAME)
                .and_then(|h| h.to_str().ok());

            // Validate CSRF token
            if csrf_header != Some(&session.csrf_token) {
                return Err(StatusCode::FORBIDDEN);
            }
        }
    }

    Ok(next.run(request).await)
}

/// Generate a random CSRF token
pub fn generate_csrf_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.gen();
    hex::encode(bytes)
}

pub fn derive_csrf_token(token_hash: &str) -> String {
    crate::auth::hash_token(&format!("csrf:{}", token_hash))
}
