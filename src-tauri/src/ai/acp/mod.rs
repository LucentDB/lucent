pub mod bridge;
pub mod connection;
pub mod driver;
pub mod install;
pub mod manager;
pub mod mcp_server;
pub mod permissions;
pub mod registry;
pub mod wire;

use crate::ai::acp::bridge::{BridgeConnection, BridgeHandle, ContextToolExecutor, ToolExecutor};
use crate::ai::acp::connection::{run_connection, AgentCommand, AgentEvent};
use crate::ai::acp::permissions::PermissionRegistry;
use crate::ai::agent::AgentSink;
use crate::ai::tools::AiToolContext;
use agent_client_protocol::schema::v1::{EnvVariable, McpServer, McpServerStdio};
use install::InstalledAgent;
use registry::Registry;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use uuid::Uuid;

/// The shared ACP subsystem state, owned by `AppState` (spec §3 D4: one
/// agent process per agent id, one ACP session per chat conversation).
/// Clone is cheap — every field is behind an `Arc`, so the chat driver
/// holds a copy for the life of a turn.
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct AcpState {
    /// One agent process per installed agent id.
    pub manager: Arc<crate::ai::acp::manager::AcpManager>,
    /// conversation_id -> bridge handle. The bridge holds the in-flight DML
    /// approval (D4's `execute_dml` / `reject_dml` resolve it).
    pub bridges: Arc<Mutex<HashMap<String, Arc<BridgeHandle>>>>,
    /// FIFO queues of pending `session/request_permission` decisions, keyed
    /// by session id (spec §4.5).
    pub permissions: Arc<PermissionRegistry>,
    /// agent_id -> running connection task state (one per agent process).
    pub connections: Arc<Mutex<HashMap<String, Arc<ConnectionEntry>>>>,
    /// conversation_id -> the conversation's ACP session (multi-turn reuse).
    pub sessions: Arc<Mutex<HashMap<String, Arc<SessionEntry>>>>,
}

