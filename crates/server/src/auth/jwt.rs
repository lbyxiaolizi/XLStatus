#![allow(dead_code)]
#![allow(unused)]

use anyhow::Result;
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use xlstatus_shared::AgentId;

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentClaims {
    pub sub: String, // agent_id
    pub exp: i64,    // expiration timestamp
    pub iat: i64,    // issued at
}

pub fn sign_agent_jwt(agent_id: AgentId, secret: &str) -> Result<String> {
    let now = Utc::now();
    let exp = now + Duration::minutes(5);

    let claims = AgentClaims {
        sub: agent_id.0.to_string(),
        exp: exp.timestamp(),
        iat: now.timestamp(),
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    Ok(token)
}

pub fn verify_agent_jwt(token: &str, secret: &str) -> Result<AgentClaims> {
    let validation = Validation::default();
    let token_data = decode::<AgentClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;

    Ok(token_data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_roundtrip() {
        let agent_id = AgentId::new();
        let secret = "test-secret";

        let token = sign_agent_jwt(agent_id, secret).unwrap();
        let claims = verify_agent_jwt(&token, secret).unwrap();

        assert_eq!(claims.sub, agent_id.0.to_string());
    }
}
