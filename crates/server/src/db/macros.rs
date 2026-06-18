#![allow(dead_code)]
#![allow(unused)]

/// Macro to execute queries on both SQLite and PostgreSQL
/// Usage: db_execute!(db, query_builder)
#[macro_export]
macro_rules! db_execute {
    ($db:expr, $query:expr) => {
        match $db {
            crate::db::DatabaseBackend::Sqlite(pool) => $query.execute(pool).await,
            crate::db::DatabaseBackend::Postgres(pool) => $query.execute(pool).await,
        }
    };
}

#[macro_export]
macro_rules! db_fetch_optional {
    ($db:expr, $query:expr) => {
        match $db {
            crate::db::DatabaseBackend::Sqlite(pool) => $query.fetch_optional(pool).await,
            crate::db::DatabaseBackend::Postgres(pool) => $query.fetch_optional(pool).await,
        }
    };
}

#[macro_export]
macro_rules! db_fetch_all {
    ($db:expr, $query:expr) => {
        match $db {
            crate::db::DatabaseBackend::Sqlite(pool) => $query.fetch_all(pool).await,
            crate::db::DatabaseBackend::Postgres(pool) => $query.fetch_all(pool).await,
        }
    };
}

#[macro_export]
macro_rules! db_fetch_one {
    ($db:expr, $query:expr) => {
        match $db {
            crate::db::DatabaseBackend::Sqlite(pool) => $query.fetch_one(pool).await,
            crate::db::DatabaseBackend::Postgres(pool) => $query.fetch_one(pool).await,
        }
    };
}
