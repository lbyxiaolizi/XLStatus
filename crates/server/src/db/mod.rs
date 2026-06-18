#![allow(dead_code)]
#![allow(unused)]

#[macro_use]
mod macros;
mod models;
pub mod repository;

pub use models::*;
pub use repository::*;

use sqlx::{Pool, Postgres, Sqlite};

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
    pub async fn connect(database_url: &str) -> anyhow::Result<Self> {
        if database_url.starts_with("sqlite:") {
            let pool = sqlx::SqlitePool::connect(database_url).await?;
            Ok(DatabaseBackend::Sqlite(pool))
        } else if database_url.starts_with("postgres:") {
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
