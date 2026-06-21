use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub security: SecurityConfig,
    #[serde(default)]
    pub oauth2: OAuth2Config,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_http_bind")]
    pub http_bind: String,

    #[serde(default = "default_grpc_bind")]
    pub grpc_bind: String,

    #[serde(default)]
    pub grpc_tls_cert_path: Option<String>,

    #[serde(default)]
    pub grpc_tls_key_path: Option<String>,

    #[serde(default)]
    pub grpc_tls_client_ca_path: Option<String>,

    #[serde(default = "default_grpc_reflection_enabled")]
    pub grpc_reflection_enabled: bool,

    #[serde(default = "default_cors_allowed_origins")]
    pub cors_allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,

    #[serde(default)]
    pub create_if_missing: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_session_secret")]
    pub session_secret: String,

    #[serde(default = "default_session_ttl_hours")]
    pub session_ttl_hours: i64,

    #[serde(default = "default_cookie_secure")]
    pub cookie_secure: bool,

    #[serde(default)]
    pub secret_encryption_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OAuth2Config {
    #[serde(default = "default_oauth2_frontend_redirect_url")]
    pub frontend_redirect_url: String,

    #[serde(default)]
    pub providers: Vec<OidcProviderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OidcProviderConfig {
    pub id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    pub redirect_url: String,
    #[serde(default = "default_oidc_scopes")]
    pub scopes: Vec<String>,
    #[serde(default = "default_oidc_token_auth_method")]
    pub token_auth_method: String,
    #[serde(default = "default_oidc_userinfo_auth_method")]
    pub userinfo_auth_method: String,
    #[serde(default)]
    pub extra_auth_params: HashMap<String, String>,
    #[serde(default = "default_oidc_subject_field")]
    pub subject_field: String,
    #[serde(default = "default_oidc_email_field")]
    pub email_field: String,
    #[serde(default = "default_oidc_name_field")]
    pub name_field: String,
    #[serde(default = "default_oidc_username_field")]
    pub username_field: String,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        // Try environment variable first
        if let Ok(database_url) = std::env::var("DATABASE_URL") {
            let config = Self {
                server: ServerConfig {
                    http_bind: std::env::var("HTTP_BIND").unwrap_or_else(|_| default_http_bind()),
                    grpc_bind: std::env::var("GRPC_BIND").unwrap_or_else(|_| default_grpc_bind()),
                    grpc_tls_cert_path: std::env::var("GRPC_TLS_CERT_PATH")
                        .ok()
                        .filter(|value| !value.trim().is_empty()),
                    grpc_tls_key_path: std::env::var("GRPC_TLS_KEY_PATH")
                        .ok()
                        .filter(|value| !value.trim().is_empty()),
                    grpc_tls_client_ca_path: std::env::var("GRPC_TLS_CLIENT_CA_PATH")
                        .ok()
                        .filter(|value| !value.trim().is_empty()),
                    grpc_reflection_enabled: std::env::var("GRPC_REFLECTION_ENABLED")
                        .ok()
                        .map(|value| parse_bool_env(&value))
                        .unwrap_or_else(default_grpc_reflection_enabled),
                    cors_allowed_origins: std::env::var("CORS_ALLOWED_ORIGINS")
                        .ok()
                        .map(|value| parse_csv_env(&value))
                        .unwrap_or_else(default_cors_allowed_origins),
                },
                database: DatabaseConfig {
                    url: database_url,
                    create_if_missing: std::env::var("DATABASE_CREATE_IF_MISSING")
                        .ok()
                        .map(|value| parse_bool_env(&value))
                        .unwrap_or(false),
                },
                security: SecurityConfig {
                    session_secret: std::env::var("SESSION_SECRET")
                        .unwrap_or_else(|_| default_session_secret()),
                    session_ttl_hours: std::env::var("SESSION_TTL_HOURS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(default_session_ttl_hours),
                    cookie_secure: std::env::var("SECURITY_COOKIE_SECURE")
                        .ok()
                        .map(|value| parse_bool_env(&value))
                        .unwrap_or_else(default_cookie_secure),
                    secret_encryption_key: std::env::var("SECRET_ENCRYPTION_KEY").ok(),
                },
                oauth2: OAuth2Config::from_env(),
            };
            config.validate_deployment_security()?;
            return Ok(config);
        }

        // Try config file
        let config_path =
            std::env::var("CONFIG_FILE").unwrap_or_else(|_| "config.toml".to_string());
        if Path::new(&config_path).exists() {
            let content = fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&content)?;
            config.validate_deployment_security()?;
            return Ok(config);
        }

        // Default config for development
        Ok(Self::default())
    }

    fn validate_deployment_security(&self) -> anyhow::Result<()> {
        let secret = self.security.session_secret.trim();
        let weak_defaults = [
            "CHANGE_ME_IN_PRODUCTION",
            "change-me-in-production",
            "replace-with-a-long-random-secret",
            "replace-with-a-different-long-random-secret",
            "replace-with-a-32-byte-random-hex-secret",
            "replace-with-a-different-32-byte-random-hex-secret",
        ];
        if weak_defaults.contains(&secret) {
            anyhow::bail!(
                "SESSION_SECRET/security.session_secret is using an insecure default value"
            );
        }
        if secret.len() < 32 {
            anyhow::bail!("SESSION_SECRET/security.session_secret must be at least 32 characters");
        }
        let secret_encryption_key = self.secret_encryption_key_material().trim();
        if weak_defaults.contains(&secret_encryption_key) {
            anyhow::bail!(
                "SECRET_ENCRYPTION_KEY/security.secret_encryption_key is using an insecure default value"
            );
        }
        if secret_encryption_key.len() < 32 {
            anyhow::bail!(
                "SECRET_ENCRYPTION_KEY/security.secret_encryption_key must be at least 32 characters"
            );
        }
        Ok(())
    }

    pub fn secret_encryption_key_material(&self) -> &str {
        self.security
            .secret_encryption_key
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(&self.security.session_secret)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                http_bind: default_http_bind(),
                grpc_bind: default_grpc_bind(),
                grpc_tls_cert_path: None,
                grpc_tls_key_path: None,
                grpc_tls_client_ca_path: None,
                grpc_reflection_enabled: default_grpc_reflection_enabled(),
                cors_allowed_origins: default_cors_allowed_origins(),
            },
            database: DatabaseConfig {
                url: "sqlite://dev.db".to_string(),
                create_if_missing: false,
            },
            security: SecurityConfig {
                session_secret: default_session_secret(),
                session_ttl_hours: default_session_ttl_hours(),
                cookie_secure: false,
                secret_encryption_key: None,
            },
            oauth2: OAuth2Config::default(),
        }
    }
}

