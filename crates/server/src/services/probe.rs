use anyhow::Result;
use chrono::{DateTime, Utc};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ServiceProbe {
    pub id: String,
    pub service_id: String,
    pub success: bool,
    pub latency_ms: Option<i32>,
    pub status_code: Option<i32>,
    pub error: Option<String>,
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
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()?;

    match client.get(url).send().await {
        Ok(response) => {
            let status = response.status();
            let latency_ms = start.elapsed().as_millis() as i32;

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
                checked_at: Utc::now(),
            })
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as i32;
            Ok(ServiceProbe {
                id: uuid::Uuid::now_v7().to_string(),
                service_id: String::new(),
                success: false,
                latency_ms: Some(latency_ms),
                status_code: None,
                error: Some(e.to_string()),
                checked_at: Utc::now(),
            })
        }
    }
}

pub async fn probe_tcp(host: &str, port: u16, timeout_secs: u64) -> Result<ServiceProbe> {
    let start = Instant::now();
    let addr = format!("{}:{}", host, port);

    match tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    {
        Ok(Ok(_stream)) => {
            let latency_ms = start.elapsed().as_millis() as i32;
            Ok(ServiceProbe {
                id: uuid::Uuid::now_v7().to_string(),
                service_id: String::new(),
                success: true,
                latency_ms: Some(latency_ms),
                status_code: None,
                error: None,
                checked_at: Utc::now(),
            })
        }
        Ok(Err(e)) => {
            let latency_ms = start.elapsed().as_millis() as i32;
            Ok(ServiceProbe {
                id: uuid::Uuid::now_v7().to_string(),
                service_id: String::new(),
                success: false,
                latency_ms: Some(latency_ms),
                status_code: None,
                error: Some(e.to_string()),
                checked_at: Utc::now(),
            })
        }
        Err(_timeout) => {
            let latency_ms = start.elapsed().as_millis() as i32;
            Ok(ServiceProbe {
                id: uuid::Uuid::now_v7().to_string(),
                service_id: String::new(),
                success: false,
                latency_ms: Some(latency_ms),
                status_code: None,
                error: Some("Connection timeout".to_string()),
                checked_at: Utc::now(),
            })
        }
    }
}

/// ICMP ping probe (requires system ping command)
pub async fn probe_icmp(host: &str, timeout_secs: u64) -> Result<ServiceProbe> {
    let start = Instant::now();

    let output = if cfg!(target_os = "windows") {
        tokio::process::Command::new("ping")
            .args(["-n", "4", "-w", &(timeout_secs * 1000).to_string(), host])
            .output()
            .await?
    } else {
        tokio::process::Command::new("ping")
            .args(["-c", "4", "-W", &timeout_secs.to_string(), host])
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
            checked_at: Utc::now(),
        })
    }
}

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
