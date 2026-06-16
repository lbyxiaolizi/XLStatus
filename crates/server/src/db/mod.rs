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
            $crate::db::DatabaseBackend::Postgres(pool) => sqlx::query($query).fetch_all(pool).await,
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
                sqlx::query(include_str!("../../migrations/sqlite/001_initial.sql")).execute(pool).await?;
                sqlx::query(include_str!("../../migrations/sqlite/002_agents.sql")).execute(pool).await?;
                sqlx::query(include_str!("../../migrations/sqlite/003_services.sql")).execute(pool).await?;
                sqlx::query(include_str!("../../migrations/sqlite/004_nat.sql")).execute(pool).await?;
                sqlx::query(include_str!("../../migrations/sqlite/005_tasks.sql")).execute(pool).await?;
                Ok(())
            }
            DatabaseBackend::Postgres(pool) => {
                sqlx::query(include_str!("../../migrations/postgres/001_initial.sql")).execute(pool).await?;
                sqlx::query(include_str!("../../migrations/postgres/002_agents.sql")).execute(pool).await?;
                sqlx::query(include_str!("../../migrations/postgres/003_services.sql")).execute(pool).await?;
                sqlx::query(include_str!("../../migrations/postgres/004_nat.sql")).execute(pool).await?;
                sqlx::query(include_str!("../../migrations/postgres/005_tasks.sql")).execute(pool).await?;
                Ok(())
            }
        }
    }
}