impl OAuth2Config {
    fn from_env() -> Self {
        let frontend_redirect_url = std::env::var("OAUTH2_FRONTEND_REDIRECT_URL")
            .unwrap_or_else(|_| default_oauth2_frontend_redirect_url());
        let enabled = std::env::var("OIDC_ENABLED")
            .ok()
            .map(|value| parse_bool_env(&value))
            .unwrap_or(false);
        if !enabled {
            return Self {
                frontend_redirect_url,
                providers: Vec::new(),
            };
        }

        let provider = match OidcProviderConfig::from_env() {
            Some(provider) => provider,
            None => {
                tracing::warn!("OIDC_ENABLED=true but one or more OIDC_* settings are missing");
                return Self {
                    frontend_redirect_url,
                    providers: Vec::new(),
                };
            }
        };

        Self {
            frontend_redirect_url,
            providers: vec![provider],
        }
    }

    pub fn provider(&self, id: &str) -> Option<&OidcProviderConfig> {
        self.providers.iter().find(|provider| provider.id == id)
    }
}

impl OidcProviderConfig {
    fn from_env() -> Option<Self> {
        Some(Self {
            id: std::env::var("OIDC_PROVIDER_ID").unwrap_or_else(|_| "oidc".to_string()),
            display_name: std::env::var("OIDC_DISPLAY_NAME").ok(),
            auth_url: std::env::var("OIDC_AUTH_URL").ok()?,
            token_url: std::env::var("OIDC_TOKEN_URL").ok()?,
            userinfo_url: std::env::var("OIDC_USERINFO_URL").ok()?,
            client_id: std::env::var("OIDC_CLIENT_ID").ok()?,
            client_secret: std::env::var("OIDC_CLIENT_SECRET").unwrap_or_default(),
            redirect_url: std::env::var("OIDC_REDIRECT_URL").ok()?,
            scopes: std::env::var("OIDC_SCOPES")
                .ok()
                .map(|value| parse_csv_or_space_env(&value))
                .filter(|scopes| !scopes.is_empty())
                .unwrap_or_else(default_oidc_scopes),
            token_auth_method: std::env::var("OIDC_TOKEN_AUTH_METHOD")
                .unwrap_or_else(|_| default_oidc_token_auth_method()),
            userinfo_auth_method: std::env::var("OIDC_USERINFO_AUTH_METHOD")
                .unwrap_or_else(|_| default_oidc_userinfo_auth_method()),
            extra_auth_params: std::env::var("OIDC_EXTRA_AUTH_PARAMS")
                .ok()
                .map(|value| parse_key_value_map_env(&value))
                .unwrap_or_default(),
            subject_field: std::env::var("OIDC_SUBJECT_FIELD")
                .unwrap_or_else(|_| default_oidc_subject_field()),
            email_field: std::env::var("OIDC_EMAIL_FIELD")
                .unwrap_or_else(|_| default_oidc_email_field()),
            name_field: std::env::var("OIDC_NAME_FIELD")
                .unwrap_or_else(|_| default_oidc_name_field()),
            username_field: std::env::var("OIDC_USERNAME_FIELD")
                .unwrap_or_else(|_| default_oidc_username_field()),
        })
    }
}

