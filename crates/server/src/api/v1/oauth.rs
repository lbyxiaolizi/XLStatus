//! Generic OAuth2/OIDC login and account binding.

use crate::api::types::{ApiResponse, UserInfo};
use crate::api::v1::auth::{
    active_waf_ban, cookie_secure_attr, record_waf_event, register_oauth_failure, AppError,
    AppState,
};
use crate::auth::middleware::{derive_csrf_token, AuthUser, CSRF_COOKIE_NAME, SESSION_COOKIE_NAME};
use crate::auth::{generate_session_token, hash_token, SessionRepository};
use crate::config::OidcProviderConfig;
use crate::db::{CreateSessionInput, DatabaseBackend, UserRepository};
use crate::security::{secure_reqwest_client_builder, validate_outbound_url_resolved};
use axum::{
    extract::{connect_info::ConnectInfo, Path, Query, State},
    http::{header, HeaderMap, HeaderValue, Uri},
    response::{AppendHeaders, IntoResponse, Redirect, Response},
    Json,
};
use axum_extra::extract::CookieJar;
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha2::Sha256;
use sqlx::Row;
use std::net::SocketAddr;
use xlstatus_shared::UserId;

type HmacSha256 = Hmac<Sha256>;

const OAUTH_STATE_COOKIE_NAME: &str = "xlstatus_oauth_state";
const OAUTH_STATE_COOKIE_PATH: &str = "/api/v1/oauth2";
const OAUTH_STATE_TTL_SECONDS: i64 = 10 * 60;
const OAUTH_MAX_PROVIDER_ID_BYTES: usize = 64;
const OAUTH_MAX_QUERY_BYTES: usize = 16 * 1024;
const OAUTH_MAX_STATE_BYTES: usize = 4096;
const OAUTH_MAX_RETURN_TO_BYTES: usize = 1024;
const OAUTH_MAX_CODE_BYTES: usize = 4096;
const OAUTH_MAX_ERROR_BYTES: usize = 1024;
const OAUTH_MAX_TOKEN_RESPONSE_BYTES: usize = 16 * 1024;
const OAUTH_MAX_USERINFO_RESPONSE_BYTES: usize = 64 * 1024;
const OAUTH_MAX_ACCESS_TOKEN_BYTES: usize = 8192;
const OAUTH_MAX_CLAIM_BYTES: usize = 1024;
const OAUTH_ALLOW_USERINFO_QUERY_TOKEN_ENV: &str = "XLSTATUS_ALLOW_OIDC_USERINFO_QUERY_TOKEN";

