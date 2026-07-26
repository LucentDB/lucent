use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// Thread-local test override for config directory path.
// Each test thread gets its own independent path so parallel test
// execution doesn't cause races.
#[cfg(test)]
thread_local! {
    pub(crate) static TEST_CONFIG_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

// ─── Types ──────────────────────────────────────────────────────────────────

/// Top-level file format. v1 always writes this wrapper.
/// Read path is backward-compatible with v0 bare arrays.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionsFile {
    #[serde(default)]
    pub profiles: Vec<ConnectionProfile>,
    #[serde(default)]
    pub ssh_tunnels: Vec<crate::ssh::SshConfig>,
}

/// A saved database connection profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_driver")]
    pub driver: String,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_user")]
    pub user: String,
    pub database: String,
    #[serde(default)]
    pub ssl_mode: SslMode,
    pub ssh_tunnel_id: Option<String>,
    pub group: Option<String>,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub last_used: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn default_driver() -> String {
    "postgres".into()
}
fn default_host() -> String {
    "127.0.0.1".into()
}
fn default_port() -> u16 {
    5432
}
fn default_user() -> String {
    "postgres".into()
}

impl ConnectionProfile {
    /// Create a new profile with sensible defaults.
    pub fn new(name: String) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            driver: "postgres".into(),
            host: "127.0.0.1".into(),
            port: 5432,
            user: "postgres".into(),
            database: "postgres".into(),
            ssl_mode: SslMode::Prefer,
            ssh_tunnel_id: None,
            group: None,
            color: None,
            icon: None,
            last_used: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

/// SSL mode for PostgreSQL connections.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
}

impl std::fmt::Display for SslMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SslMode::Disable => write!(f, "disable"),
            SslMode::Prefer => write!(f, "prefer"),
            SslMode::Require => write!(f, "require"),
        }
    }
}

// ─── File Path ──────────────────────────────────────────────────────────────

pub fn connections_file_path() -> PathBuf {
    #[cfg(test)]
    {
        let override_path = TEST_CONFIG_DIR.with(|cell| cell.borrow().clone());
        if let Some(dir) = override_path {
            let path = dir.join("lucent");
            std::fs::create_dir_all(&path).ok();
            return path.join("connections.json");
        }
    }
    if let Ok(dir) = std::env::var("LUCENT_CONFIG_DIR") {
        let path = PathBuf::from(dir).join("lucent");
        std::fs::create_dir_all(&path).ok();
        return path.join("connections.json");
    }
    let base = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lucent");
    std::fs::create_dir_all(&base).ok();
    base.join("connections.json")
}

// ─── Read / Write ───────────────────────────────────────────────────────────

/// Read all profiles from disk.
/// Tries v1 wrapper format first, falls back to v0 bare array for compat.
pub fn read_all_profiles() -> Vec<ConnectionProfile> {
    let path = connections_file_path();
    if !path.exists() {
        return vec![];
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    // Try v1 wrapper format
    if let Ok(file) = serde_json::from_str::<ConnectionsFile>(&content) {
        return file.profiles;
    }
    // Fall back to v0 bare array
    serde_json::from_str::<Vec<ConnectionProfile>>(&content).unwrap_or_default()
}

/// Write profiles and SSH configs atomically (write to temp, rename).
/// Always writes v1 wrapper format.
pub fn write_all(
    profiles: &[ConnectionProfile],
    ssh_configs: &[crate::ssh::SshConfig],
) -> Result<(), String> {
    let path = connections_file_path();
    let tmp = path.with_extension("json.tmp");
    let file = ConnectionsFile {
        profiles: profiles.to_vec(),
        ssh_tunnels: ssh_configs.to_vec(),
    };
    let content = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, &content).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

// ─── Keychain Helpers ───────────────────────────────────────────────────────

use keyring::Entry;

const KEYCHAIN_SERVICE: &str = "lucent-connection";
const KEYCHAIN_SSH_SERVICE: &str = "lucent-ssh";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeychainError {
    NotFound,
    NoStorageAccess,
    Other(String),
}

impl std::fmt::Display for KeychainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeychainError::NotFound => {
                write!(f, "No password saved for this connection")
            }
            KeychainError::NoStorageAccess => write!(
                f,
                "Cannot access system keychain — app may need signing or keychain access permission"
            ),
            KeychainError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for KeychainError {}

pub fn get_password(profile_id: &str) -> Result<String, KeychainError> {
    let entry = Entry::new(KEYCHAIN_SERVICE, profile_id)
        .map_err(|e| KeychainError::Other(e.to_string()))?;
    match entry.get_password() {
        Ok(pw) => Ok(pw),
        Err(keyring::Error::NoEntry) => Err(KeychainError::NotFound),
        Err(keyring::Error::NoStorageAccess(_)) => Err(KeychainError::NoStorageAccess),
        Err(e) => Err(KeychainError::Other(e.to_string())),
    }
}

pub fn set_password(profile_id: &str, password: &str) -> Result<(), KeychainError> {
    let entry = Entry::new(KEYCHAIN_SERVICE, profile_id)
        .map_err(|e| KeychainError::Other(e.to_string()))?;
    entry
        .set_password(password)
        .map_err(|e| KeychainError::Other(e.to_string()))
}

pub fn delete_password(profile_id: &str) -> Result<(), KeychainError> {
    let entry = Entry::new(KEYCHAIN_SERVICE, profile_id)
        .map_err(|e| KeychainError::Other(e.to_string()))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(KeychainError::Other(e.to_string())),
    }
}

