#![allow(dead_code)]
#![allow(unused)]

use anyhow::{anyhow, bail, Result};
use chrono::{Duration, Utc};
use jsonwebtoken::{
    decode, decode_header, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use xlstatus_shared::AgentId;

pub const MAX_JWT_BYTES: usize = 4096;
pub const MIN_JWT_SECRET_BYTES: usize = 32;
const MIN_JWT_SECRET_DISTINCT_BYTES: usize = 4;
const AGENT_JWT_TTL_MINUTES: i64 = 5;
const MAX_AGENT_JWT_LIFETIME_SECONDS: i64 = 15 * 60;
const MAX_CLOCK_SKEW_SECONDS: i64 = 60;
const MAX_JTI_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentClaims {
    pub sub: String, // agent_id
    pub exp: i64,    // expiration timestamp
    pub iat: i64,    // issued at
    #[serde(default)]
    pub jti: String, // unique token ID
}

pub fn sign_agent_jwt(agent_id: AgentId, secret: &str) -> Result<String> {
    let secret = validate_jwt_secret(secret)?;
    if agent_id.0.is_nil() {
        bail!("agent_id must not be nil");
    }

    let now = Utc::now();
    let exp = now + Duration::minutes(AGENT_JWT_TTL_MINUTES);

    let claims = AgentClaims {
        sub: agent_id.0.to_string(),
        exp: exp.timestamp(),
        iat: now.timestamp(),
        jti: uuid::Uuid::now_v7().to_string(),
    };

    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|_| anyhow!("failed to sign agent JWT"))?;

    Ok(token)
}

pub fn verify_agent_jwt(token: &str, secret: &str) -> Result<AgentClaims> {
    let secret = validate_jwt_secret(secret)?;
    validate_jwt_wire_format(token)?;

    let header = decode_header(token).map_err(|_| anyhow!("invalid JWT header"))?;
    if header.alg != Algorithm::HS256 {
        bail!("unsupported JWT algorithm");
    }

    let mut validation = Validation::new(Algorithm::HS256);
    validation.algorithms = vec![Algorithm::HS256];
    validation.validate_exp = true;
    validation.leeway = MAX_CLOCK_SKEW_SECONDS as u64;
    validation.set_required_spec_claims(&["sub", "exp", "iat", "jti"]);

    let token_data = decode::<AgentClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|_| anyhow!("invalid or expired JWT"))?;

    validate_agent_claims(&token_data.claims, Utc::now().timestamp())?;
    Ok(token_data.claims)
}

fn validate_jwt_secret(secret: &str) -> Result<&str> {
    let trimmed = secret.trim();
    if trimmed.len() < MIN_JWT_SECRET_BYTES {
        bail!("JWT signing secret must be at least {MIN_JWT_SECRET_BYTES} bytes");
    }
    let weak_defaults = [
        "CHANGE_ME_IN_PRODUCTION",
        "change-me-in-production",
        "replace-with-a-long-random-secret",
        "replace-with-a-different-long-random-secret",
        "replace-with-a-32-byte-random-hex-secret",
        "replace-with-a-different-32-byte-random-hex-secret",
    ];
    if weak_defaults.contains(&trimmed) {
        bail!("JWT signing secret uses an insecure placeholder value");
    }
    let distinct: HashSet<u8> = trimmed.bytes().collect();
    if distinct.len() < MIN_JWT_SECRET_DISTINCT_BYTES {
        bail!("JWT signing secret has insufficient entropy");
    }
    Ok(trimmed)
}

fn validate_jwt_wire_format(token: &str) -> Result<()> {
    if token.is_empty() || token.len() > MAX_JWT_BYTES || !token.is_ascii() {
        bail!("JWT token length or encoding is invalid");
    }
    let mut segments = token.split('.');
    let Some(header) = segments.next() else {
        bail!("malformed JWT token");
    };
    let Some(payload) = segments.next() else {
        bail!("malformed JWT token");
    };
    let Some(signature) = segments.next() else {
        bail!("malformed JWT token");
    };
    if segments.next().is_some()
        || !is_base64url_segment(header)
        || !is_base64url_segment(payload)
        || !is_base64url_segment(signature)
    {
        bail!("malformed JWT token");
    }
    Ok(())
}