#[derive(Debug, Serialize)]
pub struct OAuthProviderView {
    pub id: String,
    pub display_name: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct OAuthProviderListResponse {
    pub providers: Vec<OAuthProviderView>,
}

#[derive(Debug, Serialize)]
pub struct OAuthAccountView {
    pub provider: String,
    pub provider_display_name: String,
    pub subject: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct OAuthAccountListResponse {
    pub accounts: Vec<OAuthAccountView>,
}

#[derive(Debug, Serialize)]
pub struct OAuthStartResponse {
    pub authorization_url: String,
}

#[derive(Debug, Deserialize)]
pub struct OAuthStartQuery {
    pub return_to: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: Option<String>,
    pub state: String,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OAuthState {
    provider: String,
    flow: OAuthFlow,
    user_id: Option<String>,
    return_to: String,
    nonce: String,
    exp: i64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OAuthFlow {
    Login,
    Bind,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OidcUserInfo {
    sub: String,
    email: Option<String>,
    name: Option<String>,
    preferred_username: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenAuthMethod {
    ClientSecretPost,
    ClientSecretBasic,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserinfoAuthMethod {
    Bearer,
    Query,
    None,
}

pub async fn list_oauth_providers(
    State(state): State<AppState>,
) -> Json<ApiResponse<OAuthProviderListResponse>> {
    Json(ApiResponse::success(OAuthProviderListResponse {
        providers: state
            .config
            .oauth2
            .providers
            .iter()
            .map(provider_view)
            .collect(),
    }))
}

pub async fn start_oauth_login(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    uri: Uri,
) -> Result<Response, AppError> {
    let provider_id = parse_oauth_provider_path(&uri, false)?;
    let query = parse_oauth_start_query(&uri)?;
    let client_ip = crate::security::client_ip_from_headers_and_peer(&headers, Some(peer_addr));
    if active_waf_ban(&state.db, &client_ip).await?.is_some() {
        record_waf_event(
            &state.db,
            &client_ip,
            Some(&provider_id),
            "oauth_blocked",
            Some("active WAF ban"),
        )
        .await?;
        return Err(AppError::Forbidden(
            "IP temporarily blocked by WAF".to_string(),
        ));
    }
    let provider = oauth_provider(&state, &provider_id)?;
    let oauth_state = OAuthState {
        provider: provider.id.clone(),
        flow: OAuthFlow::Login,
        user_id: None,
        return_to: sanitize_return_to(query.return_to),
        nonce: random_nonce(),
        exp: (Utc::now() + Duration::minutes(10)).timestamp(),
    };
    oauth_start_response(&state, provider, &oauth_state)
}

pub async fn start_oauth_bind(
    State(state): State<AppState>,
    auth_user: AuthUser,
    uri: Uri,
) -> Result<Response, AppError> {
    let provider_id = parse_oauth_provider_path(&uri, true)?;
    let query = parse_oauth_start_query(&uri)?;
    require_cookie_session(&auth_user)?;
    let provider = oauth_provider(&state, &provider_id)?;
    let oauth_state = OAuthState {
        provider: provider.id.clone(),
        flow: OAuthFlow::Bind,
        user_id: Some(auth_user.user.id.0.to_string()),
        return_to: sanitize_return_to(query.return_to),
        nonce: random_nonce(),
        exp: (Utc::now() + Duration::minutes(10)).timestamp(),
    };
    oauth_start_json_response(&state, provider, &oauth_state)
}

pub async fn list_oauth_bindings(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Result<Json<ApiResponse<OAuthAccountListResponse>>, AppError> {
    require_cookie_session(&auth_user)?;
    Ok(Json(ApiResponse::success(OAuthAccountListResponse {
        accounts: load_oauth_accounts(&state, auth_user.user.id).await?,
    })))
}

pub async fn unbind_oauth_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> Result<Json<ApiResponse<()>>, AppError> {
    require_cookie_session(&auth_user)?;
    let provider_id = normalize_oauth_provider_id(&provider_id)?;
    delete_oauth_account(&state.db, auth_user.user.id, &provider_id).await?;
    Ok(Json(ApiResponse::success(())))
}

pub async fn oauth_callback(
    State(state): State<AppState>,
    ConnectInfo(peer_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    cookie_jar: CookieJar,
    uri: Uri,
) -> Result<Response, AppError> {
    let query = parse_oauth_callback_query(&uri)?;
    let client_ip = crate::security::client_ip_from_headers_and_peer(&headers, Some(peer_addr));
    if active_waf_ban(&state.db, &client_ip).await?.is_some() {
        record_waf_event(
            &state.db,
            &client_ip,
            None,
            "oauth_blocked",
            Some("active WAF ban"),
        )
        .await?;
        return Err(AppError::Forbidden(
            "IP temporarily blocked by WAF".to_string(),
        ));
    }
    if let Some(code) = query.code.as_deref() {
        ensure_oauth_text_size(code, 1, OAUTH_MAX_CODE_BYTES, "OAuth code")?;
    }
    if let Some(error) = query.error.as_deref() {
        ensure_oauth_text_size(error, 1, OAUTH_MAX_ERROR_BYTES, "OAuth error")?;
    }
    if let Some(error_description) = query.error_description.as_deref() {
        ensure_oauth_text_size(
            error_description,
            0,
            OAUTH_MAX_ERROR_BYTES,
            "OAuth error_description",
        )?;
    }
    let oauth_state = match decode_state(&state.config.security.session_secret, &query.state) {
        Ok(state) => state,
        Err(err) => {
            register_oauth_failure(&state.db, &client_ip, None, "invalid state").await?;
            return Err(err);
        }
    };
    if let Err(err) = oauth_state_cookie_matches(&cookie_jar, &query.state) {
        register_oauth_failure(
            &state.db,
            &client_ip,
            Some(&oauth_state.provider),
            match &err {
                AppError::Unauthorized(_) => "oauth state cookie missing",
                AppError::Forbidden(_) => "oauth state cookie mismatch",
                _ => "oauth state cookie invalid",
            },
        )
        .await?;
        return Err(err);
    }
    if let Some(error) = query.error {
        let message = query.error_description.unwrap_or(error);
        register_oauth_failure(&state.db, &client_ip, Some(&oauth_state.provider), &message)
            .await?;
        return oauth_redirect_with_state_cookie_clear(&state, &oauth_state, false, &message);
    }

    let provider = oauth_provider(&state, &oauth_state.provider)?;
    let bind_user_id = if oauth_state.flow == OAuthFlow::Bind {
        let user_id = oauth_state
            .user_id
            .as_deref()
            .ok_or(AppError::BadRequest("OAuth bind state missing user".into()))
            .and_then(parse_user_id)?;
        let cookie_user_id = cookie_session_user_id(&state, &cookie_jar).await?;
        if let Err(err) = bind_cookie_session_matches(user_id, cookie_user_id) {
            register_oauth_failure(
                &state.db,
                &client_ip,
                Some(&oauth_state.provider),
                match &err {
                    AppError::Forbidden(_) => "bind session user mismatch",
                    AppError::Unauthorized(_) => "bind session missing",
                    _ => "bind session invalid",
                },
            )
            .await?;
            return Err(err);
        }
        Some(user_id)
    } else {
        None
    };
    let code = query
        .code
        .ok_or(AppError::BadRequest("OAuth callback missing code".into()));
    let code = match code {
        Ok(code) => code,
        Err(err) => {
            register_oauth_failure(
                &state.db,
                &client_ip,
                Some(&oauth_state.provider),
                "missing code",
            )
            .await?;
            return Err(err);
        }
    };
    let token = match exchange_code(provider, &code).await {
        Ok(token) => token,
        Err(err) => {
            register_oauth_failure(
                &state.db,
                &client_ip,
                Some(&oauth_state.provider),
                "token exchange failed",
            )
            .await?;
            return Err(err);
        }
    };
    let userinfo = match fetch_userinfo(provider, &token.access_token).await {
        Ok(userinfo) => userinfo,
        Err(err) => {
            register_oauth_failure(
                &state.db,
                &client_ip,
                Some(&oauth_state.provider),
                "userinfo request failed",
            )
            .await?;
            return Err(err);
        }
    };

    match oauth_state.flow {
        OAuthFlow::Bind => {
            let user_id =
                bind_user_id.ok_or(AppError::BadRequest("OAuth bind state missing user".into()))?;
            bind_oauth_account(&state.db, user_id, provider, &userinfo).await?;
            oauth_redirect_with_state_cookie_clear(&state, &oauth_state, true, "oauth_bound")
        }
        OAuthFlow::Login => {
            let Some(user_id) = find_bound_user(&state.db, provider, &userinfo.sub).await? else {
                register_oauth_failure(
                    &state.db,
                    &client_ip,
                    Some(&oauth_state.provider),
                    "account not bound",
                )
                .await?;
                return oauth_redirect_with_state_cookie_clear(
                    &state,
                    &oauth_state,
                    false,
                    "oauth_account_not_bound",
                );
            };
            let user_repo = UserRepository::new(state.db.clone());
            let user = user_repo
                .find_by_id(user_id)
                .await?
                .ok_or(AppError::Unauthorized(
                    "OAuth account user not found".into(),
                ))?;
            let (session_cookie, csrf_cookie) =
                create_oauth_session(&state, &headers, peer_addr, user.id).await?;
            record_waf_event(
                &state.db,
                &client_ip,
                Some(&oauth_state.provider),
                "oauth_success",
                None,
            )
            .await?;
            Ok((
                AppendHeaders([
                    (header::SET_COOKIE, session_cookie),
                    (header::SET_COOKIE, csrf_cookie),
                    (
                        header::SET_COOKIE,
                        oauth_state_cookie_clear_header(state.config.security.cookie_secure)?,
                    ),
                ]),
                Redirect::temporary(&frontend_redirect(
                    &state,
                    &oauth_state,
                    true,
                    "oauth_login",
                )?),
            )
                .into_response())
        }
    }
}

pub async fn get_profile(auth_user: AuthUser) -> Result<Json<ApiResponse<UserInfo>>, AppError> {
    Ok(Json(ApiResponse::success(UserInfo {
        id: auth_user.user.id.0.to_string(),
        username: auth_user.user.username,
        role: auth_user.user.role.to_string(),
        created_at: Some(auth_user.user.created_at.to_rfc3339()),
        updated_at: Some(auth_user.user.updated_at.to_rfc3339()),
    })))
}

fn require_cookie_session(auth_user: &AuthUser) -> Result<(), AppError> {
    if auth_user.is_pat() {
        return Err(AppError::Forbidden("Cookie session required".into()));
    }
    Ok(())
}

fn oauth_start_response(
    state: &AppState,
    provider: &OidcProviderConfig,
    oauth_state: &OAuthState,
) -> Result<Response, AppError> {
    let encoded_state = encode_state(&state.config.security.session_secret, oauth_state)?;
    Ok((
        AppendHeaders([(
            header::SET_COOKIE,
            oauth_state_cookie_header(state.config.security.cookie_secure, &encoded_state)?,
        )]),
        Redirect::temporary(&authorize_url(
            provider,
            &encoded_state,
            &oauth_state.nonce,
        )?),
    )
        .into_response())
}

fn oauth_start_json_response(
    state: &AppState,
    provider: &OidcProviderConfig,
    oauth_state: &OAuthState,
) -> Result<Response, AppError> {
    let encoded_state = encode_state(&state.config.security.session_secret, oauth_state)?;
    Ok((
        AppendHeaders([(
            header::SET_COOKIE,
            oauth_state_cookie_header(state.config.security.cookie_secure, &encoded_state)?,
        )]),
        Json(ApiResponse::success(OAuthStartResponse {
            authorization_url: authorize_url(provider, &encoded_state, &oauth_state.nonce)?,
        })),
    )
        .into_response())
}

fn oauth_redirect_with_state_cookie_clear(
    state: &AppState,
    oauth_state: &OAuthState,
    success: bool,
    message: &str,
) -> Result<Response, AppError> {
    Ok((
        AppendHeaders([(
            header::SET_COOKIE,
            oauth_state_cookie_clear_header(state.config.security.cookie_secure)?,
        )]),
        Redirect::temporary(&frontend_redirect(state, oauth_state, success, message)?),
    )
        .into_response())
}

fn oauth_state_cookie_header(
    cookie_secure: bool,
    encoded_state: &str,
) -> Result<HeaderValue, AppError> {
    let secure_attr = cookie_secure_attr(cookie_secure);
    HeaderValue::from_str(&format!(
        "{}={}; HttpOnly; SameSite=Lax; Path={}; Max-Age={}{}",
        OAUTH_STATE_COOKIE_NAME,
        oauth_state_cookie_value(encoded_state),
        OAUTH_STATE_COOKIE_PATH,
        OAUTH_STATE_TTL_SECONDS,
        secure_attr
    ))
    .map_err(|e| AppError::BadRequest(format!("Invalid OAuth state cookie: {e}")))
}

fn oauth_state_cookie_clear_header(cookie_secure: bool) -> Result<HeaderValue, AppError> {
    let secure_attr = cookie_secure_attr(cookie_secure);
    HeaderValue::from_str(&format!(
        "{}=; HttpOnly; SameSite=Lax; Path={}; Max-Age=0{}",
        OAUTH_STATE_COOKIE_NAME, OAUTH_STATE_COOKIE_PATH, secure_attr
    ))
    .map_err(|e| AppError::BadRequest(format!("Invalid OAuth state cookie: {e}")))
}

fn oauth_state_cookie_matches(cookie_jar: &CookieJar, encoded_state: &str) -> Result<(), AppError> {
    let expected = oauth_state_cookie_value(encoded_state);
    let Some(cookie) = cookie_jar.get(OAUTH_STATE_COOKIE_NAME) else {
        return Err(AppError::Unauthorized(
            "OAuth state cookie is missing".into(),
        ));
    };
    if constant_time_eq(cookie.value().as_bytes(), expected.as_bytes()) {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "OAuth state cookie does not match".into(),
        ))
    }
}

fn oauth_state_cookie_value(encoded_state: &str) -> String {
    hash_token(encoded_state)
}

fn provider_view(provider: &OidcProviderConfig) -> OAuthProviderView {
    OAuthProviderView {
        id: provider.id.clone(),
        display_name: provider
            .display_name
            .clone()
            .unwrap_or_else(|| provider.id.clone()),
        scopes: provider.scopes.clone(),
    }
}

fn oauth_provider<'a>(
    state: &'a AppState,
    provider_id: &str,
) -> Result<&'a OidcProviderConfig, AppError> {
    let provider_id = normalize_oauth_provider_id(provider_id)?;
    state
        .config
        .oauth2
        .provider(&provider_id)
        .ok_or(AppError::NotFound("OAuth provider not found".into()))
}

fn parse_oauth_provider_path(uri: &Uri, bind: bool) -> Result<String, AppError> {
    let path = uri.path();
    let Some(rest) = path.strip_prefix("/api/v1/oauth2/") else {
        return Err(AppError::BadRequest(
            "OAuth provider path is invalid".into(),
        ));
    };
    let provider_id = if bind {
        rest.strip_suffix("/bind")
            .ok_or_else(|| AppError::BadRequest("OAuth provider path is invalid".into()))?
    } else {
        rest
    };
    if provider_id.contains('/') {
        return Err(AppError::BadRequest(
            "OAuth provider path is invalid".into(),
        ));
    }
    normalize_oauth_provider_id(provider_id)
}

fn normalize_oauth_provider_id(value: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest("OAuth provider is required".into()));
    }
    if value.len() > OAUTH_MAX_PROVIDER_ID_BYTES {
        return Err(AppError::BadRequest(format!(
            "OAuth provider must be at most {OAUTH_MAX_PROVIDER_ID_BYTES} bytes"
        )));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(AppError::BadRequest(
            "OAuth provider contains invalid characters".into(),
        ));
    }
    Ok(value.to_string())
}

fn authorize_url(
    provider: &OidcProviderConfig,
    state: &str,
    nonce: &str,
) -> Result<String, AppError> {
    let scope = provider.scopes.join(" ");
    let mut url = reqwest::Url::parse_with_params(
        &provider.auth_url,
        [
            ("response_type", "code"),
            ("client_id", provider.client_id.as_str()),
            ("redirect_uri", provider.redirect_url.as_str()),
            ("scope", scope.as_str()),
            ("state", state),
            ("nonce", nonce),
        ],
    )
    .map_err(|e| AppError::BadRequest(format!("invalid OAuth auth URL: {e}")))?;
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in &provider.extra_auth_params {
            let key = key.trim();
            if key.is_empty() || is_reserved_auth_param(key) {
                continue;
            }
            pairs.append_pair(key, value);
        }
    }
    Ok(url.to_string())
}

async fn exchange_code(
    provider: &OidcProviderConfig,
    code: &str,
) -> Result<TokenResponse, AppError> {
    let validated = validate_outbound_url_resolved(&provider.token_url, "OAuth token endpoint")
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let token_url = validated.url.clone();
    let client = secure_reqwest_client_builder(&validated)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::BadRequest(format!("OAuth token client init failed: {e}")))?;
    let token_auth_method = parse_token_auth_method(&provider.token_auth_method)?;
    if matches!(
        token_auth_method,
        TokenAuthMethod::ClientSecretPost | TokenAuthMethod::ClientSecretBasic
    ) && provider.client_secret.trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "OAuth client_secret is required for the configured token auth method".into(),
        ));
    }

    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", provider.redirect_url.as_str()),
    ];
    match token_auth_method {
        TokenAuthMethod::ClientSecretPost => {
            form.push(("client_id", provider.client_id.as_str()));
            form.push(("client_secret", provider.client_secret.as_str()));
        }
        TokenAuthMethod::ClientSecretBasic => {}
        TokenAuthMethod::None => {
            form.push(("client_id", provider.client_id.as_str()));
        }
    }

    let request = client.post(token_url).form(&form);
    let request = match token_auth_method {
        TokenAuthMethod::ClientSecretBasic => {
            request.basic_auth(&provider.client_id, Some(&provider.client_secret))
        }
        TokenAuthMethod::ClientSecretPost | TokenAuthMethod::None => request,
    };
    let response = request
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("OAuth token request failed: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "OAuth token request failed with {}",
            response.status()
        )));
    }
    let token = parse_limited_json_response::<TokenResponse>(
        response,
        OAUTH_MAX_TOKEN_RESPONSE_BYTES,
        "OAuth token response",
    )
    .await?;
    ensure_oauth_text_size(
        &token.access_token,
        1,
        OAUTH_MAX_ACCESS_TOKEN_BYTES,
        "OAuth access_token",
    )?;
    Ok(token)
}

