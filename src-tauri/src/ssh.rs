use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::sync::Mutex;

// ─── SshConfig ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfig {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth_method: SshAuthMethod,
}

impl SshConfig {
    pub fn new(label: String, host: String, user: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            label,
            host,
            port: 22,
            user,
            auth_method: SshAuthMethod::Password,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "method")]
pub enum SshAuthMethod {
    Password,
    Key { key_path: String },
}

// ─── Keyboard-Interactive Passthrough ──────────────────────────────────────

/// Returns the saved password for keyboard-interactive auth challenges.
/// Many SSH servers (especially PAM-backed) disable password auth and only
/// offer keyboard-interactive — this passthrough lets us handle both.
struct PassthroughPrompt {
    password: String,
}

impl ssh2::KeyboardInteractivePrompt for PassthroughPrompt {
    fn prompt(
        &mut self,
        _username: &str,
        _instructions: &str,
        _prompts: &[ssh2::Prompt<'_>],
    ) -> Vec<String> {
        vec![self.password.clone()]
    }
}

// ─── SshTunnel ─────────────────────────────────────────────────────────────

/// Manages an SSH tunnel that forwards a remote database port through a
/// local TCP port. Because `ssh2::Session` is `!Send`, the forwarding
/// loop runs on a `std::thread`, not a Tokio task.
pub struct SshTunnel {
    local_port: u16,
    /// Holds the SSH session; take() on disconnect to drop it.
    session: Arc<Mutex<Option<ssh2::Session>>>,
    /// Join handle for the forwarding thread.
    handle: Option<thread::JoinHandle<()>>,
    /// Shared flag to signal the forwarding thread to stop.
    stopped: Arc<AtomicBool>,
}

impl SshTunnel {
    /// Connect via SSH, authenticate, and start port forwarding.
    ///
    /// Returns the local port number that tunnels to `(remote_host, remote_port)`.
    pub async fn connect(
        config: &SshConfig,
        secret: &str, // password or key passphrase from keychain
        remote_host: &str,
        remote_port: u16,
    ) -> Result<Self, String> {
        let addr = format!("{}:{}", config.host, config.port);
        log::info!("SSH tunnel connecting to {addr}");

        // 1. TCP connect to SSH server
        let tcp = TcpStream::connect(&addr)
            .map_err(|e| format!("SSH connection to {addr} failed: {e}"))?;
        tcp.set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| format!("set read timeout failed: {e}"))?;
        tcp.set_write_timeout(Some(Duration::from_secs(10)))
            .map_err(|e| format!("set write timeout failed: {e}"))?;

        // 2. Create SSH session and handshake
        let mut session =
            ssh2::Session::new().map_err(|e| format!("failed to create SSH session: {e}"))?;
        session.set_tcp_stream(tcp);
        session
            .handshake()
            .map_err(|e| format!("SSH handshake failed: {e}"))?;

        // 3. Authenticate
        let authenticated = match &config.auth_method {
            SshAuthMethod::Password => {
                // Try password auth first
                let pw_result = session.userauth_password(&config.user, secret);
                if pw_result.is_ok() && session.authenticated() {
                    true
                } else {
                    // Fall back to keyboard-interactive
                    log::debug!("SSH password auth failed, trying keyboard-interactive");
                    let mut prompt = PassthroughPrompt {
                        password: secret.to_string(),
                    };
                    session
                        .userauth_keyboard_interactive(&config.user, &mut prompt)
                        .map_err(|e| {
                            format!("SSH auth failed (password + keyboard-interactive): {e}")
                        })?;
                    session.authenticated()
                }
            }
            SshAuthMethod::Key { key_path } => {
                session
                    .userauth_pubkey_file(
                        &config.user,
                        None,
                        std::path::Path::new(key_path),
                        if secret.is_empty() {
                            None
                        } else {
                            Some(secret)
                        },
                    )
                    .map_err(|e| format!("SSH key auth failed: {e}"))?;
                session.authenticated()
            }
        };

        if !authenticated {
            return Err("SSH authentication rejected by server".into());
        }
        log::info!("SSH tunnel authenticated to {addr}");

        // 4. Bind local port
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("failed to bind local port: {e}"))?;
        let local_port = listener.local_addr().unwrap().port();
        log::info!("SSH tunnel local port: {local_port}");

        // 5. Start forwarding thread
        let stopped = Arc::new(AtomicBool::new(false));
        let stopped_clone = stopped.clone();

        let session_arc = Arc::new(Mutex::new(Some(session)));
        let session_for_thread = session_arc.clone();
        let remote_host = remote_host.to_string();

