#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::{bail, Context, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};
use tokio::net::lookup_host;

pub const HTTP_MIN_TIMEOUT_SECONDS: u32 = 1;
pub const HTTP_DEFAULT_TIMEOUT_SECONDS: u32 = 10;
pub const HTTP_MAX_TIMEOUT_SECONDS: u32 = 30;
pub const HTTP_MAX_URL_BYTES: usize = 2048;
pub const HTTP_MAX_HEADERS: usize = 32;
pub const HTTP_MAX_HEADER_NAME_BYTES: usize = 128;
pub const HTTP_MAX_HEADER_VALUE_BYTES: usize = 4096;
pub const HTTP_MAX_RESPONSE_BODY_BYTES: usize = 64 * 1024;

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

pub fn bounded_http_timeout_seconds(value: u32) -> u32 {
    if value == 0 {
        HTTP_DEFAULT_TIMEOUT_SECONDS
    } else {
        value.clamp(HTTP_MIN_TIMEOUT_SECONDS, HTTP_MAX_TIMEOUT_SECONDS)
    }
}

pub fn validate_http_headers(headers: &[(String, String)]) -> Result<reqwest::header::HeaderMap> {
    if headers.len() > HTTP_MAX_HEADERS {
        bail!(
            "HTTP header count {} exceeds maximum of {}",
            headers.len(),
            HTTP_MAX_HEADERS
        );
    }

    let mut header_map = reqwest::header::HeaderMap::with_capacity(headers.len());
    for (key, value) in headers {
        if key.is_empty() || key.len() > HTTP_MAX_HEADER_NAME_BYTES {
            bail!(
                "Invalid HTTP header name length {} (must be 1..={} bytes)",
                key.len(),
                HTTP_MAX_HEADER_NAME_BYTES
            );
        }
        if key.contains('\0')
            || key.contains('\r')
            || key.contains('\n')
            || key.chars().any(char::is_control)
        {
            bail!(
                "HTTP header name '{}' contains disallowed control characters",
                key
            );
        }
        if value.len() > HTTP_MAX_HEADER_VALUE_BYTES {
            bail!(
                "HTTP header '{}' value exceeds maximum of {} bytes",
                key,
                HTTP_MAX_HEADER_VALUE_BYTES
            );
        }
        if value.contains('\0')
            || value.contains('\r')
            || value.contains('\n')
            || value.chars().any(|c| c.is_control() && c != '\t')
        {
            bail!(
                "HTTP header '{}' value contains disallowed newline or control characters",
                key
            );
        }

        let name = reqwest::header::HeaderName::from_bytes(key.as_bytes())
            .with_context(|| format!("Invalid HTTP header name '{}'", key))?;
        let val = reqwest::header::HeaderValue::from_str(value)
            .with_context(|| format!("Invalid HTTP header value for '{}'", key))?;
        header_map.append(name, val);
    }

    Ok(header_map)
}

fn private_probes_allowed_by_env() -> bool {
    [
        "XLSTATUS_AGENT_ALLOW_PRIVATE_PROBES",
        "XLSTATUS_ALLOW_PRIVATE_OUTBOUND",
    ]
    .iter()
    .any(|name| {
        std::env::var(name)
            .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
    })
}

pub fn is_blocked_probe_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_blocked_probe_ipv4(ip),
        IpAddr::V6(ip) => is_blocked_probe_ipv6(ip),
    }
}

fn is_blocked_probe_ipv4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_documentation()
        || o[0] == 0
        || o[0] >= 224
        || (o[0] == 100 && (64..=127).contains(&o[1]))
        || (o[0] == 169 && o[1] == 254)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 198 && (18..=19).contains(&o[1]))
        || o == [255, 255, 255, 255]
}

fn is_blocked_probe_ipv6(ip: Ipv6Addr) -> bool {
    let segments = ip.segments();
    let first = segments[0];
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8)
        || ip
            .to_ipv4_mapped()
            .map(is_blocked_probe_ipv4)
            .unwrap_or(false)
}

async fn validate_and_resolve_url(
    url: &str,
    allow_private: bool,
) -> Result<(reqwest::Url, String, Vec<SocketAddr>)> {
    let url = url.trim();
    if url.is_empty() || url.len() > HTTP_MAX_URL_BYTES {
        bail!(
            "HTTP URL length must be between 1 and {} bytes",
            HTTP_MAX_URL_BYTES
        );
    }
    if url.contains('\0') || url.chars().any(char::is_control) {
        bail!("HTTP URL contains disallowed control characters");
    }

    let parsed = reqwest::Url::parse(url).context("HTTP URL is invalid")?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => bail!("HTTP URL scheme '{}' is not allowed", scheme),
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("HTTP URL must not include credentials");
    }

    let host = parsed
        .host_str()
        .context("HTTP URL must include a host")?
        .to_string();
    let port = parsed
        .port_or_known_default()
        .context("HTTP URL must include a valid port or scheme")?;

    let host_unbracketed = host.trim_matches(['[', ']']);
    if let Ok(ip) = host_unbracketed.parse::<IpAddr>() {
        if !allow_private && is_blocked_probe_ip(ip) {
            bail!(
                "HTTP target resolves to disallowed private or special-use address {}",
                ip
            );
        }
        return Ok((parsed, host, vec![SocketAddr::new(ip, port)]));
    }

    let resolved_iter = lookup_host((host_unbracketed, port))
        .await
        .with_context(|| format!("Failed to resolve HTTP target host '{}'", host))?;
    let mut addrs = Vec::new();
    for addr in resolved_iter {
        if !allow_private && is_blocked_probe_ip(addr.ip()) {
            bail!(
                "HTTP target '{}' resolves to disallowed private or special-use address {}",
                host,
                addr.ip()
            );
        }
        addrs.push(addr);
    }
    if addrs.is_empty() {
        bail!("HTTP target host '{}' did not resolve to any address", host);
    }

    Ok((parsed, host, addrs))
}

