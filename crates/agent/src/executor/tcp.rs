use anyhow::{Context, Result};
use std::net::ToSocketAddrs;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// TCP ping result
#[derive(Debug)]
pub struct TcpPingResult {
    pub latency_ms: u64,
}

/// Execute TCP ping (connect and immediately close)
pub async fn execute_tcp_ping(
    host: &str,
    port: u16,
    timeout_seconds: u32,
) -> Result<TcpPingResult> {
    let start = Instant::now();

    let addr = format!("{}:{}", host, port)
        .to_socket_addrs()
        .context("Failed to resolve address")?
        .next()
        .context("No address resolved")?;

    let timeout_duration = Duration::from_secs(timeout_seconds as u64);

    timeout(timeout_duration, TcpStream::connect(addr))
        .await
        .context("Connection timed out")?
        .context("Failed to connect")?;

    let latency_ms = start.elapsed().as_millis() as u64;

    Ok(TcpPingResult { latency_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tcp_ping_success() {
        // Test against a known public service (Google DNS)
        let result = execute_tcp_ping("8.8.8.8", 53, 5).await;
        assert!(result.is_ok());
        let result = result.unwrap();
        assert!(result.latency_ms > 0);
        assert!(result.latency_ms < 5000);
    }

    #[tokio::test]
    async fn test_tcp_ping_refused() {
        // Try to connect to a port that's likely not open
        let result = execute_tcp_ping("127.0.0.1", 9999, 2).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tcp_ping_timeout() {
        // Use a non-routable IP to trigger timeout
        let result = execute_tcp_ping("10.255.255.1", 80, 1).await;
        assert!(result.is_err());
    }
}
