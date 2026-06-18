#![allow(dead_code)]
#![allow(unused)]

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::Next,
    response::Response,
};

use crate::api::v1::auth::AppState;
use crate::auth::AuthSession;

/// Canonical PAT scope names defined in `plan/07-security.md`.
/// PAT creation must only use these; anything else is rejected.
pub const KNOWN_SCOPES: &[&str] = &[
    "inventory:read",
    "inventory:delete",
    "server:read",
    "server:write",
    "server:delete",
    "server:exec",
    "service:read",
    "service:write",
    "service:delete",
    "alert:read",
    "alert:write",
    "alert:delete",
    "task:read",
    "task:write",
    "task:delete",
    "task:exec",
    "ddns:read",
    "ddns:write",
    "ddns:delete",
    "nat:read",
    "nat:write",
    "nat:delete",
    "notification:read",
    "notification:write",
    "notification:delete",
    "transfer:read",
    "transfer:write",
    "admin:*",
];

/// Validate that a list of PAT scopes only contains known scope names and is
/// well-formed. `admin:*` is only usable by Admin users.
pub fn validate_pat_scopes(scopes: &[String], is_admin: bool) -> Result<(), String> {
    if scopes.is_empty() {
        return Err("scopes must not be empty".to_string());
    }

    for scope in scopes {
        if scope == "*" {
            return Err("wildcard '*' is not allowed in PAT scopes".to_string());
        }

        if scope == "admin:*" {
            if !is_admin {
                return Err("admin:* scope requires admin role".to_string());
            }
            continue;
        }

        match scope.split_once(':') {
            Some((namespace, action)) => {
                if namespace.is_empty() || action.is_empty() {
                    return Err(format!("invalid scope format: {}", scope));
                }
                if !KNOWN_SCOPES.contains(&scope.as_str()) {
                    return Err(format!("unknown scope: {}", scope));
                }
            }
            None => {
                return Err(format!(
                    "scope '{}' must use 'namespace:action' format",
                    scope
                ));
            }
        }
    }

    Ok(())
}

/// Validate that a PAT server allowlist only contains valid UUIDs.
pub fn validate_server_ids(server_ids: Option<&[String]>) -> Result<(), String> {
    let Some(ids) = server_ids else {
        return Ok(());
    };

    for id in ids {
        uuid::Uuid::parse_str(id).map_err(|_| format!("invalid server id in allowlist: {}", id))?;
    }

    Ok(())
}

/// Check if a session/pat has a given scope.
/// Cookie sessions (full admin) implicitly satisfy every scope check; PATs must
/// have the scope (or a namespace wildcard like `task:*`).
pub fn has_scope(_session: &AuthSession, _required_scope: &str) -> bool {
    if !matches!(
        _session.auth_kind,
        crate::auth::middleware::AuthKind::PersonalAccessToken
    ) {
        return true;
    }

    let Some((namespace, _)) = _required_scope.split_once(':') else {
        return _session
            .scopes
            .iter()
            .any(|s| s == _required_scope || s == "*");
    };

    _session
        .scopes
        .iter()
        .any(|s| s == _required_scope || s == "*" || s == &format!("{}:*", namespace))
}

/// Check that a session/pat can access a single server id (PAT allowlist).
pub fn can_access_server(session: &AuthSession, server_id: &str) -> bool {
    match &session.server_ids {
        None => true,
        Some(ids) => ids.iter().any(|id| id == server_id),
    }
}

/// Check that every server id in a list is allowed for the session/pat.
pub fn can_access_servers(session: &AuthSession, server_ids: &[String]) -> bool {
    match &session.server_ids {
        None => true,
        Some(allow) => server_ids.iter().all(|id| allow.iter().any(|a| a == id)),
    }
}

/// Middleware that requires the caller to be an admin.
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

/// Middleware that requires any authenticated user.
pub async fn require_auth(request: Request, next: Next) -> Result<Response, StatusCode> {
    request
        .extensions()
        .get::<AuthSession>()
        .ok_or(StatusCode::UNAUTHORIZED)?;

    Ok(next.run(request).await)
}

