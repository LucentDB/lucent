//! Phase F capstone: the whole ACP tool stack end-to-end against a REAL
//! database connection — the REAL `lucent-db-tools-mcp` MCP binary speaking
//! to the REAL `bridge::serve` loop, with a `ContextToolExecutor` whose
//! `AiToolContext` is bound to a LIVE DuckDB worker connection (in-process
//! engine — no Docker, reusing the supervisor + `ConnectorClient` machinery
//! the integration tests already use).
//!
//! In production the ACP agent spawns the MCP binary itself via
//! `session/new`'s `mcpServers`; here the test plays the agent's role on the
//! binary's stdin/stdout: `tools/list` (schema parity with
//! `LucentToolEnum`), `tools/call` search_schema + run_readonly_query (the
//! structured `ToolResult`/`QueryResult` pair on the sink), and
//! `tools/call` preview_dml (the held approval, resolved exactly as
//! `execute_dml` resolves it — staged SQL on the worker, real affected
//! count — so the agent sees a slow tool call that returns data and the row
//! lands in DuckDB). The bridge must also survive the client dropping.

#![cfg(all(test, feature = "integration-tests"))]

use crate::ai::acp::bridge::{self, BridgeHandle, ContextToolExecutor, DmlOutcome, ToolExecutor};
use crate::ai::acp::mcp_server;
use crate::ai::agent::AgentSink;
use crate::ai::config::AiConfig;
use crate::ai::events::{AiEvent, DmlApprovalPayload};
use crate::ai::tools::AiToolContext;
use crate::client::ConnectorClient;
use crate::supervisor::{new_log_buffer, Supervisor};
use lucent_protocol::ConnectionConfig;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Records every `AiEvent` + `DmlApprovalPayload` the bridge sink receives
/// (same shape as bridge.rs's test sink — the capstone asserts on both).
struct RecordingSink {
    events: Arc<std::sync::Mutex<Vec<AiEvent>>>,
    approvals: Arc<std::sync::Mutex<Vec<DmlApprovalPayload>>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self {
            events: Arc::new(std::sync::Mutex::new(Vec::new())),
            approvals: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }
}

impl AgentSink for RecordingSink {
    fn event(&self, event: AiEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn dml_approval(&self, payload: DmlApprovalPayload) {
        self.approvals.lock().unwrap().push(payload);
    }
}

/// Drives the REAL `lucent-db-tools-mcp` binary over its stdin/stdout the
/// way an ACP agent would: one JSON-RPC 2.0 request per line, one response
/// per request (the binary answers only requests, never notifications).
struct McpDriver {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
    next_id: u64,
}

impl McpDriver {
    fn spawn(binary: &str, socket: &str, token: &str) -> Self {
        let mut child = tokio::process::Command::new(binary)
            .args(["--socket", socket, "--token", token])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn the lucent-db-tools-mcp binary");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        Self {
            child,
            stdin,
            stdout: tokio::io::BufReader::new(stdout),
            next_id: 1,
        }
    }

