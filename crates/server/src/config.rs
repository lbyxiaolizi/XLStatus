use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub security: SecurityConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_http_bind")]
    pub http_bind: String,

    #[serde(default = "default_grpc_bind")]
    pub grpc_bind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_session_secret")]
    pub session_secret: String,

    #[serde(default = "default_session_ttl_hours")]
    pub session_ttl_hours: i64,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        // Try environment variable first
        if let Ok(database_url) = std::env::var("DATABASE_URL") {
            return Ok(Self {
                server: ServerConfig {
                    http_bind: std::env::var("HTTP_BIND").unwrap_or_else(|_| default_http_bind()),
                    grpc_bind: std::env::var("GRPC_BIND").unwrap_or_else(|_| default_grpc_bind()),
                },
                database: DatabaseConfig { url: database_url },
                security: SecurityConfig {
                    session_secret: std::env::var("SESSION_SECRET")
                        .unwrap_or_else(|_| default_session_secret()),
                    session_ttl_hours: std::env::var("SESSION_TTL_HOURS")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_else(default_session_ttl_hours),
                },
            });
        }

        // Try config file
        let config_path =
            std::env::var("CONFIG_FILE").unwrap_or_else(|_| "config.toml".to_string());
        if Path::new(&config_path).exists() {
            let content = fs::read_to_string(&config_path)?;
            let config: Config = toml::from_str(&content)?;
            return Ok(config);
        }

        // Default config for development
        Ok(Self::default())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                http_bind: default_http_bind(),
                grpc_bind: default_grpc_bind(),
            },
            database: DatabaseConfig {
                url: "sqlite://dev.db".to_string(),
            },
            security: SecurityConfig {
                session_secret: default_session_secret(),
                session_ttl_hours: default_session_ttl_hours(),
            },
        }
    }
}

fn default_http_bind() -> String {
    "0.0.0.0:8080".to_string()
}

fn default_grpc_bind() -> String {
    "0.0.0.0:50051".to_string()
}

fn default_session_secret() -> String {
    "CHANGE_ME_IN_PRODUCTION".to_string()
}

fn default_session_ttl_hours() -> i64 {
    24
}