async fn fetch_userinfo(
    provider: &OidcProviderConfig,
    access_token: &str,
) -> Result<OidcUserInfo, AppError> {
    let userinfo_auth_method = parse_userinfo_auth_method(&provider.userinfo_auth_method)?;
    ensure_userinfo_auth_method_allowed(userinfo_auth_method, allow_oidc_userinfo_query_token())?;
    let validated =
        validate_outbound_url_resolved(&provider.userinfo_url, "OIDC userinfo endpoint")
            .await
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let mut url = validated.url.clone();
    if matches!(userinfo_auth_method, UserinfoAuthMethod::Query) {
        url.query_pairs_mut()
            .append_pair("access_token", access_token);
    }
    let client = secure_reqwest_client_builder(&validated)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::BadRequest(format!("OIDC userinfo client init failed: {e}")))?;
    let request = client.get(url);
    let request = match userinfo_auth_method {
        UserinfoAuthMethod::Bearer => request.bearer_auth(access_token),
        UserinfoAuthMethod::Query | UserinfoAuthMethod::None => request,
    };
    let response = request
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("OIDC userinfo request failed: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "OIDC userinfo request failed with {}",
            response.status()
        )));
    }
    let raw = parse_limited_json_response::<JsonValue>(
        response,
        OAUTH_MAX_USERINFO_RESPONSE_BYTES,
        "OIDC userinfo response",
    )
    .await?;
    normalize_userinfo(provider, &raw)
}

