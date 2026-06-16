use anyhow::{bail, Context, Result};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

/// Shell command execution result
#[derive(Debug)]
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

async fn collect_output(
    handle: impl tokio::io::AsyncRead + Unpin,
    max_bytes: u64,
) -> Result<(Vec<u8>, bool)> {
    let mut reader = BufReader::new(handle);
    let mut output = Vec::new();
    let mut truncated = false;
    let mut line = Vec::new();

    loop {
        line.clear();
        let bytes_read = reader.read_until(b'\n', &mut line).await?;

        if bytes_read == 0 {
            break;
        }

        if output.len() + line.len() > max_bytes as usize {
            // Truncate
            let remaining = max_bytes as usize - output.len();
            output.extend_from_slice(&line[..remaining]);
            truncated = true;
            break;
        }

        output.extend_from_slice(&line);
    }

    Ok((output, truncated))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_simple_command() {
        let result = execute_shell_command(
            "echo hello",
            None,
            &[],
            5,
            1024 * 1024,
        )
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
                "for i in {1..1000}; do echo 'This is a long line of text'; done"
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
}
