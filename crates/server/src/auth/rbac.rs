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

pub const NON_ADMIN_PAT_DENIED_SCOPES: &[&str] = &[
    "server:write",
    "server:delete",
    "server:exec",
    "task:write",
    "task:delete",
    "task:exec",
    "nat:write",
    "nat:delete",
    "transfer:write",
];

pub const SERVER_SCOPED_PAT_NAMESPACES: &[&str] =
    &["server", "service", "alert", "task", "nat", "transfer"];

pub const PAT_RUNTIME_MAX_SCOPES: usize = 64;
pub const PAT_RUNTIME_MAX_SCOPE_BYTES: usize = 128;
pub const PAT_RUNTIME_MAX_SERVER_IDS: usize = 64;
pub const PAT_RUNTIME_MAX_SERVER_ID_BYTES: usize = 64;
pub const PAT_RUNTIME_SCOPES_JSON_MAX_BYTES: usize = 16 * 1024;
pub const PAT_RUNTIME_SERVER_IDS_JSON_MAX_BYTES: usize = 8 * 1024;

fn known_scope_namespace(namespace: &str) -> bool {
    KNOWN_SCOPES.iter().any(|scope| {
        scope
            .split_once(':')
            .map(|(known_namespace, _)| known_namespace == namespace)
            .unwrap_or(false)
    })
}

fn namespace_contains_non_admin_denied_scope(namespace: &str) -> bool {
    NON_ADMIN_PAT_DENIED_SCOPES.iter().any(|scope| {
        scope
            .split_once(':')
            .map(|(denied_namespace, _)| denied_namespace == namespace)
            .unwrap_or(false)
    })
}

