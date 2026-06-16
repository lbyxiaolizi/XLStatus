use crate::db::Db;
use anyhow::Result;
use sqlx::Row;
use xlstatus_shared::nat::*;

pub struct NatMappingRepository;

impl NatMappingRepository {
    /// Create a new NAT mapping
    pub async fn create(db: &Db, mapping: &NatMapping) -> Result<()> {
        let query = r#"
            INSERT INTO nat_mappings (
                id, agent_id, local_host, local_port, public_port,
                protocol, enabled, description, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#;

        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query)
                    .bind(&mapping.id)
                    .bind(&mapping.agent_id)
                    .bind(&mapping.local_host)
                    .bind(mapping.local_port as i32)
                    .bind(mapping.public_port as i32)
                    .bind(mapping.protocol.as_str())
                    .bind(mapping.enabled)
                    .bind(&mapping.description)
                    .bind(&mapping.created_at)
                    .bind(&mapping.updated_at)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(query)
                    .bind(&mapping.id)
                    .bind(&mapping.agent_id)
                    .bind(&mapping.local_host)
                    .bind(mapping.local_port as i32)
                    .bind(mapping.public_port as i32)
                    .bind(mapping.protocol.as_str())
                    .bind(mapping.enabled)
                    .bind(&mapping.description)
                    .bind(&mapping.created_at)
                    .bind(&mapping.updated_at)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }

    /// Get a NAT mapping by ID
    pub async fn get_by_id(db: &Db, id: &str) -> Result<Option<NatMapping>> {
        let query = r#"
            SELECT id, agent_id, local_host, local_port, public_port,
                   protocol, enabled, description, created_at, updated_at
            FROM nat_mappings
            WHERE id = ?
        "#;

        let mapping = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row_opt = sqlx::query(query).bind(id).fetch_optional(pool).await?;
                row_opt.map(|row| Self::row_to_mapping(row)).transpose()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let row_opt = sqlx::query(query).bind(id).fetch_optional(pool).await?;
                row_opt.map(|row| Self::row_to_mapping(row)).transpose()?
            }
        };

        Ok(mapping)
    }

    /// Get NAT mapping by public port
    pub async fn get_by_public_port(db: &Db, port: u16) -> Result<Option<NatMapping>> {
        let query = r#"
            SELECT id, agent_id, local_host, local_port, public_port,
                   protocol, enabled, description, created_at, updated_at
            FROM nat_mappings
            WHERE public_port = ? AND enabled = 1
        "#;

        let mapping = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let row_opt = sqlx::query(query)
                    .bind(port as i32)
                    .fetch_optional(pool)
                    .await?;
                row_opt.map(|row| Self::row_to_mapping(row)).transpose()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let row_opt = sqlx::query(query)
                    .bind(port as i32)
                    .fetch_optional(pool)
                    .await?;
                row_opt.map(|row| Self::row_to_mapping(row)).transpose()?
            }
        };

        Ok(mapping)
    }

    /// List all NAT mappings for an agent
    pub async fn list_by_agent(db: &Db, agent_id: &str) -> Result<Vec<NatMapping>> {
        let query = r#"
            SELECT id, agent_id, local_host, local_port, public_port,
                   protocol, enabled, description, created_at, updated_at
            FROM nat_mappings
            WHERE agent_id = ?
            ORDER BY created_at DESC
        "#;

        let mappings = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(query).bind(agent_id).fetch_all(pool).await?;
                rows.into_iter()
                    .map(Self::row_to_mapping)
                    .collect::<Result<Vec<_>>>()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(query).bind(agent_id).fetch_all(pool).await?;
                rows.into_iter()
                    .map(Self::row_to_mapping)
                    .collect::<Result<Vec<_>>>()?
            }
        };

        Ok(mappings)
    }

    /// List all enabled NAT mappings
    pub async fn list_enabled(db: &Db) -> Result<Vec<NatMapping>> {
        let query = r#"
            SELECT id, agent_id, local_host, local_port, public_port,
                   protocol, enabled, description, created_at, updated_at
            FROM nat_mappings
            WHERE enabled = 1
            ORDER BY public_port ASC
        "#;

        let mappings = match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                let rows = sqlx::query(query).fetch_all(pool).await?;
                rows.into_iter()
                    .map(Self::row_to_mapping)
                    .collect::<Result<Vec<_>>>()?
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                let rows = sqlx::query(query).fetch_all(pool).await?;
                rows.into_iter()
                    .map(Self::row_to_mapping)
                    .collect::<Result<Vec<_>>>()?
            }
        };

        Ok(mappings)
    }

    /// Update a NAT mapping
    pub async fn update(db: &Db, mapping: &NatMapping) -> Result<()> {
        let query = r#"
            UPDATE nat_mappings
            SET local_host = ?, local_port = ?, public_port = ?,
                protocol = ?, enabled = ?, description = ?, updated_at = ?
            WHERE id = ?
        "#;

        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query)
                    .bind(&mapping.local_host)
                    .bind(mapping.local_port as i32)
                    .bind(mapping.public_port as i32)
                    .bind(mapping.protocol.as_str())
                    .bind(mapping.enabled)
                    .bind(&mapping.description)
                    .bind(&mapping.updated_at)
                    .bind(&mapping.id)
                    .execute(pool)
                    .await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(query)
                    .bind(&mapping.local_host)
                    .bind(mapping.local_port as i32)
                    .bind(mapping.public_port as i32)
                    .bind(mapping.protocol.as_str())
                    .bind(mapping.enabled)
                    .bind(&mapping.description)
                    .bind(&mapping.updated_at)
                    .bind(&mapping.id)
                    .execute(pool)
                    .await?;
            }
        }

        Ok(())
    }

    /// Delete a NAT mapping
    pub async fn delete(db: &Db, id: &str) -> Result<()> {
        let query = "DELETE FROM nat_mappings WHERE id = ?";

        match db {
            crate::db::DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query).bind(id).execute(pool).await?;
            }
            crate::db::DatabaseBackend::Postgres(pool) => {
                sqlx::query(query).bind(id).execute(pool).await?;
            }
        }

        Ok(())
    }

    /// Helper to convert row to NatMapping
    fn row_to_mapping<R: Row>(row: R) -> Result<NatMapping>
    where
        String: for<'a> sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
        i32: for<'a> sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
        bool: for<'a> sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
        Option<String>: for<'a> sqlx::Decode<'a, R::Database> + sqlx::Type<R::Database>,
        usize: sqlx::ColumnIndex<R>,
        for<'a> &'a str: sqlx::ColumnIndex<R>,
    {
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
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}
