//! Phase F real-agent smoke test (IGNORED): drives the FULL ACP stack against
//! a REAL coding agent — the same path production uses. The agent is spawned
//! by the connection task (`opencode acp`), Lucent's four DB tools ride in
//! through `session/new`'s `mcpServers` (the real `lucent-db-tools-mcp`
//! binary, spawned by the agent itself), and the bridge executes them
//! in-process against a live DuckDB worker connection — all guardrails
//! (read-only guard, row caps, DML approval) run inside Lucent.
//!
//! Two probes:
//! 1. A RAW `initialize` handshake (the same wire the connection task
//!    speaks) whose response is (a) printed for the compat matrix (spec §9
//!    Q2: which agents advertise what MCP capabilities) and (b) fed through
//!    the production capability gate (`connection::check_mcp_gate`) — an
//!    agent that fails the gate cannot use Lucent's tools.
//! 2. A REAL chat turn through `AcpChatDriver`: the agent is asked to run
//!    `SELECT 1` with the `run_readonly_query` tool; the test asserts the
//!    agent-side `tool_call_update` completed, the bridge's structured
//!    query-result event reached the sink (proof the tool executed inside
//!    Lucent), `Done` with a final message, and the conversation claim
//!    released.
//!
//! Requires: `opencode` installed and logged in (`opencode auth login`), the
//! workspace binaries built (`cargo build --bin lucent-db-tools-mcp --bin
//! lucent-driver-duckdb`), and a working LLM credential for the agent.
//!
//! Run: `cargo test -p lucent --features integration-tests -- --ignored
//! ai::acp::real_agent --nocapture`
//!
//! Agent binary override: `LUCENT_ACP_SMOKE_AGENT=<path>` (default: resolve
//! `opencode` from PATH).

#![cfg(all(test, feature = "integration-tests"))]

use crate::ai::acp::connection::{check_mcp_gate, AgentCommand};
use crate::ai::acp::driver::AcpChatDriver;
use crate::ai::acp::AcpState;
use crate::ai::agent::{AgentState, CollectorSink, ConversationState};
use crate::ai::config::{AcpAgentConfig, AiConfig};
use crate::ai::events::AiEvent;
use crate::ai::tools::AiToolContext;
use crate::client::ConnectorClient;
use crate::supervisor::{new_log_buffer, Supervisor};
use lucent_protocol::ConnectionConfig;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Mutex;

/// Resolves the agent launch command: `LUCENT_ACP_SMOKE_AGENT` env override
/// (path to the agent binary) first, else `opencode` resolved from PATH —
/// always with the `acp` subcommand (and `--print-logs` so the stderr tail
/// carries opencode's own diagnostics). `None` when no agent binary is
/// available at all — the test skips with instructions (the `#[ignore]`
/// attribute already gates it; the skip covers machines without opencode).
fn agent_command() -> Option<String> {
    if let Ok(p) = std::env::var("LUCENT_ACP_SMOKE_AGENT") {
        let p = p.trim().to_string();
        if !p.is_empty() {
            return Some(format!("{p} acp --print-logs"));
        }
    }
    // PATH resolution probe: `opencode --version` succeeds iff the
    // binary is reachable.
    match std::process::Command::new("opencode")
        .arg("--version")
        .output()
    {
        Ok(out) if out.status.success() => Some("opencode acp --print-logs".to_string()),
        _ => None,
    }
}

/// Splits a whitespace-separated command string into binary + args (the same
/// split the command override uses in `AcpManager::ensure_process` — no
/// quoting in v1, documented limitation).
fn split_cmd(cmd: &str) -> (String, Vec<String>) {
    let mut parts = cmd.split_whitespace();
    let bin = parts.next().unwrap_or(cmd).to_string();
    (bin, parts.map(|s| s.to_string()).collect())
}

