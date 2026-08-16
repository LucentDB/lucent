//! Spawns the real `lucent-driver-duckdb` binary over a real Unix socket, the
//! same way `binary_e2e_test` does for Postgres. This is the capstone proving
//! two drivers actually coexist.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use lucent_lib::client::ConnectorClient;
use lucent_lib::supervisor::{new_log_buffer, Supervisor};
use lucent_protocol::{ConnectionConfig, ObjectKind};

/// The duckdb worker binary is large — a debug build statically links
/// libduckdb and comes in around 120 MB. Under a full `cargo test --workspace`
/// run, the other suites (notably the duckdb driver's own tests) exec several
/// more 100 MB+ binaries and hold live databases, so the first exec of this
/// binary can take multiple seconds to fault in while the supervisor's 1s
/// readiness window is already ticking. Warm the binary once: spawn it
/// directly with no time limit and wait for its socket, then kill it. The
/// supervisor's own spawn then starts from resident pages and binds in
/// milliseconds.
fn warm_up_worker_binary() {
    #[cfg(unix)]
    {
        static WARMED: OnceLock<()> = OnceLock::new();
        WARMED.get_or_init(|| {
            let Some(binary) = worker_binary_path() else {
                return; // Let the supervisor report the real resolution error.
            };
            let dir =
                std::env::temp_dir().join(format!("lucent-duckdb-warmup-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok();
            let socket = dir.join("warmup.sock");
            let mut child = match Command::new(&binary)
                .arg(&socket)
                .arg("warmup")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(_) => return,
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while !socket.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&dir);
        });
    }
}

/// Resolve the duckdb worker binary the same way the supervisor does:
/// per-driver env override first, then next to the test binary.
fn worker_binary_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("LUCENT_WORKER_BINARY_DUCKDB") {
        return Some(PathBuf::from(path));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for rel in ["", "../", "../../"] {
                let candidate = parent.join(rel).join("lucent-driver-duckdb");
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

async fn connected() -> (Supervisor, ConnectorClient, lucent_protocol::ConnectionId) {
    // Blocking spawn + poll; keep it off the test runtime.
    tokio::task::spawn_blocking(warm_up_worker_binary)
        .await
        .expect("warm-up task panicked");

    let mut supervisor = Supervisor::for_driver("duckdb", new_log_buffer());
    supervisor
        .ensure_running()
        .await
        .expect("the duckdb worker binary must be built — run `cargo build --workspace`");
    let socket = supervisor.endpoint().to_string();
    let token = supervisor.handshake_token().to_string();

    let (client, cid) = ConnectorClient::connect(
        &socket,
        &token,
        ConnectionConfig::new("duckdb").with("path", ":memory:"),
    )
    .await
    .expect("connect through the worker");

    (supervisor, client, cid)
}

#[tokio::test]
async fn the_duckdb_worker_binary_answers_queries_and_catalog_requests() {
    let (mut supervisor, client, cid) = connected().await;

    client
        .execute(cid, "CREATE TABLE t (id BIGINT PRIMARY KEY, name VARCHAR)")
        .await
        .expect("create");
    client
        .execute(cid, "INSERT INTO t VALUES (1, 'a'), (2, 'b')")
        .await
        .expect("insert");

    let result = client
        .execute(cid, "SELECT id, name FROM t ORDER BY id")
        .await
        .unwrap();
    assert_eq!(result.row_count, 2);
    // Typed all the way through Plan A's JSON mapping: an integer, not a string.
    assert_eq!(result.rows[0][0], serde_json::json!(1));
    assert_eq!(result.rows[0][1], serde_json::json!("a"));

    let namespaces = client.list_namespaces(cid).await.expect("namespaces");
    assert!(!namespaces.is_empty());

    let objects = client
        .list_all_objects(cid, vec![ObjectKind::Table])
        .await
        .expect("objects");
    let t = objects
        .iter()
        .find(|o| o.reference.name == "t")
        .expect("table t");

    let details = client
        .describe_objects(cid, vec![t.reference.clone()])
        .await
        .expect("describe");
    assert_eq!(details[0].columns.len(), 2);
    assert!(details[0]
        .columns
        .iter()
        .any(|c| c.name == "id" && c.is_primary_key));

    supervisor.shutdown().await.ok();
}

#[tokio::test]
async fn the_duckdb_worker_declares_no_engine_enforced_read_only() {
    // The fact Plan C's disclosure path exists for. If this ever flips, the
    // badge, prompt, and log messages all need revisiting.
    let (mut supervisor, client, _cid) = connected().await;
    let caps = client
        .server_info
        .as_ref()
        .map(|s| s.capabilities.clone())
        .expect("capabilities on connect");

    assert_eq!(caps.id, "duckdb");
    assert!(!caps.readonly.is_engine_enforced());
    assert!(caps.readonly.disclosure().is_some());

    supervisor.shutdown().await.ok();
}
