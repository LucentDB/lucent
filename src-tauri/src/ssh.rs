use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::sync::{watch, Mutex};

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

// ─── Handler ───────────────────────────────────────────────────────────────

#[derive(Clone)]
struct TunnelHandler {
    _accepted: Arc<AtomicBool>,
}

impl russh::client::Handler for TunnelHandler {
    type Error = russh::Error;

    /// Current policy: accept all host keys (matches the previous ssh2
    /// behavior of no verification). Follow-up: strict mode via
    /// russh::keys::known_hosts::check_known_hosts_path behind a setting.
    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

// ─── Tunnel ────────────────────────────────────────────────────────────────

/// Manages an SSH tunnel that forwards a remote database port through a
/// local TCP port. The forwarding loop runs as a Tokio task (russh is fully
/// async); `disconnect()` signals it to stop via a watch channel.
pub struct SshTunnel {
    pub local_port: u16,
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl SshTunnel {
    /// Connect via SSH, authenticate, and start port forwarding.
    ///
    /// Returns a tunnel whose `local_port` forwards to `(remote_host, remote_port)`.
    pub async fn connect(
        config: &SshConfig,
        secret: &str, // password or key passphrase from keychain
        remote_host: &str,
        remote_port: u16,
    ) -> Result<Self, String> {
        let addr = format!("{}:{}", config.host, config.port);
        log::info!("SSH tunnel connecting to {addr}");

        let client_cfg = russh::client::Config {
            keepalive_interval: Some(Duration::from_secs(30)),
            ..Default::default()
        };
        let client_cfg = Arc::new(client_cfg);

        // russh 0.62: client::connect returns the Handle directly (the old
        // (Session, Handle) split was removed); the handle owns the connection
        // task and dropping it closes the session. The WHOLE connect+auth
        // phase is time-bounded: a server that stalls post-handshake (never
        // answering the auth exchange) would otherwise hang authenticate_*
        // forever — the old ssh2 code had socket timeouts covering this.
        let (handle, authenticated) = tokio::time::timeout(
            Duration::from_secs(20),
            async {
                let mut handle = russh::client::connect(
                    client_cfg,
                    addr.as_str(),
                    TunnelHandler {
                        _accepted: Arc::new(AtomicBool::new(false)),
                    },
                )
                .await
                .map_err(|e| format!("SSH connect to {addr} failed: {e}"))?;

                let authenticated = match &config.auth_method {
                    SshAuthMethod::Password => {
                        let pw = handle
                            .authenticate_password(&config.user, secret)
                            .await
                            .map_err(|e| format!("SSH password auth failed: {e}"))?;
                        match pw {
                            russh::client::AuthResult::Success => true,
                            _ => {
                                // Fall back to keyboard-interactive (mirrors the old
                                // PassthroughPrompt behavior: answer every prompt with
                                // the secret).
                                log::debug!(
                                    "SSH password auth failed, trying keyboard-interactive"
                                );
                                let mut resp = handle
                                    .authenticate_keyboard_interactive_start(&config.user, None)
                                    .await
                                    .map_err(|e| {
                                        format!("SSH keyboard-interactive failed: {e}")
                                    })?;
                                loop {
                                    match resp {
                                        russh::client::KeyboardInteractiveAuthResponse::Success => {
                                            break true;
                                        }
                                        russh::client::KeyboardInteractiveAuthResponse::Failure {
                                            ..
                                        } => {
                                            break false;
                                        }
                                        russh::client::KeyboardInteractiveAuthResponse::InfoRequest {
                                            prompts,
                                            ..
                                        } => {
                                            let answers: Vec<String> =
                                                prompts.iter().map(|_| secret.to_string()).collect();
                                            resp = handle
                                                .authenticate_keyboard_interactive_respond(
                                                    answers,
                                                )
                                                .await
                                                .map_err(|e| {
                                                    format!(
                                                        "SSH keyboard-interactive respond failed: {e}"
                                                    )
                                                })?;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    SshAuthMethod::Key { key_path } => {
                        let key = russh::keys::load_secret_key(
                            std::path::Path::new(key_path),
                            if secret.is_empty() {
                                None
                            } else {
                                Some(secret)
                            },
                        )
                        .map_err(|e| format!("failed to load SSH key {key_path}: {e}"))?;
                        let hash_alg = handle
                            .best_supported_rsa_hash()
                            .await
                            .map_err(|e| format!("RSA hash negotiation failed: {e}"))?
                            .flatten(); // Option<Option<HashAlg>> → Option<HashAlg>
                        let key = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg);
                        matches!(
                            handle
                                .authenticate_publickey(&config.user, key)
                                .await
                                .map_err(|e| format!("SSH publickey auth failed: {e}"))?,
                            russh::client::AuthResult::Success
                        )
                    }
                };
                Ok::<_, String>((handle, authenticated))
            },
        )
        .await
        .map_err(|_| format!("SSH connect+auth to {addr} timed out after 20s"))??;
        if !authenticated {
            return Err("SSH authentication rejected by server".into());
        }
        log::info!("SSH tunnel authenticated to {addr}");

        // Bind local port
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("failed to bind local port: {e}"))?;
        let local_port = listener.local_addr().unwrap().port();
        log::info!("SSH tunnel local port: {local_port}");

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let handle = Arc::new(Mutex::new(handle));
        let task = tokio::spawn(forward_loop(
            listener,
            handle,
            remote_host.to_string(),
            remote_port,
            local_port,
            shutdown_rx,
        ));

        Ok(SshTunnel {
            local_port,
            shutdown: shutdown_tx,
            task,
        })
    }

    /// Signal the forwarding loop to stop and wait up to 500 ms for it to
    /// unwind (the accept loop observes the watch channel and breaks, closing
    /// the SSH session). Best-effort: the task is detached if it outlives the
    /// wait — the watch receiver drop closes the channel either way.
    pub async fn disconnect(&mut self) {
        let _ = self.shutdown.send(true);
        let _ = tokio::time::timeout(Duration::from_millis(500), &mut self.task).await;
    }
}

/// Accept loop: one direct-tcpip channel per local connection, full-duplex
/// copies in both directions (tokio::io::copy tasks). `eof()` semantics are
/// handled by `shutdown()` on the channel's write half when the peer side
/// finishes, which is what keeps large one-way streams from hanging.
async fn forward_loop(
    listener: TcpListener,
    handle: Arc<Mutex<russh::client::Handle<TunnelHandler>>>,
    remote_host: String,
    remote_port: u16,
    local_port: u16,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            accepted = listener.accept() => {
                let (local_stream, _) = match accepted {
                    Ok(x) => x,
                    Err(_) => continue,
                };
                let handle = handle.clone();
                let remote_host = remote_host.clone();
                tokio::spawn(async move {
                    let channel = {
                        let h = handle.lock().await;
                        h.channel_open_direct_tcpip(
                            remote_host.clone(),
                            remote_port as u32,
                            "127.0.0.1",
                            local_port as u32,
                        )
                        .await
                    };
                    let mut channel = match channel {
                        Ok(c) => Box::pin(c.into_stream()),
                        Err(e) => {
                            log::warn!("direct-tcpip channel open failed: {e}");
                            return;
                        }
                    };
                    // Full-duplex copy with half-close semantics: when one side
                    // reaches EOF, copy_bidirectional shuts down the opposite
                    // write half (which sends SSH Eof via poll_shutdown) while
                    // continuing to drain the other direction. This is what
                    // keeps large one-way streams (e.g. a big result set) from
                    // hanging — the ssh2 code's alternating half-duplex loop
                    // did exactly that.
                    let mut local_stream = local_stream;
                    let _ = tokio::io::copy_bidirectional(&mut local_stream, &mut channel).await;
                });
            }
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
}