async fn read_bounded_response_body(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Failed to read response body chunk")?
    {
        let remaining = max_bytes.saturating_sub(body.len());
        if remaining == 0 {
            break;
        }
        let take = remaining.min(chunk.len());
        body.extend_from_slice(&chunk[..take]);
        if body.len() >= max_bytes {
            break;
        }
    }
    Ok(body)
}

/// Execute HTTP GET request
#[allow(dead_code)]
pub async fn execute_http_get(
    url: &str,
    timeout_seconds: u32,
    verify_tls: bool,
    headers: &[(String, String)],
) -> Result<HttpGetResult> {
    execute_http_get_with_policy(
        url,
        timeout_seconds,
        verify_tls,
        headers,
        private_probes_allowed_by_env(),
    )
    .await
}

pub async fn execute_http_get_with_policy(
    url: &str,
    timeout_seconds: u32,
    verify_tls: bool,
    headers: &[(String, String)],
    allow_private: bool,
) -> Result<HttpGetResult> {
    let start = Instant::now();
    let timeout_seconds = bounded_http_timeout_seconds(timeout_seconds);
    let header_map = validate_http_headers(headers)?;
    let (parsed_url, host, addrs) = validate_and_resolve_url(url, allow_private).await?;

    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(timeout_seconds as u64))
        .danger_accept_invalid_certs(!verify_tls);

    if host.parse::<IpAddr>().is_err() && !host.starts_with('[') {
        builder = builder.resolve_to_addrs(&host, &addrs);
    }

    let client = builder.build().context("Failed to build HTTP client")?;

    let response = client
        .get(parsed_url)
        .headers(header_map)
        .send()
        .await
        .context("Failed to send HTTP request")?;

    let latency_ms = start.elapsed().as_millis() as u64;
    let status_code = response.status().as_u16();

    // Extract TLS certificate info if available
    let (cert_fingerprint, cert_not_after) = extract_cert_info(&response);

    let body = read_bounded_response_body(response, HTTP_MAX_RESPONSE_BODY_BYTES).await?;

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

    #[test]
    fn test_validate_http_headers_rejects_crlf_and_control_chars() {
        assert!(
            validate_http_headers(&[("X-Valid".to_string(), "good-value".to_string())]).is_ok()
        );

        for (k, v) in [
            ("X-Bad\r\nHeader", "val"),
            ("X-Bad\0Header", "val"),
            ("", "val"),
            ("X-Header", "bad\r\nInjected: true"),
            ("X-Header", "bad\nvalue"),
            ("X-Header", "bad\0value"),
        ] {
            let headers = vec![(k.to_string(), v.to_string())];
            assert!(
                validate_http_headers(&headers).is_err(),
                "expected header ({k:?}, {v:?}) to be rejected"
            );
        }

        let too_many: Vec<(String, String)> = (0..=HTTP_MAX_HEADERS)
            .map(|i| (format!("X-H-{i}"), "v".to_string()))
            .collect();
        assert!(validate_http_headers(&too_many).is_err());
    }

    #[tokio::test]
    async fn test_http_rejects_ssrf_and_invalid_schemes_by_default() {
        for bad_url in [
            "http://127.0.0.1:8080/admin",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1/internal",
            "http://[::1]:8080/",
            "file:///etc/passwd",
            "https://user:pass@1.1.1.1/",
        ] {
            let res = execute_http_get_with_policy(bad_url, 5, true, &[], false).await;
            assert!(res.is_err(), "expected {bad_url} to be blocked");
        }
    }

    #[tokio::test]
    async fn test_http_disables_redirects_and_bounds_response_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        // 1. Test that 302 redirects are NOT followed (prevents SSRF redirect bypass)
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 1024];
            let _ = stream.read(&mut req_buf).await;
            let response = b"HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest/meta-data/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(response).await.unwrap();
        });

        let url = format!("http://{}/redirect", addr);
        let res = execute_http_get_with_policy(&url, 5, false, &[], true)
            .await
            .unwrap();
        assert_eq!(res.status_code, 302);
        server_task.await.unwrap();

        // 2. Test that oversized response bodies are bounded to HTTP_MAX_RESPONSE_BODY_BYTES (64 KiB)
        let listener2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr2 = listener2.local_addr().unwrap();
        let oversized_len = HTTP_MAX_RESPONSE_BODY_BYTES * 2;
        let server_task2 = tokio::spawn(async move {
            let (mut stream, _) = listener2.accept().await.unwrap();
            let mut req_buf = [0u8; 1024];
            let _ = stream.read(&mut req_buf).await;
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                oversized_len
            );
            let _ = stream.write_all(header.as_bytes()).await;
            let payload = vec![b'A'; oversized_len];
            let _ = stream.write_all(&payload).await;
        });

        let url2 = format!("http://{}/large", addr2);
        let res2 = execute_http_get_with_policy(&url2, 5, false, &[], true)
            .await
            .unwrap();
        assert_eq!(res2.status_code, 200);
        assert_eq!(res2.body.len(), HTTP_MAX_RESPONSE_BODY_BYTES);
        server_task2.await.unwrap();
    }
}
