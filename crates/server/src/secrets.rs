//! Application-level encryption for secrets stored in the database.

use crate::db::DatabaseBackend;
use anyhow::{anyhow, bail, Result};
use ring::{aead, hkdf, rand as ring_rand};
use sqlx::Row;
use std::collections::HashSet;
use std::fmt;
use std::sync::OnceLock;

const PREFIX: &str = "xlsec:v1:";
const AAD: &[u8] = b"xlstatus.secrets.v1";
const HKDF_SALT: &[u8] = b"xlstatus.secrets.salt.v1";
const HKDF_INFO: &[u8] = b"XLStatus secret encryption key v1";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const NONCE_HEX_LEN: usize = NONCE_LEN * 2;
const MIN_CIPHERTEXT_HEX_LEN: usize = TAG_LEN * 2;
pub const MAX_SECRET_PLAINTEXT_BYTES: usize = 16 * 1024;
const MAX_CIPHERTEXT_HEX_LEN: usize = (MAX_SECRET_PLAINTEXT_BYTES + TAG_LEN) * 2;
pub const MAX_ENCRYPTED_SECRET_BYTES: usize =
    PREFIX.len() + NONCE_HEX_LEN + 1 + MAX_CIPHERTEXT_HEX_LEN;
const SECRET_SETTING_VALUE_JSON_MAX_BYTES: usize = 64 * 1024;
const MIN_MASTER_KEY_BYTES: usize = 32;
const MIN_MASTER_KEY_DISTINCT_BYTES: usize = 4;

static SECRET_CRYPTO: OnceLock<SecretCrypto> = OnceLock::new();

pub fn init_secret_crypto(master_key: &str) -> Result<()> {
    let crypto = SecretCrypto::new(master_key)?;
    let _ = SECRET_CRYPTO.set(crypto);
    Ok(())
}

pub fn is_encrypted_secret(value: &str) -> bool {
    value.starts_with(PREFIX)
}

pub fn encrypt_secret(value: &str) -> Result<String> {
    active_crypto()?.encrypt(value)
}

pub fn decrypt_secret_if_needed(value: &str) -> Result<String> {
    if is_encrypted_secret(value) {
        active_crypto()?.decrypt(value)
    } else {
        if value.len() > MAX_SECRET_PLAINTEXT_BYTES {
            bail!("secret value exceeds maximum allowed size");
        }
        Ok(value.to_string())
    }
}

pub fn encrypt_optional_secret(value: Option<&str>) -> Result<Option<String>> {
    value
        .map(|value| {
            if value.trim().is_empty() || is_encrypted_secret(value) {
                Ok(value.to_string())
            } else {
                encrypt_secret(value)
            }
        })
        .transpose()
}

pub fn decrypt_optional_secret(value: Option<String>) -> Result<Option<String>> {
    value
        .map(|value| decrypt_secret_if_needed(&value))
        .transpose()
}

fn active_crypto() -> Result<&'static SecretCrypto> {
    if let Some(crypto) = SECRET_CRYPTO.get() {
        return Ok(crypto);
    }

    #[cfg(test)]
    {
        static TEST_SECRET_CRYPTO: OnceLock<SecretCrypto> = OnceLock::new();
        if TEST_SECRET_CRYPTO.get().is_none() {
            let _ = TEST_SECRET_CRYPTO.set(SecretCrypto::new(
                "xlstatus-test-secret-encryption-key-32-bytes-minimum",
            )?);
        }
        TEST_SECRET_CRYPTO
            .get()
            .ok_or_else(|| anyhow!("failed to initialize test secret crypto"))
    }

    #[cfg(not(test))]
    {
        bail!("secret encryption has not been initialized")
    }
}

struct SecretCrypto {
    key: [u8; 32],
}

impl fmt::Debug for SecretCrypto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretCrypto")
            .field("key", &"[REDACTED]")
            .finish()
    }
}

