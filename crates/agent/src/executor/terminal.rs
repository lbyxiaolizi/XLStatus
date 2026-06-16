use anyhow::{bail, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// Terminal session manager
pub struct TerminalSession {
    session_id: String,
    #[cfg(unix)]
    pty: portable_pty::PtyPair,
    input_tx: mpsc::Sender<Vec<u8>>,
    output_rx: mpsc::Receiver<Vec<u8>>,
}

impl TerminalSession {
    /// Create a new terminal session
    #[cfg(unix)]
    pub async fn new(session_id: String, cols: u16, rows: u16) -> Result<Self> {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};

        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        // Find available shell
        let shell = find_shell();

        let mut cmd = CommandBuilder::new(&shell);
        cmd.env("TERM", "xterm-256color");

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn shell")?;

        let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(32);
        let (output_tx, output_rx) = mpsc::channel::<Vec<u8>>(32);

        let reader = pair.master.try_clone_reader().context("Failed to clone reader")?;
        let writer = pair.master.take_writer().context("Failed to take writer")?;

        // Spawn input handler (uses blocking I/O in a blocking task)
        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            let mut writer = writer;
            while let Some(data) = input_rx.blocking_recv() {
                if writer.write_all(&data).is_err() {
                    break;
                }
                let _ = writer.flush();
            }
        });

        // Spawn output handler (uses blocking I/O in a blocking task)
        tokio::task::spawn_blocking(move || {
            use std::io::Read;
            let mut reader = reader;
            let mut buffer = vec![0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if output_tx.blocking_send(buffer[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Spawn process monitor
        tokio::spawn(async move {
            let _ = child.wait();
        });

        Ok(Self {
            session_id,
            pty: pair,
            input_tx,
            output_rx,
        })
    }

    #[cfg(not(unix))]
    pub async fn new(_session_id: String, _cols: u16, _rows: u16) -> Result<Self> {
        bail!("Terminal not supported on this platform");
    }

    /// Send input to terminal
    pub async fn send_input(&self, data: &[u8]) -> Result<()> {
        self.input_tx
            .send(data.to_vec())
            .await
            .context("Failed to send input")
    }

    /// Receive output from terminal
    pub async fn recv_output(&mut self) -> Option<Vec<u8>> {
        self.output_rx.recv().await
    }

    /// Resize terminal
    #[cfg(unix)]
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        use portable_pty::PtySize;

        self.pty
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to resize PTY")
    }

    #[cfg(not(unix))]
    pub fn resize(&mut self, _cols: u16, _rows: u16) -> Result<()> {
        bail!("Terminal not supported on this platform");
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

#[cfg(unix)]
fn find_shell() -> String {
    // Try to find a suitable shell
    for shell in &["zsh", "fish", "bash", "sh"] {
        if let Ok(output) = std::process::Command::new("which").arg(shell).output() {
            if output.status.success() {
                if let Ok(path) = String::from_utf8(output.stdout) {
                    return path.trim().to_string();
                }
            }
        }
    }

    // Fallback to /bin/sh
    "/bin/sh".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn test_terminal_creation() {
        let session = TerminalSession::new("test".to_string(), 80, 24).await;
        assert!(session.is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_terminal_echo() {
        let mut session = TerminalSession::new("test".to_string(), 80, 24)
            .await
            .unwrap();

        // Send a command
        session.send_input(b"echo hello\n").await.unwrap();

        // Wait a bit for output
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Should receive output
        let output = session.recv_output().await;
        assert!(output.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn test_find_shell() {
        let shell = find_shell();
        assert!(!shell.is_empty());
        assert!(shell.starts_with('/'));
    }
}
