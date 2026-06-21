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
    let client = secure_reqwest_client_builder(&validated)
        .timeout(Duration::from_secs(timeout_secs))
        .build()?;

    match client.get(parsed.clone()).send().await {
        Ok(response) => {
            let status = response.status();
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
                error: Some(e.to_string()),
                cert_fingerprint,
                cert_not_after,
                checked_at: Utc::now(),
            })
        }
    }
}

pub async fn probe_tcp(host: &str, port: u16, timeout_secs: u64) -> Result<ServiceProbe> {
    let start = Instant::now();
    let addrs = resolve_outbound_host(host, port, "TCP monitor").await?;

    for addr in addrs {
        match tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            tokio::net::TcpStream::connect(addr),
        )
        .await
        {
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
    let start = Instant::now();
    let addrs = resolve_outbound_host(host, 0, "ICMP monitor").await?;
    let target = addrs
        .first()
        .context("ICMP monitor host did not resolve to any address")?
        .ip()
        .to_string();

    let output = if cfg!(target_os = "windows") {
        tokio::process::Command::new("ping")
            .args(["-n", "4", "-w", &(timeout_secs * 1000).to_string(), &target])
            .output()
            .await?
    } else {
        tokio::process::Command::new("ping")
            .args(["-c", "4", "-W", &timeout_secs.to_string(), &target])
            .output()
            .await?
    };

    let latency_ms = start.elapsed().as_millis() as i32;

    if output.status.success() {
        // Parse output for average latency
        let stdout = String::from_utf8_lossy(&output.stdout);
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(ServiceProbe {
            id: uuid::Uuid::now_v7().to_string(),
            service_id: String::new(),
            success: false,
            latency_ms: Some(latency_ms),
            status_code: None,
            error: Some(format!("Ping failed: {}", stderr)),
            cert_fingerprint: None,
            cert_not_after: None,
            checked_at: Utc::now(),
        })
    }
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
    let host = validated.host.clone();
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