impl Drop for SecretCrypto {
    fn drop(&mut self) {
        for byte in &mut self.key {
            // SAFETY: self.key is a valid stack/heap byte array owned by self.
            unsafe {
                std::ptr::write_volatile(byte, 0);
            }
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }
}

struct HkdfKey32;

impl hkdf::KeyType for HkdfKey32 {
    fn len(&self) -> usize {
        32
    }
}

impl SecretCrypto {
    fn new(master_key: &str) -> Result<Self> {
        let master_key = master_key.trim();
        if master_key.len() < MIN_MASTER_KEY_BYTES {
            bail!("secret encryption key must be at least 32 characters");
        }
        let weak_defaults = [
            "CHANGE_ME_IN_PRODUCTION",
            "change-me-in-production",
            "replace-with-a-long-random-secret",
            "replace-with-a-different-long-random-secret",
            "replace-with-a-32-byte-random-hex-secret",
            "replace-with-a-different-32-byte-random-hex-secret",
        ];
        if weak_defaults.contains(&master_key) {
            bail!("secret encryption key uses an insecure default value");
        }
        let distinct: HashSet<u8> = master_key.bytes().collect();
        if distinct.len() < MIN_MASTER_KEY_DISTINCT_BYTES {
            bail!("secret encryption key has insufficient entropy");
        }

        let salt = hkdf::Salt::new(hkdf::HKDF_SHA256, HKDF_SALT);
        let prk = salt.extract(master_key.as_bytes());
        let info = [HKDF_INFO];
        let okm = prk
            .expand(&info, HkdfKey32)
            .map_err(|_| anyhow!("failed to derive secret encryption key"))?;
        let mut key = [0_u8; 32];
        okm.fill(&mut key)
            .map_err(|_| anyhow!("failed to derive secret encryption key"))?;
        Ok(Self { key })
    }

    fn encrypt(&self, plaintext: &str) -> Result<String> {
        if plaintext.len() > MAX_SECRET_PLAINTEXT_BYTES {
            bail!("secret value exceeds maximum allowed size");
        }

        let rng = ring_rand::SystemRandom::new();
        let mut nonce_bytes = [0_u8; NONCE_LEN];
        ring_rand::SecureRandom::fill(&rng, &mut nonce_bytes)
            .map_err(|_| anyhow!("failed to generate secret nonce"))?;

        let mut in_out = plaintext.as_bytes().to_vec();
        let key = self.less_safe_key()?;
        key.seal_in_place_append_tag(
            aead::Nonce::assume_unique_for_key(nonce_bytes),
            aead::Aad::from(AAD),
            &mut in_out,
        )
        .map_err(|_| anyhow!("failed to encrypt secret"))?;

        Ok(format!(
            "{PREFIX}{}:{}",
            hex::encode(nonce_bytes),
            hex::encode(in_out)
        ))
    }

    fn decrypt(&self, encrypted: &str) -> Result<String> {
        if encrypted.len() > MAX_ENCRYPTED_SECRET_BYTES {
            bail!("encrypted secret exceeds maximum allowed size");
        }
        let payload = encrypted
            .strip_prefix(PREFIX)
            .ok_or_else(|| anyhow!("secret is not encrypted"))?;
        let (nonce_hex, ciphertext_hex) = payload
            .split_once(':')
            .ok_or_else(|| anyhow!("encrypted secret payload is malformed"))?;
        if nonce_hex.len() != NONCE_HEX_LEN {
            bail!("encrypted secret nonce has invalid length");
        }
        if ciphertext_hex.len() < MIN_CIPHERTEXT_HEX_LEN
            || ciphertext_hex.len() > MAX_CIPHERTEXT_HEX_LEN
            || ciphertext_hex.len() % 2 != 0
        {
            bail!("encrypted secret ciphertext has invalid length");
        }

        let nonce_vec = hex::decode(nonce_hex)
            .map_err(|_| anyhow!("encrypted secret nonce is not valid hex"))?;
        let mut nonce_bytes = [0_u8; NONCE_LEN];
        nonce_bytes.copy_from_slice(&nonce_vec);

        let mut in_out = hex::decode(ciphertext_hex)
            .map_err(|_| anyhow!("encrypted secret ciphertext is not valid hex"))?;
        let key = self.less_safe_key()?;
        let plaintext = key
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce_bytes),
                aead::Aad::from(AAD),
                &mut in_out,
            )
            .map_err(|_| anyhow!("failed to decrypt secret"))?;
        String::from_utf8(plaintext.to_vec())
            .map_err(|_| anyhow!("decrypted secret is not valid UTF-8"))
    }

    fn less_safe_key(&self) -> Result<aead::LessSafeKey> {
        let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &self.key)
            .map_err(|_| anyhow!("failed to initialize secret cipher"))?;
        Ok(aead::LessSafeKey::new(unbound))
    }
}

