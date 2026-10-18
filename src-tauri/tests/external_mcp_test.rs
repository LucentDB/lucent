use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

use lucent_lib::ai::external_endpoint::discovery::read_discovery_file_at;
use lucent_lib::ai::external_endpoint::executor::ExternalEndpointContext;
use lucent_lib::ai::external_endpoint::mirror::InodeTracker;
use lucent_lib::ai::external_endpoint::server::ExternalEndpointManager;
use lucent_lib::client::ConnectorClient;
use lucent_lib::connections::ConnectionProfile;
use lucent_lib::supervisor::{new_log_buffer, Supervisor};
use lucent_protocol::{ConnectionConfig, ConnectionId};

fn test_binary_path(name: &str) -> PathBuf {
    if let Ok(path) = std::env::var(format!("CARGO_BIN_EXE_{name}")) {
        let p = PathBuf::from(path);
        if p.exists() {
            return p;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for rel in ["", "../", "../../"] {
                let candidate = parent.join(rel).join(name);
                if candidate.exists() {
                    return candidate;
                }
            }
        }
    }
    panic!("Could not locate binary {name}");
}

async fn setup_test_endpoint_with_file(
    db_path: &std::path::Path,
    discovery_path: &std::path::Path,
) -> (ExternalEndpointManager, Arc<ExternalEndpointContext>, Supervisor, ConnectorClient) {
    let mut supervisor = Supervisor::for_driver("duckdb", new_log_buffer());
    supervisor
        .ensure_running()
        .await
        .expect("duckdb worker binary must be built");

    let socket = supervisor.endpoint().to_string();
    let token = supervisor.handshake_token().to_string();

    let (client, _initial_cid) = ConnectorClient::connect(
        &socket,
        &token,
        ConnectionConfig::new("duckdb").with("path", ":memory:"),
    )
    .await
    .expect("connect through duckdb worker");

    // Initialize DuckDB file v1 with items (val=1)
    let init_cid = ConnectionId(uuid::Uuid::new_v4());
    client
        .connect_with_id(
            init_cid,
            ConnectionConfig::new("duckdb").with("path", db_path.to_string_lossy().to_string()),
        )
        .await
        .expect("connect init session");
    client
        .execute(
            init_cid,
            "CREATE TABLE items (val INT); INSERT INTO items VALUES (1);",
        )
        .await
        .expect("seed v1 items");
    let _ = client.disconnect_id(init_cid).await;

    // Connect dedicated external AI session with read_only = true and external_access = false
    let ext_ai_cid = ConnectionId(uuid::Uuid::new_v4());
    let server_info = client
        .connect_with_id(
            ext_ai_cid,
            ConnectionConfig::new("duckdb")
                .with("path", db_path.to_string_lossy().to_string())
                .with("read_only", "true")
                .with("external_access", "false"),
        )
        .await
        .expect("connect external AI session");

    ExternalEndpointManager::run_connector_canary(&client, ext_ai_cid)
        .await
        .expect("canary check on initial ext_ai_cid");

    let path_str = db_path.to_string_lossy().to_string();
    let mut params = std::collections::BTreeMap::new();
    params.insert("path".into(), path_str);
    let profile = ConnectionProfile {
        id: "test-duckdb".into(),
        name: "Test DuckDB".into(),
        driver: "duckdb".into(),
        alias: None,
        params,
        ssh_tunnel_id: None,
        group: None,
        color: None,
        icon: None,
        last_used: None,
        created_at: String::new(),
        updated_at: String::new(),
        enable_external_agents: true,
        allow_query_history: false,
    };

    let inode_tracker = InodeTracker::for_path(db_path);
    let context = Arc::new(ExternalEndpointContext {
        client: Arc::new(tokio::sync::Mutex::new(Some(client.clone()))),
        external_ai_connection_id: Arc::new(tokio::sync::Mutex::new(Some(ext_ai_cid))),
        schema_graph: Arc::new(tokio::sync::Mutex::new(None)),
        embedder: Arc::new(tokio::sync::Mutex::new(None)),
        capabilities: Arc::new(tokio::sync::Mutex::new(Some(server_info.capabilities))),
        active_profile: Arc::new(tokio::sync::Mutex::new(Some(profile))),
        inode_tracker: Arc::new(tokio::sync::Mutex::new(Some(inode_tracker))),
        reopen_lock: Arc::new(tokio::sync::Mutex::new(())),
        memory_manager: Arc::new(
            lucent_lib::ai::memory::MemoryManager::open_in_memory().unwrap(),
        ),
    });

    let manager = ExternalEndpointManager::start(context.clone(), Some(discovery_path.to_path_buf()))
        .await
        .expect("start ExternalEndpointManager");

    (manager, context, supervisor, client)
}

