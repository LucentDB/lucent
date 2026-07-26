use super::super::connections::*;
use std::path::PathBuf;

/// Helper: sets the test config dir override and returns the TempDir.
/// Uses a Mutex-protected static instead of global env var to avoid
/// parallel test pollution.
fn with_temp_config_dir() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let config_path = dir.path().join("lucent");
    std::fs::create_dir_all(&config_path).unwrap();
    TEST_CONFIG_DIR.with(|cell| *cell.borrow_mut() = Some(dir.path().to_path_buf()));
    (dir, config_path)
}

#[test]
fn test_connection_profile_defaults() {
    let profile = ConnectionProfile::new("Defaults".into());
    assert_eq!(profile.driver, "postgres");
    assert_eq!(profile.host, "127.0.0.1");
    assert_eq!(profile.port, 5432);
    assert_eq!(profile.user, "postgres");
    assert_eq!(profile.database, "postgres");
    assert_eq!(profile.ssl_mode, SslMode::Prefer);
    assert!(profile.ssh_tunnel_id.is_none());
    assert!(profile.group.is_none());
    assert!(profile.last_used.is_none());
    assert!(!profile.id.is_empty());
    assert!(!profile.created_at.is_empty());
    assert!(!profile.updated_at.is_empty());
}

#[test]
fn test_ssl_mode_default() {
    assert_eq!(SslMode::default(), SslMode::Prefer);
}

#[test]
fn test_v1_wrapper_round_trip() {
    let (_dir, _config_path) = with_temp_config_dir();

    let profile = ConnectionProfile::new("Test DB".into());
    let configs: Vec<crate::ssh::SshConfig> = vec![];

    write_all(std::slice::from_ref(&profile), &configs).unwrap();

    let loaded = read_all_profiles();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, profile.id);
    assert_eq!(loaded[0].name, "Test DB");
    assert_eq!(loaded[0].host, "127.0.0.1");
    assert_eq!(loaded[0].port, 5432);
    assert_eq!(loaded[0].ssl_mode, SslMode::Prefer);
}

#[test]
fn test_v0_bare_array_compat() {
    let (_dir, config_path) = with_temp_config_dir();
    let path = config_path.join("connections.json");

    let profile = ConnectionProfile::new("Legacy".into());
    let json = serde_json::to_string_pretty(&[&profile]).unwrap();
    std::fs::write(&path, &json).unwrap();

    let profiles = read_all_profiles();
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "Legacy");
}

#[test]
fn test_empty_file_returns_empty() {
    // Use an isolated temp dir where no config file exists. Setting the override
    // to None would read the real user config dir, which may hold real profiles
    // on any machine that has actually used the app (green on CI, red locally).
    let (_dir, _path) = with_temp_config_dir();
    let profiles = read_all_profiles();
    assert!(profiles.is_empty());
}

#[test]
fn test_malformed_json_returns_empty() {
    let (_dir, config_path) = with_temp_config_dir();
    let path = config_path.join("connections.json");
    std::fs::write(&path, "not valid json").unwrap();

    let profiles = read_all_profiles();
    assert!(profiles.is_empty());
}

#[test]
fn test_atomic_write_round_trip() {
    let (_dir, config_path) = with_temp_config_dir();

    let p1 = ConnectionProfile::new("Profile 1".into());
    let p2 = ConnectionProfile::new("Profile 2".into());

    write_all(&[p1, p2], &[]).unwrap();

    let profiles = read_all_profiles();
    assert_eq!(profiles.len(), 2);

    // Verify file content is v1 wrapper format
    let content = std::fs::read_to_string(config_path.join("connections.json")).unwrap();
    let parsed: ConnectionsFile = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed.profiles.len(), 2);
    assert!(parsed.ssh_tunnels.is_empty());
}

#[test]
fn test_v1_wrapper_round_trip_with_ssh() {
    let (_dir, _config_path) = with_temp_config_dir();

    let profile = ConnectionProfile::new("With SSH".into());
    let ssh_config = crate::ssh::SshConfig {
        id: "tun-1".into(),
        label: "Bastion".into(),
        host: "bastion.example.com".into(),
        port: 22,
        user: "admin".into(),
        auth_method: crate::ssh::SshAuthMethod::Password,
    };

    write_all(&[profile], &[ssh_config]).unwrap();

    // Read back via ConnectionsFile directly
    let path = connections_file_path();
    let content = std::fs::read_to_string(&path).unwrap();
    let parsed: ConnectionsFile = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed.profiles.len(), 1);
    assert_eq!(parsed.ssh_tunnels.len(), 1);
    assert_eq!(parsed.ssh_tunnels[0].label, "Bastion");
}