/// Raw `initialize` probe: speaks one JSON-RPC initialize directly to the
/// agent (the same wire the connection task speaks — v1 shape with
/// protocolVersion + clientInfo) and returns the raw response line. The
/// probe process is killed right after the response (the driver flow spawns
/// its own agent below).
async fn probe_initialize(agent_cmd: &str) -> String {
    let (bin, args) = split_cmd(agent_cmd);
    // The probe process must never outlive this function: it inherits the
    // test's stderr, so a survivor holds the harness's output pipe open and
    // wedges the run (observed twice in the interrupted F4 attempt). The
    // guard is installed immediately after spawn (before any I/O, so a
    // panic anywhere below still kills the child) and covers EVERY exit
    // path — `start_kill` is sync (tokio's `Child::kill()` is async and a
    // dropped-unpolled future is a no-op, which is exactly the original
    // leak).
    struct KillOnDrop(tokio::process::Child);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.start_kill();
        }
    }
    let child = tokio::process::Command::new(&bin)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn the agent for the capability probe");
    let mut child = KillOnDrop(child);
    let mut stdin = child.0.stdin.take().expect("piped stdin");
    let stdout = child.0.stdout.take().expect("piped stdout");
    let mut reader = tokio::io::BufReader::new(stdout);

    let mut line = serde_json::to_string(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": { "name": "lucent-smoke", "version": "0" }
        }
    }))
    .expect("serialize initialize");
    line.push('\n');
    // The write is under the same 60s bound as the read — an agent that
    // never drains its stdin must not hang the probe.
    tokio::time::timeout(Duration::from_secs(60), stdin.write_all(line.as_bytes()))
        .await
        .expect("write initialize within 60s")
        .expect("write initialize");
    tokio::time::timeout(Duration::from_secs(60), stdin.flush())
        .await
        .expect("flush initialize within 60s")
        .expect("flush initialize");

    let mut resp = String::new();
    let n = tokio::time::timeout(Duration::from_secs(60), reader.read_line(&mut resp))
        .await
        .expect("agent answered initialize within 60s")
        .expect("read initialize response");
    assert!(n > 0, "agent closed stdout before answering initialize");

    // Happy path: kill (sync) + reap explicitly, so no zombie lingers.
    let _ = child.0.start_kill();
    let _ = child.0.wait().await;
    drop(stdin);

    resp.trim_end().to_string()
}

