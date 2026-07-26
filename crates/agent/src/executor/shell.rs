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

    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };

    #[cfg(not(windows))]
    let mut cmd = {
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

    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().context("Failed to spawn command")?;

    #[cfg(unix)]
    let mut process_group = ProcessGroupGuard::new(
        child
            .id()
            .and_then(|id| libc::pid_t::try_from(id).ok())
            .context("Failed to get spawned command process group")?,
    );

    let stdout_handle = child.stdout.take().context("Failed to take stdout")?;
    let stderr_handle = child.stderr.take().context("Failed to take stderr")?;

    let timeout_duration = Duration::from_secs(timeout_seconds as u64);

    // Collect output with size limits
    let stdout_future = collect_output(stdout_handle, max_output_bytes);
    let stderr_future = collect_output(stderr_handle, max_output_bytes);

    let result = timeout(timeout_duration, async {
        let (stdout_result, stderr_result) = tokio::join!(stdout_future, stderr_future);
        let status = child.wait().await?;
        #[cfg(unix)]
        process_group.disarm();
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
            #[cfg(unix)]
            terminate_timed_out_process_group(&mut child, &mut process_group)
                .await
                .context("Command timed out and its process group could not be terminated")?;

            #[cfg(windows)]
            child
                .kill()
                .await
                .context("Command timed out and could not be terminated")?;

            #[cfg(not(any(unix, windows)))]
            child
                .kill()
                .await
                .context("Command timed out and could not be terminated")?;

            bail!("Command timed out after {} seconds", timeout_seconds)
        }
    }
}

#[cfg(unix)]
struct ProcessGroupGuard {
    process_group_id: libc::pid_t,
    armed: bool,
}

#[cfg(unix)]
impl ProcessGroupGuard {
    fn new(process_group_id: libc::pid_t) -> Self {
        Self {
            process_group_id,
            armed: true,
        }
    }

    fn signal_kill(&self) -> std::io::Result<()> {
        // A negative PID targets the dedicated process group created before spawning.
        let signal_result = unsafe { libc::kill(-self.process_group_id, libc::SIGKILL) };
        if signal_result == 0 {
            return Ok(());
        }

        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(unix)]
impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.signal_kill();
        }
    }
}

#[cfg(unix)]
async fn terminate_timed_out_process_group(
    child: &mut tokio::process::Child,
    process_group: &mut ProcessGroupGuard,
) -> Result<()> {
    let signal_error = process_group.signal_kill().err();

    if signal_error.is_some() {
        // Ensure the direct child cannot remain alive if group signalling failed.
        let _ = child.start_kill();
    } else {
        // SIGKILL has already been delivered to every member of the group, so
        // cancellation while awaiting the direct child cannot leave survivors.
        process_group.disarm();
    }

    child
        .wait()
        .await
        .context("Failed to reap timed out shell process")?;

    if let Some(error) = signal_error {
        return Err(error).context("Failed to kill timed out shell process group");
    }

    Ok(())
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

        let retained_bytes = max_bytes.saturating_sub(output.len()).min(bytes_read);
        output.extend_from_slice(&buffer[..retained_bytes]);

        if retained_bytes < bytes_read {
            truncated = true;
        }
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
    async fn test_collect_output_truncates_and_drains_to_eof() {
        let (mut writer, reader) = tokio::io::duplex(64);
        let writer_task = tokio::spawn(async move {
            writer.write_all(&vec![b'x'; 4096]).await?;
            writer.shutdown().await
        });

        let (output, truncated) = timeout(Duration::from_secs(1), collect_output(reader, 1024))
            .await
            .expect("output collector did not drain to EOF")
            .unwrap();
        writer_task.await.unwrap().unwrap();

        assert!(truncated);
        assert_eq!(output.len(), 1024);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_timeout_kills_descendant_processes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let ready_path = temp_dir.path().join("ready");
        let marker_path = temp_dir.path().join("marker");
        let env = vec![
            (
                "READY_PATH".to_string(),
                ready_path.to_string_lossy().into_owned(),
            ),
            (
                "MARKER_PATH".to_string(),
                marker_path.to_string_lossy().into_owned(),
            ),
        ];

        let result = execute_shell_command(
            r#"(printf ready > "$READY_PATH"; sleep 2; printf marker > "$MARKER_PATH") & wait"#,
            None,
            &env,
            1,
            1024,
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("timed out"));
        assert!(ready_path.exists(), "background child did not start");

        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(
            !marker_path.exists(),
            "background child survived the command timeout"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_abort_kills_descendant_processes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let ready_path = temp_dir.path().join("abort-ready");
        let marker_path = temp_dir.path().join("abort-marker");
        let task_ready_path = ready_path.clone();
        let task_marker_path = marker_path.clone();

        let execution = tokio::spawn(async move {
            let env = vec![
                (
                    "READY_PATH".to_string(),
                    task_ready_path.to_string_lossy().into_owned(),
                ),
                (
                    "MARKER_PATH".to_string(),
                    task_marker_path.to_string_lossy().into_owned(),
                ),
            ];
            execute_shell_command(
                r#"(printf ready > "$READY_PATH"; sleep 2; printf marker > "$MARKER_PATH") & wait"#,
                None,
                &env,
                30,
                1024,
            )
            .await
        });

        timeout(Duration::from_secs(1), async {
            while !ready_path.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("background child did not start");

        execution.abort();
        assert!(execution.await.unwrap_err().is_cancelled());

        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(
            !marker_path.exists(),
            "background child survived cancellation of the command future"
        );
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