fn default_http_bind() -> String {
    "127.0.0.1:8080".to_string()
}

fn default_grpc_bind() -> String {
    "127.0.0.1:50051".to_string()
}

fn default_grpc_reflection_enabled() -> bool {
    false
}

fn default_cors_allowed_origins() -> Vec<String> {
    vec![
        "http://localhost:3000".to_string(),
        "http://127.0.0.1:3000".to_string(),
        "http://[::1]:3000".to_string(),
    ]
}

fn default_session_secret() -> String {
    "dev-only-change-me-xlstatus-session-secret-32-bytes".to_string()
}

fn default_session_ttl_hours() -> i64 {
    24
}

fn default_cookie_secure() -> bool {
    true
}

fn default_oauth2_frontend_redirect_url() -> String {
    "http://localhost:3000/oauth/callback".to_string()
}

fn default_oidc_scopes() -> Vec<String> {
    vec![
        "openid".to_string(),
        "profile".to_string(),
        "email".to_string(),
    ]
}

fn default_oidc_token_auth_method() -> String {
    "client_secret_post".to_string()
}

fn default_oidc_userinfo_auth_method() -> String {
    "bearer".to_string()
}

fn default_oidc_subject_field() -> String {
    "sub".to_string()
}

fn default_oidc_email_field() -> String {
    "email".to_string()
}

fn default_oidc_name_field() -> String {
    "name".to_string()
}

fn default_oidc_username_field() -> String {
    "preferred_username".to_string()
}

fn parse_bool_env(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y" | "on"
    )
}

fn parse_csv_env(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn parse_csv_or_space_env(value: &str) -> Vec<String> {
    value
        .split(|ch: char| ch == ',' || ch.is_ascii_whitespace())
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn parse_key_value_map_env(value: &str) -> HashMap<String, String> {
    if let Ok(parsed) = serde_json::from_str::<HashMap<String, String>>(value) {
        return parsed
            .into_iter()
            .filter(|(key, _)| !key.trim().is_empty())
            .map(|(key, value)| (key.trim().to_string(), value))
            .collect();
    }

    value
        .split(',')
        .filter_map(|item| item.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .filter(|(key, _)| !key.is_empty())
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        parse_bool_env, parse_csv_env, parse_csv_or_space_env, parse_key_value_map_env, Config,
    };

    #[test]
    fn parses_truthy_database_create_flag() {
        for value in ["1", "true", "TRUE", "yes", "y", "on"] {
            assert!(parse_bool_env(value));
        }
    }

    #[test]
    fn parses_falsey_database_create_flag() {
        for value in ["0", "false", "no", "off", ""] {
            assert!(!parse_bool_env(value));
        }
    }

    #[test]
    fn parses_comma_separated_origins() {
        assert_eq!(
            parse_csv_env("http://localhost:3000, https://status.example.com,"),
            vec![
                "http://localhost:3000".to_string(),
                "https://status.example.com".to_string(),
            ]
        );
    }

    #[test]
    fn parses_oidc_scope_env() {
        assert_eq!(
            parse_csv_or_space_env("openid profile,email"),
            vec![
                "openid".to_string(),
                "profile".to_string(),
                "email".to_string(),
            ]
        );
    }

    #[test]
    fn parses_oidc_extra_auth_params_from_json_or_pairs() {
        let parsed =
            parse_key_value_map_env(r#"{"prompt":"select_account","access_type":"offline"}"#);
        assert_eq!(
            parsed.get("prompt").map(String::as_str),
            Some("select_account")
        );
        assert_eq!(
            parsed.get("access_type").map(String::as_str),
            Some("offline")
        );

        let parsed = parse_key_value_map_env("audience=https://api.example.com, resource=graph");
        assert_eq!(
            parsed.get("audience").map(String::as_str),
            Some("https://api.example.com")
        );
        assert_eq!(parsed.get("resource").map(String::as_str), Some("graph"));
    }

    #[test]
    fn development_default_has_usable_secret_material() {
        let config = Config::default();
        assert!(config.secret_encryption_key_material().len() >= 32);
    }

    #[test]
    fn deployment_security_rejects_documentation_secret_placeholders() {
        let mut config = Config::default();
        config.security.session_secret = "replace-with-a-32-byte-random-hex-secret".to_string();
        config.security.secret_encryption_key =
            Some("replace-with-a-different-32-byte-random-hex-secret".to_string());

        assert!(config.validate_deployment_security().is_err());
    }
}