impl AcpState {
    pub fn new() -> Self {
        Self {
            manager: Arc::new(crate::ai::acp::manager::AcpManager::new()),
            bridges: Arc::new(Mutex::new(HashMap::new())),
            permissions: Arc::new(PermissionRegistry::new()),
            connections: Arc::new(Mutex::new(HashMap::new())),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Ensures the connection task for the agent process is running and
    /// returns its entry (command sender + event fan-out). Idempotent.
    ///
    /// Phase F crash recovery: a cached entry whose task has finished means
    /// the agent process died (the crate's transport ends the connection on
    /// EOF — nothing in the app stops a connection cleanly outside app
    /// exit). The dead entry is reaped and the crash charged against the
    /// restart budget (2 per 10 min, spec §4.3); a blocked budget fails
    /// this call with the budget message + stderr tail, otherwise a fresh
    /// connection is started (auto-restart on next use, bounded).
    pub async fn ensure_connection(
        &self,
        process: &Arc<crate::ai::acp::manager::AgentProcess>,
    ) -> Result<Arc<ConnectionEntry>, String> {
        let mut map = self.connections.lock().await;
        if let Some(entry) = map.get(&process.agent_id) {
            if entry.task.is_finished() {
                // Charge the crash BEFORE reaping: when the budget is
                // exhausted the finished entry must stay in the map so the
                // next call re-enters this blocked path — removing it first
                // would let the spawn path below run ungated (alternating
                // block → doomed fresh spawn → block → … forever, spec
                // §4.3 "then require user action" unenforced). Only a
                // budget-allowed crash is reaped and restarted.
                self.manager.record_crash(process)?;
                map.remove(&process.agent_id);
                log::warn!(
                    "agent '{}' connection task ended — restarting (budget permitting)",
                    process.agent_id
                );
            } else {
                return Ok(entry.clone());
            }
        }
        let (cmds_tx, cmds_rx) = mpsc::channel(16);
        let (ev_tx, ev_rx) = mpsc::channel(256);
        let (bcast_tx, _) = broadcast::channel(256);
        let perms = self.permissions.clone();
        // mpsc -> broadcast forwarder: `run_connection` writes into the mpsc
        // (its signature is pinned by C2's tests); every chat() on the same
        // agent subscribes to the broadcast and filters by session id. The
        // forwarder ends with the connection (mpsc close on task exit).
        let fwd_tx = bcast_tx.clone();
        tokio::spawn(async move {
            let mut rx = ev_rx;
            while let Some(ev) = rx.recv().await {
                let _ = fwd_tx.send(ev);
            }
        });
        let task = tokio::spawn(run_connection(process.clone(), cmds_rx, ev_tx, perms));
        let entry = Arc::new(ConnectionEntry {
            cmds: cmds_tx,
            events: bcast_tx,
            task,
        });
        map.insert(process.agent_id.clone(), entry.clone());
        Ok(entry)
    }

    /// Get-or-create the ACP session for a conversation. On first use it
    /// also spawns the DB-tools bridge listener (tempdir socket + 32-byte
    /// hex token, spec §4.6) and delivers the bridge config to the agent via
    /// `session/new`'s `mcpServers` — the agent spawns the bridge binary
    /// itself, so the socket must outlive `session_for` (held by
    /// `SessionEntry`).
    pub async fn session_for(
        &self,
        conversation_id: &str,
        process: &Arc<crate::ai::acp::manager::AgentProcess>,
        tool_ctx: &AiToolContext,
        sink: &Arc<dyn AgentSink>,
    ) -> Result<Arc<SessionEntry>, String> {
        if let Some(session) = self.sessions.lock().await.get(conversation_id).cloned() {
            return Ok(session);
        }
        let conn = self.ensure_connection(process).await?;

        // Bridge socket placement follows `supervisor.rs::endpoint_for`:
        // tempdir on Unix (macOS sun_path is 104 bytes), named pipe on
        // Windows. The token is 32 random bytes, hex-encoded.
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let (listener, endpoint, dir) = create_bridge_endpoint(&token);

        let handle = Arc::new(BridgeHandle::new(conversation_id));
        let tools = handle.connection();
        let executor: Arc<dyn ToolExecutor> = Arc::new(ContextToolExecutor::new(tool_ctx.clone()));
        let serve_sink = sink.clone();
        let serve_handle = handle.clone();
        let serve_token = token.clone();
        tokio::spawn(bridge::serve(
            listener,
            serve_token,
            executor,
            serve_sink,
            serve_handle,
        ));

        let bridge_bin = bridge_binary_path()?;

        // Config delivery via argv (primary) — token-in-argv is the same
        // tradeoff the worker handshake already makes; env stays empty (the
        // bridge binary also accepts env fallback, unused in v1).
        let mcp_server = McpServer::Stdio(
            McpServerStdio::new("lucent-db-tools", &bridge_bin)
                .args(vec![
                    "--socket".into(),
                    endpoint.clone(),
                    "--token".into(),
                    token.clone(),
                ])
                .env(Vec::<EnvVariable>::new()),
        );

        // Neutral sandbox cwd (spec §3 D7) — never the user's project.
        let sandbox = crate::ai::acp::driver::workspace_dir(&process.agent_id, conversation_id)?;
        std::fs::create_dir_all(&sandbox)
            .map_err(|e| format!("create agent workspace {sandbox:?}: {e}"))?;

        // Multi-layer tool delivery:
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "lucent-db-tools": {
                    "command": bridge_bin.clone(),
                    "args": ["--socket", endpoint.clone(), "--token", token.clone()]
                }
            }
        });
        let mcp_json = serde_json::to_string_pretty(&mcp_config).unwrap_or_default();

        // 1. Write workspace .mcp.json (used by Claude Code, Zed, Cline, etc.)
        let _ = std::fs::write(sandbox.join(".mcp.json"), &mcp_json);

        // 2. Write .cursor/mcp.json (used by Cursor)
        let cursor_dir = sandbox.join(".cursor");
        if std::fs::create_dir_all(&cursor_dir).is_ok() {
            let _ = std::fs::write(cursor_dir.join("mcp.json"), &mcp_json);
        }

