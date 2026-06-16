use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use xlstatus_shared::UserRole;

use crate::api::v1::auth::AppState;
use crate::auth::AuthSession;

/// Require admin role
pub async fn require_admin(
    State(_state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let session = request
        .extensions()
        .get::<AuthSession>()
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if !session.role.is_admin() {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(next.run(request).await)
}

/// Require any authenticated user
pub async fn require_auth(
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    request
        .extensions()
        .get::<AuthSession>()
        .ok_or(StatusCode::UNAUTHORIZED)?;

    Ok(next.run(request).await)
}

/// Check if user has required scope (for PAT authentication)
pub fn has_scope(session: &AuthSession, required_scope: &str) -> bool {
    // TODO: Implement scope checking when we add scopes to session
    // For now, all authenticated users have all scopes
    true
}

/// Middleware to check specific scope
pub async fn require_scope(
    _scope: &'static str,
) -> impl Fn(Request, Next) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Response, StatusCode>> + Send>> + Clone {
    move |request: Request, next: Next| {
        Box::pin(async move {
            let _session = request
                .extensions()
                .get::<AuthSession>()
                .ok_or(StatusCode::UNAUTHORIZED)?;

            // TODO: Implement scope checking
            // if !has_scope(session, scope) {
            //     return Err(StatusCode::FORBIDDEN);
            // }

            Ok(next.run(request).await)
        })
    }
}