async fn parse_limited_json_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    max_bytes: usize,
    label: &str,
) -> Result<T, AppError> {
    if response
        .content_length()
        .map(|length| length > max_bytes as u64)
        .unwrap_or(false)
    {
        return Err(AppError::BadRequest(format!(
            "{label} exceeds {max_bytes} bytes"
        )));
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| AppError::BadRequest(format!("{label} read failed: {e}")))?
    {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(AppError::BadRequest(format!(
                "{label} exceeds {max_bytes} bytes"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    parse_limited_json_bytes(&bytes, max_bytes, label)
}

fn parse_limited_json_bytes<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    max_bytes: usize,
    label: &str,
) -> Result<T, AppError> {
    if bytes.len() > max_bytes {
        return Err(AppError::BadRequest(format!(
            "{label} exceeds {max_bytes} bytes"
        )));
    }
    serde_json::from_slice(bytes)
        .map_err(|e| AppError::BadRequest(format!("{label} is invalid: {e}")))
}

fn normalize_userinfo(
    provider: &OidcProviderConfig,
    raw: &JsonValue,
) -> Result<OidcUserInfo, AppError> {
    let sub = userinfo_field(raw, &provider.subject_field, "OIDC subject")?
        .ok_or(AppError::BadRequest("OIDC userinfo missing subject".into()))?;
    let userinfo = OidcUserInfo {
        sub,
        email: userinfo_field(raw, &provider.email_field, "OIDC email")?,
        name: userinfo_field(raw, &provider.name_field, "OIDC name")?,
        preferred_username: userinfo_field(raw, &provider.username_field, "OIDC username")?,
    };
    if userinfo.sub.trim().is_empty() {
        return Err(AppError::BadRequest("OIDC userinfo missing subject".into()));
    }
    Ok(userinfo)
}

fn userinfo_field(raw: &JsonValue, field: &str, label: &str) -> Result<Option<String>, AppError> {
    let field = field.trim();
    if field.is_empty() {
        return Ok(None);
    }
    let value = if field.starts_with('/') {
        match raw.pointer(field) {
            Some(value) => value,
            None => return Ok(None),
        }
    } else {
        let mut current = raw;
        for part in field.split('.') {
            let part = part.trim();
            if part.is_empty() {
                return Ok(None);
            }
            current = match current.get(part) {
                Some(value) => value,
                None => return Ok(None),
            };
        }
        current
    };
    match json_scalar(value) {
        Some(value) => normalize_oauth_claim(value, label).map(Some),
        None => Ok(None),
    }
}

fn json_scalar(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        JsonValue::Number(value) => Some(value.to_string()),
        JsonValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn normalize_oauth_claim(value: String, label: &str) -> Result<String, AppError> {
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::BadRequest(format!("{label} is empty")));
    }
    ensure_oauth_text_size(&trimmed, 1, OAUTH_MAX_CLAIM_BYTES, label)?;
    Ok(trimmed)
}

fn parse_token_auth_method(value: &str) -> Result<TokenAuthMethod, AppError> {
    match normalize_method_name(value).as_str() {
        "client_secret_post" | "post" => Ok(TokenAuthMethod::ClientSecretPost),
        "client_secret_basic" | "basic" => Ok(TokenAuthMethod::ClientSecretBasic),
        "none" | "public" => Ok(TokenAuthMethod::None),
        _ => Err(AppError::BadRequest(format!(
            "unsupported OAuth token_auth_method: {value}"
        ))),
    }
}

fn parse_userinfo_auth_method(value: &str) -> Result<UserinfoAuthMethod, AppError> {
    match normalize_method_name(value).as_str() {
        "bearer" | "bearer_header" | "authorization_header" => Ok(UserinfoAuthMethod::Bearer),
        "query" | "access_token_query" => Ok(UserinfoAuthMethod::Query),
        "none" => Ok(UserinfoAuthMethod::None),
        _ => Err(AppError::BadRequest(format!(
            "unsupported OIDC userinfo_auth_method: {value}"
        ))),
    }
}

fn ensure_userinfo_auth_method_allowed(
    method: UserinfoAuthMethod,
    allow_query_token: bool,
) -> Result<(), AppError> {
    if matches!(method, UserinfoAuthMethod::Query) && !allow_query_token {
        return Err(AppError::BadRequest(
            "OIDC userinfo query token auth is disabled".into(),
        ));
    }
    Ok(())
}

fn allow_oidc_userinfo_query_token() -> bool {
    let value = std::env::var(OAUTH_ALLOW_USERINFO_QUERY_TOKEN_ENV).ok();
    allow_oidc_userinfo_query_token_value(value.as_deref())
}

fn allow_oidc_userinfo_query_token_value(value: Option<&str>) -> bool {
    value
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes"
            )
        })
        .unwrap_or(false)
}

fn normalize_method_name(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace(['-', ' '], "_")
}

fn is_reserved_auth_param(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "response_type" | "client_id" | "redirect_uri" | "scope" | "state" | "nonce"
    )
}

