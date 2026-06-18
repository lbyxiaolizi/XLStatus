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
use sqlx::{ConnectOptions, Pool, Postgres, Sqlite};
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
                sqlx::query(include_str!("../../migrations/sqlite/003_services.sql"))
                    .execute(pool)
                    .await?;
                sqlx::query(include_str!("../../migrations/sqlite/004_nat.sql"))
                    .execute(pool)
                    .await?;
                sqlx::query(include_str!("../../migrations/sqlite/005_tasks.sql"))
                    .execute(pool)
                    .await?;
                // 006 uses ALTER TABLE which is a no-op on subsequent boots.
                let _ = sqlx::query(include_str!("../../migrations/sqlite/006_agent_state.sql"))
                    .execute(pool)
                    .await;
                sqlx::query(include_str!("../../migrations/sqlite/007_m4_m6.sql"))
                    .execute(pool)
                    .await?;
                sqlx::query(include_str!(
                    "../../migrations/sqlite/008_m8_performance.sql"
                ))
                .execute(pool)
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
                    "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
                    include_str!("../../migrations/postgres/001_initial.sql"),
                    include_str!("../../migrations/postgres/002_agents.sql"),
                    include_str!("../../migrations/postgres/003_services.sql"),
                    include_str!("../../migrations/postgres/004_nat.sql"),
                    include_str!("../../migrations/postgres/005_tasks.sql"),
                    include_str!("../../migrations/postgres/006_agent_state.sql"),
                    include_str!("../../migrations/postgres/007_m4_m6.sql"),
                    include_str!("../../migrations/postgres/008_m8_performance.sql"),
                );
                sqlx::raw_sql(batch.as_str()).execute(pool).await?;
                Ok(())
            }
        }
    }
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
    use super::{is_special_sqlite_path, sqlite_url_has_create_mode};
    use std::path::Path;

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
}
