use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const TOTP_STEP_SECONDS: i64 = 30;
const TOTP_DIGITS: u32 = 6;
const TOTP_WINDOW: i64 = 1;

pub fn generate_totp_secret() -> String {
    let mut bytes = [0_u8; 20];
    rand::thread_rng().fill_bytes(&mut bytes);
    base32_encode(&bytes)
}

pub fn verify_totp_code(secret: &str, code: &str, now: DateTime<Utc>) -> bool {
    let normalized = code.trim().replace(' ', "");
    if normalized.len() != TOTP_DIGITS as usize
        || !normalized.bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }

    let Ok(secret_bytes) = base32_decode(secret) else {
        return false;
    };
    let counter = now.timestamp().div_euclid(TOTP_STEP_SECONDS);
    (counter - TOTP_WINDOW..=counter + TOTP_WINDOW).any(|candidate| {
        candidate >= 0 && format_totp(totp_at(&secret_bytes, candidate as u64)) == normalized
    })
}

pub fn otpauth_uri(issuer: &str, account: &str, secret: &str) -> String {
    let label = format!("{}:{}", issuer, account);
    format!(
        "otpauth://totp/{}?secret={}&issuer={}&algorithm=SHA1&digits={}&period={}",
        uri_component(&label),
        secret,
        uri_component(issuer),
        TOTP_DIGITS,
        TOTP_STEP_SECONDS
    )
}

fn totp_at(secret: &[u8], counter: u64) -> u32 {
    let mut mac = HmacSha1::new_from_slice(secret).expect("HMAC accepts arbitrary key lengths");
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 0x0f) as usize;
    let binary = ((u32::from(digest[offset]) & 0x7f) << 24)
        | (u32::from(digest[offset + 1]) << 16)
        | (u32::from(digest[offset + 2]) << 8)
        | u32::from(digest[offset + 3]);
    binary % 10_u32.pow(TOTP_DIGITS)
}

fn format_totp(value: u32) -> String {
    format!("{value:0width$}", width = TOTP_DIGITS as usize)
}

fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() * 8).div_ceil(5));
    let mut buffer = 0_u32;
    let mut bits_left = 0_u8;

    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits_left += 8;
        while bits_left >= 5 {
            let index = ((buffer >> (bits_left - 5)) & 0x1f) as usize;
            out.push(BASE32_ALPHABET[index] as char);
            bits_left -= 5;
        }
    }

    if bits_left > 0 {
        let index = ((buffer << (5 - bits_left)) & 0x1f) as usize;
        out.push(BASE32_ALPHABET[index] as char);
    }

    out
}

fn base32_decode(value: &str) -> Result<Vec<u8>, ()> {
    let mut buffer = 0_u32;
    let mut bits_left = 0_u8;
    let mut out = Vec::new();

    for byte in value.bytes() {
        let byte = match byte {
            b'=' | b' ' | b'-' => continue,
            b'a'..=b'z' => byte - b'a' + b'A',
            _ => byte,
        };
        let Some(index) = BASE32_ALPHABET
            .iter()
            .position(|candidate| *candidate == byte)
        else {
            return Err(());
        };
        buffer = (buffer << 5) | index as u32;
        bits_left += 5;
        if bits_left >= 8 {
            out.push(((buffer >> (bits_left - 8)) & 0xff) as u8);
            bits_left -= 8;
        }
    }

    if out.is_empty() {
        return Err(());
    }
    Ok(out)
}

fn uri_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn matches_rfc6238_sha1_vector_truncated_to_six_digits() {
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let now = Utc.timestamp_opt(59, 0).single().unwrap();
        assert!(verify_totp_code(secret, "287082", now));
    }

    #[test]
    fn base32_round_trips_generated_secret() {
        let secret = generate_totp_secret();
        let decoded = base32_decode(&secret).unwrap();
        assert_eq!(decoded.len(), 20);
    }

    #[test]
    fn rejects_non_numeric_code() {
        assert!(!verify_totp_code("GEZDGNBVGY3TQOJQ", "12x456", Utc::now()));
    }
}
