#![allow(dead_code)]
#![allow(unused)]

use crate::db::Db;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use sqlx::postgres::PgRow;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use xlstatus_shared::nat::*;

pub struct NatMappingRepository;

impl NatMappingRepository {
    /// Create a new NAT mapping
    pub async fn create(db: &Db, mapping: &NatMapping) -> Result<()> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO nat_mappings (
                        id, agent_id, local_host, local_port, public_port,
                        protocol, enabled, description, allowed_sources, max_active_tunnels,
                        idle_timeout_seconds, max_bytes_per_tunnel, max_bandwidth_bytes_per_second,
                        rate_limit_window_seconds, max_connections_per_window, max_bytes_per_window,
                        created_at, updated_at
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    "#,
                )
                .bind(&mapping.id)
                .bind(&mapping.agent_id)
                .bind(&mapping.local_host)
                .bind(mapping.local_port as i32)
                .bind(mapping.public_port as i32)
                .bind(mapping.protocol.as_str())
                .bind(mapping.enabled)
                .bind(&mapping.description)
                .bind(&mapping.allowed_sources)
                .bind(mapping.max_active_tunnels.map(|value| value as i32))
                .bind(mapping.idle_timeout_seconds.map(|value| value as i32))
                .bind(mapping.max_bytes_per_tunnel.map(|value| value as i64))
                .bind(
                    mapping
                        .max_bandwidth_bytes_per_second
                        .map(|value| value as i64),
                )
                .bind(mapping.rate_limit_window_seconds.map(|value| value as i32))
                .bind(mapping.max_connections_per_window.map(|value| value as i32))
                .bind(mapping.max_bytes_per_window.map(|value| value as i64))
                .bind(&mapping.created_at)
                .bind(&mapping.updated_at)
                .execute(pool)
                .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO nat_mappings (
                        id, agent_id, local_host, local_port, public_port,
                        protocol, enabled, description, allowed_sources, max_active_tunnels,
                        idle_timeout_seconds, max_bytes_per_tunnel, max_bandwidth_bytes_per_second,
                        rate_limit_window_seconds, max_connections_per_window, max_bytes_per_window,
                        created_at, updated_at
                    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
                    "#,
                )
                    .bind(parse_uuid(&mapping.id, "id")?)
                    .bind(parse_uuid(&mapping.agent_id, "agent_id")?)
                    .bind(&mapping.local_host)
                    .bind(mapping.local_port as i32)
                    .bind(mapping.public_port as i32)
                    .bind(mapping.protocol.as_str())
                    .bind(mapping.enabled)
                    .bind(&mapping.description)
                    .bind(&mapping.allowed_sources)
                    .bind(mapping.max_active_tunnels.map(|value| value as i32))
                    .bind(mapping.idle_timeout_seconds.map(|value| value as i32))
                    .bind(mapping.max_bytes_per_tunnel.map(|value| value as i64))
                    .bind(
                        mapping
                            .max_bandwidth_bytes_per_second
                            .map(|value| value as i64),
                    )
                    .bind(mapping.rate_limit_window_seconds.map(|value| value as i32))
                    .bind(mapping.max_connections_per_window.map(|value| value as i32))
                    .bind(mapping.max_bytes_per_window.map(|value| value as i64))
                    .bind(parse_timestamp(&mapping.created_at, "created_at")?)
                    .bind(parse_timestamp(&mapping.updated_at, "updated_at")?)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }

    /// Get a NAT mapping by ID
    pub async fn get_by_id(db: &Db, id: &str) -> Result<Option<NatMapping>> {
        let mapping = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row_opt = sqlx::query(NAT_MAPPING_SELECT_SQLITE_BY_ID)
                    .bind(id)
                    .fetch_optional(pool)
                    .await?;
                row_opt.map(Self::sqlite_row_to_mapping).transpose()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let row_opt = sqlx::query(NAT_MAPPING_SELECT_POSTGRES_BY_ID)
                    .bind(parse_uuid(id, "id")?)
                    .fetch_optional(pool)
                    .await?;
                row_opt.map(Self::postgres_row_to_mapping).transpose()?
            }
        };

        Ok(mapping)
    }

    /// Get NAT mapping by public port
    pub async fn get_by_public_port(db: &Db, port: u16) -> Result<Option<NatMapping>> {
        let mapping = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row_opt = sqlx::query(NAT_MAPPING_SELECT_SQLITE_BY_PUBLIC_PORT)
                    .bind(port as i32)
                    .fetch_optional(pool)
                    .await?;
                row_opt.map(Self::sqlite_row_to_mapping).transpose()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let row_opt = sqlx::query(NAT_MAPPING_SELECT_POSTGRES_BY_PUBLIC_PORT)
                    .bind(port as i32)
                    .fetch_optional(pool)
                    .await?;
                row_opt.map(Self::postgres_row_to_mapping).transpose()?
            }
        };

        Ok(mapping)
    }

    /// Get an enabled NAT mapping by public port only if the target Agent is active.
    pub async fn get_active_by_public_port(db: &Db, port: u16) -> Result<Option<NatMapping>> {
        let mapping = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row_opt = sqlx::query(NAT_MAPPING_SELECT_SQLITE_ACTIVE_BY_PUBLIC_PORT)
                    .bind(port as i32)
                    .fetch_optional(pool)
                    .await?;
                row_opt.map(Self::sqlite_row_to_mapping).transpose()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let row_opt = sqlx::query(NAT_MAPPING_SELECT_POSTGRES_ACTIVE_BY_PUBLIC_PORT)
                    .bind(port as i32)
                    .fetch_optional(pool)
                    .await?;
                row_opt.map(Self::postgres_row_to_mapping).transpose()?
            }
        };

        Ok(mapping)
    }

    /// List all NAT mappings for an agent
    pub async fn list_by_agent(db: &Db, agent_id: &str) -> Result<Vec<NatMapping>> {
        let mappings = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(NAT_MAPPING_SELECT_SQLITE_BY_AGENT)
                    .bind(agent_id)
                    .fetch_all(pool)
                    .await?;
                rows.into_iter()
                    .map(Self::sqlite_row_to_mapping)
                    .collect::<Result<Vec<_>>>()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(NAT_MAPPING_SELECT_POSTGRES_BY_AGENT)
                    .bind(parse_uuid(agent_id, "agent_id")?)
                    .fetch_all(pool)
                    .await?;
                rows.into_iter()
                    .map(Self::postgres_row_to_mapping)
                    .collect::<Result<Vec<_>>>()?
            }
        };

        Ok(mappings)
    }

    /// List all enabled NAT mappings
    pub async fn list_enabled(db: &Db) -> Result<Vec<NatMapping>> {
        let mappings = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(NAT_MAPPING_SELECT_SQLITE_ENABLED)
                    .fetch_all(pool)
                    .await?;
                rows.into_iter()
                    .map(Self::sqlite_row_to_mapping)
                    .collect::<Result<Vec<_>>>()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(NAT_MAPPING_SELECT_POSTGRES_ENABLED)
                    .fetch_all(pool)
                    .await?;
                rows.into_iter()
                    .map(Self::postgres_row_to_mapping)
                    .collect::<Result<Vec<_>>>()?
            }
        };

        Ok(mappings)
    }

    /// List enabled NAT mappings whose target Agent is not revoked.
    pub async fn list_enabled_for_active_agents(db: &Db) -> Result<Vec<NatMapping>> {
        let mappings = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(NAT_MAPPING_SELECT_SQLITE_ENABLED_ACTIVE_AGENTS)
                    .fetch_all(pool)
                    .await?;
                rows.into_iter()
                    .map(Self::sqlite_row_to_mapping)
                    .collect::<Result<Vec<_>>>()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(NAT_MAPPING_SELECT_POSTGRES_ENABLED_ACTIVE_AGENTS)
                    .fetch_all(pool)
                    .await?;
                rows.into_iter()
                    .map(Self::postgres_row_to_mapping)
                    .collect::<Result<Vec<_>>>()?
            }
        };

        Ok(mappings)
    }

    /// Update a NAT mapping
    pub async fn update(db: &Db, mapping: &NatMapping) -> Result<()> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(
                    r#"
                    UPDATE nat_mappings
                    SET local_host = ?, local_port = ?, public_port = ?,
                        protocol = ?, enabled = ?, description = ?, allowed_sources = ?,
                        max_active_tunnels = ?, idle_timeout_seconds = ?,
                        max_bytes_per_tunnel = ?, max_bandwidth_bytes_per_second = ?,
                        rate_limit_window_seconds = ?, max_connections_per_window = ?,
                        max_bytes_per_window = ?,
                        updated_at = ?
                    WHERE id = ?
                    "#,
                )
                .bind(&mapping.local_host)
                .bind(mapping.local_port as i32)
                .bind(mapping.public_port as i32)
                .bind(mapping.protocol.as_str())
                .bind(mapping.enabled)
                .bind(&mapping.description)
                .bind(&mapping.allowed_sources)
                .bind(mapping.max_active_tunnels.map(|value| value as i32))
                .bind(mapping.idle_timeout_seconds.map(|value| value as i32))
                .bind(mapping.max_bytes_per_tunnel.map(|value| value as i64))
                .bind(
                    mapping
                        .max_bandwidth_bytes_per_second
                        .map(|value| value as i64),
                )
                .bind(mapping.rate_limit_window_seconds.map(|value| value as i32))
                .bind(mapping.max_connections_per_window.map(|value| value as i32))
                .bind(mapping.max_bytes_per_window.map(|value| value as i64))
                .bind(&mapping.updated_at)
                .bind(&mapping.id)
                .execute(pool)
                .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(
                    r#"
                    UPDATE nat_mappings
                    SET local_host = $1, local_port = $2, public_port = $3,
                        protocol = $4, enabled = $5, description = $6, allowed_sources = $7,
                        max_active_tunnels = $8, idle_timeout_seconds = $9,
                        max_bytes_per_tunnel = $10, max_bandwidth_bytes_per_second = $11,
                        rate_limit_window_seconds = $12, max_connections_per_window = $13,
                        max_bytes_per_window = $14,
                        updated_at = $15
                    WHERE id = $16
                    "#,
                )
                .bind(&mapping.local_host)
                .bind(mapping.local_port as i32)
                .bind(mapping.public_port as i32)
                .bind(mapping.protocol.as_str())
                .bind(mapping.enabled)
                .bind(&mapping.description)
                .bind(&mapping.allowed_sources)
                .bind(mapping.max_active_tunnels.map(|value| value as i32))
                .bind(mapping.idle_timeout_seconds.map(|value| value as i32))
                .bind(mapping.max_bytes_per_tunnel.map(|value| value as i64))
                .bind(
                    mapping
                        .max_bandwidth_bytes_per_second
                        .map(|value| value as i64),
                )
                .bind(mapping.rate_limit_window_seconds.map(|value| value as i32))
                .bind(mapping.max_connections_per_window.map(|value| value as i32))
                .bind(mapping.max_bytes_per_window.map(|value| value as i64))
                .bind(parse_timestamp(&mapping.updated_at, "updated_at")?)
                .bind(parse_uuid(&mapping.id, "id")?)
                .execute(pool)
                .await?;
            }
        }

        Ok(())
    }

    /// Delete a NAT mapping
    pub async fn delete(db: &Db, id: &str) -> Result<()> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query("DELETE FROM nat_mappings WHERE id = ?")
                    .bind(id)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query("DELETE FROM nat_mappings WHERE id = $1")
                    .bind(parse_uuid(id, "id")?)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }

    /// Helper to convert SQLite row to NatMapping
    fn sqlite_row_to_mapping(row: SqliteRow) -> Result<NatMapping> {
        let protocol_str: String = row.try_get("protocol")?;
        let protocol = Protocol::from_str(&protocol_str).unwrap_or(Protocol::Tcp);

        Ok(NatMapping {
            id: row.try_get("id")?,
            agent_id: row.try_get("agent_id")?,
            local_host: row.try_get("local_host")?,
            local_port: row.try_get::<i32, _>("local_port")? as u16,
            public_port: row.try_get::<i32, _>("public_port")? as u16,
            protocol,
            enabled: row.try_get("enabled")?,
            description: row.try_get("description")?,
            allowed_sources: row.try_get("allowed_sources")?,
            max_active_tunnels: row
                .try_get::<Option<i32>, _>("max_active_tunnels")?
                .map(|value| value as u32),
            idle_timeout_seconds: row
                .try_get::<Option<i32>, _>("idle_timeout_seconds")?
                .map(|value| value as u32),
            max_bytes_per_tunnel: row
                .try_get::<Option<i64>, _>("max_bytes_per_tunnel")?
                .map(|value| value as u64),
            max_bandwidth_bytes_per_second: row
                .try_get::<Option<i64>, _>("max_bandwidth_bytes_per_second")?
                .map(|value| value as u64),
            rate_limit_window_seconds: row
                .try_get::<Option<i32>, _>("rate_limit_window_seconds")?
                .map(|value| value as u32),
            max_connections_per_window: row
                .try_get::<Option<i32>, _>("max_connections_per_window")?
                .map(|value| value as u32),
            max_bytes_per_window: row
                .try_get::<Option<i64>, _>("max_bytes_per_window")?
                .map(|value| value as u64),
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }

    /// Helper to convert Postgres row to NatMapping
    fn postgres_row_to_mapping(row: PgRow) -> Result<NatMapping> {
        let protocol_str: String = row.try_get("protocol")?;
        let protocol = Protocol::from_str(&protocol_str).unwrap_or(Protocol::Tcp);
        let id: uuid::Uuid = row.try_get("id")?;
        let agent_id: uuid::Uuid = row.try_get("agent_id")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;
        let updated_at: DateTime<Utc> = row.try_get("updated_at")?;

        Ok(NatMapping {
            id: id.to_string(),
            agent_id: agent_id.to_string(),
            local_host: row.try_get("local_host")?,
            local_port: row.try_get::<i32, _>("local_port")? as u16,
            public_port: row.try_get::<i32, _>("public_port")? as u16,
            protocol,
            enabled: row.try_get("enabled")?,
            description: row.try_get("description")?,
            allowed_sources: row.try_get("allowed_sources")?,
            max_active_tunnels: row
                .try_get::<Option<i32>, _>("max_active_tunnels")?
                .map(|value| value as u32),
            idle_timeout_seconds: row
                .try_get::<Option<i32>, _>("idle_timeout_seconds")?
                .map(|value| value as u32),
            max_bytes_per_tunnel: row
                .try_get::<Option<i64>, _>("max_bytes_per_tunnel")?
                .map(|value| value as u64),
            max_bandwidth_bytes_per_second: row
                .try_get::<Option<i64>, _>("max_bandwidth_bytes_per_second")?
                .map(|value| value as u64),
            rate_limit_window_seconds: row
                .try_get::<Option<i32>, _>("rate_limit_window_seconds")?
                .map(|value| value as u32),
            max_connections_per_window: row
                .try_get::<Option<i32>, _>("max_connections_per_window")?
                .map(|value| value as u32),
            max_bytes_per_window: row
                .try_get::<Option<i64>, _>("max_bytes_per_window")?
                .map(|value| value as u64),
            created_at: created_at.to_rfc3339(),
            updated_at: updated_at.to_rfc3339(),
        })
    }
}

macro_rules! nat_mapping_select {
    ($suffix:literal) => {
        concat!(
            "SELECT id, agent_id, local_host, local_port, public_port, ",
            "protocol, enabled, description, allowed_sources, max_active_tunnels, ",
            "idle_timeout_seconds, max_bytes_per_tunnel, ",
            "max_bandwidth_bytes_per_second, rate_limit_window_seconds, ",
            "max_connections_per_window, max_bytes_per_window, ",
            "created_at, updated_at FROM nat_mappings ",
            $suffix
        )
    };
}

const NAT_MAPPING_SELECT_SQLITE_BY_ID: &str = nat_mapping_select!("WHERE id = ?");
const NAT_MAPPING_SELECT_POSTGRES_BY_ID: &str = nat_mapping_select!("WHERE id = $1");
const NAT_MAPPING_SELECT_SQLITE_BY_PUBLIC_PORT: &str =
    nat_mapping_select!("WHERE public_port = ? AND enabled = 1");
const NAT_MAPPING_SELECT_POSTGRES_BY_PUBLIC_PORT: &str =
    nat_mapping_select!("WHERE public_port = $1 AND enabled = TRUE");
const NAT_MAPPING_SELECT_SQLITE_ACTIVE_BY_PUBLIC_PORT: &str = r#"
    SELECT m.id, m.agent_id, m.local_host, m.local_port, m.public_port,
           m.protocol, m.enabled, m.description, m.allowed_sources,
           m.max_active_tunnels, m.idle_timeout_seconds, m.max_bytes_per_tunnel,
           m.max_bandwidth_bytes_per_second, m.rate_limit_window_seconds,
           m.max_connections_per_window, m.max_bytes_per_window,
           m.created_at, m.updated_at
    FROM nat_mappings m
    JOIN agents a ON a.id = m.agent_id
    WHERE m.public_port = ? AND m.enabled = 1 AND a.revoked_at IS NULL
"#;
const NAT_MAPPING_SELECT_POSTGRES_ACTIVE_BY_PUBLIC_PORT: &str = r#"
    SELECT m.id, m.agent_id, m.local_host, m.local_port, m.public_port,
           m.protocol, m.enabled, m.description, m.allowed_sources,
           m.max_active_tunnels, m.idle_timeout_seconds, m.max_bytes_per_tunnel,
           m.max_bandwidth_bytes_per_second, m.rate_limit_window_seconds,
           m.max_connections_per_window, m.max_bytes_per_window,
           m.created_at, m.updated_at
    FROM nat_mappings m
    JOIN agents a ON a.id = m.agent_id
    WHERE m.public_port = $1 AND m.enabled = TRUE AND a.revoked_at IS NULL
"#;
const NAT_MAPPING_SELECT_SQLITE_BY_AGENT: &str =
    nat_mapping_select!("WHERE agent_id = ? ORDER BY created_at DESC");
const NAT_MAPPING_SELECT_POSTGRES_BY_AGENT: &str =
    nat_mapping_select!("WHERE agent_id = $1 ORDER BY created_at DESC");
const NAT_MAPPING_SELECT_SQLITE_ENABLED: &str =
    nat_mapping_select!("WHERE enabled = 1 ORDER BY public_port ASC");
const NAT_MAPPING_SELECT_POSTGRES_ENABLED: &str =
    nat_mapping_select!("WHERE enabled = TRUE ORDER BY public_port ASC");
const NAT_MAPPING_SELECT_SQLITE_ENABLED_ACTIVE_AGENTS: &str = r#"
    SELECT m.id, m.agent_id, m.local_host, m.local_port, m.public_port,
           m.protocol, m.enabled, m.description, m.allowed_sources,
           m.max_active_tunnels, m.idle_timeout_seconds, m.max_bytes_per_tunnel,
           m.max_bandwidth_bytes_per_second, m.rate_limit_window_seconds,
           m.max_connections_per_window, m.max_bytes_per_window,
           m.created_at, m.updated_at
    FROM nat_mappings m
    JOIN agents a ON a.id = m.agent_id
    WHERE m.enabled = 1 AND a.revoked_at IS NULL
    ORDER BY m.public_port ASC
"#;
const NAT_MAPPING_SELECT_POSTGRES_ENABLED_ACTIVE_AGENTS: &str = r#"
    SELECT m.id, m.agent_id, m.local_host, m.local_port, m.public_port,
           m.protocol, m.enabled, m.description, m.allowed_sources,
           m.max_active_tunnels, m.idle_timeout_seconds, m.max_bytes_per_tunnel,
           m.max_bandwidth_bytes_per_second, m.rate_limit_window_seconds,
           m.max_connections_per_window, m.max_bytes_per_window,
           m.created_at, m.updated_at
    FROM nat_mappings m
    JOIN agents a ON a.id = m.agent_id
    WHERE m.enabled = TRUE AND a.revoked_at IS NULL
    ORDER BY m.public_port ASC
"#;

fn parse_uuid(value: &str, field: &str) -> Result<uuid::Uuid> {
    uuid::Uuid::parse_str(value).with_context(|| format!("invalid NAT mapping {field} UUID"))
}

fn parse_timestamp(value: &str, field: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .with_context(|| format!("invalid NAT mapping {field} timestamp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{CreateAgentInput, CreateUserInput, DatabaseBackend, UserRepository};
    use xlstatus_shared::{AgentId, UserRole};

    #[tokio::test]
    async fn nat_usage_window_persists_connection_and_byte_counts() {
        let db = test_db().await;
        let mapping = create_test_mapping(&db).await;
        let window = Utc::now();

        let first = NatMappingRepository::record_connection_window(
            &db,
            &mapping.id,
            "203.0.113.10",
            window,
        )
        .await
        .unwrap();
        let second = NatMappingRepository::record_connection_window(
            &db,
            &mapping.id,
            "203.0.113.10",
            window,
        )
        .await
        .unwrap();
        let bytes = NatMappingRepository::record_window_bytes(
            &db,
            &mapping.id,
            "203.0.113.10",
            window,
            4096,
        )
        .await
        .unwrap();

        assert_eq!(first.connection_count, 1);
        assert_eq!(second.connection_count, 2);
        assert_eq!(bytes.connection_count, 2);
        assert_eq!(bytes.bytes_transferred, 4096);
    }

    #[tokio::test]
    async fn nat_enabled_runtime_queries_ignore_revoked_agents() {
        let db = test_db().await;
        let active_mapping = create_test_mapping_with_port(&db, 12080, false).await;
        let revoked_mapping = create_test_mapping_with_port(&db, 12081, true).await;

        let mappings = NatMappingRepository::list_enabled_for_active_agents(&db)
            .await
            .unwrap();
        let ids = mappings
            .iter()
            .map(|mapping| mapping.id.as_str())
            .collect::<Vec<_>>();

        assert!(ids.contains(&active_mapping.id.as_str()));
        assert!(!ids.contains(&revoked_mapping.id.as_str()));
        assert!(
            NatMappingRepository::get_active_by_public_port(&db, active_mapping.public_port)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            NatMappingRepository::get_active_by_public_port(&db, revoked_mapping.public_port)
                .await
                .unwrap()
                .is_none()
        );
    }

    async fn create_test_mapping(db: &DatabaseBackend) -> NatMapping {
        create_test_mapping_with_port(db, 12080, false).await
    }

    async fn create_test_mapping_with_port(
        db: &DatabaseBackend,
        public_port: u16,
        revoke_agent: bool,
    ) -> NatMapping {
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: format!("nat-owner-{}", uuid::Uuid::now_v7()),
                password: "password123".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();
        let agent_id = AgentId(uuid::Uuid::now_v7());
        crate::db::repository::AgentRepository::new(db.clone())
            .create_with_id(
                agent_id,
                CreateAgentInput {
                    name: "nat-agent".into(),
                    public_key: "public-key".into(),
                    owner_user_id: user.id,
                },
            )
            .await
            .unwrap();
        if revoke_agent {
            crate::db::repository::AgentRepository::new(db.clone())
                .revoke(agent_id)
                .await
                .unwrap();
        }
        let now = Utc::now().to_rfc3339();
        let mapping = NatMapping {
            id: uuid::Uuid::now_v7().to_string(),
            agent_id: agent_id.0.to_string(),
            local_host: "127.0.0.1".into(),
            local_port: 80,
            public_port,
            protocol: Protocol::Tcp,
            enabled: true,
            description: None,
            allowed_sources: None,
            max_active_tunnels: None,
            idle_timeout_seconds: None,
            max_bytes_per_tunnel: None,
            max_bandwidth_bytes_per_second: None,
            rate_limit_window_seconds: Some(60),
            max_connections_per_window: Some(2),
            max_bytes_per_window: Some(4096),
            created_at: now.clone(),
            updated_at: now,
        };
        NatMappingRepository::create(db, &mapping).await.unwrap();
        mapping
    }

    async fn test_db() -> DatabaseBackend {
        let path = std::env::temp_dir().join(format!(
            "xlstatus-nat-repo-test-{}.db",
            uuid::Uuid::now_v7()
        ));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NatUsageWindow {
    pub connection_count: i64,
    pub bytes_transferred: i64,
}

impl NatMappingRepository {
    pub async fn record_connection_window(
        db: &Db,
        mapping_id: &str,
        source_ip: &str,
        window_start: DateTime<Utc>,
    ) -> Result<NatUsageWindow> {
        let now = Utc::now();
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let window = window_start.to_rfc3339();
                let now = now.to_rfc3339();
                sqlx::query(
                    r#"
                    INSERT INTO nat_usage_windows
                        (mapping_id, source_ip, window_start, connection_count, bytes_transferred, updated_at)
                    VALUES (?, ?, ?, 1, 0, ?)
                    ON CONFLICT(mapping_id, source_ip, window_start)
                    DO UPDATE SET
                        connection_count = connection_count + 1,
                        updated_at = excluded.updated_at
                    "#,
                )
                .bind(mapping_id)
                .bind(source_ip)
                .bind(&window)
                .bind(&now)
                .execute(pool)
                .await?;
                let row = sqlx::query(
                    r#"
                    SELECT connection_count, bytes_transferred
                    FROM nat_usage_windows
                    WHERE mapping_id = ? AND source_ip = ? AND window_start = ?
                    "#,
                )
                .bind(mapping_id)
                .bind(source_ip)
                .bind(&window)
                .fetch_one(pool)
                .await?;
                Ok(NatUsageWindow {
                    connection_count: row.try_get("connection_count")?,
                    bytes_transferred: row.try_get("bytes_transferred")?,
                })
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let mapping_uuid = uuid::Uuid::parse_str(mapping_id)?;
                let row = sqlx::query(
                    r#"
                    INSERT INTO nat_usage_windows
                        (mapping_id, source_ip, window_start, connection_count, bytes_transferred, updated_at)
                    VALUES ($1, $2, $3, 1, 0, $4)
                    ON CONFLICT(mapping_id, source_ip, window_start)
                    DO UPDATE SET
                        connection_count = nat_usage_windows.connection_count + 1,
                        updated_at = EXCLUDED.updated_at
                    RETURNING connection_count, bytes_transferred
                    "#,
                )
                .bind(mapping_uuid)
                .bind(source_ip)
                .bind(window_start)
                .bind(now)
                .fetch_one(pool)
                .await?;
                Ok(NatUsageWindow {
                    connection_count: row.try_get("connection_count")?,
                    bytes_transferred: row.try_get("bytes_transferred")?,
                })
            }
        }
    }

    pub async fn record_window_bytes(
        db: &Db,
        mapping_id: &str,
        source_ip: &str,
        window_start: DateTime<Utc>,
        bytes: u64,
    ) -> Result<NatUsageWindow> {
        let now = Utc::now();
        let bytes = i64::try_from(bytes).unwrap_or(i64::MAX);
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let window = window_start.to_rfc3339();
                let now = now.to_rfc3339();
                sqlx::query(
                    r#"
                    INSERT INTO nat_usage_windows
                        (mapping_id, source_ip, window_start, connection_count, bytes_transferred, updated_at)
                    VALUES (?, ?, ?, 0, ?, ?)
                    ON CONFLICT(mapping_id, source_ip, window_start)
                    DO UPDATE SET
                        bytes_transferred = bytes_transferred + excluded.bytes_transferred,
                        updated_at = excluded.updated_at
                    "#,
                )
                .bind(mapping_id)
                .bind(source_ip)
                .bind(&window)
                .bind(bytes)
                .bind(&now)
                .execute(pool)
                .await?;
                let row = sqlx::query(
                    r#"
                    SELECT connection_count, bytes_transferred
                    FROM nat_usage_windows
                    WHERE mapping_id = ? AND source_ip = ? AND window_start = ?
                    "#,
                )
                .bind(mapping_id)
                .bind(source_ip)
                .bind(&window)
                .fetch_one(pool)
                .await?;
                Ok(NatUsageWindow {
                    connection_count: row.try_get("connection_count")?,
                    bytes_transferred: row.try_get("bytes_transferred")?,
                })
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let mapping_uuid = uuid::Uuid::parse_str(mapping_id)?;
                let row = sqlx::query(
                    r#"
                    INSERT INTO nat_usage_windows
                        (mapping_id, source_ip, window_start, connection_count, bytes_transferred, updated_at)
                    VALUES ($1, $2, $3, 0, $4, $5)
                    ON CONFLICT(mapping_id, source_ip, window_start)
                    DO UPDATE SET
                        bytes_transferred = nat_usage_windows.bytes_transferred + EXCLUDED.bytes_transferred,
                        updated_at = EXCLUDED.updated_at
                    RETURNING connection_count, bytes_transferred
                    "#,
                )
                .bind(mapping_uuid)
                .bind(source_ip)
                .bind(window_start)
                .bind(bytes)
                .bind(now)
                .fetch_one(pool)
                .await?;
                Ok(NatUsageWindow {
                    connection_count: row.try_get("connection_count")?,
                    bytes_transferred: row.try_get("bytes_transferred")?,
                })
            }
        }
    }

    pub async fn prune_usage_windows(db: &Db, before: DateTime<Utc>) -> Result<u64> {
        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let result = sqlx::query("DELETE FROM nat_usage_windows WHERE window_start < ?")
                    .bind(before.to_rfc3339())
                    .execute(pool)
                    .await?;
                Ok(result.rows_affected())
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let result = sqlx::query("DELETE FROM nat_usage_windows WHERE window_start < $1")
                    .bind(before)
                    .execute(pool)
                    .await?;
                Ok(result.rows_affected())
            }
        }
    }
}