fn is_base64url_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn validate_agent_claims(claims: &AgentClaims, now: i64) -> Result<()> {
    if claims.sub.trim().is_empty() {
        bail!("JWT sub claim must not be empty");
    }
    let parsed_sub =
        uuid::Uuid::parse_str(&claims.sub).map_err(|_| anyhow!("JWT sub must be a valid UUID"))?;
    if parsed_sub.is_nil() || parsed_sub.to_string() != claims.sub {
        bail!("JWT sub must be a canonical non-nil UUID");
    }

    let jti = claims.jti.trim();
    if jti.is_empty() || jti.len() > MAX_JTI_BYTES || jti.bytes().any(|b| b.is_ascii_control()) {
        bail!("JWT jti claim is invalid");
    }

    if claims.iat <= 0 || claims.iat > now + MAX_CLOCK_SKEW_SECONDS {
        bail!("JWT iat claim is out of acceptable clock skew bounds");
    }
    if claims.exp <= now - MAX_CLOCK_SKEW_SECONDS {
        bail!("JWT has expired");
    }
    if claims.exp <= claims.iat || (claims.exp - claims.iat) > MAX_AGENT_JWT_LIFETIME_SECONDS {
        bail!("JWT lifetime is invalid");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_SECRET: &str = "test-secret-key-for-development-only-2026";

    fn base64url_encode(input: &[u8]) -> String {
        const TABLE: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
        for chunk in input.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
            let triple = (b0 << 16) | (b1 << 8) | b2;
            out.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
            out.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
            if chunk.len() > 1 {
                out.push(TABLE[((triple >> 6) & 0x3f) as usize] as char);
            }
            if chunk.len() > 2 {
                out.push(TABLE[(triple & 0x3f) as usize] as char);
            }
        }
        out
    }

    #[test]
    fn test_jwt_roundtrip() {
        let agent_id = AgentId::new();

        let token = sign_agent_jwt(agent_id, VALID_SECRET).unwrap();
        let claims = verify_agent_jwt(&token, VALID_SECRET).unwrap();

        assert_eq!(claims.sub, agent_id.0.to_string());
        assert!(!claims.jti.is_empty());
    }

    #[test]
    fn rejects_short_or_low_entropy_signing_secrets() {
        let agent_id = AgentId::new();
        assert!(sign_agent_jwt(agent_id, "too-short-secret").is_err());
        assert!(sign_agent_jwt(agent_id, &"a".repeat(32)).is_err());
        assert!(sign_agent_jwt(agent_id, "replace-with-a-32-byte-random-hex-secret").is_err());

        let valid_token = sign_agent_jwt(agent_id, VALID_SECRET).unwrap();
        assert!(verify_agent_jwt(&valid_token, "too-short-secret").is_err());
        assert!(verify_agent_jwt(&valid_token, &"a".repeat(32)).is_err());
    }

    #[test]
    fn rejects_oversized_and_malformed_jwt_tokens() {
        let oversized = format!("a.{}.c", "b".repeat(MAX_JWT_BYTES));
        assert!(verify_agent_jwt(&oversized, VALID_SECRET).is_err());
        assert!(verify_agent_jwt("", VALID_SECRET).is_err());
        assert!(verify_agent_jwt("header.payload.", VALID_SECRET).is_err());
    }

    #[test]
    fn rejects_alg_none_and_non_hs256_headers() {
        let agent_id = AgentId::new();
        let now = Utc::now().timestamp();
        let claims = serde_json::json!({
            "sub": agent_id.0.to_string(),
            "iat": now,
            "exp": now + 300,
            "jti": uuid::Uuid::now_v7().to_string(),
        });
        let none_header = base64url_encode(br#"{"alg":"none","typ":"JWT"}"#);
        let payload = base64url_encode(claims.to_string().as_bytes());
        let unsigned_token = format!("{none_header}.{payload}.sig");
        assert!(verify_agent_jwt(&unsigned_token, VALID_SECRET).is_err());

        let hs384_token = encode(
            &Header::new(Algorithm::HS384),
            &AgentClaims {
                sub: agent_id.0.to_string(),
                iat: now,
                exp: now + 300,
                jti: uuid::Uuid::now_v7().to_string(),
            },
            &EncodingKey::from_secret(VALID_SECRET.as_bytes()),
        )
        .unwrap();
        assert!(verify_agent_jwt(&hs384_token, VALID_SECRET).is_err());
    }

    #[test]
    fn rejects_future_iat_expired_exp_and_empty_sub_or_jti() {
        let agent_id = AgentId::new();
        let now = Utc::now().timestamp();

        let future_iat_token = encode(
            &Header::new(Algorithm::HS256),
            &AgentClaims {
                sub: agent_id.0.to_string(),
                iat: now + MAX_CLOCK_SKEW_SECONDS + 30,
                exp: now + 300,
                jti: uuid::Uuid::now_v7().to_string(),
            },
            &EncodingKey::from_secret(VALID_SECRET.as_bytes()),
        )
        .unwrap();
        assert!(verify_agent_jwt(&future_iat_token, VALID_SECRET).is_err());

        let expired_token = encode(
            &Header::new(Algorithm::HS256),
            &AgentClaims {
                sub: agent_id.0.to_string(),
                iat: now - 600,
                exp: now - MAX_CLOCK_SKEW_SECONDS - 10,
                jti: uuid::Uuid::now_v7().to_string(),
            },
            &EncodingKey::from_secret(VALID_SECRET.as_bytes()),
        )
        .unwrap();
        assert!(verify_agent_jwt(&expired_token, VALID_SECRET).is_err());

        let empty_sub_token = encode(
            &Header::new(Algorithm::HS256),
            &AgentClaims {
                sub: String::new(),
                iat: now,
                exp: now + 300,
                jti: uuid::Uuid::now_v7().to_string(),
            },
            &EncodingKey::from_secret(VALID_SECRET.as_bytes()),
        )
        .unwrap();
        assert!(verify_agent_jwt(&empty_sub_token, VALID_SECRET).is_err());

        let empty_jti_token = encode(
            &Header::new(Algorithm::HS256),
            &AgentClaims {
                sub: agent_id.0.to_string(),
                iat: now,
                exp: now + 300,
                jti: String::new(),
            },
            &EncodingKey::from_secret(VALID_SECRET.as_bytes()),
        )
        .unwrap();
        assert!(verify_agent_jwt(&empty_jti_token, VALID_SECRET).is_err());
    }
}
