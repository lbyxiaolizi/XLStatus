#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

pub const MAX_TERMINAL_SESSIONS: usize = 16;
pub const TERMINAL_MIN_DIMENSION: u16 = 1;
pub const TERMINAL_MAX_DIMENSION: u16 = 500;
pub const TERMINAL_MAX_INPUT_BYTES: usize = 16 * 1024;
pub const TERMINAL_MAX_SESSION_ID_BYTES: usize = 256;

static ACTIVE_TERMINAL_SESSIONS: AtomicUsize = AtomicUsize::new(0);

struct TerminalSlotGuard;

impl TerminalSlotGuard {
    fn acquire(max_sessions: usize) -> Result<Self> {
        loop {
            let current = ACTIVE_TERMINAL_SESSIONS.load(Ordering::Acquire);
            if current >= max_sessions {
                bail!(
                    "Maximum concurrent terminal sessions ({}) reached",
                    max_sessions
                );
            }
            if ACTIVE_TERMINAL_SESSIONS
                .compare_exchange(current, current + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(Self);
            }
        }
    }
}

impl Drop for TerminalSlotGuard {
    fn drop(&mut self) {
        ACTIVE_TERMINAL_SESSIONS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub fn active_terminal_session_count() -> usize {
    ACTIVE_TERMINAL_SESSIONS.load(Ordering::Acquire)
}

pub fn clamp_terminal_dimensions(cols: u16, rows: u16) -> (u16, u16) {
    (
        cols.clamp(TERMINAL_MIN_DIMENSION, TERMINAL_MAX_DIMENSION),
        rows.clamp(TERMINAL_MIN_DIMENSION, TERMINAL_MAX_DIMENSION),
    )
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.trim().is_empty() {
        bail!("Terminal session_id must not be empty");
    }
    if session_id.len() > TERMINAL_MAX_SESSION_ID_BYTES {
        bail!(
            "Terminal session_id exceeds maximum length of {} bytes",
            TERMINAL_MAX_SESSION_ID_BYTES
        );
    }
    if session_id.contains('\0') || session_id.chars().any(char::is_control) {
        bail!("Terminal session_id contains disallowed control characters");
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) fn sanitize_pty_command_env(cmd: &mut portable_pty::CommandBuilder) {
    for &key in super::shell::SENSITIVE_AGENT_ENV_VARS {
        cmd.env_remove(key);
    }
    for (key, _) in std::env::vars_os() {
        let key_lossy = key.to_string_lossy();
        if super::shell::is_sensitive_inherited_env_key(&key_lossy) {
            cmd.env_remove(key);
        }
    }
}

/// Terminal session manager for tracking and bounding concurrent PTY sessions
pub struct TerminalManager {
    max_sessions: usize,
    sessions: Mutex<HashMap<String, Arc<TerminalSession>>>,
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalManager {
    pub fn new() -> Self {
        Self::with_max_sessions(MAX_TERMINAL_SESSIONS)
    }

    pub fn with_max_sessions(max_sessions: usize) -> Self {
        Self {
            max_sessions: max_sessions.clamp(1, MAX_TERMINAL_SESSIONS),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub async fn open_session(
        &self,
        session_id: String,
        cols: u16,
        rows: u16,
    ) -> Result<Arc<TerminalSession>> {
        validate_session_id(&session_id)?;
        let mut sessions = self.sessions.lock().await;
        if sessions.contains_key(&session_id) {
            bail!("Terminal session '{}' already exists", session_id);
        }
        if sessions.len() >= self.max_sessions {
            bail!(
                "Maximum concurrent terminal sessions ({}) reached",
                self.max_sessions
            );
        }
        let session = Arc::new(
            TerminalSession::new_with_limit(session_id.clone(), cols, rows, self.max_sessions)
                .await?,
        );
        sessions.insert(session_id, session.clone());
        Ok(session)
    }

    pub async fn get_session(&self, session_id: &str) -> Option<Arc<TerminalSession>> {
        self.sessions.lock().await.get(session_id).cloned()
    }

    pub async fn resize_session(&self, session_id: &str, cols: u16, rows: u16) -> Result<()> {
        let session = self
            .get_session(session_id)
            .await
            .context("Terminal session not found")?;
        session.resize(cols, rows)
    }

    pub async fn send_input(&self, session_id: &str, data: &[u8]) -> Result<()> {
        let session = self
            .get_session(session_id)
            .await
            .context("Terminal session not found")?;
        session.send_input(data).await
    }

    pub async fn close_session(&self, session_id: &str) -> bool {
        if let Some(session) = self.sessions.lock().await.remove(session_id) {
            session.close();
            true
        } else {
            false
        }
    }

    pub async fn close_all(&self) {
        let removed = {
            let mut sessions = self.sessions.lock().await;
            std::mem::take(&mut *sessions)
        };
        for session in removed.into_values() {
            session.close();
        }
    }

    pub async fn session_count(&self) -> usize {
        self.sessions.lock().await.len()
    }
}

/// Terminal session
pub struct TerminalSession {
    session_id: String,
    #[cfg(unix)]
    master: std::sync::Mutex<Box<dyn portable_pty::MasterPty + Send>>,
    #[cfg(unix)]
    child_killer: std::sync::Mutex<Option<Box<dyn portable_pty::ChildKiller + Send + Sync>>>,
    #[cfg(unix)]
    child_pid: Option<libc::pid_t>,
    #[cfg(unix)]
    reap_state: Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>,
    slot_guard: std::sync::Mutex<Option<TerminalSlotGuard>>,
    closed: AtomicBool,
    dimensions: std::sync::Mutex<(u16, u16)>,
    input_tx: std::sync::Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    output_rx: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
}

impl TerminalSession {
    /// Create a new terminal session
    pub async fn new(session_id: String, cols: u16, rows: u16) -> Result<Self> {
        Self::new_with_limit(session_id, cols, rows, MAX_TERMINAL_SESSIONS).await
    }

    #[cfg(unix)]
    async fn new_with_limit(
        session_id: String,
        cols: u16,
        rows: u16,
        max_sessions: usize,
    ) -> Result<Self> {
        use portable_pty::{native_pty_system, CommandBuilder, PtySize};

        validate_session_id(&session_id)?;
        let (cols, rows) = clamp_terminal_dimensions(cols, rows);
        let slot_guard = TerminalSlotGuard::acquire(max_sessions)?;

        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        // Find available shell from fixed trusted system paths
        let shell = find_shell();

        let mut cmd = CommandBuilder::new(&shell);
        sanitize_pty_command_env(&mut cmd);
        cmd.env("TERM", "xterm-256color");

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn shell")?;
        drop(pair.slave);

        let child_pid = child
            .process_id()
            .and_then(|id| libc::pid_t::try_from(id).ok())
            .filter(|&pid| pid > 1);
        let child_killer = Some(child.clone_killer());

        let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(32);
        let (output_tx, output_rx) = mpsc::channel::<Vec<u8>>(32);

        let reader = pair
            .master
            .try_clone_reader()
            .context("Failed to clone reader")?;
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

        // Spawn blocking process reaper so child.wait() does not block a Tokio worker thread
        let reap_state = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let reap_state_worker = reap_state.clone();
        tokio::task::spawn_blocking(move || {
            let _ = child.wait();
            let (lock, cvar) = &*reap_state_worker;
            if let Ok(mut reaped) = lock.lock() {
                *reaped = true;
                cvar.notify_all();
            }
        });

        Ok(Self {
            session_id,
            master: std::sync::Mutex::new(pair.master),
            child_killer: std::sync::Mutex::new(child_killer),
            child_pid,
            reap_state,
            slot_guard: std::sync::Mutex::new(Some(slot_guard)),
            closed: AtomicBool::new(false),
            dimensions: std::sync::Mutex::new((cols, rows)),
            input_tx: std::sync::Mutex::new(Some(input_tx)),
            output_rx: Arc::new(Mutex::new(output_rx)),
        })
    }

    #[cfg(not(unix))]
    async fn new_with_limit(
        _session_id: String,
        _cols: u16,
        _rows: u16,
        _max_sessions: usize,
    ) -> Result<Self> {
        bail!("Terminal not supported on this platform");
    }

    /// Send input to terminal
    pub async fn send_input(&self, data: &[u8]) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            bail!("Terminal session is closed");
        }
        if data.len() > TERMINAL_MAX_INPUT_BYTES {
            bail!(
                "Terminal input chunk length {} exceeds maximum of {} bytes",
                data.len(),
                TERMINAL_MAX_INPUT_BYTES
            );
        }
        if data.is_empty() {
            return Ok(());
        }
        let sender = self
            .input_tx
            .lock()
            .expect("terminal input lock poisoned")
            .clone()
            .context("Terminal session input channel is closed")?;
        sender
            .send(data.to_vec())
            .await
            .context("Failed to send input")
    }

    /// Receive output from terminal
    pub async fn recv_output(&self) -> Option<Vec<u8>> {
        let mut rx = self.output_rx.lock().await;
        rx.recv().await
    }

    /// Resize terminal
    #[cfg(unix)]
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        use portable_pty::PtySize;

        if self.closed.load(Ordering::Acquire) {
            bail!("Terminal session is closed");
        }
        let (cols, rows) = clamp_terminal_dimensions(cols, rows);

        self.master
            .lock()
            .expect("terminal master lock poisoned")
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to resize PTY")?;

        *self
            .dimensions
            .lock()
            .expect("terminal dimensions lock poisoned") = (cols, rows);
        Ok(())
    }

    #[cfg(not(unix))]
    pub fn resize(&self, _cols: u16, _rows: u16) -> Result<()> {
        bail!("Terminal not supported on this platform");
    }

    pub fn dimensions(&self) -> (u16, u16) {
        *self
            .dimensions
            .lock()
            .expect("terminal dimensions lock poisoned")
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    #[cfg(unix)]
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let _ = self
            .input_tx
            .lock()
            .expect("terminal input lock poisoned")
            .take();

        if let Some(pid) = self.child_pid {
            // The PTY child is the session and process-group leader (setsid).
            // Signal the entire process group first so background descendants are terminated.
            unsafe {
                let _ = libc::kill(-pid, libc::SIGHUP);
                let _ = libc::kill(-pid, libc::SIGKILL);
                let _ = libc::kill(pid, libc::SIGKILL);
            }
        }

        if let Some(mut child_killer) = self
            .child_killer
            .lock()
            .expect("terminal child lock poisoned")
            .take()
        {
            let _ = child_killer.kill();
        }

        // Wait briefly for the blocking reaper task to finish reaping the child
        let (lock, cvar) = &*self.reap_state;
        if let Ok(reaped) = lock.lock() {
            if !*reaped {
                let _ = cvar.wait_timeout_while(reaped, Duration::from_millis(250), |done| !*done);
            }
        }

        let _ = self
            .slot_guard
            .lock()
            .expect("terminal slot lock poisoned")
            .take();
    }

    #[cfg(not(unix))]
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let _ = self
            .slot_guard
            .lock()
            .expect("terminal slot lock poisoned")
            .take();
    }
}

#[cfg(unix)]
impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(unix)]
fn find_shell() -> String {
    const TRUSTED_SHELLS: &[&str] = &[
        "/bin/zsh",
        "/usr/bin/zsh",
        "/bin/bash",
        "/usr/bin/bash",
        "/bin/sh",
        "/usr/bin/sh",
    ];
    for &candidate in TRUSTED_SHELLS {
        if std::path::Path::new(candidate).is_file() {
            return candidate.to_string();
        }
    }

    "/bin/sh".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_PTY_LOCK: Mutex<()> = Mutex::const_new(());

    #[cfg(unix)]
    #[tokio::test]
    async fn test_terminal_creation() {
        let _guard = TEST_PTY_LOCK.lock().await;
        let session = TerminalSession::new("test".to_string(), 80, 24).await;
        assert!(session.is_ok());
        let session = session.unwrap();
        session.close();
        assert!(session.is_closed());
    }

    #[test]
    fn test_clamp_terminal_dimensions() {
        assert_eq!(
            clamp_terminal_dimensions(0, 0),
            (TERMINAL_MIN_DIMENSION, TERMINAL_MIN_DIMENSION)
        );
        assert_eq!(clamp_terminal_dimensions(80, 24), (80, 24));
        assert_eq!(
            clamp_terminal_dimensions(10_000, 999),
            (TERMINAL_MAX_DIMENSION, TERMINAL_MAX_DIMENSION)
        );
    }

    #[test]
    fn test_validate_session_id_rejects_malformed_ids() {
        assert!(validate_session_id("").is_err());
        assert!(validate_session_id("   ").is_err());
        assert!(validate_session_id("bad\0id").is_err());
        assert!(validate_session_id("bad\nid").is_err());
        assert!(validate_session_id(&"x".repeat(TERMINAL_MAX_SESSION_ID_BYTES + 1)).is_err());
        assert!(validate_session_id("session-123").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_sanitize_pty_command_env_strips_secrets() {
        std::env::set_var("XLSTATUS_ENROLL_TOKEN", "xle_secret_enroll");
        std::env::set_var("XLSTATUS_AGENT_JWT", "jwt_secret_value");
        std::env::set_var("XLSTATUS_PRIVATE_KEY", "priv_key_value");
        std::env::set_var("XLSTATUS_TOKEN", "bootstrap_token_value");

        let mut cmd = portable_pty::CommandBuilder::new("/bin/sh");
        sanitize_pty_command_env(&mut cmd);

        assert!(cmd.get_env("XLSTATUS_ENROLL_TOKEN").is_none());
        assert!(cmd.get_env("XLSTATUS_AGENT_JWT").is_none());
        assert!(cmd.get_env("XLSTATUS_PRIVATE_KEY").is_none());
        assert!(cmd.get_env("XLSTATUS_TOKEN").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_terminal_bounds_input_resize_and_session_cap() {
        let _guard = TEST_PTY_LOCK.lock().await;
        let manager = TerminalManager::with_max_sessions(2);

        let s1 = manager
            .open_session("sess-1".to_string(), 0, 9999)
            .await
            .expect("first session should open");
        assert_eq!(s1.dimensions(), (1, 500));

        let s2 = manager
            .open_session("sess-2".to_string(), 80, 24)
            .await
            .expect("second session should open");
        assert_eq!(manager.session_count().await, 2);

        // Exceeding max_sessions must fail
        let s3 = manager.open_session("sess-3".to_string(), 80, 24).await;
        assert!(s3.is_err());

        // Resize clamps to 1..=500
        s1.resize(2000, 0).unwrap();
        assert_eq!(s1.dimensions(), (500, 1));

        // Oversized input chunk must be rejected
        let oversized = vec![b'a'; TERMINAL_MAX_INPUT_BYTES + 1];
        assert!(s1.send_input(&oversized).await.is_err());

        // Closing a session reaps the child and frees the slot
        assert!(manager.close_session("sess-1").await);
        assert!(s1.is_closed());
        assert!(s1.send_input(b"echo test\n").await.is_err());

        let s3_retry = manager
            .open_session("sess-3".to_string(), 80, 24)
            .await
            .expect("slot should be freed after close_session");
        s2.close();
        s3_retry.close();
        manager.close_all().await;
    }

    #[cfg(unix)]
    #[tokio::test]
    #[ignore = "requires an interactive PTY environment and can hang in automated test harnesses"]
    async fn test_terminal_echo() {
        let session = TerminalSession::new("test".to_string(), 80, 24)
            .await
            .unwrap();

        session
            .send_input(b"printf '__xlstatus_hello__\\n'\n")
            .await
            .unwrap();

        let read = async {
            let mut combined = Vec::new();
            while let Some(chunk) = session.recv_output().await {
                combined.extend_from_slice(&chunk);
                if String::from_utf8_lossy(&combined).contains("__xlstatus_hello__") {
                    return true;
                }
            }
            false
        };

        let found = tokio::time::timeout(tokio::time::Duration::from_secs(3), read)
            .await
            .unwrap_or(false);
        assert!(found);
    }

    #[cfg(unix)]
    #[test]
    fn test_find_shell() {
        let shell = find_shell();
        assert!(!shell.is_empty());
        assert!(shell.starts_with('/'));
    }
}