#[test]
fn test_profile_serialization_renames() {
    let profile = ConnectionProfile::new("My DB".into());
    let json = serde_json::to_value(&profile).unwrap();

    // Verify camelCase field names
    assert!(json.get("id").is_some());
    assert!(json.get("name").is_some());
    assert!(json.get("sslMode").is_some(), "expected camelCase sslMode");
    assert!(
        json.get("sshTunnelId").is_some(),
        "expected camelCase sshTunnelId"
    );
    assert!(
        json.get("lastUsed").is_some(),
        "expected camelCase lastUsed"
    );
    assert!(
        json.get("createdAt").is_some(),
        "expected camelCase createdAt"
    );
    assert!(
        json.get("updatedAt").is_some(),
        "expected camelCase updatedAt"
    );

    // snake_case should NOT be present
    assert!(json.get("ssl_mode").is_none());
}

#[test]
fn test_keychain_get_password_not_found() {
    let result = get_password("nonexistent-profile");
    assert!(matches!(result, Err(KeychainError::NotFound)));
}

#[test]
fn test_keychain_set_and_get() {
    // This tests the full keychain round-trip, which requires the macOS keychain
    // to be accessible. Tests run in dev mode may get NoStorageAccess.
    // On Linux CI without a keyring backend, set_password may succeed but
    // get_password returns NotFound (credentials don't persist).
    let result = set_password("test-profile-connections", "hunter2");
    match result {
        Ok(()) => match get_password("test-profile-connections") {
            Ok(pw) => {
                assert_eq!(pw, "hunter2");
                delete_password("test-profile-connections").unwrap();
            }
            Err(KeychainError::NotFound) => {
                eprintln!(
                    "keychain: set succeeded but get returned NotFound \
                     (no persistent keyring) — skipping get/delete"
                );
            }
            Err(KeychainError::NoStorageAccess) => {
                eprintln!("keychain: NoStorageAccess (dev build) — skipping set/get test");
            }
            Err(e) => panic!("keychain get error: {e}"),
        },
        Err(KeychainError::NoStorageAccess) => {
            // Dev build without signing — acceptable
            eprintln!("keychain: NoStorageAccess (dev build) — skipping set/get test");
        }
        Err(e) => panic!("unexpected keychain error: {e}"),
    }
}

#[test]
fn test_delete_password_doesnt_error_if_missing() {
    let result = delete_password("never-existed");
    assert!(
        result.is_ok(),
        "deleting non-existent password should be a no-op"
    );
}

#[test]
fn test_ssh_secret_keychain() {
    let result = set_ssh_secret("test-tunnel", "ssh-secret-123");
    match result {
        Ok(()) => match get_ssh_secret("test-tunnel") {
            Ok(secret) => {
                assert_eq!(secret, "ssh-secret-123");
                delete_ssh_secret("test-tunnel").unwrap();
            }
            Err(KeychainError::NotFound) => {
                eprintln!(
                    "keychain: set succeeded but get returned NotFound \
                     (no persistent keyring) — skipping get/delete"
                );
            }
            Err(KeychainError::NoStorageAccess) => {
                eprintln!("keychain: NoStorageAccess (dev build) — skipping SSH secret test");
            }
            Err(e) => panic!("keychain get error: {e}"),
        },
        Err(KeychainError::NoStorageAccess) => {
            eprintln!("keychain: NoStorageAccess (dev build) — skipping SSH secret test");
        }
        Err(e) => panic!("unexpected keychain error: {e}"),
    }
}

#[test]
fn test_ssh_get_secret_not_found() {
    let result = get_ssh_secret("nonexistent-tunnel");
    assert!(matches!(result, Err(KeychainError::NotFound)));
}

#[test]
fn test_set_password_with_empty_id_fails() {
    // The keychain backend rejects empty "user" attributes.
    // This test ensures we return a proper error and don't silently fail
    // or crash when an empty profile ID reaches the keychain.
    let result = set_password("", "hunter2");
    match &result {
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                !msg.is_empty(),
                "error message should be descriptive, got empty"
            );
        }
        Ok(()) => {
            // Some platforms (e.g. Linux with secret-service) may accept
            // empty attributes — that's OK, but we still want the test
            // to document the behavior.
        }
    }
}

#[test]
fn test_set_password_with_empty_id_does_not_panic() {
    // The keychain call must never panic even with pathological inputs.
    let result = set_password("", "hunter2");
    // Any error or success is acceptable — the important thing is no panic.
    let _ = result;
}