pub async fn migrate_plaintext_secrets(db: &DatabaseBackend) -> Result<usize> {
    let ddns = migrate_ddns_secrets(db).await?;
    let settings = migrate_setting_secrets(db).await?;
    let totp = migrate_totp_secrets(db).await?;
    Ok(ddns + settings + totp)
}

async fn migrate_ddns_secrets(db: &DatabaseBackend) -> Result<usize> {
    let mut changed = 0_usize;
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                "SELECT id, api_token, api_key, api_secret, webhook_url FROM ddns_configs",
            )
            .fetch_all(pool)
            .await?;
            for row in rows {
                let id: String = row.try_get("id")?;
                let current = DdnsSecretColumns {
                    api_token: row.try_get("api_token")?,
                    api_key: row.try_get("api_key")?,
                    api_secret: row.try_get("api_secret")?,
                    webhook_url: row.try_get("webhook_url")?,
                };
                let encrypted = current.encrypted()?;
                if encrypted != current {
                    sqlx::query(
                        "UPDATE ddns_configs SET api_token = ?, api_key = ?, api_secret = ?, webhook_url = ? WHERE id = ?",
                    )
                    .bind(encrypted.api_token)
                    .bind(encrypted.api_key)
                    .bind(encrypted.api_secret)
                    .bind(encrypted.webhook_url)
                    .bind(id)
                    .execute(pool)
                    .await?;
                    changed += 1;
                }
            }
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                "SELECT id::text AS id, api_token, api_key, api_secret, webhook_url FROM ddns_configs",
            )
            .fetch_all(pool)
            .await?;
            for row in rows {
                let id: String = row.try_get("id")?;
                let current = DdnsSecretColumns {
                    api_token: row.try_get("api_token")?,
                    api_key: row.try_get("api_key")?,
                    api_secret: row.try_get("api_secret")?,
                    webhook_url: row.try_get("webhook_url")?,
                };
                let encrypted = current.encrypted()?;
                if encrypted != current {
                    sqlx::query(
                        "UPDATE ddns_configs SET api_token = $1, api_key = $2, api_secret = $3, webhook_url = $4 WHERE id = $5::uuid",
                    )
                    .bind(encrypted.api_token)
                    .bind(encrypted.api_key)
                    .bind(encrypted.api_secret)
                    .bind(encrypted.webhook_url)
                    .bind(id)
                    .execute(pool)
                    .await?;
                    changed += 1;
                }
            }
        }
    }
    Ok(changed)
}

async fn migrate_setting_secrets(db: &DatabaseBackend) -> Result<usize> {
    const SECRET_KEYS: &[&str] = &["geoip_ipinfo_token", "cloudflared_token"];
    let mut changed = 0_usize;
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query("SELECT key, value_json FROM system_settings")
                .fetch_all(pool)
                .await?;
            for row in rows {
                let key: String = row.try_get("key")?;
                if !SECRET_KEYS.contains(&key.as_str()) {
                    continue;
                }
                let value_json: String = row.try_get("value_json")?;
                let value = match parse_migratable_secret_setting(&key, &value_json) {
                    Some(value) => value,
                    None => continue,
                };
                if value.trim().is_empty() || is_encrypted_secret(&value) {
                    continue;
                }
                let encrypted_json = serde_json::to_string(&encrypt_secret(&value)?)?;
                sqlx::query("UPDATE system_settings SET value_json = ? WHERE key = ?")
                    .bind(encrypted_json)
                    .bind(key)
                    .execute(pool)
                    .await?;
                changed += 1;
            }
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query("SELECT key, value_json FROM system_settings")
                .fetch_all(pool)
                .await?;
            for row in rows {
                let key: String = row.try_get("key")?;
                if !SECRET_KEYS.contains(&key.as_str()) {
                    continue;
                }
                let value_json: String = row.try_get("value_json")?;
                let value = match parse_migratable_secret_setting(&key, &value_json) {
                    Some(value) => value,
                    None => continue,
                };
                if value.trim().is_empty() || is_encrypted_secret(&value) {
                    continue;
                }
                let encrypted_json = serde_json::to_string(&encrypt_secret(&value)?)?;
                sqlx::query("UPDATE system_settings SET value_json = $1 WHERE key = $2")
                    .bind(encrypted_json)
                    .bind(key)
                    .execute(pool)
                    .await?;
                changed += 1;
            }
        }
    }
    Ok(changed)
}