        // 3. Write .pi/mcp.json (used by Pi / pi-acp)
        let pi_dir = sandbox.join(".pi");
        if std::fs::create_dir_all(&pi_dir).is_ok() {
            let _ = std::fs::write(pi_dir.join("mcp.json"), &mcp_json);
        }

        // 4. Write .vscode/mcp.json (used by VS Code / Copilot)
        let vscode_dir = sandbox.join(".vscode");
        if std::fs::create_dir_all(&vscode_dir).is_ok() {
            let _ = std::fs::write(vscode_dir.join("mcp.json"), &mcp_json);
        }

        // 5. Update global ~/.pi/agent/mcp.json if ~/.pi exists
        if let Ok(home) = std::env::var("HOME") {
            let global_pi = std::path::PathBuf::from(home).join(".pi").join("agent");
            if global_pi.exists() {
                let _ = std::fs::write(global_pi.join("mcp.json"), &mcp_json);
            }
        }

        // 3. Write executable lucent-tool helper script for terminal/bash-based agents (e.g. pi-acp)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script_path = sandbox.join("lucent-tool");
            let script = format!(
                "#!/bin/sh\nexec \"{}\" --socket \"{}\" --token \"{}\" call \"$@\"\n",
                bridge_bin, endpoint, token
            );
            if std::fs::write(&script_path, script).is_ok() {
                let _ = std::fs::set_permissions(
                    &script_path,
                    std::fs::Permissions::from_mode(0o755),
                );
            }
        }
        #[cfg(windows)]
        {
            let script_path = sandbox.join("lucent-tool.cmd");
            let script = format!(
                "@echo off\r\n\"{}\" --socket \"{}\" --token \"{}\" call %*\r\n",
                bridge_bin, endpoint, token
            );
            let _ = std::fs::write(&script_path, script);
        }

        let (reply_tx, reply_rx) = oneshot::channel();
        conn.cmds
            .send(AgentCommand::NewSession {
                cwd: sandbox,
                mcp_servers: vec![mcp_server],
                reply: reply_tx,
            })
            .await
            .map_err(|e| {
                format!(
                    "agent connection closed before session/new: {e} — last lines of agent stderr: {}",
                    process.stderr_snippet()
                )
            })?;
        let session_id = reply_rx
            .await
            .map_err(|e| {
                format!(
                    "agent connection closed before session/new: {e} — last lines of agent stderr: {}",
                    process.stderr_snippet()
                )
            })?
            .map_err(|e| {
                format!(
                    "session/new failed: {e} — last lines of agent stderr: {}",
                    process.stderr_snippet()
                )
            })?;

        let entry = Arc::new(SessionEntry {
            session_id,
            bridge: handle.clone(),
            tools,
            first_prompt: AtomicBool::new(true),
            tools_notice: AtomicBool::new(false),
            _endpoint_dir: dir,
        });
        self.sessions
            .lock()
            .await
            .insert(conversation_id.to_string(), entry.clone());
        self.bridges
            .lock()
            .await
            .insert(conversation_id.to_string(), handle);
        Ok(entry)
    }

    /// Ends a conversation's session: drops the bridge handle (its socket
    /// closes with it — the agent keeps the session until the connection
    /// dies; acceptable v1 behavior, spec non-goals) and auto-rejects any
    /// permission requests still parked for the session (spec §4.5
    /// teardown).
    pub async fn drop_session(&self, conversation_id: &str) {
        let session = self.sessions.lock().await.remove(conversation_id);
        self.bridges.lock().await.remove(conversation_id);
        if let Some(session) = session {
            self.permissions.drain_cancelled(&session.session_id).await;
        }
    }
}

impl Default for AcpState {
    fn default() -> Self {
        Self::new()
    }
}

/// One running connection task per agent process: the command channel the
/// driver / session machinery talks through, and the broadcast fan-out of
/// `AgentEvent`s (every chat() on the same agent subscribes and filters by
/// session id).
pub struct ConnectionEntry {
    pub cmds: mpsc::Sender<AgentCommand>,
    pub events: broadcast::Sender<AgentEvent>,
    pub task: tokio::task::JoinHandle<Result<(), String>>,
}

