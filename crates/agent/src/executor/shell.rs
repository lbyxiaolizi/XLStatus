#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::{bail, Context, Result};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

const SHELL_MIN_TIMEOUT_SECONDS: u32 = 1;
const SHELL_DEFAULT_TIMEOUT_SECONDS: u32 = 30;
const SHELL_MAX_TIMEOUT_SECONDS: u32 = 60;
const SHELL_DEFAULT_OUTPUT_MAX_BYTES: u64 = 64 * 1024;
const SHELL_MAX_OUTPUT_BYTES: u64 = 64 * 1024;

/// Shell command execution result
#[derive(Debug)]
#[allow(dead_code)]
pub struct ShellResult {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub output_truncated: bool,
    pub execution_time_ms: u64,
}

/// Execute a shell command with timeout and output limits
pub async fn execute_shell_command(
    command: &str,
    working_dir: Option<&str>,
    env: &[(String, String)],
    timeout_seconds: u32,
    max_output_bytes: u64,
) -> Result<ShellResult> {
    let start = Instant::now();
    let timeout_seconds = bounded_timeout_seconds(timeout_seconds);
    let max_output_bytes = bounded_output_max_bytes(max_output_bytes);

    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };

    if let Some(dir) = working_dir {
        cmd.current_dir(dir);
    }

    for (key, value) in env {
        cmd.env(key, value);
    }

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn().context("Failed to spawn command")?;

    let stdout_handle = child.stdout.take().context("Failed to take stdout")?;
    let stderr_handle = child.stderr.take().context("Failed to take stderr")?;

    let timeout_duration = Duration::from_secs(timeout_seconds as u64);

    // Collect output with size limits
    let stdout_future = collect_output(stdout_handle, max_output_bytes);
    let stderr_future = collect_output(stderr_handle, max_output_bytes);

    let result = timeout(timeout_duration, async {
        let (stdout_result, stderr_result) = tokio::join!(stdout_future, stderr_future);
        let status = child.wait().await?;
        Ok::<_, anyhow::Error>((status, stdout_result?, stderr_result?))
    })
    .await;

    let execution_time_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(Ok((status, (stdout, stdout_truncated), (stderr, stderr_truncated)))) => {
            Ok(ShellResult {
                exit_code: status.code().unwrap_or(-1),
                stdout,
                stderr,
                output_truncated: stdout_truncated || stderr_truncated,
                execution_time_ms,
            })
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            // Timeout - kill the process
            let _ = child.kill().await;
            bail!("Command timed out after {} seconds", timeout_seconds)
        }
    }
}

fn bounded_timeout_seconds(value: u32) -> u32 {
    if value == 0 {
        SHELL_DEFAULT_TIMEOUT_SECONDS
    } else {
        value.clamp(SHELL_MIN_TIMEOUT_SECONDS, SHELL_MAX_TIMEOUT_SECONDS)
    }
}

fn bounded_output_max_bytes(value: u64) -> u64 {
    if value == 0 {
        SHELL_DEFAULT_OUTPUT_MAX_BYTES
    } else {
        value.min(SHELL_MAX_OUTPUT_BYTES)
    }
}

async fn collect_output(
    mut handle: impl tokio::io::AsyncRead + Unpin,
    max_bytes: u64,
) -> Result<(Vec<u8>, bool)> {
    let max_bytes = max_bytes as usize;
    let mut output = Vec::new();
    let mut truncated = false;
    let mut buffer = [0u8; 8192];

    loop {
        let bytes_read = handle.read(&mut buffer).await?;

        if bytes_read == 0 {
            break;
        }

        let remaining = max_bytes.saturating_sub(output.len());
        if remaining == 0 {
            truncated = true;
            break;
        }

        if bytes_read > remaining {
            output.extend_from_slice(&buffer[..remaining]);
            truncated = true;
            break;
        }

        output.extend_from_slice(&buffer[..bytes_read]);
    }

    Ok((output, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn test_simple_command() {
        let result = execute_shell_command("echo hello", None, &[], 5, 1024 * 1024)
            .await
            .unwrap();

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("hello"));
        assert!(!result.output_truncated);
    }

    #[tokio::test]
    async fn test_command_with_env() {
        let env = vec![("TEST_VAR".to_string(), "test_value".to_string())];

        let result = execute_shell_command(
            if cfg!(target_os = "windows") {
                "echo %TEST_VAR%"
            } else {
                "echo $TEST_VAR"
            },
            None,
            &env,
            5,
            1024 * 1024,
        )
        .await
        .unwrap();

        assert_eq!(result.exit_code, 0);
        assert!(String::from_utf8_lossy(&result.stdout).contains("test_value"));
    }

    #[tokio::test]
    async fn test_command_timeout() {
        let result = execute_shell_command(
            if cfg!(target_os = "windows") {
                "timeout /t 10"
            } else {
                "sleep 10"
            },
            None,
            &[],
            1,
            1024 * 1024,
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test]
    async fn test_output_truncation() {
        let result = execute_shell_command(
            if cfg!(target_os = "windows") {
                "for /L %i in (1,1,1000) do @echo This is a long line of text"
            } else {
                "i=0; while [ $i -lt 1000 ]; do echo 'This is a long line of text'; i=$((i + 1)); done"
            },
            None,
            &[],
            5,
            1024, // Only 1 KiB
        )
        .await
        .unwrap();

        assert!(result.output_truncated);
        assert_eq!(result.stdout.len(), 1024);
    }

    #[tokio::test]
    async fn test_collect_output_truncates_long_line_without_buffering_it_all() {
        let (mut writer, reader) = tokio::io::duplex(4096);
        writer.write_all(&vec![b'x'; 2048]).await.unwrap();
        drop(writer);

        let (output, truncated) = collect_output(reader, 1024).await.unwrap();

        assert!(truncated);
        assert_eq!(output.len(), 1024);
    }

    #[test]
    fn test_shell_limits_are_bounded() {
        assert_eq!(bounded_timeout_seconds(0), SHELL_DEFAULT_TIMEOUT_SECONDS);
        assert_eq!(bounded_timeout_seconds(1), SHELL_MIN_TIMEOUT_SECONDS);
        assert_eq!(bounded_timeout_seconds(u32::MAX), SHELL_MAX_TIMEOUT_SECONDS);
        assert_eq!(bounded_output_max_bytes(0), SHELL_DEFAULT_OUTPUT_MAX_BYTES);
        assert_eq!(bounded_output_max_bytes(u64::MAX), SHELL_MAX_OUTPUT_BYTES);
    }
}
