#![allow(dead_code)]
#![allow(unused_imports)]

use crate::security::{
    resolve_outbound_host, secure_reqwest_client_builder, validate_outbound_url_resolved,
    ValidatedOutboundUrl,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::{DigitallySignedStruct, SignatureScheme};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use x509_parser::prelude::{FromDer, X509Certificate};

pub const SERVICE_HTTP_PROBE_MAX_RESPONSE_BYTES: usize = 64 * 1024;
const SERVICE_PING_OUTPUT_MAX_BYTES: usize = 4096;
const SERVICE_PING_PROCESS_TIMEOUT_GRACE_SECONDS: u64 = 2;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ServiceProbe {
    pub id: String,
    pub service_id: String,
    pub success: bool,
    pub latency_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub error: Option<String>,
    pub cert_fingerprint: Option<String>,
    pub cert_not_after: Option<DateTime<Utc>>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum ProbeType {
    Http,
    Tcp,
    Icmp,
}

impl ProbeType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "http" => Some(ProbeType::Http),
            "tcp" => Some(ProbeType::Tcp),
            "icmp" => Some(ProbeType::Icmp),
            _ => None,
        }
    }
}

pub async fn probe_http(url: &str, timeout_secs: u64) -> Result<ServiceProbe> {
    let timeout_secs = timeout_secs.clamp(1, 30);
    let start = Instant::now();
    let validated = validate_outbound_url_resolved(url, "HTTP monitor").await?;
    let parsed = validated.url.clone();
    let cert = if parsed.scheme() == "https" {
        match probe_tls_certificate(&validated, timeout_secs).await {
            Ok(cert) => Some(cert),
            Err(e) => {
                tracing::warn!("failed to inspect TLS certificate for {}: {}", url, e);
                None
            }
        }
    } else {
        None
    };
    let client = build_http_probe_client(&validated, timeout_secs)?;

    match client.get(parsed.clone()).send().await {
        Ok(response) => {
            let status = response.status();
            let _ =
                read_bounded_probe_response_body(response, SERVICE_HTTP_PROBE_MAX_RESPONSE_BYTES)
                    .await;
            let latency_ms = start.elapsed().as_millis() as i32;
            let (cert_fingerprint, cert_not_after) = cert_fields(cert);

            Ok(ServiceProbe {
                id: uuid::Uuid::now_v7().to_string(),
                service_id: String::new(), // Will be set by caller
                success: status.is_success(),
                latency_ms: Some(latency_ms),
                status_code: Some(status.as_u16() as i32),
                error: if status.is_success() {
                    None
                } else {
                    Some(format!("HTTP {}", status.as_u16()))
                },
                cert_fingerprint,
                cert_not_after,
                checked_at: Utc::now(),
            })
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as i32;
            let (cert_fingerprint, cert_not_after) = cert_fields(cert);
            Ok(ServiceProbe {
                id: uuid::Uuid::now_v7().to_string(),
                service_id: String::new(),
                success: false,
                latency_ms: Some(latency_ms),
                status_code: None,
                error: Some(bounded_ping_output_text(e.to_string().as_bytes())),
                cert_fingerprint,
                cert_not_after,
                checked_at: Utc::now(),
            })
        }
    }
}

fn build_http_probe_client(
    validated: &ValidatedOutboundUrl,
    timeout_secs: u64,
) -> Result<reqwest::Client> {
    let mut builder = secure_reqwest_client_builder(validated)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&validated.host, &validated.addrs)
        .timeout(Duration::from_secs(timeout_secs));
    let normalized_host = validated
        .host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.');
    if normalized_host != validated.host
        && !normalized_host.is_empty()
        && normalized_host.parse::<std::net::IpAddr>().is_err()
    {
        builder = builder.resolve_to_addrs(normalized_host, &validated.addrs);
    }
    builder.build().context("failed to build HTTP probe client")
}

async fn read_bounded_probe_response_body(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Vec<u8> {
    if let Some(content_length) = response.content_length() {
        if content_length > max_bytes as u64 {
            return Vec::new();
        }
    }
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        let remaining = max_bytes.saturating_sub(body.len());
        if remaining == 0 {
            break;
        }
        if chunk.len() >= remaining {
            body.extend_from_slice(&chunk[..remaining]);
            break;
        }
        body.extend_from_slice(&chunk);
    }
    body
}