async fn load_oauth_accounts(
    state: &AppState,
    user_id: UserId,
) -> Result<Vec<OAuthAccountView>, AppError> {
    let accounts = match &state.db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT provider, subject, email, display_name, created_at, updated_at
                FROM oauth_accounts
                WHERE user_id = ?
                ORDER BY provider ASC
                "#,
            )
            .bind(user_id.0.to_string())
            .fetch_all(pool)
            .await?;
            rows.into_iter()
                .map(|row| {
                    oauth_account_view(
                        state,
                        row.get("provider"),
                        row.get("subject"),
                        row.get("email"),
                        row.get("display_name"),
                        row.get("created_at"),
                        row.get("updated_at"),
                    )
                })
                .collect()
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                r#"
                SELECT
                    provider,
                    subject,
                    email,
                    display_name,
                    created_at::text AS created_at,
                    updated_at::text AS updated_at
                FROM oauth_accounts
                WHERE user_id = $1
                ORDER BY provider ASC
                "#,
            )
            .bind(user_id.0)
            .fetch_all(pool)
            .await?;
            rows.into_iter()
                .map(|row| {
                    oauth_account_view(
                        state,
                        row.get("provider"),
                        row.get("subject"),
                        row.get("email"),
                        row.get("display_name"),
                        row.get("created_at"),
                        row.get("updated_at"),
                    )
                })
                .collect()
        }
    };
    Ok(accounts)
}

fn oauth_account_view(
    state: &AppState,
    provider: String,
    subject: String,
    email: Option<String>,
    display_name: Option<String>,
    created_at: String,
    updated_at: String,
) -> OAuthAccountView {
    let provider_display_name = state
        .config
        .oauth2
        .provider(&provider)
        .and_then(|provider| provider.display_name.clone())
        .unwrap_or_else(|| provider.clone());
    OAuthAccountView {
        provider,
        provider_display_name,
        subject,
        email,
        display_name,
        created_at,
        updated_at,
    }
}

async fn find_bound_user(
    db: &DatabaseBackend,
    provider: &OidcProviderConfig,
    subject: &str,
) -> Result<Option<UserId>, AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let row: Option<(String,)> = sqlx::query_as(
                "SELECT user_id FROM oauth_accounts WHERE provider = ? AND subject = ?",
            )
            .bind(&provider.id)
            .bind(subject)
            .fetch_optional(pool)
            .await?;
            row.map(|(id,)| parse_user_id(&id)).transpose()
        }
        DatabaseBackend::Postgres(pool) => {
            let row: Option<(uuid::Uuid,)> = sqlx::query_as(
                "SELECT user_id FROM oauth_accounts WHERE provider = $1 AND subject = $2",
            )
            .bind(&provider.id)
            .bind(subject)
            .fetch_optional(pool)
            .await?;
            Ok(row.map(|(id,)| UserId(id)))
        }
    }
}