/// The DuckDB worker binary is large (~120 MB debug) — warm it once so the
/// supervisor's 1s readiness window never races the first exec (same helper
/// as the F1 capstone / tests/duckdb_e2e_test.rs).
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
/// client, and the connection id (kept alive by the caller) — the executor
/// the bridge runs the agent's tool calls against.
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
#[ignore = "needs a live opencode agent install and login"]
async fn real_agent_smoke_opencode() {
    // ── Agent availability ────────────────────────────────────────────────
    let Some(agent_cmd) = agent_command() else {
        eprintln!(
            "SKIP: no agent binary found. Install opencode (or set \
             LUCENT_ACP_SMOKE_AGENT=<path>) and log in with `opencode auth login`, \
             then run: cargo test -p lucent --features integration-tests -- \
             --ignored ai::acp::real_agent --nocapture"
        );
        return;
    };
    eprintln!("SMOKE agent command: {agent_cmd}");

    // ── Probe 1: raw initialize — the compat-matrix record (spec §9 Q2) ──
    let init_line = probe_initialize(&agent_cmd).await;
    eprintln!("SMOKE initialize response:\n{init_line}");
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&init_line) {
        eprintln!(
            "SMOKE advertised capabilities: {}",
            serde_json::to_string_pretty(&v["result"]).unwrap_or_else(|_| "(unparseable)".into())
        );
    }

    // The production MCP gate (connection task) must accept this agent —
    // without MCP the agent cannot use Lucent's tools.
    if let Err(e) = check_mcp_gate(&init_line) {
        panic!("agent failed the MCP capability gate: {e}\ninitialize response: {init_line}");
    }
    eprintln!("SMOKE MCP capability gate: PASS");

    // ── Probe 2: a real chat turn through the full stack ─────────────────
    // Hermetic sandbox: the agent's session cwd lives in a tempdir, never
    // the user's project (spec §3 D7).
    let _ws = tempfile::tempdir().expect("tempdir for the agent workspace");
    std::env::set_var(
        "LUCENT_ACP_WORKSPACE",
        _ws.path().to_string_lossy().into_owned(),
    );

    // Live DuckDB worker + AiToolContext bound to it — the bridge's
    // executor runs the agent's tool calls against this connection.
    let (mut supervisor, mut client, conn_id) = duckdb_worker().await;
    let capabilities = client.server_info.as_ref().map(|s| s.capabilities.clone());
    let tool_ctx = AiToolContext {
        db: Arc::new(Mutex::new(Some(client.clone()))),
        connection_id: Some(conn_id),
        capabilities,
        config: AiConfig::default(),
        schema_graph: Arc::new(Mutex::new(None)),
        embedder: Arc::new(Mutex::new(None)),
        reranker: Arc::new(Mutex::new(None)),
    };

    let acp_cfg = AcpAgentConfig {
        agent_id: "opencode".into(),
        command: Some(agent_cmd),
        env: HashMap::new(),
        auto_deny_permissions: false,
    };
    let acp_state = AcpState::new();
    let driver = AcpChatDriver::new(acp_state.clone(), acp_cfg, tool_ctx);
    let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
    let conv = Arc::new(Mutex::new(ConversationState::new("conv-smoke".into())));

    // The driver's agent (spawned by the crate inside the connection task)
    // must not outlive the test. `#[tokio::test]` runs on a current-thread
    // runtime: once the test future panics, the runtime drops tasks WITHOUT
    // polling them, so a Shutdown message queued at that point would never
    // be processed and the crate's process-group kill (which fires on
    // connection close) would never run — the agent would survive, holding
    // the inherited stderr pipe. Therefore the teardown runs HERE, before
    // any assertion/panic: every assertion input (events, final message,
    // conversation state) is final once chat() returns, and the agent is
    // reaped deterministically before the first failure point.
    let cancel = tokio_util::sync::CancellationToken::new();
    let ai_config = AiConfig::default();
    let mut chat_fut = Box::pin(driver.chat(
        "Use the run_readonly_query tool to run: SELECT 1".into(),
        &ai_config,
        "system preamble".into(),
        conv.clone(),
        sink.clone(),
        cancel.clone(),
    ));
    // A real agent turn takes 10–90 s (LLM + tool round-trip + bridge).
    let outcome = match tokio::time::timeout(Duration::from_secs(180), &mut chat_fut).await {
        Ok(res) => res,
        Err(_elapsed) => {
            // The turn outlived the budget: ask the driver to cancel it
            // (session/cancel), then give the chat future a bounded window
            // to finish so the connection task returns to its command loop
            // and can process the Shutdown below. The teardown still runs
            // before the panic.
            eprintln!("SMOKE turn timed out after 180s — cancelling the agent");
            cancel.cancel();
            let _ = tokio::time::timeout(Duration::from_secs(20), &mut chat_fut).await;
            Err("real-agent turn timed out after 180s".to_string())
        }
    };

    // ── Teardown (BEFORE any assertion or panic — see above) ────────────
    if let Some(entry) = acp_state.connections.lock().await.get("opencode").cloned() {
        let _ = entry.cmds.try_send(AgentCommand::Shutdown);
        // Bounded poll, not a heuristic sleep: the JoinHandle resolves when
        // the connection task ends, and the task's end IS the crate's
        // process-group kill (the client drops on loop break). Deterministic
        // reap, up to 5s.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if entry.task.is_finished() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                eprintln!("SMOKE WARN: agent connection task still running 5s after Shutdown");
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    // Only now may the turn outcome be unwrapped / assertions run — the
    // agent is already reaped on every path above (except a genuinely hung
    // agent that ignored the cancel AND Shutdown, which is a residual risk
    // documented in the report).
    outcome.expect("real-agent chat succeeded");

    let events = sink.0.lock().unwrap().clone();
    eprintln!("SMOKE events: {events:?}");

    // The agent called the tool: its completed `tool_call_update` maps to a
    // `ToolResult` whose (title-enriched) name carries the tool.
    let agent_side = events.iter().any(|e| {
        matches!(e, AiEvent::ToolResult { output: None, tool, .. }
            if tool.contains("run_readonly_query"))
    });
    assert!(
        agent_side,
        "the agent's completed tool_call_update for run_readonly_query reached the sink: {events:?}"
    );

    // The tool executed INSIDE Lucent: the bridge emitted the structured
    // query-result event (the interactive grid path) — proof the agent's MCP
    // call crossed the socket into the main process and ran the real tool
    // against DuckDB.
    let structured = events.iter().any(|e| {
        matches!(e, AiEvent::ToolResult { output: Some(out), .. }
            if out["type"] == "query_result")
    });
    assert!(
        structured,
        "the bridge's structured query_result event reached the sink: {events:?}"
    );

    // The turn ended with a final message (stop_reason end_turn).
    let done = events.iter().find_map(|e| match e {
        AiEvent::Done { final_message, .. } => Some(final_message.clone()),
        _ => None,
    });
    let final_message = done.expect("Done event present after the turn");
    assert!(
        !final_message.trim().is_empty(),
        "the agent produced a final message"
    );
    eprintln!("SMOKE final message: {final_message:?}");

    // The conversation claim was released — follow-up messages can begin.
    assert!(
        matches!(conv.lock().await.state, AgentState::Idle),
        "conversation returns to Idle after the turn"
    );

    client.shutdown().await.expect("client shutdown");
    let _ = supervisor.shutdown().await;
    eprintln!("SMOKE PASS — full round-trip OK");
}