pub async fn probe_tcp(host: &str, port: u16, timeout_secs: u64) -> Result<ServiceProbe> {
    let timeout_secs = timeout_secs.clamp(1, 30);
    ensure_safe_probe_host(host, "TCP monitor")?;
    let start = Instant::now();
    let deadline = start + Duration::from_secs(timeout_secs);
    let addrs = resolve_outbound_host(host, port, "TCP monitor").await?;

    for addr in addrs {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, tokio::net::TcpStream::connect(addr)).await {
            Ok(Ok(_stream)) => {
                let latency_ms = start.elapsed().as_millis() as i32;
                return Ok(ServiceProbe {
                    id: uuid::Uuid::now_v7().to_string(),
                    service_id: String::new(),
                    success: true,
                    latency_ms: Some(latency_ms),
                    status_code: None,
                    error: None,
                    cert_fingerprint: None,
                    cert_not_after: None,
                    checked_at: Utc::now(),
                });
            }
            Ok(Err(_)) => {}
            Err(_timeout) => {}
        }
    }

    let latency_ms = start.elapsed().as_millis() as i32;
    Ok(ServiceProbe {
        id: uuid::Uuid::now_v7().to_string(),
        service_id: String::new(),
        success: false,
        latency_ms: Some(latency_ms),
        status_code: None,
        error: Some("Connection failed".to_string()),
        cert_fingerprint: None,
        cert_not_after: None,
        checked_at: Utc::now(),
    })
}

/// ICMP ping probe (requires system ping command)
#[allow(dead_code)]
pub async fn probe_icmp(host: &str, timeout_secs: u64) -> Result<ServiceProbe> {
    ensure_safe_probe_host(host, "ICMP monitor")?;
    let start = Instant::now();
    let addrs = resolve_outbound_host(host, 0, "ICMP monitor").await?;
    let target = addrs
        .first()
        .context("ICMP monitor host did not resolve to any address")?
        .ip()
        .to_string();
    ensure_safe_probe_host(&target, "ICMP resolved target")?;

    let ping_timeout = Duration::from_secs(
        timeout_secs.saturating_add(SERVICE_PING_PROCESS_TIMEOUT_GRACE_SECONDS),
    );
    let output = if cfg!(target_os = "windows") {
        let mut command = tokio::process::Command::new("ping");
        command.kill_on_drop(true);
        command.args(["-n", "4", "-w", &(timeout_secs * 1000).to_string(), &target]);
        match tokio::time::timeout(ping_timeout, command.output()).await {
            Ok(output) => output?,
            Err(_) => return Ok(failed_ping_probe(start, "ping timeout")),
        }
    } else {
        let mut command = tokio::process::Command::new("ping");
        command.kill_on_drop(true);
        command.args(["-c", "4", "-W", &timeout_secs.to_string(), &target]);
        match tokio::time::timeout(ping_timeout, command.output()).await {
            Ok(output) => output?,
            Err(_) => return Ok(failed_ping_probe(start, "ping timeout")),
        }
    };

    let latency_ms = start.elapsed().as_millis() as i32;

    if output.status.success() {
        // Parse output for average latency
        let stdout = bounded_ping_output_text(&output.stdout);
        let avg_latency = parse_ping_latency(&stdout).unwrap_or(latency_ms);

        Ok(ServiceProbe {
            id: uuid::Uuid::now_v7().to_string(),
            service_id: String::new(),
            success: true,
            latency_ms: Some(avg_latency),
            status_code: None,
            error: None,
            cert_fingerprint: None,
            cert_not_after: None,
            checked_at: Utc::now(),
        })
    } else {
        let message = ping_failure_message(&output.stderr, &output.stdout);
        Ok(failed_ping_probe_with_latency(latency_ms, message))
    }
}

fn failed_ping_probe(start: Instant, error: impl Into<String>) -> ServiceProbe {
    failed_ping_probe_with_latency(start.elapsed().as_millis() as i32, error)
}

fn failed_ping_probe_with_latency(latency_ms: i32, error: impl Into<String>) -> ServiceProbe {
    ServiceProbe {
        id: uuid::Uuid::now_v7().to_string(),
        service_id: String::new(),
        success: false,
        latency_ms: Some(latency_ms),
        status_code: None,
        error: Some(error.into()),
        cert_fingerprint: None,
        cert_not_after: None,
        checked_at: Utc::now(),
    }
}

