#![allow(dead_code)]
#![allow(unused)]

#[macro_use]
mod macros;
mod models;
pub mod repository;

pub use models::*;
pub use repository::*;

use anyhow::Context;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, Pool, Postgres, Row, Sqlite};
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::str::FromStr;

pub type Db = DatabaseBackend;

#[derive(Clone)]
pub enum DatabaseBackend {
    Sqlite(Pool<Sqlite>),
    Postgres(Pool<Postgres>),
}

// Macro to execute queries on either database backend
#[macro_export]
macro_rules! db_query {
    ($db:expr, $query:expr) => {
        match $db {
            $crate::db::DatabaseBackend::Sqlite(pool) => sqlx::query($query).fetch_all(pool).await,
            $crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query($query).fetch_all(pool).await
            }
        }
    };
}

impl DatabaseBackend {
    pub async fn connect(database_url: &str, create_if_missing: bool) -> anyhow::Result<Self> {
        if database_url.starts_with("sqlite:") {
            let options = prepare_sqlite_options(database_url, create_if_missing)?;
            let pool = sqlx::SqlitePool::connect_with(options).await?;
            Ok(DatabaseBackend::Sqlite(pool))
        } else if database_url.starts_with("postgres:") || database_url.starts_with("postgresql:") {
            let pool = sqlx::PgPool::connect(database_url).await?;
            Ok(DatabaseBackend::Postgres(pool))
        } else {
            anyhow::bail!("Unsupported database URL: {}", database_url)
        }
    }

    pub fn sqlite_pool(&self) -> Option<&Pool<Sqlite>> {
        match self {
            DatabaseBackend::Sqlite(pool) => Some(pool),
            _ => None,
        }
    }

    pub fn postgres_pool(&self) -> Option<&Pool<Postgres>> {
        match self {
            DatabaseBackend::Postgres(pool) => Some(pool),
            _ => None,
        }
    }

