use axum::{
    async_trait,
    extract::{FromRequestParts, Request},
    http::{request::Parts, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use serde::{Deserialize, Serialize};
use xlstatus_shared::{UserId, UserRole};

use crate::auth::hash_token;
use crate::db::{DatabaseBackend, User, UserRepository};

pub const SESSION_COOKIE_NAME: &str = "xlstatus_session";
pub const CSRF_HEADER_NAME: &str = "x-csrf-token";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSession {
    pub user_id: UserId,
    pub username: String,
    pub role: UserRole,
    pub csrf_token: String,
}

/// Extract authenticated user from session cookie
pub struct AuthUser {
    pub user: User,
    pub csrf_token: String,
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

        // For now, we don't have the full User object in the session
        // This is a placeholder that will be improved
        Err(StatusCode::UNAUTHORIZED)
    }
}

/// Middleware to extract and validate session
pub async fn session_middleware(
    cookie_jar: CookieJar,
    mut request: Request,
    next: Next,
) -> Response {
    // Extract session cookie
    if let Some(cookie) = cookie_jar.get(SESSION_COOKIE_NAME) {
        let token = cookie.value();
        let token_hash = hash_token(token);

        // TODO: Look up session in database and validate
        // For now, we skip this and let requests through
        // This will be implemented when we connect to the DB in middleware
    }

    next.run(request).await
}

/// Middleware to validate CSRF token for state-changing requests
pub async fn csrf_middleware(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let method = request.method().clone();

    // Only check CSRF for state-changing methods
    if matches!(
        method.as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE"
    ) {
        // Extract session from extensions
        let session = request
            .extensions()
            .get::<AuthSession>();

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