fn ping_failure_message(stderr: &[u8], stdout: &[u8]) -> String {
    let stderr = bounded_ping_output_text(stderr);
    let stdout = bounded_ping_output_text(stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    if detail.is_empty() {
        "Ping failed".to_string()
    } else {
        format!("Ping failed: {detail}")
    }
}

fn bounded_ping_output_text(bytes: &[u8]) -> String {
    if bytes.len() <= SERVICE_PING_OUTPUT_MAX_BYTES {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut end = SERVICE_PING_OUTPUT_MAX_BYTES;
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    let mut text = String::from_utf8_lossy(&bytes[..end]).into_owned();
    text.push_str("... [truncated]");
    text
}

#[derive(Debug, Clone)]
pub struct CertificateStatus {
    pub fingerprint: String,
    pub not_after: DateTime<Utc>,
}

fn cert_fields(cert: Option<CertificateStatus>) -> (Option<String>, Option<DateTime<Utc>>) {
    match cert {
        Some(cert) => (Some(cert.fingerprint), Some(cert.not_after)),
        None => (None, None),
    }
}

async fn probe_tls_certificate(
    validated: &ValidatedOutboundUrl,
    timeout_secs: u64,
) -> Result<CertificateStatus> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let host = validated
        .host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let mut last_error = None;

    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));

    for addr in &validated.addrs {
        let stream =
            match tokio::time::timeout(Duration::from_secs(timeout_secs), TcpStream::connect(addr))
                .await
            {
                Ok(Ok(stream)) => stream,
                Ok(Err(err)) => {
                    last_error = Some(err.to_string());
                    continue;
                }
                Err(_) => {
                    last_error = Some("TLS TCP connect timed out".to_string());
                    continue;
                }
            };
        let server_name = ServerName::try_from(host.clone())
            .with_context(|| format!("invalid TLS server name: {host}"))?;
        let tls_stream = match tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            connector.connect(server_name, stream),
        )
        .await
        {
            Ok(Ok(stream)) => stream,
            Ok(Err(err)) => {
                last_error = Some(err.to_string());
                continue;
            }
            Err(_) => {
                last_error = Some("TLS handshake timed out".to_string());
                continue;
            }
        };

        let (_, session) = tls_stream.get_ref();
        let certs = session
            .peer_certificates()
            .context("TLS peer did not present a certificate")?;
        let leaf = certs
            .first()
            .context("TLS peer certificate chain is empty")?;
        return parse_certificate_status(leaf);
    }

    anyhow::bail!(
        "TLS certificate probe failed: {}",
        last_error.unwrap_or_else(|| "no resolved addresses".to_string())
    )
}

fn parse_certificate_status(leaf: &CertificateDer<'_>) -> Result<CertificateStatus> {
    let fingerprint = hex::encode(Sha256::digest(leaf.as_ref()));
    let (_, cert) =
        X509Certificate::from_der(leaf.as_ref()).context("failed to parse TLS certificate")?;
    let not_after = cert.validity().not_after.to_datetime();
    let not_after =
        DateTime::<Utc>::from_timestamp(not_after.unix_timestamp(), not_after.nanosecond())
            .context("certificate not_after is outside supported timestamp range")?;
    Ok(CertificateStatus {
        fingerprint,
        not_after,
    })
}

#[derive(Debug)]
struct NoCertificateVerification;

impl ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

pub(crate) fn ensure_safe_probe_host(host: &str, purpose: &str) -> Result<()> {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{purpose} host is required");
    }
    if trimmed.starts_with('-') {
        anyhow::bail!("{purpose} host must not start with '-'");
    }
    if trimmed.chars().any(|ch| {
        ch.is_control()
            || ch.is_whitespace()
            || matches!(
                ch,
                '/' | '\\' | '?' | '#' | '@' | ';' | '|' | '&' | '$' | '`' | '"' | '\''
            )
    }) {
        anyhow::bail!("{purpose} host contains invalid characters");
    }
    Ok(())
}