    /// Sends one request and awaits the matching response envelope.
    async fn request(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        let mut line = serde_json::to_string(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .expect("serialize MCP request");
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .expect("write MCP request");
        self.stdin.flush().await.expect("flush MCP request");

        let mut resp_line = String::new();
        let n = self
            .stdout
            .read_line(&mut resp_line)
            .await
            .expect("read MCP response");
        assert!(n > 0, "MCP binary closed stdout before answering");
        let resp: serde_json::Value =
            serde_json::from_str(resp_line.trim_end()).expect("parse MCP response");
        assert_eq!(resp["id"], serde_json::json!(id), "id echoed: {resp}");
        resp
    }
}

impl Drop for McpDriver {
    fn drop(&mut self) {
        // Kill the binary — the bridge then sees EOF on the socket, the same
        // way an agent's session teardown ends the MCP server connection.
        // `start_kill` is the sync form: tokio's async `kill()`/`wait()`
        // would be dropped unpolled in `Drop` (no-ops). The child exits on
        // stdin EOF anyway; this is belt-and-braces + zombie hygiene.
        let _ = self.child.start_kill();
    }
}

/// The DuckDB worker binary is large (~120 MB debug) — warm it once so the
/// supervisor's 1s readiness window never races the first exec (same helper
/// as src-tauri/tests/duckdb_e2e_test.rs).
fn warm_up_worker_binary() {
    #[cfg(unix)]
    {
        use std::sync::OnceLock;
        static WARMED: OnceLock<()> = OnceLock::new();
        WARMED.get_or_init(|| {
            let name = crate::supervisor::worker_binary_name("duckdb");
            let Some(binary) = (|| {
                if let Ok(path) = std::env::var("LUCENT_WORKER_BINARY_DUCKDB") {
                    return Some(std::path::PathBuf::from(path));
                }
                let exe = std::env::current_exe().ok()?;
                let parent = exe.parent()?;
                for rel in ["", "../", "../../"] {
                    let candidate = parent.join(rel).join(&name);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
                None
            })() else {
                return; // let the supervisor report the real resolution error
            };
            let dir =
                std::env::temp_dir().join(format!("lucent-duckdb-warmup-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok();
            let socket = dir.join("warmup.sock");
            let mut child = match std::process::Command::new(&binary)
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

/// Boots a live DuckDB worker and returns the supervisor, the app-side
/// client, and the connection id (kept alive by the caller).
async fn duckdb_worker() -> (Supervisor, ConnectorClient, lucent_protocol::ConnectionId) {
    tokio::task::spawn_blocking(warm_up_worker_binary)
        .await
        .expect("warm-up task panicked");

    let mut supervisor = Supervisor::for_driver("duckdb", new_log_buffer());
    supervisor.ensure_running().await.expect(
        "the duckdb worker binary must be built — run `cargo build --bin lucent-driver-duckdb`",
    );
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
async fn capstone_tool_roundtrip_and_dml_approval() {
    // 1. Live DuckDB worker + a seeded table.
    let (mut supervisor, mut client, conn_id) = duckdb_worker().await;
    client
        .execute(
            conn_id,
            "CREATE TABLE t (id BIGINT PRIMARY KEY, name VARCHAR)",
        )
        .await
        .expect("create table");
    client
        .execute(conn_id, "INSERT INTO t VALUES (1, 'a'), (2, 'b')")
        .await
        .expect("seed rows");

    // 2. AiToolContext bound to the live connection + the real tool adapter.
    let capabilities = client.server_info.as_ref().map(|s| s.capabilities.clone());
    let ctx = AiToolContext {
        db: Arc::new(Mutex::new(Some(client.clone()))),
        connection_id: Some(conn_id),
        capabilities,
        config: AiConfig::default(),
        schema_graph: Arc::new(Mutex::new(None)),
        embedder: Arc::new(Mutex::new(None)),
        reranker: Arc::new(Mutex::new(None)),
    };
    let schemas = mcp_server::lucent_tools_schema(ctx.clone());
    let executor: Arc<dyn ToolExecutor> = Arc::new(ContextToolExecutor::new(ctx));
    let sink = Arc::new(RecordingSink::new());
    let handle = Arc::new(BridgeHandle::new("conv-1"));

    // 3. The bridge listener (tempdir socket + 64-hex token, matching
    //    production `create_bridge_endpoint`) and the real serve loop.
    let dir = tempfile::tempdir().expect("tempdir for the bridge socket");
    let bridge_path = dir.path().join("bridge.sock");
    let listener = tokio::net::UnixListener::bind(&bridge_path).expect("bind bridge socket");
    let bridge_token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let serve_task = tokio::spawn(bridge::serve(
        listener,
        bridge_token.clone(),
        executor,
        sink.clone(),
        handle.clone(),
    ));

    // 4. Spawn the REAL MCP binary (in production the ACP agent does this via
    //    session/new's mcpServers; here the test plays the agent).
    let binary = crate::ai::acp::bridge_binary_path()
        .expect("lucent-db-tools-mcp resolves next to the test");
    let mut mcp = McpDriver::spawn(&binary, &bridge_path.to_string_lossy(), &bridge_token);

    // 5. tools/list — the four tools, schemas identical to LucentToolEnum.
    let resp = mcp.request("tools/list", serde_json::json!({})).await;
    let listed = resp["result"]["tools"].as_array().expect("tools array");
    assert_eq!(listed.len(), schemas.len(), "four tools: {resp}");
    for (got, want) in listed.iter().zip(schemas.iter()) {
        assert_eq!(got["name"], serde_json::json!(want.name), "tool name");
        assert_eq!(
            got["description"],
            serde_json::json!(want.description),
            "description for {}",
            want.name
        );
        assert_eq!(
            got["inputSchema"], want.input_schema,
            "input schema for {} is byte-identical to the rig path",
            want.name
        );
    }

    // 6. tools/call search_schema — a text tool round-trip (keyword mode
    //    falls back cleanly without a schema graph; the catalog seam answers
    //    through the worker).
    let resp = mcp
        .request(
            "tools/call",
            serde_json::json!({"name": "search_schema", "arguments": {"query": "t"}}),
        )
        .await;
    assert!(
        !resp["result"]["isError"].as_bool().unwrap_or(true),
        "search_schema must succeed: {resp}"
    );
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(!text.is_empty(), "search_schema returned a summary");

    // 7. tools/call run_readonly_query — the structured UI path: the sink
    //    receives ToolResult (output.type == "query_result") + QueryResult,
    //    while the agent's own result carries only the text summary.
    let resp = mcp
        .request(
            "tools/call",
            serde_json::json!({
                "name": "run_readonly_query",
                "arguments": { "sql": "SELECT count(*) AS n FROM t" }
            }),
        )
        .await;
    assert!(
        !resp["result"]["isError"].as_bool().unwrap_or(true),
        "run_readonly_query must succeed: {resp}"
    );
    assert!(
        resp["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|t| t.contains("2")),
        "agent text summary mentions the count: {resp}"
    );
    let events = sink.events.lock().unwrap().clone();
    let tool_result = events
        .iter()
        .find_map(|e| match e {
            AiEvent::ToolResult {
                output: Some(output),
                ..
            } => Some(output.clone()),
            _ => None,
        })
        .expect("ToolResult with structured output reached the sink");
    assert_eq!(tool_result["type"], "query_result");
    // `row_count` is the number of RETURNED rows (one for count(*)); the
    // count itself is the first cell of the first row.
    assert_eq!(tool_result["row_count"], 1, "one result row: {tool_result}");
    assert_eq!(
        tool_result["rows"][0][0],
        serde_json::json!(2),
        "seed count"
    );
    assert_eq!(tool_result["sql"], "SELECT count(*) AS n FROM t");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AiEvent::QueryResult { row_count: 1, .. })),
        "QueryResult event reached the sink: {events:?}"
    );

    // 8. tools/call preview_dml — the held approval. The bridge parks the
    //    call and surfaces the approval payload; the test resolves it exactly
    //    as execute_dml does (staged SQL on the worker, real affected count),
    //    so the agent sees a slow tool call that returns data. The pinned
    //    request future borrows `mcp`, so this whole section is scoped — the
    //    borrow ends before the driver is dropped below.
    let dml_sql = "INSERT INTO t VALUES (3, 'c')";
    let (resp, rows_affected) = {
        let dml_call = mcp.request(
            "tools/call",
            serde_json::json!({"name": "preview_dml", "arguments": { "sql": dml_sql }}),
        );
        tokio::pin!(dml_call);

        // 8a. The approval payload surfaces while the MCP call stays open.
        let approval = {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
            loop {
                tokio::select! {
                    resp = &mut dml_call => {
                        panic!("preview_dml answered without surfacing an approval: {resp}");
                    }
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {
                        if let Some(p) = sink.approvals.lock().unwrap().first().cloned() {
                            break p;
                        }
                        assert!(
                            tokio::time::Instant::now() < deadline,
                            "dml_approval never surfaced"
                        );
                    }
                }
            }
        };
        assert_eq!(approval.conversation_id, "conv-1");
        assert_eq!(approval.sql, dml_sql);
        assert!(approval.tables_affected.contains(&"t".to_string()));

        // 8b. Act as execute_dml: take the slot, run the staged SQL through
        //     the real worker, resolve the oneshot with the REAL affected
        //     count (take() — not a peek — so a second approval can never
        //     double-send). The DuckDB driver reports DML as an empty
        //     tabular result (`rows_affected` is a Postgres-only field), so
        //     the affected count comes from the row delta — the number a
        //     real approval would surface.
        let pending = handle
            .pending_dml
            .lock()
            .await
            .take()
            .expect("pending DML registered for the held call");
        let pre = client
            .execute(conn_id, "SELECT count(*) AS n FROM t")
            .await
            .expect("count before DML")
            .rows[0][0]
            .as_i64()
            .expect("count cell");
        client
            .execute(conn_id, &pending.sql)
            .await
            .expect("staged DML executes on the worker");
        let post = client
            .execute(conn_id, "SELECT count(*) AS n FROM t")
            .await
            .expect("count after DML")
            .rows[0][0]
            .as_i64()
            .expect("count cell");
        let rows_affected = (post - pre) as u64;
        pending
            .tx
            .send(Ok(DmlOutcome { rows_affected }))
            .expect("resolve the held preview_dml call");

        // 8c. The held MCP call completes with the execution summary.
        let resp = tokio::time::timeout(Duration::from_secs(10), &mut dml_call)
            .await
            .expect("the held call completes after approval");
        (resp, rows_affected)
    };
    assert!(
        !resp["result"]["isError"].as_bool().unwrap_or(true),
        "approved DML must not be an error: {resp}"
    );
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("dml text content");
    assert!(
        text.contains(&format!("{rows_affected} rows affected")),
        "execution summary: {text}"
    );

    // 8d. The row actually landed in DuckDB (the approval gate executes in
    //     Lucent — the agent cannot execute DML on its own).
    let check = client
        .execute(conn_id, "SELECT count(*) AS n FROM t")
        .await
        .expect("verify the row landed");
    assert_eq!(
        check.rows[0][0],
        serde_json::json!(3),
        "row landed in DuckDB"
    );

    // 9. The bridge survives the client dropping: EOF on the socket ends the
    //    serve loop cleanly (the binary exits when its stdin closes).
    drop(mcp);
    tokio::time::timeout(Duration::from_secs(10), serve_task)
        .await
        .expect("serve loop finishes after the client drops")
        .expect("serve task did not panic")
        .expect("serve returns Ok on EOF");

    client.shutdown().await.expect("client shutdown");
    let _ = supervisor.shutdown().await;
}

/// Locates the compiled stub-agent binary (same walk as the other acp test
/// helpers; `CARGO_BIN_EXE_*` is only set for integration tests).
fn stub_binary() -> String {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_lucent-acp-stub-agent") {
        return p;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(target_dir) = exe.parent().and_then(|p| p.parent()) {
            let candidate = target_dir.join("lucent-acp-stub-agent");
            if candidate.exists() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    panic!(
        "lucent-acp-stub-agent binary not found — run `cargo build --bin lucent-acp-stub-agent` first"
    );
}

#[tokio::test]
async fn agent_spawning_mcp_binary_marks_bridge_connected() {
    // The FULL agent → MCP → bridge path: the stub agent plays a real
    // agent's MCP-client role and spawns the REAL `lucent-db-tools-mcp`
    // binary from session/new's `mcpServers` (exactly what opencode / the
    // claude-acp adapter do). The binary connects to the bridge socket on
    // startup, so `session_for`'s connect signal must fire — the ground
    // truth behind the honest-preamble logic in the driver.
    let _ws = tempfile::tempdir().unwrap();
    std::env::set_var(
        "LUCENT_ACP_WORKSPACE",
        _ws.path().to_string_lossy().into_owned(),
    );

    let mut env = std::collections::HashMap::new();
    env.insert("STUB_SPAWN_MCP".to_string(), "1".to_string());
    let process = Arc::new(crate::ai::acp::manager::AgentProcess {
        agent_id: "stub-mcp".into(),
        launch: crate::ai::acp::install::LaunchSpec {
            cmd: stub_binary(),
            args: vec![],
            env,
        },
        stderr_tail: Arc::new(std::sync::Mutex::new(String::new())),
        spawns: Arc::new(std::sync::Mutex::new(Vec::new())),
    });
    let sink: Arc<dyn crate::ai::agent::AgentSink> = Arc::new(crate::ai::agent::CollectorSink(
        std::sync::Mutex::new(Vec::new()),
    ));
    let ctx = crate::ai::tools::AiToolContext {
        db: Arc::new(Mutex::new(None)),
        connection_id: None,
        capabilities: None,
        config: AiConfig::default(),
        schema_graph: Arc::new(Mutex::new(None)),
        embedder: Arc::new(Mutex::new(None)),
        reranker: Arc::new(Mutex::new(None)),
    };

    let acp = crate::ai::acp::AcpState::new();
    let session = acp
        .session_for("conv-connect", &process, &ctx, &sink)
        .await
        .expect("session/new round-trips; the stub spawns the MCP binary");

    assert!(
        session.tools.wait_connected(Duration::from_secs(10)).await,
        "the agent-spawned MCP binary must connect the bridge — stderr: {}",
        process.stderr_snippet()
    );

    acp.drop_session("conv-connect").await;
}