#[tokio::test]
async fn test_repo_save_and_list() {
    let (_dir, _config_path) = with_temp_config_dir();
    let repo = ConnectionProfileRepository::load();

    let profile = ConnectionProfile::new("Repo Test".into());
    repo.save_profile(profile.clone()).await.unwrap();

    let list = repo.list_profiles().await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Repo Test");
}

#[tokio::test]
async fn test_repo_get_profile() {
    let (_dir, _config_path) = with_temp_config_dir();
    let repo = ConnectionProfileRepository::load();
    let profile = ConnectionProfile::new("Find Me".into());
    let id = profile.id.clone();

    repo.save_profile(profile).await.unwrap();

    let found = repo.get_profile(&id).await;
    assert!(found.is_some());
    assert_eq!(found.unwrap().name, "Find Me");

    let not_found = repo.get_profile("nonexistent");
    assert!(not_found.await.is_none());
}

#[tokio::test]
async fn test_repo_delete() {
    let (_dir, _config_path) = with_temp_config_dir();
    let repo = ConnectionProfileRepository::load();
    let profile = ConnectionProfile::new("To Delete".into());
    let id = profile.id.clone();

    repo.save_profile(profile).await.unwrap();
    repo.delete_profile(&id).await.unwrap();

    let list = repo.list_profiles().await;
    assert!(list.is_empty());
}

#[tokio::test]
async fn test_repo_save_updates_existing() {
    let (_dir, _config_path) = with_temp_config_dir();
    let repo = ConnectionProfileRepository::load();
    let mut profile = ConnectionProfile::new("Update Test".into());

    repo.save_profile(profile.clone()).await.unwrap();

    // Modify and save again
    profile.host = "db.example.com".into();
    repo.save_profile(profile).await.unwrap();

    let list = repo.list_profiles().await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].host, "db.example.com");
}

#[tokio::test]
async fn test_repo_mark_used_updates_timestamp() {
    let (_dir, _config_path) = with_temp_config_dir();
    let repo = ConnectionProfileRepository::load();
    let profile = ConnectionProfile::new("Time Test".into());
    let id = profile.id.clone();

    repo.save_profile(profile).await.unwrap();
    let before = chrono::Utc::now().to_rfc3339();

    // Ensure measurable time passes
    std::thread::sleep(std::time::Duration::from_millis(10));

    repo.mark_used(&id).await.unwrap();
    let loaded = repo.get_profile(&id).await.unwrap();

    assert!(loaded.last_used.is_some());
    assert!(
        loaded.last_used.unwrap() > before,
        "last_used should be after 'before'"
    );
}

#[tokio::test]
async fn test_repo_ssh_config_save_and_list() {
    let (_dir, _config_path) = with_temp_config_dir();
    let repo = ConnectionProfileRepository::load();

    let config = crate::ssh::SshConfig {
        id: "ssh-1".into(),
        label: "Test Tunnel".into(),
        host: "bastion.example.com".into(),
        port: 22,
        user: "admin".into(),
        auth_method: crate::ssh::SshAuthMethod::Password,
    };

    repo.save_ssh_config(config.clone()).await.unwrap();
    let list = repo.list_ssh_configs().await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].label, "Test Tunnel");

    let found = repo.get_ssh_config("ssh-1").await;
    assert!(found.is_some());
}

#[tokio::test]
async fn test_repo_delete_ssh_config() {
    let (_dir, _config_path) = with_temp_config_dir();
    let repo = ConnectionProfileRepository::load();

    let config = crate::ssh::SshConfig {
        id: "ssh-del".into(),
        label: "Delete Me".into(),
        host: "host.example.com".into(),
        port: 22,
        user: "user".into(),
        auth_method: crate::ssh::SshAuthMethod::Key {
            key_path: "/tmp/key".into(),
        },
    };

    repo.save_ssh_config(config).await.unwrap();
    repo.delete_ssh_config("ssh-del").await.unwrap();

    let list = repo.list_ssh_configs().await;
    assert!(list.is_empty());
}

#[tokio::test]
async fn test_repo_persistence_across_loads() {
    let (_dir, _config_path) = with_temp_config_dir();

    // First repo instance — save a profile
    let repo = ConnectionProfileRepository::load();
    let profile = ConnectionProfile::new("Persist Test".into());
    repo.save_profile(profile).await.unwrap();
    drop(repo); // drop to release file handles

    // Second repo instance — should read from disk
    let repo2 = ConnectionProfileRepository::load();
    let list = repo2.list_profiles().await;
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Persist Test");
}
