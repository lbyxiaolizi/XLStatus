#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::{bail, Context, Result};
use std::net::IpAddr;
use std::time::{Duration, Instant};
use tokio::net::{lookup_host, TcpStream};
use tokio::time::timeout;

const TCP_PING_MIN_TIMEOUT_SECONDS: u32 = 1;
const TCP_PING_DEFAULT_TIMEOUT_SECONDS: u32 = 10;
const TCP_PING_MAX_TIMEOUT_SECONDS: u32 = 30;
const TCP_PING_MAX_HOST_BYTES: usize = 253;

/// TCP ping result
#[derive(Debug)]
#[allow(dead_code)]
pub struct TcpPingResult {
    pub latency_ms: u64,
}

pub fn bounded_tcp_timeout_seconds(value: u32) -> u32 {
    if value == 0 {
        TCP_PING_DEFAULT_TIMEOUT_SECONDS
    } else {
        value.clamp(TCP_PING_MIN_TIMEOUT_SECONDS, TCP_PING_MAX_TIMEOUT_SECONDS)
    }
}

pub fn validate_tcp_host(host: &str) -> Result<&str> {
    if host.is_empty() || host.len() > TCP_PING_MAX_HOST_BYTES {
        bail!(
            "TCP ping host length must be between 1 and {} bytes",
            TCP_PING_MAX_HOST_BYTES
        );
    }
    if host.starts_with('-') {
        bail!("TCP ping host must not start with '-'");
    }
    if host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!("TCP ping host must not contain whitespace or control characters");
    }
    let unbracketed = host.trim_matches(['[', ']']);
    if unbracketed.is_empty() {
        bail!("TCP ping host must not be empty");
    }
    if unbracketed.parse::<IpAddr>().is_err()
        && !unbracketed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
    {
        bail!("TCP ping host contains invalid characters");
    }
    Ok(unbracketed)
}

/// Execute TCP ping (connect and immediately close)
#[allow(dead_code)]
pub async fn execute_tcp_ping(
    host: &str,
    port: u16,
    timeout_seconds: u32,
) -> Result<TcpPingResult> {
    if port == 0 {
        bail!("TCP ping port must be greater than 0");
    }
    let host = validate_tcp_host(host)?;
    let timeout_seconds = bounded_tcp_timeout_seconds(timeout_seconds);
    let timeout_duration = Duration::from_secs(timeout_seconds as u64);

    let start = Instant::now();

    timeout(timeout_duration, async {
        let mut addrs = lookup_host((host, port))
            .await
            .context("Failed to resolve address")?;
        let first_addr = addrs.next().context("No address resolved")?;
        TcpStream::connect(first_addr)
            .await
            .context("Failed to connect")
    })
    .await
    .context("Connection timed out")??;

    let latency_ms = start.elapsed().as_millis() as u64;

    Ok(TcpPingResult { latency_ms })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_tcp_ping_success() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accept_task = tokio::spawn(async move {
            let _ = listener.accept().await;
        });

        let result = execute_tcp_ping("127.0.0.1", port, 5).await;
        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(result.latency_ms < 5000);
        accept_task.await.unwrap();
    }

    #[tokio::test]
    async fn test_tcp_ping_refused() {
        // Try to connect to a port that's likely not open
        let result = execute_tcp_ping("127.0.0.1", 9999, 2).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tcp_ping_unreachable() {
        let result = execute_tcp_ping("127.0.0.1", 0, 1).await;
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_tcp_host_and_timeout_bounds() {
        assert_eq!(
            bounded_tcp_timeout_seconds(0),
            TCP_PING_DEFAULT_TIMEOUT_SECONDS
        );
        assert_eq!(
            bounded_tcp_timeout_seconds(u32::MAX),
            TCP_PING_MAX_TIMEOUT_SECONDS
        );

        assert!(validate_tcp_host("127.0.0.1").is_ok());
        assert!(validate_tcp_host("[::1]").is_ok());
        assert!(validate_tcp_host("example.com").is_ok());

        for bad in ["", "-flag", "bad host", "bad\nhost", "bad\0host"] {
            assert!(
                validate_tcp_host(bad).is_err(),
                "expected '{bad}' to be rejected"
            );
        }
    }
}
