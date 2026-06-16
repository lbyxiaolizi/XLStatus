mod jwt;
pub mod middleware;
mod rbac;
mod session;

pub use jwt::{sign_agent_jwt, verify_agent_jwt, AgentClaims};
pub use middleware::{
    csrf_middleware, generate_csrf_token, session_middleware, AuthSession, AuthUser,
    CSRF_HEADER_NAME, SESSION_COOKIE_NAME,
};
pub use rbac::{has_scope, require_admin, require_auth, require_scope};
pub use session::SessionRepository;

use rand::Rng;
use sha2::{Digest, Sha256};

/// Generate a random session token
pub fn generate_session_token() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.gen();
    hex::encode(bytes)
}

/// Hash a session token for storage
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Generate a Personal Access Token with xlp_ prefix
pub fn generate_pat() -> (String, String) {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 32] = rng.gen();
    let token_body = hex::encode(bytes);
    let token = format!("xlp_{}", token_body);
    let hash = hash_token(&token);
    (token, hash)
}
