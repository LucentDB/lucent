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

/// A saved database connection profile (v2).
///
/// Connection parameters live in `params`, keyed per driver, because
/// `host`/`port`/`user` mean nothing to a DuckDB file or a BigQuery dataset.
/// The password never appears here — it lives in the keychain, keyed by `id`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_driver")]
    pub driver: String,
    /// Short handle for `@mention` in AI chat. Derived from `name` on creation
    /// and on migration; the user may override it.
    #[serde(default)]
    pub alias: Option<String>,
    /// Driver-defined connection parameters. See `crate::drivers`.
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, String>,
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

impl ConnectionProfile {
    /// A new Postgres profile with the defaults the form starts from.
    pub fn new(name: String) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        let alias = slugify_alias(&name);
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            alias: (!alias.is_empty()).then_some(alias),
            driver: default_driver(),
            params: crate::drivers::default_params("postgres"),
            name,
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

/// Lowercase, hyphen-separated, alphanumerics only — safe to type after `@`.
///
/// Returns an empty string when nothing usable survives; the caller stores
/// `None` rather than an empty alias, which would match every mention.
pub fn slugify_alias(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut pending_sep = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('-');
            }
            pending_sep = false;
            out.push(ch.to_ascii_lowercase());
        } else {
            pending_sep = true;
        }
    }
    out
}

/// Read one v0/v1 profile object into the v2 shape.
///
/// Returns `None` only when the object lacks an `id`, which makes it
/// unusable — every other field has a defensible default, and dropping a
/// user's saved connection because a field was absent is not acceptable.
pub fn migrate_v1_profile(value: &serde_json::Value) -> Option<ConnectionProfile> {
    let id = value.get("id")?.as_str()?.to_string();
    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    let s = |key: &str, fallback: &str| {
        value
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or(fallback)
            .to_string()
    };

    let mut params = std::collections::BTreeMap::new();
    params.insert("host".to_string(), s("host", "127.0.0.1"));
    params.insert(
        "port".to_string(),
        value
            .get("port")
            .and_then(|v| v.as_u64())
            .map(|p| p.to_string())
            .unwrap_or_else(|| "5432".to_string()),
    );
    params.insert("user".to_string(), s("user", "postgres"));
    params.insert("database".to_string(), s("database", "postgres"));
    params.insert("ssl_mode".to_string(), s("sslMode", "prefer"));

    let alias = value
        .get("alias")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| slugify_alias(&name));
    let now = chrono::Utc::now().to_rfc3339();

    Some(ConnectionProfile {
        id,
        driver: s("driver", "postgres"),
        alias: (!alias.is_empty()).then_some(alias),
        params,
        ssh_tunnel_id: value
            .get("sshTunnelId")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        group: value
            .get("group")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        color: value
            .get("color")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        icon: value
            .get("icon")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        last_used: value
            .get("lastUsed")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        created_at: s("createdAt", &now),
        updated_at: s("updatedAt", &now),
        name,
    })
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

/// Parse a connections file at any version.
///
/// v2 first, then v1/v0 through the migration. A profile that fails to migrate
/// is skipped with a warning rather than discarding the whole file — one bad
/// entry must not cost the user every saved connection.
fn parse_connections_file(content: &str) -> (Vec<ConnectionProfile>, Vec<crate::ssh::SshConfig>) {
    if let Ok(file) = serde_json::from_str::<ConnectionsFile>(content) {
        // A v1 file also parses as v2 (every new field has a default), but its
        // profiles come back with empty `params`. That is the signal to migrate.
        if file.profiles.iter().all(|p| !p.params.is_empty()) || file.profiles.is_empty() {
            return (file.profiles, file.ssh_tunnels);
        }
    }

    let Ok(raw) = serde_json::from_str::<serde_json::Value>(content) else {
        log::warn!("connections.json is not valid JSON; starting with no profiles");
        return (Vec::new(), Vec::new());
    };

    let raw_profiles = raw
        .get("profiles")
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| raw.as_array().cloned())
        .unwrap_or_default();

    let mut profiles = Vec::with_capacity(raw_profiles.len());
    for value in &raw_profiles {
        match migrate_v1_profile(value) {
            Some(p) => profiles.push(p),
            None => log::warn!("skipping an unmigratable connection profile: {value}"),
        }
    }

    let ssh = raw
        .get("ssh_tunnels")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    log::info!("migrated {} connection profiles to v2", profiles.len());
    (profiles, ssh)
}

/// Read all profiles from disk.
pub fn read_all_profiles() -> Vec<ConnectionProfile> {
    let path = connections_file_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return vec![];
    };
    parse_connections_file(&content).0
}