    pub async fn run_migrations(&self) -> anyhow::Result<()> {
        match self {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(include_str!("../../migrations/sqlite/001_initial.sql"))
                    .execute(pool)
                    .await?;
                sqlx::query(include_str!("../../migrations/sqlite/002_agents.sql"))
                    .execute(pool)
                    .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "servers",
                    "agent_id",
                    "agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL",
                )
                .await?;
                sqlx::query("CREATE INDEX IF NOT EXISTS idx_servers_agent ON servers(agent_id)")
                    .execute(pool)
                    .await?;
                sqlx::query(include_str!("../../migrations/sqlite/003_services.sql"))
                    .execute(pool)
                    .await?;
                sqlx::query(include_str!("../../migrations/sqlite/004_nat.sql"))
                    .execute(pool)
                    .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "nat_mappings",
                    "allowed_sources",
                    "allowed_sources TEXT",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "nat_mappings",
                    "max_active_tunnels",
                    "max_active_tunnels INTEGER",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "nat_mappings",
                    "idle_timeout_seconds",
                    "idle_timeout_seconds INTEGER",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "nat_mappings",
                    "max_bytes_per_tunnel",
                    "max_bytes_per_tunnel INTEGER",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "nat_mappings",
                    "max_bandwidth_bytes_per_second",
                    "max_bandwidth_bytes_per_second INTEGER",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "nat_mappings",
                    "rate_limit_window_seconds",
                    "rate_limit_window_seconds INTEGER",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "nat_mappings",
                    "max_connections_per_window",
                    "max_connections_per_window INTEGER",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "nat_mappings",
                    "max_bytes_per_window",
                    "max_bytes_per_window INTEGER",
                )
                .await?;
                sqlx::query(include_str!("../../migrations/sqlite/005_tasks.sql"))
                    .execute(pool)
                    .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "agents",
                    "last_state_json",
                    "last_state_json TEXT",
                )
                .await?;
                sqlite_add_column_if_missing(pool, "agents", "last_state_at", "last_state_at TEXT")
                    .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "agents",
                    "last_info_json",
                    "last_info_json TEXT",
                )
                .await?;
                sqlite_add_column_if_missing(pool, "agents", "last_info_at", "last_info_at TEXT")
                    .await?;
                sqlite_add_column_if_missing(pool, "agents", "remark", "remark TEXT").await?;
                sqlite_add_column_if_missing(pool, "agents", "expires_at", "expires_at TEXT")
                    .await?;
                sqlite_add_column_if_missing(pool, "agents", "renewal_price", "renewal_price TEXT")
                    .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "agents",
                    "dashboard_metadata_json",
                    "dashboard_metadata_json TEXT",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "services",
                    "owner_user_id",
                    "owner_user_id TEXT REFERENCES users(id) ON DELETE SET NULL",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "services",
                    "server_id",
                    "server_id TEXT REFERENCES agents(id) ON DELETE SET NULL",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "services",
                    "cover_mode",
                    "cover_mode TEXT NOT NULL DEFAULT 'local'",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "services",
                    "exclude_server_ids_json",
                    "exclude_server_ids_json TEXT",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "services",
                    "failure_task_ids_json",
                    "failure_task_ids_json TEXT",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "services",
                    "recovery_task_ids_json",
                    "recovery_task_ids_json TEXT",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "alert_rules",
                    "fail_task_ids_json",
                    "fail_task_ids_json TEXT",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "alert_rules",
                    "recover_task_ids_json",
                    "recover_task_ids_json TEXT",
                )
                .await?;
                sqlx::query(
                    "UPDATE services SET cover_mode = 'specific' WHERE server_id IS NOT NULL AND TRIM(server_id) <> '' AND cover_mode = 'local'",
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE INDEX IF NOT EXISTS idx_services_server ON services(server_id)",
                )
                .execute(pool)
                .await?;
                sqlx::query(include_str!(
                    "../../migrations/sqlite/010_service_servers.sql"
                ))
                .execute(pool)
                .await?;
                sqlx::query(
                    r#"
                    UPDATE services
                    SET owner_user_id = (
                        SELECT MIN(a.owner_user_id)
                        FROM service_servers ss
                        JOIN agents a ON a.id = ss.server_id
                        WHERE ss.service_id = services.id
                    )
                    WHERE owner_user_id IS NULL
                      AND 1 = (
                          SELECT COUNT(DISTINCT a.owner_user_id)
                          FROM service_servers ss
                          JOIN agents a ON a.id = ss.server_id
                          WHERE ss.service_id = services.id
                      )
                    "#,
                )
                .execute(pool)
                .await?;
                sqlx::query(
                    "CREATE INDEX IF NOT EXISTS idx_services_owner ON services(owner_user_id)",
                )
                .execute(pool)
                .await?;
                sqlx::query(include_str!("../../migrations/sqlite/007_m4_m6.sql"))
                    .execute(pool)
                    .await?;
                sqlx::query(include_str!(
                    "../../migrations/sqlite/008_m8_performance.sql"
                ))
                .execute(pool)
                .await?;
                sqlx::query(include_str!("../../migrations/sqlite/011_waf.sql"))
                    .execute(pool)
                    .await?;
                sqlx::query(include_str!("../../migrations/sqlite/012_auth_totp.sql"))
                    .execute(pool)
                    .await?;
                sqlite_add_column_if_missing(pool, "users", "totp_secret", "totp_secret TEXT")
                    .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "users",
                    "totp_enabled",
                    "totp_enabled INTEGER NOT NULL DEFAULT 0",
                )
                .await?;
                sqlx::query(include_str!(
                    "../../migrations/sqlite/013_oauth_accounts.sql"
                ))
                .execute(pool)
                .await?;
                sqlx::query(include_str!(
                    "../../migrations/sqlite/014_agent_ip_events.sql"
                ))
                .execute(pool)
                .await?;
                sqlx::query(include_str!(
                    "../../migrations/sqlite/015_server_groups.sql"
                ))
                .execute(pool)
                .await?;
                sqlx::query(include_str!(
                    "../../migrations/sqlite/016_system_settings.sql"
                ))
                .execute(pool)
                .await?;
                sqlx::query(include_str!(
                    "../../migrations/sqlite/019_server_owner_transfers.sql"
                ))
                .execute(pool)
                .await?;
                sqlx::query(include_str!(
                    "../../migrations/sqlite/020_temporary_transfer_tokens.sql"
                ))
                .execute(pool)
                .await?;
                sqlx::query(include_str!(
                    "../../migrations/sqlite/021_pat_expiration_backfill.sql"
                ))
                .execute(pool)
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "temporary_transfer_tokens",
                    "used_ip",
                    "used_ip TEXT",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "temporary_transfer_tokens",
                    "used_status",
                    "used_status TEXT",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "temporary_transfer_tokens",
                    "used_error",
                    "used_error TEXT",
                )
                .await?;
                sqlite_add_column_if_missing(
                    pool,
                    "temporary_transfer_tokens",
                    "agent_task_id",
                    "agent_task_id TEXT",
                )
                .await?;
                Ok(())
            }
            DatabaseBackend::Postgres(pool) => {
                // PostgreSQL prepared-statement protocol does not accept multi-statement
                // SQL; route each migration through the simple protocol which does.
                // Running all five files in a single simple-query batch keeps the
                // foreign-key checks valid (each FK reference can see its target
                // table within the same transaction). Splitting them into separate
                // raw_sql calls causes "foreign key constraint cannot be implemented"
                // errors on later files.
                let batch = format!(
                    "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
                    include_str!("../../migrations/postgres/001_initial.sql"),
                    include_str!("../../migrations/postgres/002_agents.sql"),
                    include_str!("../../migrations/postgres/003_services.sql"),
                    include_str!("../../migrations/postgres/004_nat.sql"),
                    include_str!("../../migrations/postgres/005_tasks.sql"),
                    include_str!("../../migrations/postgres/006_agent_state.sql"),
                    include_str!("../../migrations/postgres/007_m4_m6.sql"),
                    include_str!("../../migrations/postgres/008_m8_performance.sql"),
                    include_str!("../../migrations/postgres/009_agent_service_metadata.sql"),
                    include_str!("../../migrations/postgres/010_service_servers.sql"),
                    include_str!("../../migrations/postgres/011_waf.sql"),
                    include_str!("../../migrations/postgres/012_service_cover_mode.sql"),
                    include_str!("../../migrations/postgres/013_auth_totp.sql"),
                    include_str!("../../migrations/postgres/014_oauth_accounts.sql"),
                    include_str!("../../migrations/postgres/015_agent_ip_events.sql"),
                    include_str!("../../migrations/postgres/016_server_groups.sql"),
                    include_str!("../../migrations/postgres/017_system_settings.sql"),
                    include_str!("../../migrations/postgres/018_trigger_task_ids.sql"),
                    include_str!("../../migrations/postgres/019_server_owner_transfers.sql"),
                    include_str!("../../migrations/postgres/020_temporary_transfer_tokens.sql"),
                    include_str!("../../migrations/postgres/021_pat_expiration_backfill.sql"),
                    include_str!("../../migrations/postgres/022_service_owner.sql"),
                );
                sqlx::raw_sql(batch.as_str()).execute(pool).await?;
                Ok(())
            }
        }
    }
}