fn parse_migratable_secret_setting(key: &str, value_json: &str) -> Option<String> {
    if value_json.len() > SECRET_SETTING_VALUE_JSON_MAX_BYTES {
        tracing::warn!(
            "historical secret setting {key} skipped during plaintext migration: value_json exceeds {SECRET_SETTING_VALUE_JSON_MAX_BYTES} bytes"
        );
        return None;
    }
    match serde_json::from_str::<String>(value_json) {
        Ok(value) if value.len() <= MAX_SECRET_PLAINTEXT_BYTES => Some(value),
        Ok(_) => {
            tracing::warn!(
                "historical secret setting {key} skipped during plaintext migration: decoded string exceeds {MAX_SECRET_PLAINTEXT_BYTES} bytes"
            );
            None
        }
        Err(_) => {
            tracing::warn!(
                "historical secret setting {key} skipped during plaintext migration: value_json is not a valid JSON string"
            );
            None
        }
    }
}

async fn migrate_totp_secrets(db: &DatabaseBackend) -> Result<usize> {
    let mut changed = 0_usize;
    match db {
        DatabaseBackend::Sqlite(pool) => {
            let rows = sqlx::query(
                "SELECT id, totp_secret FROM users WHERE totp_secret IS NOT NULL AND TRIM(totp_secret) <> ''",
            )
            .fetch_all(pool)
            .await?;
            for row in rows {
                let id: String = row.try_get("id")?;
                let secret: String = row.try_get("totp_secret")?;
                if is_encrypted_secret(&secret) {
                    continue;
                }
                sqlx::query("UPDATE users SET totp_secret = ? WHERE id = ?")
                    .bind(encrypt_secret(&secret)?)
                    .bind(id)
                    .execute(pool)
                    .await?;
                changed += 1;
            }
        }
        DatabaseBackend::Postgres(pool) => {
            let rows = sqlx::query(
                "SELECT id::text AS id, totp_secret FROM users WHERE totp_secret IS NOT NULL AND TRIM(totp_secret) <> ''",
            )
            .fetch_all(pool)
            .await?;
            for row in rows {
                let id: String = row.try_get("id")?;
                let secret: String = row.try_get("totp_secret")?;
                if is_encrypted_secret(&secret) {
                    continue;
                }
                sqlx::query("UPDATE users SET totp_secret = $1 WHERE id = $2::uuid")
                    .bind(encrypt_secret(&secret)?)
                    .bind(id)
                    .execute(pool)
                    .await?;
                changed += 1;
            }
        }
    }
    Ok(changed)
}

#[derive(Clone, PartialEq, Eq)]
struct DdnsSecretColumns {
    api_token: Option<String>,
    api_key: Option<String>,
    api_secret: Option<String>,
    webhook_url: Option<String>,
}