/// Write profiles and SSH configs atomically (write to temp, rename).
/// Always writes v1 wrapper format.
pub fn write_all(
    profiles: &[ConnectionProfile],
    ssh_configs: &[crate::ssh::SshConfig],
) -> Result<(), String> {
    write_all_at(&connections_file_path(), profiles, ssh_configs)
}

/// The atomic write body. Takes the path explicitly so async callers can
/// resolve it on the calling thread before crossing into the blocking pool
/// (G1) — the test override is thread-local.
fn write_all_at(
    path: &std::path::Path,
    profiles: &[ConnectionProfile],
    ssh_configs: &[crate::ssh::SshConfig],
) -> Result<(), String> {
    // Process-unique tmp name: even a stray concurrent writer (or a
    // crashed process's leftover) cannot be truncated under another
    // writer's rename (G2). The repository's write_lock already serializes
    // writers within this process; the unique name is defense-in-depth.
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let file = ConnectionsFile {
        profiles: profiles.to_vec(),
        ssh_tunnels: ssh_configs.to_vec(),
    };
    let content = serde_json::to_string_pretty(&file).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, &content).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Async wrapper: file I/O must not run on the Tokio runtime (G1). Runs on
/// the blocking pool; the repository's write_lock is held by the caller
/// across this await, which is safe (the lock is a tokio mutex and the
/// guard is Send). Path resolved on the calling thread.
pub async fn write_all_async(
    profiles: Vec<ConnectionProfile>,
    ssh_configs: Vec<crate::ssh::SshConfig>,
) -> Result<(), String> {
    let path = connections_file_path();
    tokio::task::spawn_blocking(move || write_all_at(&path, &profiles, &ssh_configs))
        .await
        .map_err(|e| format!("persistence task panicked: {e}"))?
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
    /// Serializes the read-modify-write persistence cycle. Without it,
    /// `save_profile` (profiles write → ssh read) and `save_ssh_config`
    /// (ssh write → profiles read) take the two rw-locks in OPPOSITE order
    /// — a classic lock-order inversion that can deadlock — and both write
    /// the same fixed `connections.json.tmp`, racing truncate/write/rename
    /// (G2).
    write_lock: tokio::sync::Mutex<()>,
}

impl ConnectionProfileRepository {
    /// Load from disk on startup. Backward-compatible with v0/v1 formats.
    pub fn load() -> Self {
        let path = connections_file_path();
        let (profiles, ssh_configs) = if path.exists() {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            parse_connections_file(&content)
        } else {
            (vec![], vec![])
        };
        Self {
            profiles: RwLock::new(profiles),
            ssh_configs: RwLock::new(ssh_configs),
            write_lock: tokio::sync::Mutex::new(()),
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
        let _write = self.write_lock.lock().await;
        let mut profiles = self.profiles.write().await;
        if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile;
        } else {
            profiles.push(profile);
        }
        let ssh = self.ssh_configs.read().await.clone();
        write_all_async(profiles.clone(), ssh).await
    }

    pub async fn delete_profile(&self, id: &str) -> Result<(), String> {
        let _write = self.write_lock.lock().await;
        let mut profiles = self.profiles.write().await;
        profiles.retain(|p| p.id != id);
        let ssh = self.ssh_configs.read().await.clone();
        write_all_async(profiles.clone(), ssh).await?;
        delete_password(id).ok(); // best-effort
        Ok(())
    }

    pub async fn mark_used(&self, id: &str) -> Result<(), String> {
        let _write = self.write_lock.lock().await;
        let mut profiles = self.profiles.write().await;
        if let Some(p) = profiles.iter_mut().find(|p| p.id == id) {
            p.last_used = Some(chrono::Utc::now().to_rfc3339());
            p.updated_at = chrono::Utc::now().to_rfc3339();
        }
        let ssh = self.ssh_configs.read().await.clone();
        write_all_async(profiles.clone(), ssh).await
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
        let _write = self.write_lock.lock().await;
        let mut ssh = self.ssh_configs.write().await;
        if let Some(existing) = ssh.iter_mut().find(|c| c.id == config.id) {
            *existing = config;
        } else {
            ssh.push(config);
        }
        let profiles = self.profiles.read().await.clone();
        write_all_async(profiles.clone(), ssh.clone()).await
    }

    pub async fn delete_ssh_config(&self, id: &str) -> Result<(), String> {
        let _write = self.write_lock.lock().await;
        let mut ssh = self.ssh_configs.write().await;
        ssh.retain(|c| c.id != id);
        let profiles = self.profiles.read().await.clone();
        write_all_async(profiles.clone(), ssh.clone()).await?;
        delete_ssh_secret(id).ok();
        Ok(())
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tests/connections_test.rs"]
mod connections_tests;
