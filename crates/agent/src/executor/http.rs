#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::{Context, Result};
use std::time::{Duration, Instant};

/// HTTP GET execution result
#[derive(Debug)]
#[allow(dead_code)]
pub struct HttpGetResult {
    pub status_code: u16,
    pub latency_ms: u64,
    pub body: Vec<u8>,
    pub cert_fingerprint: Option<String>,
    pub cert_not_after: Option<i64>,
}

/// Execute HTTP GET request
#[allow(dead_code)]
pub async fn execute_http_get(
    url: &str,
    timeout_seconds: u32,
    verify_tls: bool,
    headers: &[(String, String)],
) -> Result<HttpGetResult> {
    let start = Instant::now();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds as u64))
        .danger_accept_invalid_certs(!verify_tls)
        .build()
        .context("Failed to build HTTP client")?;

    let mut request = client.get(url);

    for (key, value) in headers {
        request = request.header(key, value);
    }

    let response = request
        .send()
        .await
        .context("Failed to send HTTP request")?;

    let latency_ms = start.elapsed().as_millis() as u64;
    let status_code = response.status().as_u16();

    // Extract TLS certificate info if available
    let (cert_fingerprint, cert_not_after) = extract_cert_info(&response);

    let body = response
        .bytes()
        .await
        .context("Failed to read response body")?
        .to_vec();

    Ok(HttpGetResult {
        status_code,
        latency_ms,
        body,
        cert_fingerprint,
        cert_not_after,
    })
}

#[allow(dead_code)]
fn extract_cert_info(_response: &reqwest::Response) -> (Option<String>, Option<i64>) {
    // TODO: Extract certificate information from TLS connection
    // This requires access to the underlying connection which reqwest doesn't expose directly
    // For now, return None - we can implement this in a later iteration using a custom connector
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "depends on httpbin.org availability"]
    async fn test_http_get_success() {
        // Use a reliable public endpoint
        let result = execute_http_get("https://httpbin.org/get", 10, true, &[]).await;

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.status_code, 200);
        assert!(result.latency_ms > 0);
    }

    #[tokio::test]
    #[ignore = "depends on httpbin.org availability"]
    async fn test_http_get_with_headers() {
        let headers = vec![("User-Agent".to_string(), "XLStatus-Agent/0.1.0".to_string())];

        let result = execute_http_get("https://httpbin.org/headers", 10, true, &headers).await;

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.status_code, 200);
        let body_str = String::from_utf8_lossy(&result.body);
        assert!(body_str.contains("XLStatus-Agent"));
    }

    #[tokio::test]
    #[ignore = "depends on httpbin.org availability"]
    async fn test_http_get_404() {
        let result = execute_http_get("https://httpbin.org/status/404", 10, true, &[]).await;

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.status_code, 404);
    }

    #[tokio::test]
    #[ignore = "depends on httpbin.org availability"]
    async fn test_http_get_timeout() {
        let result = execute_http_get("https://httpbin.org/delay/10", 1, true, &[]).await;

        assert!(result.is_err());
    }
}
