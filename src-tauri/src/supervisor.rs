use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

// TODO: dead — remove or wire (see trust+quality pass spec)
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum SupervisorState {
    Stopped,
    Running,
    Failed(String),
}

/// Shared in-memory ring buffer of worker stderr lines for the in-app Logs
/// drawer. A background task per worker spawn drains `child.stderr` into it;
/// the `get_logs` command tails it from the frontend.
pub type LogBuffer = Arc<Mutex<VecDeque<String>>>;

/// Cap on lines retained in [`LogBuffer`] — the oldest line is dropped beyond
/// this so a chatty worker cannot grow memory without bound.
pub const LOG_BUFFER_CAP: usize = 1000;

/// Creates an empty shared log buffer.
pub fn new_log_buffer() -> LogBuffer {
    Arc::new(Mutex::new(VecDeque::new()))
}

/// Appends one line to the buffer, dropping the oldest line at the cap.
pub async fn push_log_line(logs: &LogBuffer, line: String) {
    let mut buf = logs.lock().await;
    if buf.len() >= LOG_BUFFER_CAP {
        buf.pop_front();
    }
    buf.push_back(line);
}

/// Spawns a background task draining `child`'s stderr one line per entry into
/// the shared log buffer. The task exits when the pipe closes (worker exit),
/// so a respawned worker gets its own fresh drain task on the same buffer.
fn spawn_stderr_drain(child: &mut Child, logs: LogBuffer) {
    if let Some(stderr) = child.stderr.take() {
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                log::warn!("worker stderr: {line}");
                push_log_line(&logs, line).await;
            }
        });
    }
}

pub struct Supervisor {
    child: Option<Child>,
    socket_path: PathBuf,
    handshake_token: String,
    _temp_dir: Option<TempDir>,
    last_error: Option<String>,
    logs: LogBuffer,
}

impl Default for Supervisor {
    fn default() -> Self {
        Self::new()
    }
}

impl Supervisor {
    pub fn new() -> Self {
        Self::with_logs(new_log_buffer())
    }

    /// Constructs a supervisor that drains worker stderr into the given
    /// shared buffer (owned by `AppState` so the Logs drawer sees it).
    pub fn with_logs(logs: LogBuffer) -> Self {
        let temp_dir = TempDir::new().expect("failed to create temp dir for worker socket");
        let socket_path = temp_dir.path().join("worker.sock");
        let handshake_token = uuid::Uuid::new_v4().to_string();

        Self {
            child: None,
            socket_path,
            handshake_token,
            _temp_dir: Some(temp_dir),
            last_error: None,
            logs,
        }
    }

    fn worker_binary_path(&self) -> PathBuf {
        if let Ok(path) = std::env::var("LUCENT_WORKER_BINARY") {
            return PathBuf::from(path);
        }

        // Search relative to the current executable's directory.
        // The running binary is either in target/debug/ (tauri dev) or
        // target/debug/deps/ (test). The worker sits in target/debug/.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                for rel in &["", "../", "../../"] {
                    let candidate = parent.join(rel).join("lucent-driver-postgres");
                    if let Ok(canonical) = candidate.canonicalize() {
                        log::info!("Found worker binary at: {}", canonical.display());
                        return canonical;
                    }
                }
            }
        }

        // Also check from the current working directory (common during dev).
        if let Ok(cwd) = std::env::current_dir() {
            for rel in &["target/debug/", "../target/debug/"] {
                let candidate = cwd.join(rel).join("lucent-driver-postgres");
                if let Ok(canonical) = candidate.canonicalize() {
                    log::info!("Found worker binary at: {}", canonical.display());
                    return canonical;
                }
            }
        }

        log::warn!("Worker binary not found; falling back to PATH lookup");
        PathBuf::from("lucent-driver-postgres")
    }

    pub async fn ensure_running(&mut self) -> Result<&Path, String> {
        // Check if existing worker is still alive
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(_)) => {
                    // Worker exited — clear child and fall through to respawn
                    log::warn!("Worker exited, will respawn");
                    self.child = None;
                }
                Ok(None) => {
                    // Worker is alive — verify the socket actually works
                    // by checking if it exists (worker may have exited between
                    // try_wait and our socket check due to the async gap).
                    if self.socket_path.exists() {
                        return Ok(&self.socket_path);
                    }
                    // Socket gone even though process reports alive — race.
                    // Fall through to respawn.
                    log::warn!("Worker socket missing, will respawn");
                    self.child = None;
                }
                Err(e) => {
                    log::warn!("Failed to check worker status: {e}");
                    self.child = None;
                    // Fall through to respawn
                }
            }
        }

        let binary = self.worker_binary_path();
        let spawn_result = Command::new(&binary)
            .arg(&self.socket_path)
            .arg(&self.handshake_token)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();

        let mut child = match spawn_result {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("failed to spawn worker: {e}");
                self.last_error = Some(msg.clone());
                return Err(msg);
            }
        };

        // Drain the worker's stderr into the shared log buffer so panics and
        // diagnostics appear in the in-app Logs drawer, and so the pipe never
        // fills and blocks the worker. The drain task owns the stderr handle,
        // which is why the timeout read below was removed.
        spawn_stderr_drain(&mut child, self.logs.clone());

        self.child = Some(child);

        for _ in 0..50 {
            if self.socket_path.exists() {
                self.last_error = None;
                return Ok(&self.socket_path);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // The socket never appeared. The drain task has been capturing worker
        // stderr into the shared logs buffer all along; point at it.
        let msg = "worker socket did not appear within 1s (see Logs drawer for worker stderr)"
            .to_string();
        self.last_error = Some(msg.clone());
        Err(msg)
    }

    pub async fn shutdown(&mut self) -> Result<(), String> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        let _ = std::fs::remove_file(&self.socket_path);
        self._temp_dir = None;
        self.last_error = None;
        Ok(())
    }

    // TODO: dead — remove or wire (see trust+quality pass spec)
    #[allow(dead_code)]
    pub fn state(&self) -> SupervisorState {
        if self.child.is_some() {
            SupervisorState::Running
        } else if let Some(ref err) = self.last_error {
            SupervisorState::Failed(err.clone())
        } else {
            SupervisorState::Stopped
        }
    }

    pub fn handshake_token(&self) -> &str {
        &self.handshake_token
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn push_log_line_caps_at_limit_and_drops_oldest() {
        let logs = new_log_buffer();
        for i in 0..(LOG_BUFFER_CAP + 5) {
            push_log_line(&logs, format!("line {i}")).await;
        }
        let buf = logs.lock().await;
        assert_eq!(buf.len(), LOG_BUFFER_CAP);
        assert_eq!(buf.front().map(String::as_str), Some("line 5"));
        assert_eq!(
            buf.back().map(String::as_str),
            Some(format!("line {}", LOG_BUFFER_CAP + 4).as_str())
        );
    }

    #[tokio::test]
    async fn drain_task_forwards_child_stderr_lines_into_buffer() {
        let logs = new_log_buffer();
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("echo first >&2; echo second >&2")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        spawn_stderr_drain(&mut child, logs.clone());
        let _ = child.wait().await.unwrap();

        for _ in 0..50 {
            if logs.lock().await.len() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let buf = logs.lock().await;
        let lines: Vec<String> = buf.iter().cloned().collect();
        assert_eq!(lines, vec!["first".to_string(), "second".to_string()]);
    }
}
