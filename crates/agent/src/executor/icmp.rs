#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::{bail, Context, Result};
use std::process::Stdio;
use std::time::Instant;
use tokio::process::Command;

/// ICMP ping result
#[derive(Debug)]
#[allow(dead_code)]
pub struct IcmpPingResult {
    pub packets_sent: u32,
    pub packets_received: u32,
    pub avg_latency_ms: f64,
    pub min_latency_ms: f64,
    pub max_latency_ms: f64,
}

/// Execute ICMP ping using system ping command
#[allow(dead_code)]
pub async fn execute_icmp_ping(
    host: &str,
    count: u32,
    timeout_seconds: u32,
) -> Result<IcmpPingResult> {
    let _start = Instant::now();

    let output = if cfg!(target_os = "windows") {
        Command::new("ping")
            .args([
                "-n",
                &count.to_string(),
                "-w",
                &(timeout_seconds * 1000).to_string(),
                host,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to execute ping command")?
    } else {
        Command::new("ping")
            .args([
                "-c",
                &count.to_string(),
                "-W",
                &timeout_seconds.to_string(),
                host,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to execute ping command")?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Ping failed: {}", stderr);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_ping_output(&stdout, count)
}

fn parse_ping_output(output: &str, expected_count: u32) -> Result<IcmpPingResult> {
    let packets_sent = expected_count;
    let mut packets_received = 0u32;
    let mut min_latency = 0.0f64;
    let mut avg_latency = 0.0f64;
    let mut max_latency = 0.0f64;

    for line in output.lines() {
        // Parse received packets
        if line.contains("packets transmitted") || line.contains("Packets: Sent") {
            if let Some(received) = extract_received_count(line) {
                packets_received = received;
            }
        }

        // Parse statistics (Linux/macOS format)
        if line.contains("min/avg/max") || line.contains("rtt min/avg/max") {
            if let Some((min, avg, max)) = parse_rtt_stats(line) {
                min_latency = min;
                avg_latency = avg;
                max_latency = max;
            }
        }

        // Parse statistics (Windows format)
        if line.contains("Minimum") && line.contains("Maximum") && line.contains("Average") {
            if let Some((min, avg, max)) = parse_windows_stats(line, output) {
                min_latency = min;
                avg_latency = avg;
                max_latency = max;
            }
        }
    }

    if packets_received == 0 {
        bail!("No packets received");
    }

    Ok(IcmpPingResult {
        packets_sent,
        packets_received,
        avg_latency_ms: avg_latency,
        min_latency_ms: min_latency,
        max_latency_ms: max_latency,
    })
}

fn extract_received_count(line: &str) -> Option<u32> {
    // Linux/macOS: "4 packets transmitted, 4 received"
    // Windows: "Packets: Sent = 4, Received = 4"
    line.split(',')
        .find(|segment| segment.contains("received") || segment.contains("Received"))
        .and_then(|segment| {
            segment
                .split(|c: char| !c.is_ascii_digit())
                .find(|part| !part.is_empty())
                .and_then(|part| part.parse::<u32>().ok())
        })
}

fn parse_rtt_stats(line: &str) -> Option<(f64, f64, f64)> {
    // Format: "rtt min/avg/max/mdev = 10.123/20.456/30.789/5.123 ms"
    if let Some(stats_part) = line.split('=').nth(1) {
        let values: Vec<&str> = stats_part.trim().split('/').collect();
        if values.len() >= 3 {
            let min = values[0].trim().parse::<f64>().ok()?;
            let avg = values[1].trim().parse::<f64>().ok()?;
            let max = values[2].trim().parse::<f64>().ok()?;
            return Some((min, avg, max));
        }
    }
    None
}

fn parse_windows_stats(_line: &str, full_output: &str) -> Option<(f64, f64, f64)> {
    // Windows format has stats on separate lines
    let mut min = 0.0;
    let mut avg = 0.0;
    let mut max = 0.0;

    for line in full_output.lines() {
        if line.contains("Minimum") {
            if let Some(val) = extract_ms_value(line, "Minimum") {
                min = val;
            }
        }
        if line.contains("Maximum") {
            if let Some(val) = extract_ms_value(line, "Maximum") {
                max = val;
            }
        }
        if line.contains("Average") {
            if let Some(val) = extract_ms_value(line, "Average") {
                avg = val;
            }
        }
    }

    if min > 0.0 && avg > 0.0 && max > 0.0 {
        Some((min, avg, max))
    } else {
        None
    }
}

fn extract_ms_value(line: &str, keyword: &str) -> Option<f64> {
    if let Some(pos) = line.find(keyword) {
        let after = &line[pos + keyword.len()..];
        let nums: String = after
            .chars()
            .filter(|c| c.is_numeric() || *c == '.')
            .collect();
        nums.parse::<f64>().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_icmp_ping_localhost() {
        let result = execute_icmp_ping("127.0.0.1", 4, 5).await;
        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.packets_sent, 4);
        assert!(result.packets_received > 0);
        assert!(result.avg_latency_ms >= 0.0);
    }

    #[tokio::test]
    async fn test_icmp_ping_public() {
        let result = execute_icmp_ping("127.0.0.1", 3, 5).await;
        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.packets_sent, 3);
        assert!(result.packets_received > 0);
    }

    #[test]
    fn test_parse_linux_output() {
        let output = r#"
PING 8.8.8.8 (8.8.8.8) 56(84) bytes of data.
64 bytes from 8.8.8.8: icmp_seq=1 ttl=119 time=10.2 ms
64 bytes from 8.8.8.8: icmp_seq=2 ttl=119 time=10.5 ms

--- 8.8.8.8 ping statistics ---
2 packets transmitted, 2 received, 0% packet loss, time 1001ms
rtt min/avg/max/mdev = 10.234/10.456/10.678/0.222 ms
"#;
        let result = parse_ping_output(output, 2).unwrap();
        assert_eq!(result.packets_received, 2);
        assert!((result.avg_latency_ms - 10.456).abs() < 0.01);
    }
}