/// Validate that a list of PAT scopes only contains known scope names and is
/// well-formed. `admin:*` is only usable by Admin users.
pub fn validate_pat_scopes(scopes: &[String], is_admin: bool) -> Result<(), String> {
    validate_pat_scope_resource_limits(scopes)?;

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

        if !is_admin && NON_ADMIN_PAT_DENIED_SCOPES.contains(&scope.as_str()) {
            return Err(format!("scope {scope} requires admin role"));
        }

        match scope.split_once(':') {
            Some((namespace, action)) => {
                if namespace.is_empty() || action.is_empty() {
                    return Err(format!("invalid scope format: {}", scope));
                }
                if action == "*" {
                    if !known_scope_namespace(namespace) {
                        return Err(format!("unknown scope namespace: {}", namespace));
                    }
                    if !is_admin && namespace_contains_non_admin_denied_scope(namespace) {
                        return Err(format!("scope {scope} includes admin-only permissions"));
                    }
                    continue;
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

fn validate_pat_scope_resource_limits(scopes: &[String]) -> Result<(), String> {
    if scopes.is_empty() {
        return Err("scopes must not be empty".to_string());
    }
    if scopes.len() > PAT_RUNTIME_MAX_SCOPES {
        return Err(format!(
            "scopes must contain at most {PAT_RUNTIME_MAX_SCOPES} items"
        ));
    }

    for scope in scopes {
        if scope.len() > PAT_RUNTIME_MAX_SCOPE_BYTES {
            return Err(format!(
                "scope must be at most {PAT_RUNTIME_MAX_SCOPE_BYTES} bytes"
            ));
        }
    }

    Ok(())
}

pub fn validate_pat_policy_resource_limits(
    scopes: &[String],
    server_ids: Option<&[String]>,
) -> Result<(), String> {
    validate_pat_scope_resource_limits(scopes)?;
    validate_server_ids(server_ids)
}

fn pat_scope_grants(stored_scope: &str, required_scope: &str, is_admin: bool) -> bool {
    if stored_scope == "*" || validate_pat_scopes(&[stored_scope.to_string()], is_admin).is_err() {
        return false;
    }

    if stored_scope == required_scope {
        return true;
    }

    let Some((stored_namespace, stored_action)) = stored_scope.split_once(':') else {
        return false;
    };
    if stored_action != "*" {
        return false;
    }

    required_scope
        .split_once(':')
        .map(|(required_namespace, _)| required_namespace == stored_namespace)
        .unwrap_or(false)
}

pub fn pat_scopes_require_server_allowlist(scopes: &[String]) -> bool {
    scopes.iter().any(|scope| {
        scope
            .split_once(':')
            .map(|(namespace, _)| SERVER_SCOPED_PAT_NAMESPACES.contains(&namespace))
            .unwrap_or(false)
    })
}

pub fn validate_pat_runtime(
    scopes: &[String],
    is_admin: bool,
    server_ids: Option<&[String]>,
) -> Result<(), String> {
    validate_pat_scopes(scopes, is_admin)?;
    validate_server_ids(server_ids)?;

    if !is_admin
        && pat_scopes_require_server_allowlist(scopes)
        && server_ids.map(|ids| ids.is_empty()).unwrap_or(true)
    {
        return Err(
            "non-admin PATs with server-scoped permissions require a non-empty server allowlist"
                .to_string(),
        );
    }

    Ok(())
}

pub fn member_cookie_scope_allowed(required_scope: &str) -> bool {
    matches!(
        required_scope,
        "inventory:read"
            | "server:read"
            | "service:read"
            | "alert:read"
            | "task:read"
            | "ddns:read"
            | "nat:read"
            | "notification:read"
            | "transfer:read"
    )
}

/// Validate that a PAT server allowlist only contains valid UUIDs.
pub fn validate_server_ids(server_ids: Option<&[String]>) -> Result<(), String> {
    let Some(ids) = server_ids else {
        return Ok(());
    };

    if ids.len() > PAT_RUNTIME_MAX_SERVER_IDS {
        return Err(format!(
            "server allowlist must contain at most {PAT_RUNTIME_MAX_SERVER_IDS} items"
        ));
    }

    for id in ids {
        if id.len() > PAT_RUNTIME_MAX_SERVER_ID_BYTES {
            return Err(format!(
                "server id in allowlist must be at most {PAT_RUNTIME_MAX_SERVER_ID_BYTES} bytes"
            ));
        }
        uuid::Uuid::parse_str(id).map_err(|_| format!("invalid server id in allowlist: {}", id))?;
    }

    Ok(())
}

/// Check if a session/pat has a given scope.
/// Admin cookie sessions satisfy every scope; member cookie sessions are
/// limited to read-only scopes; PATs must have the exact scope or a supported
/// namespace scope like `task:*`. A bare `*` is never honored at runtime.
pub fn has_scope(session: &AuthSession, required_scope: &str) -> bool {
    if !matches!(
        session.auth_kind,
        crate::auth::middleware::AuthKind::PersonalAccessToken
    ) {
        return session.role.is_admin() || member_cookie_scope_allowed(required_scope);
    }

    let Some((_namespace, _)) = required_scope.split_once(':') else {
        return session.scopes.iter().any(|s| s == required_scope);
    };

    session
        .scopes
        .iter()
        .any(|s| pat_scope_grants(s, required_scope, session.role.is_admin()))
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

    fn cookie_session(role: UserRole) -> AuthSession {
        AuthSession {
            session_id: "sess".into(),
            user_id: UserId(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap()),
            username: "u".into(),
            role,
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
    fn validate_pat_runtime_resource_limits_are_explicit() {
        assert_eq!(PAT_RUNTIME_MAX_SCOPES, 64);
        assert_eq!(PAT_RUNTIME_MAX_SCOPE_BYTES, 128);
        assert_eq!(PAT_RUNTIME_MAX_SERVER_IDS, 64);
        assert_eq!(PAT_RUNTIME_MAX_SERVER_ID_BYTES, 64);
        assert_eq!(PAT_RUNTIME_SCOPES_JSON_MAX_BYTES, 16 * 1024);
        assert_eq!(PAT_RUNTIME_SERVER_IDS_JSON_MAX_BYTES, 8 * 1024);
    }

    #[test]
    fn validate_pat_runtime_rejects_oversized_scope_lists() {
        let scopes = (0..=PAT_RUNTIME_MAX_SCOPES)
            .map(|_| "notification:read".to_string())
            .collect::<Vec<_>>();

        assert!(validate_pat_runtime(&scopes, true, None).is_err());
    }

    #[test]
    fn validate_pat_runtime_rejects_oversized_scope_values() {
        let scopes = vec!["x".repeat(PAT_RUNTIME_MAX_SCOPE_BYTES + 1)];

        assert!(validate_pat_runtime(&scopes, true, None).is_err());
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
    fn validate_pat_scopes_accepts_admin_namespace_wildcard() {
        let s = vec!["task:*".to_string()];
        assert!(validate_pat_scopes(&s, true).is_ok());
    }

    #[test]
    fn validate_pat_scopes_rejects_non_admin_namespace_wildcard_with_denied_actions() {
        let s = vec!["task:*".to_string()];
        assert!(validate_pat_scopes(&s, false).is_err());
    }

    #[test]
    fn validate_pat_scopes_accepts_non_admin_read_only_namespace_wildcard() {
        let s = vec!["notification:*".to_string()];
        assert!(validate_pat_scopes(&s, false).is_ok());
    }

    #[test]
    fn validate_pat_scopes_rejects_unknown_namespace_wildcard() {
        let s = vec!["unknown:*".to_string()];
        assert!(validate_pat_scopes(&s, true).is_err());
    }

    #[test]
    fn validate_pat_scopes_admin_star_requires_admin() {
        let s = vec!["admin:*".to_string()];
        assert!(validate_pat_scopes(&s, false).is_err());
        assert!(validate_pat_scopes(&s, true).is_ok());
    }

    #[test]
    fn validate_pat_scopes_rejects_high_risk_scope_for_non_admin() {
        let s = vec!["server:exec".to_string()];
        assert!(validate_pat_scopes(&s, false).is_err());
        assert!(validate_pat_scopes(&s, true).is_ok());
    }

    #[test]
    fn pat_scopes_require_server_allowlist_for_server_scoped_scopes() {
        let server_scoped = vec!["server:read".to_string(), "task:read".to_string()];
        let global = vec!["notification:read".to_string()];
        assert!(pat_scopes_require_server_allowlist(&server_scoped));
        assert!(!pat_scopes_require_server_allowlist(&global));
    }

    #[test]
    fn validate_pat_runtime_rejects_legacy_bare_wildcard() {
        let scopes = vec!["*".to_string()];
        assert!(validate_pat_runtime(&scopes, true, None).is_err());
    }

    #[test]
    fn validate_pat_runtime_rejects_legacy_non_admin_denied_scope() {
        let scopes = vec!["server:exec".to_string()];
        let server_ids = vec!["00000000-0000-0000-0000-000000000001".to_string()];
        assert!(validate_pat_runtime(&scopes, false, Some(&server_ids)).is_err());
    }

    #[test]
    fn validate_pat_runtime_rejects_non_admin_server_scoped_without_allowlist() {
        let scopes = vec!["server:read".to_string()];
        assert!(validate_pat_runtime(&scopes, false, None).is_err());
    }

    #[test]
    fn validate_pat_runtime_rejects_oversized_server_allowlists() {
        let scopes = vec!["notification:read".to_string()];
        let server_ids = (0..=PAT_RUNTIME_MAX_SERVER_IDS)
            .map(|idx| uuid::Uuid::from_u128(idx as u128 + 1).to_string())
            .collect::<Vec<_>>();

        assert!(validate_pat_runtime(&scopes, true, Some(&server_ids)).is_err());
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

    #[test]
    fn validate_server_ids_rejects_oversized_id_text() {
        let v = vec!["a".repeat(PAT_RUNTIME_MAX_SERVER_ID_BYTES + 1)];
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
    fn has_scope_admin_cookie_session_allows_all() {
        let s = cookie_session(UserRole::Admin);
        assert!(has_scope(&s, "task:read"));
        assert!(has_scope(&s, "admin:write"));
    }

    #[test]
    fn has_scope_member_cookie_session_allows_only_read_scopes() {
        let s = cookie_session(UserRole::Member);
        assert!(has_scope(&s, "task:read"));
        assert!(has_scope(&s, "server:read"));
        assert!(!has_scope(&s, "task:exec"));
        assert!(!has_scope(&s, "server:write"));
        assert!(!has_scope(&s, "admin:write"));
    }

    #[test]
    fn has_scope_pat_with_exact_scope() {
        let s = pat_session(vec!["task:read"], None);
        assert!(has_scope(&s, "task:read"));
        assert!(!has_scope(&s, "task:write"));
    }

    #[test]
    fn has_scope_admin_pat_with_namespace_wildcard() {
        let mut s = pat_session(vec!["task:*"], None);
        s.role = UserRole::Admin;
        assert!(has_scope(&s, "task:read"));
        assert!(has_scope(&s, "task:write"));
        assert!(!has_scope(&s, "nat:read"));
    }

    #[test]
    fn has_scope_non_admin_pat_rejects_denied_namespace_wildcard_at_runtime() {
        let s = pat_session(vec!["task:*"], None);
        assert!(!has_scope(&s, "task:read"));
        assert!(!has_scope(&s, "task:write"));
    }

    #[test]
    fn has_scope_pat_rejects_bare_wildcard_at_runtime() {
        let s = pat_session(vec!["*"], None);
        assert!(!has_scope(&s, "task:read"));
        assert!(!has_scope(&s, "nat:delete"));
    }

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