async fn sqlite_add_column_if_missing(
    pool: &Pool<Sqlite>,
    table: &str,
    column: &str,
    definition: &str,
) -> anyhow::Result<()> {
    let pragma = format!("PRAGMA table_info({table})");
    let rows = sqlx::query(&pragma).fetch_all(pool).await?;
    let exists = rows
        .iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .any(|name| name == column);

    if !exists {
        let sql = format!("ALTER TABLE {table} ADD COLUMN {definition}");
        sqlx::query(&sql).execute(pool).await?;
    }

    Ok(())
}

fn prepare_sqlite_options(
    database_url: &str,
    create_if_missing: bool,
) -> anyhow::Result<SqliteConnectOptions> {
    let options = SqliteConnectOptions::from_str(database_url)
        .with_context(|| format!("Invalid SQLite database URL: {database_url}"))?;
    let db_path = options.get_filename().to_path_buf();

    if is_special_sqlite_path(&db_path) || db_path.exists() {
        return Ok(options);
    }

    let create_allowed = create_if_missing || sqlite_url_has_create_mode(database_url);
    if create_allowed || confirm_create_sqlite_database(&db_path)? {
        if let Some(parent) = db_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create SQLite database directory: {}",
                    parent.display()
                )
            })?;
        }
        tracing::warn!(
            "SQLite database file does not exist; creating {}",
            db_path.display()
        );
        return Ok(options.create_if_missing(true));
    }

    anyhow::bail!(
        "SQLite database file does not exist: {}. Create it first, add `?mode=rwc` to DATABASE_URL, set `[database] create_if_missing = true`, or set DATABASE_CREATE_IF_MISSING=true.",
        db_path.display()
    );
}

fn sqlite_url_has_create_mode(database_url: &str) -> bool {
    database_url
        .split_once('?')
        .map(|(_, query)| {
            query
                .split('&')
                .filter_map(|pair| pair.split_once('='))
                .any(|(key, value)| key == "mode" && value.eq_ignore_ascii_case("rwc"))
        })
        .unwrap_or(false)
}

fn is_special_sqlite_path(path: &Path) -> bool {
    let path = path.to_string_lossy();
    path == ":memory:" || path.starts_with("file:sqlx-in-memory-")
}