async fn bind_oauth_account(
    db: &DatabaseBackend,
    user_id: UserId,
    provider: &OidcProviderConfig,
    userinfo: &OidcUserInfo,
) -> Result<(), AppError> {
    let id = uuid::Uuid::now_v7();
    let now = Utc::now();
    let display_name = userinfo
        .name
        .as_deref()
        .or(userinfo.preferred_username.as_deref());
    match db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query("DELETE FROM oauth_accounts WHERE provider = ? AND user_id = ?")
                .bind(&provider.id)
                .bind(user_id.0.to_string())
                .execute(pool)
                .await?;
            sqlx::query(
                r#"
                INSERT INTO oauth_accounts (id, user_id, provider, subject, email, display_name, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(provider, subject) DO UPDATE SET
                    user_id = excluded.user_id,
                    email = excluded.email,
                    display_name = excluded.display_name,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(id.to_string())
            .bind(user_id.0.to_string())
            .bind(&provider.id)
            .bind(&userinfo.sub)
            .bind(&userinfo.email)
            .bind(display_name)
            .bind(now.to_rfc3339())
            .bind(now.to_rfc3339())
            .execute(pool)
            .await?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query("DELETE FROM oauth_accounts WHERE provider = $1 AND user_id = $2")
                .bind(&provider.id)
                .bind(user_id.0)
                .execute(pool)
                .await?;
            sqlx::query(
                r#"
                INSERT INTO oauth_accounts (id, user_id, provider, subject, email, display_name, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                ON CONFLICT(provider, subject) DO UPDATE SET
                    user_id = excluded.user_id,
                    email = excluded.email,
                    display_name = excluded.display_name,
                    updated_at = excluded.updated_at
                "#,
            )
            .bind(id)
            .bind(user_id.0)
            .bind(&provider.id)
            .bind(&userinfo.sub)
            .bind(&userinfo.email)
            .bind(display_name)
            .bind(now)
            .bind(now)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

async fn delete_oauth_account(
    db: &DatabaseBackend,
    user_id: UserId,
    provider_id: &str,
) -> Result<(), AppError> {
    match db {
        DatabaseBackend::Sqlite(pool) => {
            sqlx::query("DELETE FROM oauth_accounts WHERE provider = ? AND user_id = ?")
                .bind(provider_id)
                .bind(user_id.0.to_string())
                .execute(pool)
                .await?;
        }
        DatabaseBackend::Postgres(pool) => {
            sqlx::query("DELETE FROM oauth_accounts WHERE provider = $1 AND user_id = $2")
                .bind(provider_id)
                .bind(user_id.0)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

async fn create_oauth_session(
    state: &AppState,
    headers: &HeaderMap,
    peer_addr: SocketAddr,
    user_id: UserId,
) -> Result<(HeaderValue, HeaderValue), AppError> {
    let session_token = generate_session_token();
    let token_hash = hash_token(&session_token);
    let session_repo = crate::auth::SessionRepository::new(state.db.clone());
    let expires_at = Utc::now() + Duration::hours(state.config.security.session_ttl_hours);
    session_repo
        .create(
            CreateSessionInput {
                user_id,
                ip: Some(client_ip_from_headers(headers, peer_addr)),
                user_agent: header_value(headers, header::USER_AGENT.as_str()),
                expires_at,
            },
            token_hash.clone(),
        )
        .await?;

    let csrf_token = derive_csrf_token(&token_hash);
    let secure_attr = cookie_secure_attr(state.config.security.cookie_secure);
    let session_cookie = HeaderValue::from_str(&format!(
        "{}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age={}{}",
        SESSION_COOKIE_NAME,
        session_token,
        state.config.security.session_ttl_hours * 3600,
        secure_attr
    ))
    .map_err(|e| AppError::BadRequest(format!("Invalid session cookie: {e}")))?;
    let csrf_cookie = HeaderValue::from_str(&format!(
        "{}={}; SameSite=Lax; Path=/; Max-Age={}{}",
        CSRF_COOKIE_NAME,
        csrf_token,
        state.config.security.session_ttl_hours * 3600,
        secure_attr
    ))
    .map_err(|e| AppError::BadRequest(format!("Invalid session cookie: {e}")))?;
    Ok((session_cookie, csrf_cookie))
}

async fn cookie_session_user_id(
    state: &AppState,
    cookie_jar: &CookieJar,
) -> Result<Option<UserId>, AppError> {
    let Some(cookie) = cookie_jar.get(SESSION_COOKIE_NAME) else {
        return Ok(None);
    };
    let token_hash = hash_token(cookie.value());
    let session_repo = SessionRepository::new(state.db.clone());
    let Some(session) = session_repo.find_by_token_hash(&token_hash).await? else {
        return Ok(None);
    };
    Ok(Some(session.user_id))
}

fn bind_cookie_session_matches(
    state_user_id: UserId,
    cookie_user_id: Option<UserId>,
) -> Result<(), AppError> {
    match cookie_user_id {
        Some(cookie_user_id) if cookie_user_id == state_user_id => Ok(()),
        Some(_) => Err(AppError::Forbidden(
            "OAuth bind session does not match state user".into(),
        )),
        None => Err(AppError::Unauthorized(
            "OAuth bind requires an active cookie session".into(),
        )),
    }
}

fn frontend_redirect(
    state: &AppState,
    oauth_state: &OAuthState,
    success: bool,
    message: &str,
) -> Result<String, AppError> {
    let url = reqwest::Url::parse_with_params(
        &state.config.oauth2.frontend_redirect_url,
        &[
            ("oauth", if success { "success" } else { "error" }),
            ("message", message),
            ("return_to", oauth_state.return_to.as_str()),
        ],
    )
    .map_err(|e| AppError::BadRequest(format!("invalid OAuth frontend redirect URL: {e}")))?;
    Ok(url.to_string())
}

fn parse_oauth_start_query(uri: &Uri) -> Result<OAuthStartQuery, AppError> {
    ensure_oauth_query_size(uri)?;
    Query::<OAuthStartQuery>::try_from_uri(uri)
        .map(|Query(query)| query)
        .map_err(|_| AppError::BadRequest("OAuth query is invalid".into()))
}

fn parse_oauth_callback_query(uri: &Uri) -> Result<OAuthCallbackQuery, AppError> {
    ensure_oauth_query_size(uri)?;
    Query::<OAuthCallbackQuery>::try_from_uri(uri)
        .map(|Query(query)| query)
        .map_err(|_| AppError::BadRequest("OAuth query is invalid".into()))
}

fn ensure_oauth_query_size(uri: &Uri) -> Result<(), AppError> {
    let raw_query = uri.query().unwrap_or_default();
    if raw_query.len() > OAUTH_MAX_QUERY_BYTES {
        return Err(AppError::BadRequest(format!(
            "OAuth query must be at most {OAUTH_MAX_QUERY_BYTES} bytes"
        )));
    }
    Ok(())
}

fn encode_state(secret: &str, state: &OAuthState) -> Result<String, AppError> {
    let payload = serde_json::to_vec(state)
        .map_err(|e| AppError::BadRequest(format!("OAuth state encode failed: {e}")))?;
    let payload_hex = hex::encode(&payload);
    let signature = sign_state(secret, payload_hex.as_bytes())?;
    Ok(format!("{payload_hex}.{signature}"))
}

fn decode_state(secret: &str, encoded: &str) -> Result<OAuthState, AppError> {
    ensure_oauth_text_size(encoded, 1, OAUTH_MAX_STATE_BYTES, "OAuth state")?;
    let Some((payload_hex, signature)) = encoded.split_once('.') else {
        return Err(AppError::BadRequest("OAuth state is invalid".into()));
    };
    if payload_hex.len() > OAUTH_MAX_STATE_BYTES || signature.len() != 64 {
        return Err(AppError::BadRequest("OAuth state is invalid".into()));
    }
    if !payload_hex.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !signature.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AppError::BadRequest("OAuth state is invalid".into()));
    }
    let expected = sign_state(secret, payload_hex.as_bytes())?;
    if !constant_time_eq(signature.as_bytes(), expected.as_bytes()) {
        return Err(AppError::BadRequest(
            "OAuth state signature is invalid".into(),
        ));
    }
    let payload = hex::decode(payload_hex)
        .map_err(|e| AppError::BadRequest(format!("OAuth state payload is invalid: {e}")))?;
    let state: OAuthState = serde_json::from_slice(&payload)
        .map_err(|e| AppError::BadRequest(format!("OAuth state payload is invalid: {e}")))?;
    if state.exp < Utc::now().timestamp() {
        return Err(AppError::BadRequest("OAuth state has expired".into()));
    }
    Ok(state)
}

fn ensure_oauth_text_size(
    value: &str,
    min_bytes: usize,
    max_bytes: usize,
    field: &str,
) -> Result<(), AppError> {
    let len = value.len();
    if len < min_bytes || len > max_bytes {
        return Err(AppError::BadRequest(format!(
            "{field} must be between {min_bytes} and {max_bytes} bytes"
        )));
    }
    Ok(())
}

fn sign_state(secret: &str, payload: &[u8]) -> Result<String, AppError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| AppError::BadRequest(format!("OAuth state signing failed: {e}")))?;
    mac.update(payload);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn sanitize_return_to(value: Option<String>) -> String {
    value
        .filter(|item| valid_local_return_to(item))
        .unwrap_or_else(|| "/dashboard".to_string())
}

fn valid_local_return_to(value: &str) -> bool {
    value.starts_with('/')
        && !value.starts_with("//")
        && value.len() <= OAUTH_MAX_RETURN_TO_BYTES
        && !value.contains('\\')
        && !value.bytes().any(|byte| byte.is_ascii_control())
}

fn random_nonce() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    hex::encode(bytes)
}

fn parse_user_id(id: &str) -> Result<UserId, AppError> {
    uuid::Uuid::parse_str(id)
        .map(UserId)
        .map_err(|e| AppError::BadRequest(format!("invalid user id: {e}")))
}

fn client_ip_from_headers(headers: &HeaderMap, peer_addr: SocketAddr) -> String {
    crate::security::client_ip_from_headers_and_peer(headers, Some(peer_addr))
}

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum_extra::extract::cookie::Cookie;
    use std::collections::HashMap;

    fn test_provider() -> OidcProviderConfig {
        OidcProviderConfig {
            id: "oidc".into(),
            display_name: None,
            auth_url: "https://login.example.com/auth".into(),
            token_url: "https://login.example.com/token".into(),
            userinfo_url: "https://login.example.com/userinfo".into(),
            client_id: "client".into(),
            client_secret: "secret".into(),
            redirect_url: "https://status.example.com/oauth/callback".into(),
            scopes: vec!["openid".into(), "profile".into()],
            token_auth_method: "client_secret_post".into(),
            userinfo_auth_method: "bearer".into(),
            extra_auth_params: HashMap::new(),
            subject_field: "sub".into(),
            email_field: "email".into(),
            name_field: "name".into(),
            username_field: "preferred_username".into(),
        }
    }

    #[test]
    fn signed_state_round_trips() {
        let state = OAuthState {
            provider: "oidc".into(),
            flow: OAuthFlow::Login,
            user_id: None,
            return_to: "/dashboard".into(),
            nonce: "abc".into(),
            exp: (Utc::now() + Duration::minutes(1)).timestamp(),
        };
        let encoded = encode_state("secret", &state).unwrap();
        let decoded = decode_state("secret", &encoded).unwrap();
        assert_eq!(decoded.provider, state.provider);
        assert_eq!(decoded.flow, OAuthFlow::Login);
    }

    #[test]
    fn rejects_tampered_state() {
        let state = OAuthState {
            provider: "oidc".into(),
            flow: OAuthFlow::Login,
            user_id: None,
            return_to: "/dashboard".into(),
            nonce: "abc".into(),
            exp: (Utc::now() + Duration::minutes(1)).timestamp(),
        };
        let mut encoded = encode_state("secret", &state).unwrap();
        encoded.push('0');
        assert!(decode_state("secret", &encoded).is_err());
    }

    #[test]
    fn rejects_oversized_or_malformed_state_before_decoding() {
        let oversized = "a".repeat(OAUTH_MAX_STATE_BYTES + 1);
        let oversized_err = decode_state("secret", &oversized).unwrap_err();
        assert!(matches!(
            oversized_err,
            AppError::BadRequest(message) if message.contains("OAuth state")
        ));

        assert!(decode_state(
            "secret",
            "not-hex.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        )
        .is_err());
        assert!(decode_state("secret", "00.not-hex").is_err());
        assert!(decode_state("secret", "00.aa").is_err());
    }

    #[test]
    fn parses_limited_oauth_json_bytes() {
        let parsed: TokenResponse = parse_limited_json_bytes(
            br#"{"access_token":"abc"}"#,
            OAUTH_MAX_TOKEN_RESPONSE_BYTES,
            "OAuth token response",
        )
        .unwrap();
        assert_eq!(parsed.access_token, "abc");

        let oversized = vec![b' '; OAUTH_MAX_TOKEN_RESPONSE_BYTES + 1];
        let err = parse_limited_json_bytes::<TokenResponse>(
            &oversized,
            OAUTH_MAX_TOKEN_RESPONSE_BYTES,
            "OAuth token response",
        )
        .unwrap_err();
        assert!(matches!(
            err,
            AppError::BadRequest(message) if message.contains("exceeds")
        ));
    }

    #[test]
    fn validates_oauth_text_size_boundaries() {
        assert_eq!(OAUTH_MAX_PROVIDER_ID_BYTES, 64);
        assert_eq!(OAUTH_MAX_QUERY_BYTES, 16 * 1024);
        assert!(ensure_oauth_text_size("abc", 1, 3, "field").is_ok());
        assert!(ensure_oauth_text_size("", 1, 3, "field").is_err());
        assert!(ensure_oauth_text_size("abcd", 1, 3, "field").is_err());
    }

    #[test]
    fn oauth_provider_path_is_bounded_before_path_deserialize() {
        let uri: Uri = "/api/v1/oauth2/oidc?return_to=%2Fsettings".parse().unwrap();
        assert_eq!(parse_oauth_provider_path(&uri, false).unwrap(), "oidc");

        let uri: Uri = "/api/v1/oauth2/oidc-prod_1/bind?return_to=%2Fsettings"
            .parse()
            .unwrap();
        assert_eq!(
            parse_oauth_provider_path(&uri, true).unwrap(),
            "oidc-prod_1"
        );

        let uri: Uri = format!(
            "/api/v1/oauth2/{}?return_to=%2Fsettings",
            "a".repeat(OAUTH_MAX_PROVIDER_ID_BYTES + 1)
        )
        .parse()
        .unwrap();
        assert!(parse_oauth_provider_path(&uri, false).is_err());

        let uri: Uri = "/api/v1/oauth2/oidc%2Fbad?return_to=%2Fsettings"
            .parse()
            .unwrap();
        assert!(parse_oauth_provider_path(&uri, false).is_err());
    }

    #[test]
    fn oauth_query_resource_budget_is_bounded_before_deserialize() {
        let uri: Uri = format!(
            "/api/v1/oauth2/callback?state={}",
            "a".repeat(OAUTH_MAX_QUERY_BYTES)
        )
        .parse()
        .unwrap();
        let err = parse_oauth_callback_query(&uri).unwrap_err();
        assert!(matches!(
            err,
            AppError::BadRequest(message) if message.contains("OAuth query")
        ));

        let uri: Uri = "/api/v1/oauth2/oidc?return_to=%2Fsettings".parse().unwrap();
        let query = parse_oauth_start_query(&uri).unwrap();
        assert_eq!(query.return_to.as_deref(), Some("/settings"));
    }

    #[test]
    fn return_to_must_be_local_path() {
        assert_eq!(sanitize_return_to(Some("/settings".into())), "/settings");
        assert_eq!(
            sanitize_return_to(Some("https://x.test".into())),
            "/dashboard"
        );
        assert_eq!(sanitize_return_to(Some("//x.test".into())), "/dashboard");
        assert_eq!(
            sanitize_return_to(Some("/\\evil.test".into())),
            "/dashboard"
        );
        assert_eq!(
            sanitize_return_to(Some("/\tevil.test".into())),
            "/dashboard"
        );
        assert_eq!(
            sanitize_return_to(Some(format!("/{}", "a".repeat(OAUTH_MAX_RETURN_TO_BYTES)))),
            "/dashboard"
        );
    }

    #[test]
    fn bind_callback_requires_matching_cookie_session() {
        let user_id = UserId(uuid::Uuid::from_bytes([1; 16]));
        assert!(bind_cookie_session_matches(user_id, Some(user_id)).is_ok());

        let missing = bind_cookie_session_matches(user_id, None).unwrap_err();
        assert!(matches!(missing, AppError::Unauthorized(_)));

        let other_id = UserId(uuid::Uuid::from_bytes([2; 16]));
        let mismatch = bind_cookie_session_matches(user_id, Some(other_id)).unwrap_err();
        assert!(matches!(mismatch, AppError::Forbidden(_)));
    }

    #[test]
    fn oauth_state_cookie_binds_callback_to_starting_browser() {
        let encoded_state = "payload.signature";
        let cookie_value = oauth_state_cookie_value(encoded_state);
        assert_ne!(cookie_value, encoded_state);

        let jar = CookieJar::new().add(Cookie::new(OAUTH_STATE_COOKIE_NAME, cookie_value));
        assert!(oauth_state_cookie_matches(&jar, encoded_state).is_ok());

        let missing = oauth_state_cookie_matches(&CookieJar::new(), encoded_state).unwrap_err();
        assert!(matches!(missing, AppError::Unauthorized(_)));

        let wrong = CookieJar::new().add(Cookie::new(OAUTH_STATE_COOKIE_NAME, "wrong"));
        let mismatch = oauth_state_cookie_matches(&wrong, encoded_state).unwrap_err();
        assert!(matches!(mismatch, AppError::Forbidden(_)));
    }

    #[test]
    fn oauth_state_cookie_header_is_http_only_and_path_scoped() {
        let encoded_state = "payload.signature";
        let header = oauth_state_cookie_header(false, encoded_state)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        assert!(header.starts_with(&format!("{OAUTH_STATE_COOKIE_NAME}=")));
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("SameSite=Lax"));
        assert!(header.contains(&format!("Path={OAUTH_STATE_COOKIE_PATH}")));
        assert!(header.contains(&format!("Max-Age={OAUTH_STATE_TTL_SECONDS}")));
        assert!(!header.contains(encoded_state));
    }

    #[test]
    fn authorize_url_includes_extra_params_without_overriding_core() {
        let mut provider = test_provider();
        provider
            .extra_auth_params
            .insert("prompt".into(), "select_account".into());
        provider
            .extra_auth_params
            .insert("client_id".into(), "evil".into());
        let url = authorize_url(&provider, "state", "nonce").unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        let params: Vec<(String, String)> = parsed.query_pairs().into_owned().collect();
        assert!(params.contains(&("prompt".into(), "select_account".into())));
        assert!(params.contains(&("client_id".into(), "client".into())));
        assert!(!params.contains(&("client_id".into(), "evil".into())));
    }

    #[tokio::test]
    async fn oauth_bind_start_json_sets_state_cookie_and_returns_authorization_url() {
        let state = test_app_state().await;
        let provider = test_provider();
        let oauth_state = OAuthState {
            provider: provider.id.clone(),
            flow: OAuthFlow::Bind,
            user_id: Some(uuid::Uuid::from_bytes([1; 16]).to_string()),
            return_to: "/settings".into(),
            nonce: "abc".into(),
            exp: (Utc::now() + Duration::minutes(1)).timestamp(),
        };

        let response = oauth_start_json_response(&state, &provider, &oauth_state).unwrap();
        let headers = response.headers();
        let set_cookie = headers
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("state cookie");

        assert!(set_cookie.starts_with(&format!("{OAUTH_STATE_COOKIE_NAME}=")));
        assert!(set_cookie.contains("HttpOnly"));
        assert!(set_cookie.contains(&format!("Path={OAUTH_STATE_COOKIE_PATH}")));

        let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let authorization_url = payload["data"]["authorization_url"].as_str().unwrap();
        let parsed = reqwest::Url::parse(&authorization_url).unwrap();
        assert_eq!(
            parsed.as_str().split('?').next().unwrap(),
            provider.auth_url
        );
        assert!(parsed.query_pairs().any(|(key, _)| key == "state"));
        assert!(parsed
            .query_pairs()
            .any(|(key, value)| key == "nonce" && value == "abc"));
    }

    #[test]
    fn normalizes_userinfo_with_custom_claim_paths() {
        let mut provider = test_provider();
        provider.subject_field = "data.id".into();
        provider.email_field = "/profile/mail".into();
        provider.name_field = "profile.displayName".into();
        provider.username_field = "login".into();
        let raw = serde_json::json!({
            "data": { "id": 42 },
            "profile": { "mail": " alice@example.com ", "displayName": "Alice" },
            "login": true
        });
        let userinfo = normalize_userinfo(&provider, &raw).unwrap();
        assert_eq!(userinfo.sub, "42");
        assert_eq!(userinfo.email.as_deref(), Some("alice@example.com"));
        assert_eq!(userinfo.name.as_deref(), Some("Alice"));
        assert_eq!(userinfo.preferred_username.as_deref(), Some("true"));
    }

    #[test]
    fn rejects_oversized_userinfo_claims() {
        let provider = test_provider();
        let raw = serde_json::json!({
            "sub": "user-1",
            "email": "a".repeat(OAUTH_MAX_CLAIM_BYTES + 1)
        });
        let err = normalize_userinfo(&provider, &raw).unwrap_err();
        assert!(matches!(
            err,
            AppError::BadRequest(message) if message.contains("OIDC email")
        ));

        let raw = serde_json::json!({
            "sub": "a".repeat(OAUTH_MAX_CLAIM_BYTES + 1)
        });
        let err = normalize_userinfo(&provider, &raw).unwrap_err();
        assert!(matches!(
            err,
            AppError::BadRequest(message) if message.contains("OIDC subject")
        ));
    }

    #[test]
    fn parses_oauth_auth_method_aliases() {
        assert_eq!(
            parse_token_auth_method("client-secret-basic").unwrap(),
            TokenAuthMethod::ClientSecretBasic
        );
        assert_eq!(
            parse_token_auth_method("public").unwrap(),
            TokenAuthMethod::None
        );
        assert_eq!(
            parse_userinfo_auth_method("access token query").unwrap(),
            UserinfoAuthMethod::Query
        );
    }

    #[test]
    fn userinfo_query_token_auth_requires_explicit_escape_hatch() {
        assert!(ensure_userinfo_auth_method_allowed(UserinfoAuthMethod::Bearer, false).is_ok());
        assert!(ensure_userinfo_auth_method_allowed(UserinfoAuthMethod::None, false).is_ok());

        let err =
            ensure_userinfo_auth_method_allowed(UserinfoAuthMethod::Query, false).unwrap_err();
        assert!(matches!(
            err,
            AppError::BadRequest(message) if message.contains("query token auth is disabled")
        ));
        assert!(ensure_userinfo_auth_method_allowed(UserinfoAuthMethod::Query, true).is_ok());

        assert!(!allow_oidc_userinfo_query_token_value(None));
        assert!(!allow_oidc_userinfo_query_token_value(Some("false")));
        assert!(allow_oidc_userinfo_query_token_value(Some("1")));
        assert!(allow_oidc_userinfo_query_token_value(Some("true")));
        assert!(allow_oidc_userinfo_query_token_value(Some("yes")));
    }

    #[tokio::test]
    async fn oauth_outbound_requests_reject_private_targets() {
        let mut provider = test_provider();
        provider.token_url = "http://127.0.0.1:8080/token".into();
        let token_err = exchange_code(&provider, "code").await.unwrap_err();
        assert!(matches!(
            token_err,
            AppError::BadRequest(message) if message.contains("disallowed private address")
        ));

        provider.token_url = "https://login.example.com/token".into();
        provider.userinfo_url = "http://127.0.0.1:8080/userinfo".into();
        let userinfo_err = fetch_userinfo(&provider, "access-token").await.unwrap_err();
        assert!(matches!(
            userinfo_err,
            AppError::BadRequest(message) if message.contains("disallowed private address")
        ));
    }

    async fn test_app_state() -> AppState {
        let db = DatabaseBackend::connect("sqlite::memory:", true)
            .await
            .unwrap();
        db.run_migrations().await.unwrap();
        let mut config = crate::config::Config::default();
        config.security.session_secret = "test-secret-with-enough-bytes".into();
        config.security.cookie_secure = false;

        AppState {
            db,
            config: std::sync::Arc::new(config),
            agent_jwt_challenges: std::sync::Arc::new(tokio::sync::RwLock::new(
                std::collections::HashMap::new(),
            )),
            metrics: xlstatus_tsdb::MetricStore::in_memory(),
            realtime: crate::realtime::BroadcastHub::new(),
            session_registry: crate::grpc::SessionRegistry::new(),
            terminal_sessions: crate::api::v1::terminal::TerminalSessionRegistry::new(),
            io_registry: crate::grpc::IoRegistry::new(),
        }
    }
}