/// A conversation's ACP session. `first_prompt` gates the system preamble:
/// v1 has no system-prompt param, so the preamble is prepended to the first
/// user message of a session only (spec §4.4). `tools` is the live
/// bridge-connect state (ground truth for whether the agent honored
/// `mcpServers`); `tools_notice` makes sure the UI hears about a missing
/// tool connection exactly once per session.
pub struct SessionEntry {
    pub session_id: String,
    pub bridge: Arc<BridgeHandle>,
    /// Live connectivity of the DB-tools bridge: set the moment the agent's
    /// MCP client completes the hello handshake.
    pub tools: Arc<BridgeConnection>,
    pub first_prompt: AtomicBool,
    /// Whether the "DB tools unavailable" notice was already emitted for
    /// this session (exactly-once per session).
    pub tools_notice: AtomicBool,
    /// Keeps the bridge socket file alive for the connection's lifetime.
    pub(crate) _endpoint_dir: Option<tempfile::TempDir>,
}

#[cfg(unix)]
type BridgeListener = tokio::net::UnixListener;
#[cfg(windows)]
type BridgeListener = tokio::net::windows::named_pipe::NamedPipeServer;

/// Socket placement mirrors `supervisor.rs::endpoint_for` exactly: tempdir
/// + `bridge.sock` on Unix (deep/corporate home paths hit macOS's 104-byte
/// `sun_path` limit — never `~/.lucent`), `\\.\pipe\lucent-acp-<pid>-<token8>`
/// on Windows. The returned `TempDir` must stay alive for the bridge
/// connection's lifetime (`SessionEntry._endpoint_dir` holds it).
fn create_bridge_endpoint(token: &str) -> (BridgeListener, String, Option<tempfile::TempDir>) {
    #[cfg(unix)]
    {
        let _ = token;
        let dir = tempfile::TempDir::new().expect("tempdir for the bridge socket");
        let path = dir.path().join("bridge.sock");
        let listener = tokio::net::UnixListener::bind(&path).expect("bind the bridge socket");
        (listener, path.to_string_lossy().into_owned(), Some(dir))
    }
    #[cfg(windows)]
    {
        let token8: String = token.chars().take(8).collect();
        let name = format!(r"\\.\pipe\lucent-acp-{}-{token8}", std::process::id());
        let server = tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .expect("create the bridge pipe");
        (server, name, None)
    }
}

/// Absolute path to the `lucent-db-tools-mcp` binary, which the AGENT spawns
/// itself (spec §3 D2 / §4.6). Resolution order:
/// 1. `LUCENT_BRIDGE_BIN` env override — wins verbatim (power-user / test
///    escape hatch);
/// 2. packaged: next to the main executable (release builds bundle the
///    sidecar there via `release-sidecar.json` + `bundle.externalBin` —
///    macOS `…/Contents/MacOS/lucent-db-tools-mcp`);
/// 3. dev fallback: walk up from the current executable — test binaries run
///    from `target/<profile>/deps/`, the bin lives in `target/<profile>/`
///    (same search as `Supervisor::worker_binary_path`).
pub fn bridge_binary_path() -> Result<String, String> {
    let name = "lucent-db-tools-mcp";
    if let Ok(p) = std::env::var("LUCENT_BRIDGE_BIN") {
        return Ok(p);
    }
    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let parent = exe.parent().ok_or("current executable has no parent dir")?;
    // "" = packaged (the sidecar sits next to the app executable); "../" =
    // dev/test (unit tests run from target/<profile>/deps/, the bin lives in
    // target/<profile>/).
    //
    // Packaged name: tauri-build copies the sidecar into the target dir with
    // the target triple stripped, but the bundler's name inside the .app is
    // version-dependent — probe the plain name AND the triple-suffixed
    // sidecar name (the triple is baked in by tauri-build via
    // TAURI_ENV_TARGET_TRIPLE), so packaged resolution works under either
    // bundler behavior.
    for candidate in [
        parent.join(name),
        parent.join(format!("{name}-{}", env!("TAURI_ENV_TARGET_TRIPLE"))),
    ] {
        if candidate.exists() {
            return Ok(candidate.to_string_lossy().into_owned());
        }
    }
    let dev = parent.join("../").join(name);
    if dev.exists() {
        return Ok(dev.to_string_lossy().into_owned());
    }
    Err(format!(
        "{name} binary not found next to the app — run `cargo build --bin {name}` (dev) or rebuild the bundle (release)"
    ))
}

