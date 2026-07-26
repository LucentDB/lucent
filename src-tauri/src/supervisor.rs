use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tempfile::TempDir;
use tokio::process::{Child, Command};

// TODO: dead — remove or wire (see trust+quality pass spec)
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub enum SupervisorState {
    Stopped,
    Running,
    Failed(String),
}

pub struct Supervisor {
    child: Option<Child>,
    socket_path: PathBuf,
    handshake_token: String,
    _temp_dir: Option<TempDir>,
    last_error: Option<String>,
}

impl Supervisor {
    pub fn new() -> Self {
        let temp_dir = TempDir::new().expect("failed to create temp dir for worker socket");
        let socket_path = temp_dir.path().join("worker.sock");
        let handshake_token = uuid::Uuid::new_v4().to_string();

        Self {
            child: None,
            socket_path,
            handshake_token,
            _temp_dir: Some(temp_dir),
            last_error: None,
        }
    }

    fn worker_binary_path(&self) -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .map(|d| d.join("lucent-driver-postgres"))
            .unwrap_or_else(|| PathBuf::from("lucent-driver-postgres"))
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

        let child = match spawn_result {
            Ok(c) => c,
            Err(e) => {
                let msg = format!("failed to spawn worker: {e}");
                self.last_error = Some(msg.clone());
                return Err(msg);
            }
        };

        self.child = Some(child);

        for _ in 0..50 {
            if self.socket_path.exists() {
                self.last_error = None;
                return Ok(&self.socket_path);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // Socket didn't appear — capture any stderr from the worker for diagnostics
        let stderr_output = if let Some(ref mut c) = self.child {
            if let Some(stderr) = c.stderr.take() {
                use tokio::io::AsyncReadExt;
                let mut reader = stderr;
                let mut buf = String::new();
                let _ = tokio::time::timeout(Duration::from_millis(50), async {
                    let _ = reader.read_to_string(&mut buf).await;
                })
                .await;
                buf
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        if !stderr_output.is_empty() {
            log::error!("Worker stderr: {stderr_output}");
        }

        let msg = "worker socket did not appear within 1s".to_string();
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
