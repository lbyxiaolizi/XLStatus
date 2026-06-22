#![allow(dead_code)]
#![allow(unused)]

use axum::{
    async_trait,
    extract::{connect_info::ConnectInfo, FromRequestParts, Request, State},
    http::{header, request::Parts, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use axum_extra::extract::CookieJar;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use xlstatus_shared::{UserId, UserRole};

use crate::api::v1::auth::{active_waf_ban, record_waf_event, register_pat_failure, AppState};
use crate::auth::{hash_token, SessionRepository};
use crate::db::{PATRepository, User, UserRepository};
use crate::security::client_ip_from_headers_and_peer;

pub const SESSION_COOKIE_NAME: &str = "xlstatus_session";
pub const CSRF_COOKIE_NAME: &str = "xlstatus_csrf";
pub const CSRF_HEADER_NAME: &str = "x-csrf-token";
const SESSION_TOKEN_BYTES: usize = 64;
const PAT_PREFIX: &str = "xlp_";
const PAT_TOKEN_HEX_BYTES: usize = 64;
const PAT_TOKEN_BYTES: usize = PAT_PREFIX.len() + PAT_TOKEN_HEX_BYTES;
const PAT_AUTHORIZATION_HEADER_MAX_BYTES: usize = "Bearer ".len() + PAT_TOKEN_BYTES;

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
    pub fn auth_session(&self) -> AuthSession {
        AuthSession {
            session_id: self.session_id.clone(),
            user_id: self.user.id,
            username: self.user.username.clone(),
            role: self.user.role,
            csrf_token: self.csrf_token.clone(),
            auth_kind: self.auth_kind.clone(),
            scopes: self.scopes.clone(),
            server_ids: self.server_ids.clone(),
            pat_id: self.pat_id.clone(),
        }
    }

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
        crate::auth::rbac::has_scope(
            &AuthSession {
                session_id: self.session_id.clone(),
                user_id: self.user.id,
                username: self.user.username.clone(),
                role: self.user.role,
                csrf_token: self.csrf_token.clone(),
                auth_kind: self.auth_kind.clone(),
                scopes: self.scopes.clone(),
                server_ids: self.server_ids.clone(),
                pat_id: self.pat_id.clone(),
            },
            required_scope,
        )
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
    let peer_addr = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| *addr);
    let client_ip = client_ip_from_headers_and_peer(request.headers(), peer_addr);
    match pat_bearer_token_from_headers(request.headers()) {
        Err(reason) => {
            let _ = register_pat_failure(&state.db, &client_ip, None, reason).await;
        }
        Ok(Some(token)) => {
            if active_waf_ban(&state.db, &client_ip)
                .await
                .ok()
                .flatten()
                .is_some()
            {
                let _ = record_waf_event(
                    &state.db,
                    &client_ip,
                    None,
                    "pat_blocked",
                    Some("active WAF ban"),
                )
                .await;
                return StatusCode::FORBIDDEN.into_response();
            }

            let token_hash = hash_token(&token);
            let pat_repo = PATRepository::new(state.db.clone());
            match pat_repo.find_by_token_hash(&token_hash).await {
                Ok(Some(pat)) => {
                    let is_expired = pat
                        .expires_at
                        .map(|expires_at| expires_at <= Utc::now())
                        .unwrap_or(true);
                    if is_expired {
                        let _ = register_pat_failure(
                            &state.db,
                            &client_ip,
                            Some(&pat.id),
                            "expired token",
                        )
                        .await;
                    } else {
                        let user_repo = UserRepository::new(state.db.clone());
                        match user_repo.find_by_id(pat.user_id).await {
                            Ok(Some(user)) => {
                                match crate::auth::rbac::validate_pat_runtime(
                                    &pat.scopes,
                                    user.role.is_admin(),
                                    pat.server_ids.as_deref(),
                                ) {
                                    Ok(()) => {
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
                                        let _ = pat_repo.mark_used(&pat.id, Some(&client_ip)).await;
                                        request.extensions_mut().insert(auth_session);
                                        request.extensions_mut().insert(user);
                                    }
                                    Err(reason) => {
                                        let _ = register_pat_failure(
                                            &state.db,
                                            &client_ip,
                                            Some(&pat.id),
                                            &format!("invalid token policy: {reason}"),
                                        )
                                        .await;
                                    }
                                }
                            }
                            Ok(None) => {
                                let _ = register_pat_failure(
                                    &state.db,
                                    &client_ip,
                                    Some(&pat.id),
                                    "token user not found",
                                )
                                .await;
                            }
                            Err(err) => {
                                tracing::warn!("failed to load PAT user: {}", err);
                            }
                        }
                    }
                }
                Ok(None) => {
                    let _ =
                        register_pat_failure(&state.db, &client_ip, None, "invalid token").await;
                }
                Err(err) => {
                    tracing::warn!("failed to load PAT by token hash: {}", err);
                }
            }
        }
        Ok(None) => {}
    }

    if request.extensions().get::<AuthSession>().is_none() {
        if let Some(cookie) = cookie_jar.get(SESSION_COOKIE_NAME) {
            let token = cookie.value();
            if session_cookie_token_is_valid(token) {
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
        }
    }

    if matches!(
        request.method().as_str(),
        "POST" | "PUT" | "PATCH" | "DELETE"
    ) {
        match request.extensions().get::<AuthSession>() {
            Some(session) => {
                if session_request_requires_csrf(request.method().as_str(), Some(session)) {
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

fn pat_bearer_token_from_headers(headers: &HeaderMap) -> Result<Option<String>, &'static str> {
    let Some(value) = headers.get(header::AUTHORIZATION) else {
        return Ok(None);
    };
    let value = value.to_str().map_err(|_| "malformed bearer token")?.trim();
    let Some(token) = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
    else {
        return Ok(None);
    };
    let token = token.trim();
    if token.is_empty() {
        return Ok(None);
    }
    if !token.starts_with(PAT_PREFIX) {
        return Ok(None);
    }
    if value.len() > PAT_AUTHORIZATION_HEADER_MAX_BYTES {
        return Err("bearer token too long");
    }
    if pat_token_is_valid(token) {
        Ok(Some(token.to_string()))
    } else {
        Err("malformed PAT bearer token")
    }
}

fn pat_token_is_valid(token: &str) -> bool {
    token.len() == PAT_TOKEN_BYTES
        && token.starts_with(PAT_PREFIX)
        && token[PAT_PREFIX.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
}

fn session_cookie_token_is_valid(token: &str) -> bool {
    token.len() == SESSION_TOKEN_BYTES && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn session_request_requires_csrf(method: &str, session: Option<&AuthSession>) -> bool {
    matches!(method, "POST" | "PUT" | "PATCH" | "DELETE")
        && session
            .map(|session| matches!(session.auth_kind, AuthKind::Session))
            .unwrap_or(false)
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

fn client_ip_from_headers(headers: &HeaderMap) -> String {
    crate::security::client_ip_from_headers(headers)
}

#[cfg(test)]
mod tests {
    use super::{
        pat_bearer_token_from_headers, session_cookie_token_is_valid,
        session_request_requires_csrf, AuthKind, AuthSession, PAT_AUTHORIZATION_HEADER_MAX_BYTES,
        PAT_TOKEN_BYTES,
    };
    use axum::http::{header, HeaderMap};
    use xlstatus_shared::{UserId, UserRole};

    #[test]
    fn cookie_session_post_requires_csrf() {
        let session = auth_session(AuthKind::Session);

        assert!(session_request_requires_csrf("POST", Some(&session)));
        assert!(session_request_requires_csrf("PUT", Some(&session)));
        assert!(session_request_requires_csrf("PATCH", Some(&session)));
        assert!(session_request_requires_csrf("DELETE", Some(&session)));
    }

    #[test]
    fn safe_methods_and_pat_do_not_require_csrf() {
        let session = auth_session(AuthKind::Session);
        let pat = auth_session(AuthKind::PersonalAccessToken);

        assert!(!session_request_requires_csrf("GET", Some(&session)));
        assert!(!session_request_requires_csrf("HEAD", Some(&session)));
        assert!(!session_request_requires_csrf("POST", Some(&pat)));
        assert!(!session_request_requires_csrf("POST", None));
    }

    #[test]
    fn pat_bearer_token_shape_is_bounded_before_hashing() {
        let token = format!("xlp_{}", "a".repeat(PAT_TOKEN_BYTES - "xlp_".len()));
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        assert_eq!(
            pat_bearer_token_from_headers(&headers).unwrap().as_deref(),
            Some(token.as_str())
        );

        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer xlp_{}", "g".repeat(64)).parse().unwrap(),
        );
        assert_eq!(
            pat_bearer_token_from_headers(&headers),
            Err("malformed PAT bearer token")
        );

        headers.insert(
            header::AUTHORIZATION,
            format!(
                "Bearer xlp_{}",
                "a".repeat(PAT_AUTHORIZATION_HEADER_MAX_BYTES)
            )
            .parse()
            .unwrap(),
        );
        assert_eq!(
            pat_bearer_token_from_headers(&headers),
            Err("bearer token too long")
        );
    }

    #[test]
    fn non_pat_bearer_is_ignored_for_cookie_fallback() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer oauth-token".parse().unwrap());
        assert_eq!(pat_bearer_token_from_headers(&headers).unwrap(), None);
    }

    #[test]
    fn session_cookie_token_shape_is_bounded_before_hashing() {
        assert!(session_cookie_token_is_valid(&"a".repeat(64)));
        assert!(session_cookie_token_is_valid(&"A".repeat(64)));
        assert!(!session_cookie_token_is_valid(&"a".repeat(65)));
        assert!(!session_cookie_token_is_valid(&"g".repeat(64)));
    }

    fn auth_session(auth_kind: AuthKind) -> AuthSession {
        AuthSession {
            session_id: "session".into(),
            user_id: UserId(uuid::Uuid::now_v7()),
            username: "user".into(),
            role: UserRole::Admin,
            csrf_token: "csrf".into(),
            auth_kind,
            scopes: vec!["*".into()],
            server_ids: None,
            pat_id: None,
        }
    }
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}
