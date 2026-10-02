use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EndpointStatus {
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionSummary {
    pub profile_id: String,
    pub name: String,
    pub driver: String,
    pub database: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveryInfo {
    pub version: String,
    pub status: EndpointStatus,
    pub socket: String,
    pub token: String,
    pub pid: u32,
    pub connection: Option<ConnectionSummary>,
    pub tools: Vec<String>,
}

/// Resolves the default discovery file path (`external-mcp.json`).
///
/// Honors `LUCENT_EXTERNAL_MCP` environment variable override if set.
/// Otherwise resolves:
/// - macOS: `~/Library/Application Support/Lucent/external-mcp.json`
/// - Linux: `~/.config/lucent/external-mcp.json`
/// - Windows: `%APPDATA%\Lucent\external-mcp.json`
pub fn default_discovery_file_path() -> Result<PathBuf, String> {
    if let Ok(env_path) = std::env::var("LUCENT_EXTERNAL_MCP") {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    #[cfg(target_os = "macos")]
    {
        let data_dir = dirs::data_dir().ok_or_else(|| "Could not determine macOS Application Support directory".to_string())?;
        Ok(data_dir.join("Lucent").join("external-mcp.json"))
    }

    #[cfg(target_os = "linux")]
    {
        let base = dirs::config_dir()
            .or_else(dirs::data_dir)
            .ok_or_else(|| "Could not determine Linux config or data directory".to_string())?;
        Ok(base.join("lucent").join("external-mcp.json"))
    }

    #[cfg(target_os = "windows")]
    {
        let data_dir = dirs::data_dir().ok_or_else(|| "Could not determine Windows APPDATA directory".to_string())?;
        Ok(data_dir.join("Lucent").join("external-mcp.json"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let base = dirs::data_dir()
            .or_else(dirs::config_dir)
            .ok_or_else(|| "Could not determine user data directory".to_string())?;
        Ok(base.join("lucent").join("external-mcp.json"))
    }
}

/// Resolves the dedicated private socket directory (`lucent/ipc`).
///
/// Under Unix, permissions must be strictly 0700 (owner read/write/execute only).
pub fn socket_dir_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    let base = dirs::runtime_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| "Could not determine Linux runtime or data directory".to_string())?;

    #[cfg(not(target_os = "linux"))]
    let base = dirs::data_dir()
        .ok_or_else(|| "Could not determine user data directory".to_string())?;

    let dir = base.join("lucent").join("ipc");
    Ok(dir)
}

/// Ensures the socket directory exists with strict 0700 permissions on Unix.
pub fn ensure_socket_dir() -> Result<PathBuf, String> {
    let dir = socket_dir_path()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create socket dir {}: {e}", dir.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("Failed to set 0700 permissions on socket dir {}: {e}", dir.display()))?;
    }

    Ok(dir)
}

/// Default socket path (`lucent/ipc/external.sock`).
pub fn default_socket_path() -> Result<PathBuf, String> {
    let dir = socket_dir_path()?;
    Ok(dir.join("external.sock"))
}

/// Atomically writes discovery file with strict 0600 permissions on Unix.
pub fn write_discovery_file_at(path: &Path, info: &DiscoveryInfo) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent dir: {e}"))?;
    }

    let json = serde_json::to_string_pretty(info).map_err(|e| format!("Failed to serialize discovery info: {e}"))?;

    let temp_file_name = format!(
        ".{}.tmp.{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("external-mcp"),
        uuid::Uuid::new_v4()
    );
    let temp_path = path.parent().unwrap_or_else(|| Path::new(".")).join(temp_file_name);

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp_path)
            .map_err(|e| format!("Failed to create temp discovery file at {}: {e}", temp_path.display()))?;

        file.write_all(json.as_bytes())
            .map_err(|e| format!("Failed to write discovery json: {e}"))?;
        file.flush()
            .map_err(|e| format!("Failed to flush discovery json: {e}"))?;
    }

    #[cfg(not(unix))]
    {
        std::fs::write(&temp_path, json.as_bytes())
            .map_err(|e| format!("Failed to write discovery json: {e}"))?;
    }

    // Atomic replace
    std::fs::rename(&temp_path, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        format!("Failed to rename temp discovery file to {}: {e}", path.display())
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

/// Reads and parses discovery file.
pub fn read_discovery_file_at(path: &Path) -> Result<DiscoveryInfo, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read discovery file at {}: {e}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse discovery file at {}: {e}", path.display()))
}

/// Cleans up discovery file if present.
pub fn cleanup_discovery_file_at(path: &Path) -> Result<(), String> {
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|e| format!("Failed to remove discovery file at {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Checks whether a given process ID is alive.
pub fn is_pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    #[cfg(unix)]
    {
        unsafe {
            let ret = libc::kill(pid as libc::pid_t, 0);
            if ret == 0 {
                true
            } else {
                std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
            }
        }
    }

    #[cfg(windows)]
    {
        // On Windows: OpenProcess with PROCESS_QUERY_LIMITED_INFORMATION
        false
    }

    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_file_round_trip_with_explicit_path() {
        let temp_dir = tempfile::tempdir().unwrap();
        let custom_file = temp_dir.path().join("external-mcp.json");

        let info = DiscoveryInfo {
            version: "1.0".into(),
            status: EndpointStatus::Connected,
            socket: "/tmp/lucent/ipc/test.sock".into(),
            token: "0123456789abcdef0123456789abcdef".into(),
            pid: std::process::id(),
            connection: Some(ConnectionSummary {
                profile_id: "prof_1".into(),
                name: "analytics.duckdb".into(),
                driver: "duckdb".into(),
                database: "analytics".into(),
                read_only: true,
            }),
            tools: vec!["run_readonly_query".into(), "search_schema".into()],
        };

        write_discovery_file_at(&custom_file, &info).expect("write discovery file");
        let loaded = read_discovery_file_at(&custom_file).expect("read discovery file");
        assert_eq!(info, loaded);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&custom_file).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }

        cleanup_discovery_file_at(&custom_file).expect("cleanup discovery file");
        assert!(!custom_file.exists());
    }

    #[test]
    fn pid_liveness_detects_current_and_dead_processes() {
        assert!(is_pid_alive(std::process::id()));
        assert!(!is_pid_alive(999_999_999));
    }

    #[test]
    fn ensure_socket_dir_creates_0700_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let sock_dir = temp_dir.path().join("lucent").join("ipc");

        std::fs::create_dir_all(&sock_dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&sock_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
            let meta = std::fs::metadata(&sock_dir).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        }
    }
}