#[allow(dead_code)]
fn parse_ping_latency(output: &str) -> Option<i32> {
    // Try to extract average latency from ping output
    for line in output.lines() {
        if line.contains("min/avg/max") || line.contains("rtt min/avg/max") {
            // Linux/macOS format: "rtt min/avg/max/mdev = 10.1/20.2/30.3/5.4 ms"
            if let Some(stats_part) = line.split('=').nth(1) {
                let values: Vec<&str> = stats_part.trim().split('/').collect();
                if values.len() >= 2 {
                    if let Ok(avg) = values[1].trim().parse::<f64>() {
                        return Some(avg as i32);
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_output_text_is_bounded_and_utf8_safe() {
        assert_eq!(SERVICE_PING_OUTPUT_MAX_BYTES, 4096);
        assert_eq!(SERVICE_PING_PROCESS_TIMEOUT_GRACE_SECONDS, 2);

        let mut output = "a".repeat(SERVICE_PING_OUTPUT_MAX_BYTES - 1);
        output.push('é');
        output.push_str(&"b".repeat(32));

        let bounded = bounded_ping_output_text(output.as_bytes());

        assert!(bounded.ends_with("... [truncated]"));
        assert!(bounded.is_char_boundary(bounded.len()));
        assert!(bounded.len() <= SERVICE_PING_OUTPUT_MAX_BYTES + "... [truncated]".len());
        assert!(!bounded.ends_with("�... [truncated]"));
    }

    #[test]
    fn ping_failure_message_uses_bounded_stderr_or_stdout() {
        let stderr = "error".repeat(SERVICE_PING_OUTPUT_MAX_BYTES);
        let message = ping_failure_message(stderr.as_bytes(), b"stdout detail");

        assert!(message.starts_with("Ping failed: error"));
        assert!(message.ends_with("... [truncated]"));
        assert!(
            message.len()
                <= "Ping failed: ".len() + SERVICE_PING_OUTPUT_MAX_BYTES + "... [truncated]".len()
        );

        let fallback = ping_failure_message(b"   ", b"stdout detail");
        assert_eq!(fallback, "Ping failed: stdout detail");

        let empty = ping_failure_message(b"", b"");
        assert_eq!(empty, "Ping failed");
    }

    #[test]
    fn safe_probe_host_rejects_option_injection_and_metacharacters() {
        assert!(ensure_safe_probe_host("example.com", "ICMP").is_ok());
        assert!(ensure_safe_probe_host("203.0.113.10", "ICMP").is_ok());
        assert!(ensure_safe_probe_host("[2606:4700:4700::1111]", "ICMP").is_ok());
        assert!(ensure_safe_probe_host("-f", "ICMP").is_err());
        assert!(ensure_safe_probe_host("--flood", "ICMP").is_err());
        assert!(ensure_safe_probe_host("example.com;id", "ICMP").is_err());
        assert!(ensure_safe_probe_host("example.com/path", "ICMP").is_err());
        assert!(ensure_safe_probe_host("host with space", "ICMP").is_err());
    }

    #[tokio::test]
    async fn http_probe_response_body_is_bounded_and_pins_resolved_addr() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        assert_eq!(SERVICE_HTTP_PROBE_MAX_RESPONSE_BYTES, 64 * 1024);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server_task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut req_buf = [0u8; 1024];
            let _ = socket.read(&mut req_buf).await;
            let header = "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n";
            let _ = socket.write_all(header.as_bytes()).await;
            let chunk = vec![b'x'; 16 * 1024];
            for _ in 0..8 {
                if socket.write_all(&chunk).await.is_err() {
                    break;
                }
            }
        });

        let url = format!("http://probe.invalid:{}/status", addr.port());
        let validated = ValidatedOutboundUrl {
            url: reqwest::Url::parse(&url).unwrap(),
            host: "probe.invalid".into(),
            addrs: vec![addr],
        };
        let client = build_http_probe_client(&validated, 5).unwrap();
        let response = client
            .get(validated.url.clone())
            .send()
            .await
            .expect("pinned HTTP probe client must connect to resolved SocketAddr");
        assert_eq!(response.status(), reqwest::StatusCode::OK);

        let body =
            read_bounded_probe_response_body(response, SERVICE_HTTP_PROBE_MAX_RESPONSE_BYTES).await;
        assert_eq!(body.len(), SERVICE_HTTP_PROBE_MAX_RESPONSE_BYTES);
        let _ = server_task.await;
    }
}