impl fmt::Debug for DdnsSecretColumns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DdnsSecretColumns")
            .field("api_token", &self.api_token.as_ref().map(|_| "[REDACTED]"))
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field(
                "api_secret",
                &self.api_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "webhook_url",
                &self.webhook_url.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl DdnsSecretColumns {
    fn encrypted(&self) -> Result<Self> {
        Ok(Self {
            api_token: encrypt_optional_secret(self.api_token.as_deref())?,
            api_key: encrypt_optional_secret(self.api_key.as_deref())?,
            api_secret: encrypt_optional_secret(self.api_secret.as_deref())?,
            webhook_url: encrypt_optional_secret(self.webhook_url.as_deref())?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{repository::ddns::DdnsConfigRepository, CreateUserInput, UserRepository};
    use xlstatus_shared::UserRole;

    #[test]
    fn encrypts_and_decrypts_secret() {
        let crypto = SecretCrypto::new("01234567890123456789012345678901").unwrap();
        let encrypted = crypto.encrypt("secret-value").unwrap();
        assert!(is_encrypted_secret(&encrypted));
        assert_ne!(encrypted, "secret-value");
        assert_eq!(crypto.decrypt(&encrypted).unwrap(), "secret-value");
    }

    #[test]
    fn rejects_wrong_key() {
        let one = SecretCrypto::new("01234567890123456789012345678901").unwrap();
        let two = SecretCrypto::new("abcdefabcdefabcdefabcdefabcdef12").unwrap();
        let encrypted = one.encrypt("secret-value").unwrap();
        assert!(two.decrypt(&encrypted).is_err());
    }

    #[test]
    fn rejects_weak_or_low_entropy_master_keys_and_redacts_debug() {
        assert!(SecretCrypto::new("short-key").is_err());
        assert!(SecretCrypto::new(&"a".repeat(32)).is_err());
        assert!(SecretCrypto::new("replace-with-a-32-byte-random-hex-secret").is_err());

        let crypto = SecretCrypto::new("01234567890123456789012345678901").unwrap();
        let debug_str = format!("{crypto:?}");
        assert!(debug_str.contains("[REDACTED]"));
        assert!(!debug_str.contains("0123456789"));

        let ddns_cols = DdnsSecretColumns {
            api_token: Some("super-secret-ddns-token".into()),
            api_key: Some("super-secret-ddns-key".into()),
            api_secret: Some("super-secret-ddns-secret".into()),
            webhook_url: Some("https://hooks.example.com/secret".into()),
        };
        let ddns_debug = format!("{ddns_cols:?}");
        assert!(ddns_debug.contains("[REDACTED]"));
        assert!(!ddns_debug.contains("super-secret"));
    }

    #[test]
    fn rejects_oversized_plaintext_and_corrupt_ciphertext_before_allocation() {
        let crypto = SecretCrypto::new("01234567890123456789012345678901").unwrap();
        let oversized_plain = "x".repeat(MAX_SECRET_PLAINTEXT_BYTES + 1);
        assert!(crypto.encrypt(&oversized_plain).is_err());
        assert!(decrypt_secret_if_needed(&oversized_plain).is_err());

        let oversized_cipher = format!(
            "{PREFIX}{}:{}",
            "00".repeat(NONCE_LEN),
            "aa".repeat(MAX_SECRET_PLAINTEXT_BYTES + TAG_LEN + 1)
        );
        assert!(crypto.decrypt(&oversized_cipher).is_err());

        let truncated_cipher = format!("{PREFIX}{}:001122", "00".repeat(NONCE_LEN));
        assert!(crypto.decrypt(&truncated_cipher).is_err());
    }

    #[tokio::test]
    async fn migrates_plaintext_database_secrets() {
        let db = test_db().await;
        let user = UserRepository::new(db.clone())
            .create(CreateUserInput {
                username: "owner".into(),
                password: "secret".into(),
                role: UserRole::Admin,
            })
            .await
            .unwrap();

        let row = crate::db::repository::ddns::DdnsConfigRow {
            id: uuid::Uuid::now_v7().to_string(),
            owner_user_id: user.id.0.to_string(),
            agent_id: None,
            name: "cf".into(),
            provider: "cloudflare".into(),
            domain: "example.com".into(),
            record_id: Some("record".into()),
            zone_id: Some("zone".into()),
            api_token: Some("plain-token".into()),
            api_key: Some("plain-key".into()),
            api_secret: Some("plain-secret".into()),
            webhook_url: Some("https://hook.example.com/token".into()),
            current_ip: None,
            last_applied_ip: None,
            last_applied_at: None,
            enabled: true,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        DdnsConfigRepository::create(&db, &row).await.unwrap();

        raw_sqlite(&db, "UPDATE ddns_configs SET api_token = 'legacy-token'")
            .await
            .unwrap();
        raw_sqlite(
            &db,
            "INSERT INTO system_settings (key, value_json, updated_at) VALUES ('geoip_ipinfo_token', '\"legacy-ipinfo\"', 'now')",
        )
        .await
        .unwrap();
        raw_sqlite(
            &db,
            "UPDATE users SET totp_secret = 'LEGACYTOTP', totp_enabled = 1",
        )
        .await
        .unwrap();

        assert_eq!(migrate_plaintext_secrets(&db).await.unwrap(), 3);
        assert_eq!(migrate_plaintext_secrets(&db).await.unwrap(), 0);

        let ddns_raw = scalar_sqlite(
            &db,
            "SELECT api_token FROM ddns_configs WHERE id = (SELECT id FROM ddns_configs LIMIT 1)",
        )
        .await
        .unwrap();
        assert!(is_encrypted_secret(&ddns_raw));
        assert_eq!(decrypt_secret_if_needed(&ddns_raw).unwrap(), "legacy-token");

        let setting_raw = scalar_sqlite(
            &db,
            "SELECT value_json FROM system_settings WHERE key = 'geoip_ipinfo_token'",
        )
        .await
        .unwrap();
        let setting_value: String = serde_json::from_str(&setting_raw).unwrap();
        assert!(is_encrypted_secret(&setting_value));
        assert_eq!(
            decrypt_secret_if_needed(&setting_value).unwrap(),
            "legacy-ipinfo"
        );

        let totp_raw = scalar_sqlite(&db, "SELECT totp_secret FROM users LIMIT 1")
            .await
            .unwrap();
        assert!(is_encrypted_secret(&totp_raw));
        assert_eq!(decrypt_secret_if_needed(&totp_raw).unwrap(), "LEGACYTOTP");
    }

    #[tokio::test]
    async fn secret_setting_migration_skips_invalid_historical_values() {
        let db = test_db().await;
        raw_sqlite(
            &db,
            "INSERT INTO system_settings (key, value_json, updated_at) VALUES ('geoip_ipinfo_token', '{not-json', 'now')",
        )
        .await
        .unwrap();
        raw_sqlite(
            &db,
            "INSERT INTO system_settings (key, value_json, updated_at) VALUES ('cloudflared_token', '123', 'now')",
        )
        .await
        .unwrap();
        raw_sqlite(
            &db,
            "INSERT INTO system_settings (key, value_json, updated_at) VALUES ('public_site_name', '123', 'now')",
        )
        .await
        .unwrap();

        assert_eq!(migrate_plaintext_secrets(&db).await.unwrap(), 0);
        assert_eq!(migrate_plaintext_secrets(&db).await.unwrap(), 0);

        let geoip_raw = scalar_sqlite(
            &db,
            "SELECT value_json FROM system_settings WHERE key = 'geoip_ipinfo_token'",
        )
        .await
        .unwrap();
        assert_eq!(geoip_raw, "{not-json");

        let cloudflared_raw = scalar_sqlite(
            &db,
            "SELECT value_json FROM system_settings WHERE key = 'cloudflared_token'",
        )
        .await
        .unwrap();
        assert_eq!(cloudflared_raw, "123");
    }

    #[tokio::test]
    async fn secret_setting_migration_bounds_historical_value_json() {
        let db = test_db().await;
        let oversized = format!("'{}'", "x".repeat(SECRET_SETTING_VALUE_JSON_MAX_BYTES + 1));
        sqlx::query(
            "INSERT INTO system_settings (key, value_json, updated_at) VALUES (?, ?, 'now')",
        )
        .bind("geoip_ipinfo_token")
        .bind(oversized)
        .execute(match &db {
            DatabaseBackend::Sqlite(pool) => pool,
            _ => unreachable!(),
        })
        .await
        .unwrap();

        assert_eq!(migrate_plaintext_secrets(&db).await.unwrap(), 0);
        let raw = scalar_sqlite(
            &db,
            "SELECT value_json FROM system_settings WHERE key = 'geoip_ipinfo_token'",
        )
        .await
        .unwrap();
        assert_eq!(raw.len(), SECRET_SETTING_VALUE_JSON_MAX_BYTES + 3);
    }

    async fn test_db() -> DatabaseBackend {
        let path =
            std::env::temp_dir().join(format!("xlstatus-secret-test-{}.db", uuid::Uuid::now_v7()));
        let url = format!("sqlite://{}?mode=rwc", path.to_string_lossy());
        let db = DatabaseBackend::connect(&url, true).await.unwrap();
        db.run_migrations().await.unwrap();
        db
    }

    async fn raw_sqlite(db: &DatabaseBackend, query: &str) -> Result<()> {
        match db {
            DatabaseBackend::Sqlite(pool) => {
                sqlx::query(query).execute(pool).await?;
                Ok(())
            }
            DatabaseBackend::Postgres(_) => unreachable!(),
        }
    }

    async fn scalar_sqlite(db: &DatabaseBackend, query: &str) -> Result<String> {
        match db {
            DatabaseBackend::Sqlite(pool) => {
                let row = sqlx::query(query).fetch_one(pool).await?;
                Ok(row.try_get(0)?)
            }
            DatabaseBackend::Postgres(_) => unreachable!(),
        }
    }
}