#[tokio::test]
async fn external_mcp_auto_discovers_and_handles_rename_swap() {
    let temp_dir = tempfile::tempdir().unwrap();
    let discovery_path = temp_dir.path().join("external-mcp.json");
    let db1_path = temp_dir.path().join("active.duckdb");
    let db2_path = temp_dir.path().join("staged.duckdb");

    // 1 & 2. Start endpoint manager with db1_path
    let (mut manager, _context, mut supervisor, client) =
        setup_test_endpoint_with_file(&db1_path, &discovery_path).await;

    // Verify discovery file written and status is Connected
    let disc = read_discovery_file_at(&discovery_path).unwrap();
    assert_eq!(disc.status, lucent_lib::ai::external_endpoint::discovery::EndpointStatus::Connected);

    // 3. Spawn real compiled lucent-db-tools-mcp binary with LUCENT_EXTERNAL_MCP
    let binary = test_binary_path("lucent-db-tools-mcp");
    let mut child = tokio::process::Command::new(&binary)
        .env("LUCENT_EXTERNAL_MCP", discovery_path.to_str().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn lucent-db-tools-mcp");

    let mut stdin = BufWriter::new(child.stdin.take().unwrap());
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // 4. Initialize
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": { "protocolVersion": "2024-11-05" }
    });
    stdin.write_all(format!("{}\n", init_req).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    let mut line = String::new();
    stdout.read_line(&mut line).await.unwrap();
    let init_res: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(init_res["id"], 1);

    // 5. Tools list: verify advertised tools filtered by descriptor
    let list_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    stdin.write_all(format!("{}\n", list_req).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    line.clear();
    stdout.read_line(&mut line).await.unwrap();
    let list_res: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(list_res["id"], 2);
    let tool_list = list_res["result"]["tools"].as_array().unwrap();
    let tool_names: Vec<&str> = tool_list.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(tool_names.contains(&"run_readonly_query"));
    assert!(tool_names.contains(&"search_schema"));
    assert!(tool_names.contains(&"get_objects_info"));
    assert!(tool_names.contains(&"get_preflight_context"));
    assert!(!tool_names.contains(&"search_query_history"));

    // 6. Query v1: SELECT sum(val) FROM items -> expect 1
    let query_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "run_readonly_query",
            "arguments": { "sql": "SELECT sum(val) as s FROM items" }
        }
    });
    stdin.write_all(format!("{}\n", query_req).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    line.clear();
    stdout.read_line(&mut line).await.unwrap();
    let query_res: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(query_res["id"], 3);
    assert_eq!(query_res["result"]["isError"], false);
    let text = query_res["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("1"), "Expected query result to contain 1, got: {text}");

    // 7. Test sandbox: replacement scan must be blocked
    let exploit_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "run_readonly_query",
            "arguments": { "sql": "SELECT * FROM 'secret.csv'" }
        }
    });
    stdin.write_all(format!("{}\n", exploit_req).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    line.clear();
    stdout.read_line(&mut line).await.unwrap();
    let exploit_res: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(exploit_res["id"], 4);
    assert_eq!(exploit_res["result"]["isError"], true);

    // 8. Atomic rename swap to v2
    {
        let init_cid2 = ConnectionId(uuid::Uuid::new_v4());
        client
            .connect_with_id(
                init_cid2,
                ConnectionConfig::new("duckdb").with("path", db2_path.to_string_lossy().to_string()),
            )
            .await
            .expect("connect init session for v2");
        client
            .execute(
                init_cid2,
                "CREATE TABLE items (val INT); INSERT INTO items VALUES (100);",
            )
            .await
            .expect("seed v2 items");
        let _ = client.disconnect_id(init_cid2).await;
    }
    std::fs::rename(&db2_path, &db1_path).unwrap();

    // 9. Query again: must detect inode change and return 100
    let query_req2 = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "run_readonly_query",
            "arguments": { "sql": "SELECT sum(val) as s FROM items" }
        }
    });
    stdin.write_all(format!("{}\n", query_req2).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    line.clear();
    stdout.read_line(&mut line).await.unwrap();
    let query_res2: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(query_res2["id"], 5);
    assert_eq!(query_res2["result"]["isError"], false);
    let text2 = query_res2["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text2.contains("100"), "Expected query result to contain 100 after swap, got: {text2}");

    // 10. Clean shutdown
    manager.shutdown().await.unwrap();
    supervisor.shutdown().await.unwrap();
    let _ = child.kill().await;
}

#[tokio::test]
async fn external_mcp_dynamic_disconnect_when_profile_toggled_off() {
    let temp_dir = tempfile::tempdir().unwrap();
    let discovery_path = temp_dir.path().join("external-mcp.json");
    let db_path = temp_dir.path().join("toggle.duckdb");

    // 1. Start endpoint manager with db_path
    let (mut manager, context, mut supervisor, _client) =
        setup_test_endpoint_with_file(&db_path, &discovery_path).await;

    // 2. Spawn real compiled `lucent-db-tools-mcp` binary via stdio
    let binary = test_binary_path("lucent-db-tools-mcp");
    let mut child = tokio::process::Command::new(&binary)
        .env("LUCENT_EXTERNAL_MCP", discovery_path.to_str().unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn lucent-db-tools-mcp");

    let mut stdin = BufWriter::new(child.stdin.take().unwrap());
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // 3. Initialize
    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test-harness", "version": "1.0" }
        }
    });
    stdin.write_all(format!("{}\n", init_req).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    let mut line = String::new();
    stdout.read_line(&mut line).await.unwrap();
    let init_res: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(init_res["id"], 1);

    // 4. Query before disconnect: must succeed
    let query_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "run_readonly_query",
            "arguments": { "sql": "SELECT sum(val) as s FROM items" }
        }
    });
    stdin.write_all(format!("{}\n", query_req).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    line.clear();
    stdout.read_line(&mut line).await.unwrap();
    let query_res: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(query_res["id"], 2);
    assert_eq!(query_res["result"]["isError"], false);

    // 5. Dynamically toggle profile flag off
    {
        let mut guard = context.active_profile.lock().await;
        if let Some(ref mut p) = *guard {
            p.enable_external_agents = false;
        }
    }
    manager.sync_profile().await.expect("sync profile disconnected");

    // 6. Query after disconnect: must fail with spec §4.1 message
    let query_req2 = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "run_readonly_query",
            "arguments": { "sql": "SELECT sum(val) as s FROM items" }
        }
    });
    stdin.write_all(format!("{}\n", query_req2).as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    line.clear();
    stdout.read_line(&mut line).await.unwrap();
    let query_res2: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(query_res2["id"], 3);
    assert_eq!(query_res2["result"]["isError"], true);
    let err_text = query_res2["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        err_text.contains("External agent access is disabled for the active connection profile."),
        "expected spec §4.1 disabled message, got: {err_text}"
    );

    // 7. Clean shutdown
    manager.shutdown().await.unwrap();
    supervisor.shutdown().await.unwrap();
    let _ = child.kill().await;
}