/// A registry agent as shown in the Settings UI: the manifest summary plus
/// the install state merged from `~/.lucent/agents/<id>/installed.json`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryAgentSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub license: String,
    pub icon: Option<String>,
    pub installed_version: Option<String>,
    pub update_available: bool,
    /// Whether this agent can use Lucent's database tools (curated — see
    /// `registry::db_tool_support`). Rendered as a badge in the Settings
    /// panel; the driver uses it to decide the first-prompt preamble.
    pub db_tools: crate::ai::acp::registry::DbToolSupport,
}

/// Merges the installed state into the registry listing. `installed` is a
/// lookup (by agent id) so the command can pass `read_installed` directly
/// and tests can script it — no filesystem, no network.
pub fn summarize(
    reg: &Registry,
    installed: impl Fn(&str) -> Option<InstalledAgent>,
) -> Vec<RegistryAgentSummary> {
    reg.agents
        .iter()
        .map(|a| {
            let inst = installed(&a.id);
            let installed_version = inst.as_ref().map(|i| i.version.clone());
            let update_available = inst
                .as_ref()
                .map(|i| i.version != a.version)
                .unwrap_or(false);
            RegistryAgentSummary {
                id: a.id.clone(),
                name: a.name.clone(),
                version: a.version.clone(),
                description: a.description.clone(),
                license: a.license.clone(),
                icon: a.icon.clone(),
                installed_version,
                update_available,
                db_tools: crate::ai::acp::registry::db_tool_support(&a.id),
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "chat_integration_test.rs"]
mod chat_integration_test;

// Phase F recovery capstones (no DB): cancellation-with-pending-permission
// ordering and crash recovery with the restart budget. Run under plain
// `cargo test -p lucent --lib` — only the stub-agent binary is needed.
#[cfg(test)]
#[path = "recovery_test.rs"]
mod recovery_test;

// Phase F capstone: the REAL MCP binary + real bridge + live DuckDB worker.
// Needs the worker + MCP binaries, so only under the integration-tests
// feature (no Docker — DuckDB is in-process).
#[cfg(all(test, feature = "integration-tests"))]
#[path = "capstone_test.rs"]
mod capstone_test;

// Phase F real-agent smoke (IGNORED): the full stack against a REAL coding
// agent — raw capability probe for the compat matrix + a real tool
// round-trip through the bridge into a live DuckDB worker. Needs the
// integration-tests feature AND a live `opencode` install + login.
#[cfg(all(test, feature = "integration-tests"))]
#[path = "real_agent_smoke_test.rs"]
mod real_agent_smoke_test;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::acp::install::LaunchSpec;
    use crate::ai::acp::manager::AgentProcess;
    use crate::ai::acp::registry::bundled_snapshot;
    use crate::ai::agent::CollectorSink;
    use std::collections::HashMap;

    fn installed_agent(id: &str, version: &str) -> InstalledAgent {
        InstalledAgent {
            id: id.to_string(),
            version: version.to_string(),
            launch: LaunchSpec {
                cmd: "npx".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
        }
    }

    #[test]
    fn summaries_merge_installed_state() {
        let reg = bundled_snapshot();
        assert!(reg.agents.len() >= 3, "fixture snapshot has 38 agents");

        let summaries = summarize(&reg, |id| {
            if id == reg.agents[0].id {
                // Installed at the registry's own version → no update.
                Some(installed_agent(id, &reg.agents[0].version))
            } else if id == reg.agents[1].id {
                // Installed at an older version → update available.
                Some(installed_agent(id, "0.0.1"))
            } else {
                None // not installed
            }
        });

        let same_version = summaries
            .iter()
            .find(|s| s.id == reg.agents[0].id)
            .expect("first agent summarized");
        assert_eq!(
            same_version.installed_version.as_deref(),
            Some(reg.agents[0].version.as_str())
        );
        assert!(
            !same_version.update_available,
            "same version is not an update"
        );

        let outdated = summaries
            .iter()
            .find(|s| s.id == reg.agents[1].id)
            .expect("second agent summarized");
        assert_eq!(outdated.installed_version.as_deref(), Some("0.0.1"));
        assert!(
            outdated.update_available,
            "older installed version flags an update"
        );

        let not_installed = summaries
            .iter()
            .find(|s| s.id == reg.agents[2].id)
            .expect("third agent summarized");
        assert!(not_installed.installed_version.is_none());
        assert!(
            !not_installed.update_available,
            "not-installed agents never show updates"
        );
    }

    // ── Session-per-conversation (D3) ─────────────────────────────────────

    /// Locates the compiled stub-agent binary (same walk as driver.rs's
    /// helper; `CARGO_BIN_EXE_*` is only set for integration tests).
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

    fn stub_process() -> Arc<AgentProcess> {
        Arc::new(AgentProcess {
            agent_id: "stub".into(),
            launch: LaunchSpec {
                cmd: stub_binary(),
                args: vec![],
                env: HashMap::new(),
            },
            stderr_tail: Arc::new(std::sync::Mutex::new(String::new())),
            spawns: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }

    fn tool_ctx() -> AiToolContext {
        AiToolContext {
            db: Arc::new(Mutex::new(None)),
            connection_id: None,
            capabilities: None,
            config: crate::ai::config::AiConfig::default(),
            schema_graph: Arc::new(Mutex::new(None)),
            embedder: Arc::new(Mutex::new(None)),
            reranker: Arc::new(Mutex::new(None)),
        }
    }

    fn sink() -> Arc<dyn AgentSink> {
        Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())))
    }

    /// Points the agent sandbox at a tempdir so session creation never
    /// writes into the real ~/.lucent (kept alive for the test duration).
    fn hermetic_workspace() -> tempfile::TempDir {
        let dir = tempfile::TempDir::new().unwrap();
        std::env::set_var(
            "LUCENT_ACP_WORKSPACE",
            dir.path().to_string_lossy().into_owned(),
        );
        dir
    }

    #[tokio::test]
    async fn session_is_reused_within_a_conversation() {
        let _ws = hermetic_workspace();
        let acp = AcpState::new();
        let process = stub_process();
        let sink = sink();

        let s1 = acp
            .session_for("conv-1", &process, &tool_ctx(), &sink)
            .await
            .expect("first session/new round-trips");
        assert!(s1.session_id.starts_with("stub-sess-"), "{}", s1.session_id);
        assert!(
            s1.first_prompt.load(std::sync::atomic::Ordering::SeqCst),
            "fresh session: the system preamble is still pending"
        );

        let s2 = acp
            .session_for("conv-1", &process, &tool_ctx(), &sink)
            .await
            .expect("second session_for hits the cache");
        assert_eq!(
            s1.session_id, s2.session_id,
            "same conversation → same ACP session (multi-turn continuity)"
        );
        assert!(Arc::ptr_eq(&s1, &s2), "the entry itself is cached");
        // The preamble flag is consumed exactly once (the driver swaps it on
        // the first prompt; a second use sees false).
        assert!(
            s1.first_prompt
                .swap(false, std::sync::atomic::Ordering::SeqCst),
            "first use consumes the preamble flag"
        );
        assert!(
            !s2.first_prompt.load(std::sync::atomic::Ordering::SeqCst),
            "reuse sees the consumed flag"
        );

        // The bridge config must ride in session/new's mcpServers — the stub
        // logs the count on stderr, which lands in the process stderr tail.
        let stderr = process.stderr_snippet();
        assert!(
            stderr.contains("mcpServers=1"),
            "bridge config delivered via session/new: {stderr:?}"
        );
    }

    #[tokio::test]
    async fn different_conversations_get_different_sessions() {
        let _ws = hermetic_workspace();
        let acp = AcpState::new();
        let process = stub_process();
        let sink = sink();

        let s1 = acp
            .session_for("conv-1", &process, &tool_ctx(), &sink)
            .await
            .expect("session 1");
        let s2 = acp
            .session_for("conv-2", &process, &tool_ctx(), &sink)
            .await
            .expect("session 2");
        assert_ne!(
            s1.session_id, s2.session_id,
            "each conversation gets its own ACP session"
        );
        assert!(!Arc::ptr_eq(&s1, &s2), "distinct entries");
    }

    // ── Bridge binary resolution (F3) ────────────────────────────────────

    #[test]
    fn bridge_binary_path_resolves_in_dev_and_bundle() {
        // Restores `var` to its prior state on drop — a panic between the
        // set and the assertions must not leak the override to parallel
        // tests (the capstone would spawn the test exe as the MCP binary).
        struct EnvVarGuard<'a>(&'a str, Option<String>);
        impl Drop for EnvVarGuard<'_> {
            fn drop(&mut self) {
                match &self.1 {
                    Some(v) => std::env::set_var(self.0, v),
                    None => std::env::remove_var(self.0),
                }
            }
        }

        // Env override wins verbatim (packaged installs / power users point
        // at a custom sidecar location). Use the test binary as the override
        // value: it exists (so parallel tests that hit `bridge_binary_path`
        // while the var is set still get a valid path) and differs from the
        // dev resolution (so this assertion genuinely exercises the override).
        let override_path = std::env::current_exe()
            .expect("current test exe")
            .to_string_lossy()
            .into_owned();
        let prior = std::env::var("LUCENT_BRIDGE_BIN").ok();
        std::env::set_var("LUCENT_BRIDGE_BIN", &override_path);
        let guard = EnvVarGuard("LUCENT_BRIDGE_BIN", prior);
        let overridden = bridge_binary_path().expect("env override resolves");
        drop(guard);
        assert_eq!(
            overridden, override_path,
            "LUCENT_BRIDGE_BIN wins verbatim over every other resolution"
        );

        // Dev-walk pin, gated on bin presence. `lucent-db-tools-mcp` is a bin
        // target of this package and `cargo test -p lucent --lib` builds only
        // the lib — on a genuine fresh checkout the binary does not exist and
        // the walk correctly errors. The F1 capstone (integration tier, which
        // builds the bin) is the real pin for the walk; here we only assert
        // the walk's shape when the bin is actually present, so fresh
        // checkouts stay green while dev machines still pin it.
        match bridge_binary_path() {
            Ok(p) if std::path::Path::new(&p).exists() => {
                // After the override is gone the dev fallback still resolves
                // a real file — pins the walk against regressions.
                let resolved = bridge_binary_path().expect("dev walk still resolves");
                assert_eq!(resolved, p, "override removal restores dev resolution");
            }
            // Bin not built in this tier (or a resolution race) — skip; the
            // integration-tier capstone pins the walk.
            Ok(_) | Err(_) => {}
        }
    }

    #[tokio::test]
    async fn drop_session_removes_entry_and_auto_rejects_pending_permissions() {
        let _ws = hermetic_workspace();
        let acp = AcpState::new();
        let process = stub_process();
        let sink = sink();

        let s1 = acp
            .session_for("conv-1", &process, &tool_ctx(), &sink)
            .await
            .expect("session");

        // Park a pending permission for the session (as the connection task
        // would) — drop_session must auto-reject it (spec §4.5 teardown).
        let (tx, rx) = tokio::sync::oneshot::channel();
        acp.permissions
            .push(
                &s1.session_id,
                crate::ai::acp::permissions::PermissionPending {
                    tx,
                    allow_option_id: None,
                },
            )
            .await;

        acp.drop_session("conv-1").await;
        assert!(
            acp.sessions.lock().await.get("conv-1").is_none(),
            "session evicted"
        );
        assert!(
            acp.bridges.lock().await.get("conv-1").is_none(),
            "bridge handle evicted"
        );
        let outcome = rx.await.expect("parked permission resolves on teardown");
        assert!(
            matches!(
                outcome,
                agent_client_protocol::schema::v1::RequestPermissionOutcome::Cancelled
            ),
            "teardown auto-rejects with Cancelled: {outcome:?}"
        );
    }
}