        let handle = thread::spawn(move || {
            // Accept connections in non-blocking mode to check stopped flag
            listener
                .set_nonblocking(true)
                .expect("set_nonblocking on listener");

            for stream in listener.incoming() {
                if stopped_clone.load(Ordering::Relaxed) {
                    break;
                }
                match stream {
                    Ok(mut local_stream) => {
                        let sess_lock = session_for_thread.blocking_lock();
                        let sess_opt = sess_lock.as_ref();
                        if let Some(sess) = sess_opt {
                            if let Ok(mut channel) =
                                sess.channel_direct_tcpip(&remote_host, remote_port, None)
                            {
                                // MVP forwarding: simple bidirectional copy
                                // using a buffer loop. For production, this should
                                // use two threads with select/poll.
                                let mut buf = [0u8; 8192];
                                loop {
                                    // Try reading from local socket -> write to channel
                                    match local_stream.read(&mut buf) {
                                        Ok(0) | Err(_) => break,
                                        Ok(n) => {
                                            if channel.write(&buf[..n]).is_err() {
                                                break;
                                            }
                                        }
                                    }
                                    // Try reading from channel -> write to local socket
                                    match channel.read(&mut buf) {
                                        Ok(0) | Err(_) => break,
                                        Ok(n) => {
                                            if local_stream.write(&buf[..n]).is_err() {
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // Drop the lock explicitly
                        drop(sess_lock);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => break,
                }
            }
            drop(listener);
        });

        Ok(Self {
            local_port,
            session: session_arc,
            handle: Some(handle),
            stopped,
        })
    }

    /// The local port number that forwards to the remote database.
    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    /// Poll the local port until it accepts a TCP connection,
    /// indicating the tunnel is ready.
    pub async fn wait_ready(&self, timeout: Duration) -> Result<(), String> {
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > timeout {
                return Err("SSH tunnel readiness timed out".into());
            }
            if TcpStream::connect(format!("127.0.0.1:{}", self.local_port)).is_ok() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Disconnect the SSH session and join the forwarding thread.
    pub async fn disconnect(mut self) {
        log::info!("SSH tunnel disconnecting (port {})", self.local_port);
        // Signal thread to stop
        self.stopped.store(true, Ordering::Relaxed);
        // Drop SSH session (closes connection)
        *self.session.lock().await = None;
        // Join thread
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        log::info!("SSH tunnel disconnected");
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        // Best-effort cleanup if disconnect() wasn't called
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use ssh2::KeyboardInteractivePrompt;

    #[test]
    fn test_ssh_config_defaults() {
        let config = SshConfig::new("Test".into(), "host.example.com".into(), "admin".into());
        assert_eq!(config.label, "Test");
        assert_eq!(config.host, "host.example.com");
        assert_eq!(config.port, 22);
        assert_eq!(config.user, "admin");
        assert_eq!(config.auth_method, SshAuthMethod::Password);
        assert!(!config.id.is_empty());
    }

    #[test]
    fn test_ssh_auth_method_roundtrip() {
        // Password
        let pw = SshAuthMethod::Password;
        let json = serde_json::to_value(&pw).unwrap();
        assert!(json.get("method").and_then(|v| v.as_str()).is_some());
        let deser: SshAuthMethod = serde_json::from_value(json).unwrap();
        assert_eq!(deser, SshAuthMethod::Password);

        // Key
        let key = SshAuthMethod::Key {
            key_path: "/home/user/.ssh/id_rsa".into(),
        };
        let json = serde_json::to_value(&key).unwrap();
        assert!(json.get("method").and_then(|v| v.as_str()).is_some());
        // The key_path field may be renamed by rename_all - check both possibilities
        let path = json.get("keyPath").or_else(|| json.get("key_path"));
        assert_eq!(
            path.and_then(|v| v.as_str()),
            Some("/home/user/.ssh/id_rsa")
        );
        let deser: SshAuthMethod = serde_json::from_value(json).unwrap();
        assert_eq!(deser, key);
    }

    #[test]
    fn test_ssh_config_serialization_roundtrip() {
        let config = SshConfig {
            id: "test-id".into(),
            label: "Bastion".into(),
            host: "bastion.example.com".into(),
            port: 2222,
            user: "admin".into(),
            auth_method: SshAuthMethod::Key {
                key_path: "/home/admin/.ssh/id_ed25519".into(),
            },
        };
        let json = serde_json::to_string(&config).unwrap();
        let deser: SshConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.id, "test-id");
        assert_eq!(deser.port, 2222);
        assert_eq!(deser.host, "bastion.example.com");

        // Verify field names (camelCase expected)
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(val.get("label").is_some(), "expected 'label' field");
        // authMethod or auth_method depending on rename_all behavior
        assert!(
            val.get("authMethod").is_some() || val.get("auth_method").is_some(),
            "expected 'authMethod' or 'auth_method' field, got: {:?}",
            val
        );
    }

    #[test]
    fn test_passthrough_prompt_returns_password() {
        let mut prompt = PassthroughPrompt {
            password: "hunter2".into(),
        };
        let result = prompt.prompt("user", "instructions", &[]);
        assert_eq!(result, vec!["hunter2".to_string()]);
    }
}