fn confirm_create_sqlite_database(path: &Path) -> anyhow::Result<bool> {
    if !io::stdin().is_terminal() {
        return Ok(false);
    }

    eprint!(
        "SQLite database file does not exist: {}. Create it now? [y/N] ",
        path.display()
    );
    io::stderr().flush().ok();

    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("Failed to read SQLite database creation confirmation")?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

#[cfg(test)]
mod tests {
    use super::{is_special_sqlite_path, sqlite_url_has_create_mode, DatabaseBackend};
    use chrono::{DateTime, Duration, Utc};
    use sqlx::Row;
    use std::path::Path;
    use uuid::Uuid;

    #[test]
    fn detects_sqlite_create_mode() {
        assert!(sqlite_url_has_create_mode(
            "sqlite://data/xlstatus.db?mode=rwc"
        ));
        assert!(sqlite_url_has_create_mode(
            "sqlite://data/xlstatus.db?cache=shared&mode=rwc"
        ));
        assert!(!sqlite_url_has_create_mode(
            "sqlite://data/xlstatus.db?mode=rw"
        ));
        assert!(!sqlite_url_has_create_mode("sqlite://data/xlstatus.db"));
    }

    #[test]
    fn detects_special_sqlite_paths() {
        assert!(is_special_sqlite_path(Path::new(":memory:")));
        assert!(is_special_sqlite_path(Path::new("file:sqlx-in-memory-0")));
        assert!(!is_special_sqlite_path(Path::new(
            "/var/lib/xlstatus/xlstatus.db"
        )));
    }

    #[tokio::test]
    async fn sqlite_migrations_are_idempotent_for_existing_file() {
        let db_path =
            std::env::temp_dir().join(format!("xlstatus-migrations-{}.db", Uuid::now_v7()));
        let url = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());

        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db.run_migrations().await.unwrap();
        drop(db);

        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn sqlite_migrations_add_service_owner_to_legacy_table_before_index() {
        let db_path =
            std::env::temp_dir().join(format!("xlstatus-legacy-services-{}.db", Uuid::now_v7()));
        let url = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        let DatabaseBackend::Sqlite(pool) = &db else {
            unreachable!();
        };

        sqlx::query(
            r#"
            CREATE TABLE services (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                type TEXT NOT NULL,
                target TEXT NOT NULL,
                interval_seconds INTEGER NOT NULL DEFAULT 60,
                timeout_seconds INTEGER NOT NULL DEFAULT 10,
                enabled INTEGER NOT NULL DEFAULT 1,
                notification_group_id TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await
        .unwrap();

        db.run_migrations().await.unwrap();
        db.run_migrations().await.unwrap();

        let columns = sqlx::query("PRAGMA table_info(services)")
            .fetch_all(pool)
            .await
            .unwrap();
        assert!(columns.iter().any(|row| {
            row.try_get::<String, _>("name")
                .map(|name| name == "owner_user_id")
                .unwrap_or(false)
        }));

        let indexes = sqlx::query("PRAGMA index_list(services)")
            .fetch_all(pool)
            .await
            .unwrap();
        assert!(indexes.iter().any(|row| {
            row.try_get::<String, _>("name")
                .map(|name| name == "idx_services_owner")
                .unwrap_or(false)
        }));

        drop(db);
        let _ = std::fs::remove_file(&db_path);
    }

    #[tokio::test]
    async fn sqlite_migration_backfills_legacy_pat_expiration() {
        let db_path =
            std::env::temp_dir().join(format!("xlstatus-pat-expiration-{}.db", Uuid::now_v7()));
        let url = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();

        let DatabaseBackend::Sqlite(pool) = &db else {
            unreachable!();
        };
        let user_id = Uuid::now_v7().to_string();
        let pat_id = Uuid::now_v7().to_string();
        let created_at = "2026-06-21T12:00:00Z";
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, role, created_at, updated_at)
             VALUES (?, ?, ?, 'admin', ?, ?)",
        )
        .bind(&user_id)
        .bind(format!("user-{user_id}"))
        .bind("hash")
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO personal_access_tokens
             (id, user_id, name, token_hash, scopes, created_at)
             VALUES (?, ?, 'legacy', ?, '[\"server:read\"]', ?)",
        )
        .bind(&pat_id)
        .bind(&user_id)
        .bind(format!("hash-{pat_id}"))
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();

        db.run_migrations().await.unwrap();

        let row = sqlx::query("SELECT expires_at FROM personal_access_tokens WHERE id = ?")
            .bind(&pat_id)
            .fetch_one(pool)
            .await
            .unwrap();
        let expires_at: String = row.try_get("expires_at").unwrap();
        let parsed = DateTime::parse_from_rfc3339(&expires_at)
            .unwrap()
            .with_timezone(&Utc);
        let expected = DateTime::parse_from_rfc3339(created_at)
            .unwrap()
            .with_timezone(&Utc)
            + Duration::days(90);

        assert_eq!(parsed, expected);

        drop(db);
        let _ = std::fs::remove_file(&db_path);
    }
}
