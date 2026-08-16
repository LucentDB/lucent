use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;
use tokio::io::AsyncBufReadExt;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

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
    driver_id: String,
    child: Option<Child>,
    endpoint: String,
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

/// The worker binary for a driver id. One process per driver *type*, not per
/// connection (see `AGENTS.md`).
pub fn worker_binary_name(driver_id: &str) -> String {
    format!("lucent-driver-{driver_id}")
}

/// The per-driver override env var.
///
/// Deliberately scoped: a single `LUCENT_WORKER_BINARY` would point every
/// driver at one binary, which silently runs DuckDB queries against Postgres.
pub fn worker_binary_env_var(driver_id: &str) -> String {
    format!("LUCENT_WORKER_BINARY_{}", driver_id.to_uppercase())
}

impl Supervisor {
    /// A supervisor for the Postgres worker — the original single-driver
    /// behaviour. Driver-aware callers use [`Supervisor::for_driver`].
    pub fn new() -> Self {
        Self::for_driver("postgres", new_log_buffer())
    }

    /// A supervisor for one driver.
    pub fn for_driver(driver_id: &str, logs: LogBuffer) -> Self {
        let handshake_token = uuid::Uuid::new_v4().to_string();
        let (endpoint, temp_dir) = Self::endpoint_for(&handshake_token);

        Self {
            driver_id: driver_id.to_string(),
            child: None,
            endpoint,
            handshake_token,
            _temp_dir: temp_dir,
            last_error: None,
            logs,
        }
    }

    pub fn driver_id(&self) -> &str {
        &self.driver_id
    }

    /// The worker's IPC endpoint: a socket file path on Unix, a named-pipe
    /// name (`\\.\pipe\...`) on Windows. A filesystem path is NOT a valid
    /// pipe name; the pipe name embeds a token prefix so it is unguessable
    /// per launch.
    fn endpoint_for(token: &str) -> (String, Option<TempDir>) {
        #[cfg(unix)]
        {
            let _ = token; // unused on Unix; Windows embeds it in the pipe name
            let temp_dir = TempDir::new().expect("failed to create temp dir for worker socket");
            let endpoint = temp_dir
                .path()
                .join("worker.sock")
                .to_string_lossy()
                .into_owned();
            (endpoint, Some(temp_dir))
        }
        #[cfg(windows)]
        {
            let token8: String = token.chars().take(8).collect();
            let endpoint = format!(r"\\.\pipe\lucent-{}-{token8}", std::process::id());
            (endpoint, None)
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    fn worker_binary_path(&self) -> PathBuf {
        let name = worker_binary_name(&self.driver_id);

        if let Ok(path) = std::env::var(worker_binary_env_var(&self.driver_id)) {
            return PathBuf::from(path);
        }

        // Search relative to the current executable's directory.
        // The running binary is either in target/debug/ (tauri dev) or
        // target/debug/deps/ (test). The worker sits in target/debug/.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                for rel in &["", "../", "../../"] {
                    let candidate = parent.join(rel).join(&name);
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
                let candidate = cwd.join(rel).join(&name);
                if let Ok(canonical) = candidate.canonicalize() {
                    log::info!("Found worker binary at: {}", canonical.display());
                    return canonical;
                }
            }
        }

        log::warn!("Worker binary {name:?} not found; falling back to PATH lookup");
        PathBuf::from(name)
    }

    pub async fn ensure_running(&mut self) -> Result<(), String> {
        // Check if existing worker is still alive
        if let Some(ref mut child) = self.child {
            match child.try_wait() {
                Ok(Some(_)) => {
                    // Worker exited — clear child and fall through to respawn
                    log::warn!("Worker exited, will respawn");
                    self.child = None;
                }
                Ok(None) => {
                    // Worker is alive — verify the endpoint actually works
                    // (worker may have exited between try_wait and our check
                    // due to the async gap). On Unix that means the socket
                    // file exists; on Windows a live process owns the pipe.
                    #[cfg(unix)]
                    let endpoint_ok = std::path::Path::new(&self.endpoint).exists();
                    #[cfg(windows)]
                    let endpoint_ok = true;
                    if endpoint_ok {
                        return Ok(());
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
            .arg(&self.endpoint)
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

        // On Windows the pipe is created synchronously in the worker's main()
        // before serve(); give the freshly spawned process a beat to bind it
        // (opening the pipe to probe it would consume the worker's single
        // client).
        #[cfg(windows)]
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Budget for the worker to exec and bind. The debug DuckDB binary is
        // ~120 MB and can take seconds to start under memory pressure; the old
        // 1s budget failed first connects spuriously. 5s is generous for both
        // drivers and the loop still returns as soon as the endpoint appears.
        for _ in 0..250 {
            #[cfg(unix)]
            let ready = std::path::Path::new(&self.endpoint).exists();
            #[cfg(windows)]
            let ready = self
                .child
                .as_mut()
                .map(|child| child.try_wait().map(|s| s.is_none()).unwrap_or(false))
                .unwrap_or(false);
            if ready {
                self.last_error = None;
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        // The endpoint never became ready. The drain task has been capturing
        // worker stderr into the shared logs buffer all along; point at it.
        // Kill the half-started worker: leaving it running would leak an idle
        // worker process whose endpoint can never be connected to.
        if let Some(mut child) = self.child.take() {
            log::warn!("Worker never became ready; killing it");
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        let msg =
            "worker endpoint did not become ready within 5s (see Logs drawer for worker stderr)"
                .to_string();
        self.last_error = Some(msg.clone());
        Err(msg)
    }

    pub async fn shutdown(&mut self) -> Result<(), String> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.endpoint);
        }
        self._temp_dir = None;
        self.last_error = None;
        Ok(())
    }

    pub fn handshake_token(&self) -> &str {
        &self.handshake_token
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        // Best-effort kill of a live worker when the supervisor is dropped
        // without an explicit shutdown (panic paths, early returns). `kill`
        // is async, so `start_kill` is the synchronous form: it sends SIGKILL
        // and the worker dies even though the exit status is never awaited.
        // Explicit paths (shutdown, driver switch) already take+kill+wait;
        // this is the safety net for the rest.
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
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

    #[test]
    fn the_worker_binary_name_is_derived_from_the_driver_id() {
        assert_eq!(worker_binary_name("postgres"), "lucent-driver-postgres");
        assert_eq!(worker_binary_name("duckdb"), "lucent-driver-duckdb");
    }

    #[test]
    fn the_env_override_is_scoped_per_driver() {
        // A single LUCENT_WORKER_BINARY would point both drivers at one
        // binary, which silently runs every DuckDB query against Postgres.
        assert_eq!(
            worker_binary_env_var("postgres"),
            "LUCENT_WORKER_BINARY_POSTGRES"
        );
        assert_eq!(
            worker_binary_env_var("duckdb"),
            "LUCENT_WORKER_BINARY_DUCKDB"
        );
    }

    #[test]
    fn a_supervisor_remembers_which_driver_it_runs() {
        let sup = Supervisor::for_driver("duckdb", new_log_buffer());
        assert_eq!(sup.driver_id(), "duckdb");
    }
}