/// SSH secret helpers
pub fn get_ssh_secret(tunnel_id: &str) -> Result<String, KeychainError> {
    let entry = Entry::new(KEYCHAIN_SSH_SERVICE, tunnel_id)
        .map_err(|e| KeychainError::Other(e.to_string()))?;
    match entry.get_password() {
        Ok(pw) => Ok(pw),
        Err(keyring::Error::NoEntry) => Err(KeychainError::NotFound),
        Err(keyring::Error::NoStorageAccess(_)) => Err(KeychainError::NoStorageAccess),
        Err(e) => Err(KeychainError::Other(e.to_string())),
    }
}

pub fn set_ssh_secret(tunnel_id: &str, secret: &str) -> Result<(), KeychainError> {
    let entry = Entry::new(KEYCHAIN_SSH_SERVICE, tunnel_id)
        .map_err(|e| KeychainError::Other(e.to_string()))?;
    entry
        .set_password(secret)
        .map_err(|e| KeychainError::Other(e.to_string()))
}

pub fn delete_ssh_secret(tunnel_id: &str) -> Result<(), KeychainError> {
    let entry = Entry::new(KEYCHAIN_SSH_SERVICE, tunnel_id)
        .map_err(|e| KeychainError::Other(e.to_string()))?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(KeychainError::Other(e.to_string())),
    }
}

// ─── Repository ─────────────────────────────────────────────────────────────

use tokio::sync::RwLock;

/// Repository encapsulating all profile and SSH config storage.
/// Stored as a single Arc'd field in AppState.
pub struct ConnectionProfileRepository {
    profiles: RwLock<Vec<ConnectionProfile>>,
    ssh_configs: RwLock<Vec<crate::ssh::SshConfig>>,
}

impl ConnectionProfileRepository {
    /// Load from disk on startup. Backward-compatible with v0 format.
    pub fn load() -> Self {
        let path = connections_file_path();
        let (profiles, ssh_configs) = if path.exists() {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            if let Ok(file) = serde_json::from_str::<ConnectionsFile>(&content) {
                (file.profiles, file.ssh_tunnels)
            } else {
                // v0 bare array fallback
                let profiles =
                    serde_json::from_str::<Vec<ConnectionProfile>>(&content).unwrap_or_default();
                (profiles, vec![])
            }
        } else {
            (vec![], vec![])
        };
        Self {
            profiles: RwLock::new(profiles),
            ssh_configs: RwLock::new(ssh_configs),
        }
    }

    // ─── Profile operations ─────────────────────────────────────────────

    pub async fn list_profiles(&self) -> Vec<ConnectionProfile> {
        self.profiles.read().await.clone()
    }

    pub async fn get_profile(&self, id: &str) -> Option<ConnectionProfile> {
        self.profiles
            .read()
            .await
            .iter()
            .find(|p| p.id == id)
            .cloned()
    }

    pub async fn save_profile(&self, profile: ConnectionProfile) -> Result<(), String> {
        let mut profiles = self.profiles.write().await;
        if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile;
        } else {
            profiles.push(profile);
        }
        let ssh = self.ssh_configs.read().await.clone();
        write_all(&profiles, &ssh)
    }

    pub async fn delete_profile(&self, id: &str) -> Result<(), String> {
        let mut profiles = self.profiles.write().await;
        profiles.retain(|p| p.id != id);
        let ssh = self.ssh_configs.read().await.clone();
        write_all(&profiles, &ssh)?;
        delete_password(id).ok(); // best-effort
        Ok(())
    }

    pub async fn mark_used(&self, id: &str) -> Result<(), String> {
        let mut profiles = self.profiles.write().await;
        if let Some(p) = profiles.iter_mut().find(|p| p.id == id) {
            p.last_used = Some(chrono::Utc::now().to_rfc3339());
            p.updated_at = chrono::Utc::now().to_rfc3339();
        }
        let ssh = self.ssh_configs.read().await.clone();
        write_all(&profiles, &ssh)
    }

    // ─── SSH config operations ───────────────────────────────────────────

    pub async fn list_ssh_configs(&self) -> Vec<crate::ssh::SshConfig> {
        self.ssh_configs.read().await.clone()
    }

    pub async fn get_ssh_config(&self, id: &str) -> Option<crate::ssh::SshConfig> {
        self.ssh_configs
            .read()
            .await
            .iter()
            .find(|c| c.id == id)
            .cloned()
    }

    pub async fn save_ssh_config(&self, config: crate::ssh::SshConfig) -> Result<(), String> {
        let mut ssh = self.ssh_configs.write().await;
        if let Some(existing) = ssh.iter_mut().find(|c| c.id == config.id) {
            *existing = config;
        } else {
            ssh.push(config);
        }
        let profiles = self.profiles.read().await.clone();
        write_all(&profiles, &ssh)
    }

    pub async fn delete_ssh_config(&self, id: &str) -> Result<(), String> {
        let mut ssh = self.ssh_configs.write().await;
        ssh.retain(|c| c.id != id);
        let profiles = self.profiles.read().await.clone();
        write_all(&profiles, &ssh)?;
        delete_ssh_secret(id).ok();
        Ok(())
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tests/connections_test.rs"]
mod connections_tests;