/// Middleware that requires a specific scope.
pub async fn require_scope(
    _scope: &'static str,
) -> impl Fn(
    Request,
    Next,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Response, StatusCode>> + Send>,
> + Clone {
    move |request: Request, next: Next| {
        Box::pin(async move {
            let session = request
                .extensions()
                .get::<AuthSession>()
                .ok_or(StatusCode::UNAUTHORIZED)?;

            if !has_scope(session, _scope) {
                return Err(StatusCode::FORBIDDEN);
            }

            Ok(next.run(request).await)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::middleware::AuthKind;
    use xlstatus_shared::{UserId, UserRole};

    fn pat_session(scopes: Vec<&str>, server_ids: Option<Vec<&str>>) -> AuthSession {
        AuthSession {
            session_id: "sess".into(),
            user_id: UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()),
            username: "pat".into(),
            role: UserRole::Member,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::PersonalAccessToken,
            scopes: scopes.into_iter().map(|s| s.to_string()).collect(),
            server_ids: server_ids.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
            pat_id: Some("pat-id".into()),
        }
    }

    fn cookie_session() -> AuthSession {
        AuthSession {
            session_id: "sess".into(),
            user_id: UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()),
            username: "u".into(),
            role: UserRole::Admin,
            csrf_token: "csrf".into(),
            auth_kind: AuthKind::Session,
            scopes: vec![],
            server_ids: None,
            pat_id: None,
        }
    }

    // ---- validate_pat_scopes ----

    #[test]
    fn validate_pat_scopes_rejects_empty() {
        let r = validate_pat_scopes(&[], true);
        assert!(r.is_err());
    }

    #[test]
    fn validate_pat_scopes_rejects_bare_wildcard() {
        let s = vec!["*".to_string()];
        assert!(validate_pat_scopes(&s, true).is_err());
    }

    #[test]
    fn validate_pat_scopes_rejects_unknown_scope() {
        let s = vec!["nope:read".to_string()];
        assert!(validate_pat_scopes(&s, true).is_err());
    }

    #[test]
    fn validate_pat_scopes_rejects_malformed_scope() {
        let s = vec!["taskwrite".to_string()];
        assert!(validate_pat_scopes(&s, true).is_err());
    }

    #[test]
    fn validate_pat_scopes_rejects_empty_namespace() {
        let s = vec![":read".to_string()];
        assert!(validate_pat_scopes(&s, true).is_err());
    }

    #[test]
    fn validate_pat_scopes_accepts_known_scope() {
        let s = vec!["task:read".to_string()];
        assert!(validate_pat_scopes(&s, true).is_ok());
    }

    #[test]
    fn validate_pat_scopes_accepts_multiple_known_scopes() {
        let s = vec!["task:read".to_string(), "task:write".to_string()];
        assert!(validate_pat_scopes(&s, true).is_ok());
    }

    #[test]
    fn validate_pat_scopes_admin_star_requires_admin() {
        let s = vec!["admin:*".to_string()];
        assert!(validate_pat_scopes(&s, false).is_err());
        assert!(validate_pat_scopes(&s, true).is_ok());
    }

    // ---- validate_server_ids ----

    #[test]
    fn validate_server_ids_none_is_ok() {
        assert!(validate_server_ids(None).is_ok());
    }

    #[test]
    fn validate_server_ids_empty_list_is_ok() {
        let v: Vec<String> = vec![];
        assert!(validate_server_ids(Some(&v)).is_ok());
    }

    #[test]
    fn validate_server_ids_accepts_valid_uuids() {
        let v = vec!["00000000-0000-0000-0000-000000000001".to_string()];
        assert!(validate_server_ids(Some(&v)).is_ok());
    }

    #[test]
    fn validate_server_ids_rejects_non_uuid() {
        let v = vec!["not-a-uuid".to_string()];
        assert!(validate_server_ids(Some(&v)).is_err());
    }

    // ---- can_access_server / can_access_servers ----

    #[test]
    fn can_access_server_none_allowlist_allows_all() {
        let s = pat_session(vec!["task:read"], None);
        assert!(can_access_server(&s, "anything"));
    }

    #[test]
    fn can_access_server_allowlist_filters() {
        let allow = vec!["aaaa", "bbbb"];
        let s = pat_session(vec!["task:read"], Some(allow));
        assert!(can_access_server(&s, "aaaa"));
        assert!(!can_access_server(&s, "cccc"));
    }

    #[test]
    fn can_access_servers_requires_all_in_allowlist() {
        let allow = vec!["aaaa", "bbbb"];
        let s = pat_session(vec!["task:read"], Some(allow));
        let both = vec!["aaaa".to_string(), "bbbb".to_string()];
        let mixed = vec!["aaaa".to_string(), "cccc".to_string()];
        assert!(can_access_servers(&s, &both));
        assert!(!can_access_servers(&s, &mixed));
    }

    // ---- has_scope ----

    #[test]
    fn has_scope_cookie_session_always_true() {
        let s = cookie_session();
        assert!(has_scope(&s, "task:read"));
        assert!(has_scope(&s, "admin:write"));
    }

    #[test]
    fn has_scope_pat_with_exact_scope() {
        let s = pat_session(vec!["task:read"], None);
        assert!(has_scope(&s, "task:read"));
        assert!(!has_scope(&s, "task:write"));
    }

    #[test]
    fn has_scope_pat_with_namespace_wildcard() {
        let s = pat_session(vec!["task:*"], None);
        assert!(has_scope(&s, "task:read"));
        assert!(has_scope(&s, "task:write"));
        assert!(!has_scope(&s, "nat:read"));
    }

    #[test]
    fn has_scope_pat_with_bare_wildcard() {
        let s = pat_session(vec!["*"], None);
        assert!(has_scope(&s, "task:read"));
        assert!(has_scope(&s, "nat:delete"));
    }

    #[test]
    #[test]
    fn has_scope_pat_admin_star_is_literal_match_only() {
        // admin:* is a literal scope name; it does NOT namespace-wildcard
        // into other domains. Issuance is gated separately by role in
        // validate_pat_scopes; runtime handlers enforce the admin role
        // via require_admin / role.is_admin().
        let mut s = pat_session(vec!["admin:*"], None);
        s.role = UserRole::Admin;
        assert!(has_scope(&s, "admin:anything"));
        assert!(!has_scope(&s, "task:read"));
        assert!(!has_scope(&s, "nat:write"));
    }
}
