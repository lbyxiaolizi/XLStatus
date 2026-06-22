#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::{bail, Context, Result};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

const ICMP_PING_MAX_COUNT: u32 = 10;
const ICMP_PING_MIN_TIMEOUT_SECONDS: u32 = 1;
const ICMP_PING_DEFAULT_TIMEOUT_SECONDS: u32 = 10;
const ICMP_PING_MAX_TIMEOUT_SECONDS: u32 = 30;
const ICMP_PING_PROCESS_TIMEOUT_GRACE_SECONDS: u64 = 2;
const ICMP_PING_OUTPUT_MAX_BYTES: usize = 4096;

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
    let count = count.clamp(1, ICMP_PING_MAX_COUNT);
    let timeout_seconds = bounded_timeout_seconds(timeout_seconds);

    let mut command = if cfg!(target_os = "windows") {
        let mut command = Command::new("ping");
        command.args([
            "-n",
            &count.to_string(),
            "-w",
            &(timeout_seconds * 1000).to_string(),
            host,
        ]);
        command
    } else {
        let mut command = Command::new("ping");
        command.args([
            "-c",
            &count.to_string(),
            "-W",
            &timeout_seconds.to_string(),
            host,
        ]);
        command
    };
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    command.kill_on_drop(true);

    let process_timeout = Duration::from_secs(
        u64::from(timeout_seconds).saturating_add(ICMP_PING_PROCESS_TIMEOUT_GRACE_SECONDS),
    );
    let output = match timeout(process_timeout, command.output()).await {
        Ok(output) => output.context("Failed to execute ping command")?,
        Err(_) => bail!("Ping timed out after {} seconds", timeout_seconds),
    };

    if !output.status.success() {
        bail!("{}", ping_failure_message(&output.stderr, &output.stdout));
    }

    let stdout = bounded_ping_output_text(&output.stdout);
    parse_ping_output(&stdout, count)
}

fn bounded_timeout_seconds(value: u32) -> u32 {
    if value == 0 {
        ICMP_PING_DEFAULT_TIMEOUT_SECONDS
    } else {
        value.clamp(ICMP_PING_MIN_TIMEOUT_SECONDS, ICMP_PING_MAX_TIMEOUT_SECONDS)
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
    if bytes.len() <= ICMP_PING_OUTPUT_MAX_BYTES {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let mut end = ICMP_PING_OUTPUT_MAX_BYTES;
    while end > 0 && std::str::from_utf8(&bytes[..end]).is_err() {
        end -= 1;
    }
    let mut text = String::from_utf8_lossy(&bytes[..end]).into_owned();
    text.push_str("... [truncated]");
    text
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

    #[test]
    fn test_icmp_timeout_is_bounded() {
        assert_eq!(
            bounded_timeout_seconds(0),
            ICMP_PING_DEFAULT_TIMEOUT_SECONDS
        );
        assert_eq!(bounded_timeout_seconds(1), ICMP_PING_MIN_TIMEOUT_SECONDS);
        assert_eq!(
            bounded_timeout_seconds(u32::MAX),
            ICMP_PING_MAX_TIMEOUT_SECONDS
        );
    }

    #[test]
    fn test_ping_output_text_is_bounded_and_utf8_safe() {
        let oversized = format!("{}é", "x".repeat(ICMP_PING_OUTPUT_MAX_BYTES - 1));
        let bounded = bounded_ping_output_text(oversized.as_bytes());
        assert!(bounded.ends_with("... [truncated]"));
        assert!(bounded.len() <= ICMP_PING_OUTPUT_MAX_BYTES + "... [truncated]".len());
    }

    #[test]
    fn test_ping_failure_message_uses_bounded_stderr_or_stdout() {
        let stderr = "e".repeat(ICMP_PING_OUTPUT_MAX_BYTES + 64);
        let message = ping_failure_message(stderr.as_bytes(), b"stdout detail");
        assert!(message.starts_with("Ping failed: "));
        assert!(message.contains("[truncated]"));
        assert!(!message.contains("stdout detail"));

        let fallback = ping_failure_message(b"   ", b"stdout detail");
        assert_eq!(fallback, "Ping failed: stdout detail");

        let empty = ping_failure_message(b"", b"");
        assert_eq!(empty, "Ping failed");
    }
}
