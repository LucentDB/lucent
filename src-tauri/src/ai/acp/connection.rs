//! Connection task: one per agent process, owns the `agent-client-protocol`
//! client for that process, and translates crate events into `AgentEvent`s
//! on a tokio channel while serving `AgentCommand`s from a FIFO.
//!
//! The task is the single place that talks to the crate: it initializes,
//! runs the MCP capability gate (see `check_mcp_gate`), creates sessions,
//! sends prompts, forwards cancellation, and answers
//! `session/request_permission` requests through the permission registry
//! (C4). The driver (C3) and `AcpState` (D) talk only to the channels.

use crate::ai::acp::manager::AgentProcess;
use crate::ai::acp::permissions::{PermissionPending, PermissionRegistry};
use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, InitializeRequest, McpServer, NewSessionRequest,
    PermissionOptionKind, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SessionNotification, StopReason, TextContent,
};
use agent_client_protocol::{AcpAgent, Agent, Client, ConnectionTo, LineDirection};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

/// Why a prompt failed. `Rpc` means the agent answered with a JSON-RPC error
/// (the connection stays alive — a stale session id, a refused turn, …);
/// `Transport` means the request could not be dispatched or the connection
/// died (terminal for the task; the reap path handles budget + restart).
#[derive(Debug, Clone, PartialEq)]
pub enum PromptError {
    Rpc(String),
    Transport(String),
}

/// Commands the driver / AcpState send to the connection task.
pub enum AgentCommand {
    /// Create a session. Replies with the new `session_id`.
    NewSession {
        cwd: PathBuf,
        mcp_servers: Vec<McpServer>,
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Prompt a session. The reply resolves when the turn ends (v1: the
    /// prompt request itself resolves with the stop reason — there is no
    /// idle notification). The `text` field stays empty here; the driver
    /// accumulates it from the event stream (C3).
    Prompt {
        session_id: String,
        text: String,
        reply: oneshot::Sender<Result<PromptOutcome, PromptError>>,
    },
    /// Cancel the in-flight turn of a session (session/cancel notification).
    Cancel { session_id: String },
    /// Stop the loop; the connection then tears down the agent process.
    Shutdown,
}

/// What a prompt turn produced. `text` is filled by the driver from the
/// event stream; the connection task only carries the stop reason.
#[derive(Debug)]
pub struct PromptOutcome {
    pub stop_reason: StopReason,
    pub text: String,
}

/// Events the connection task emits for consumers (the driver, the
/// permission bridge).
#[derive(Clone, Debug)]
pub enum AgentEvent {
    /// A `session/update` notification arrived for a session.
    SessionUpdate {
        session_id: String,
        update: agent_client_protocol::schema::v1::SessionUpdate,
    },
    /// The agent is asking the user for permission. The corresponding
    /// responder lives in the connection task, parked on a oneshot in the
    /// permission registry (C4).
    PermissionRequest {
        session_id: String,
        request: RequestPermissionRequest,
    },
}

/// Runs the ACP client for one agent process until the command channel
/// closes or the process dies. Errors are the connection's terminal state
/// (process crash, protocol failure); per-command failures travel back
/// through the command's reply channel instead.
pub async fn run_connection(
    agent: Arc<AgentProcess>,
    mut cmds: mpsc::Receiver<AgentCommand>,
    events: mpsc::Sender<AgentEvent>,
    permissions: Arc<PermissionRegistry>,
) -> Result<(), String> {
    let mut cfg = AcpAgent::new(McpServer::Stdio(
        agent_client_protocol::schema::v1::McpServerStdio::new(
            format!("agent-{}", agent.agent_id),
            &agent.launch.cmd,
        )
        .args(agent.launch.args.clone())
        .env(
            agent
                .launch
                .env
                .iter()
                .map(|(k, v)| {
                    agent_client_protocol::schema::v1::EnvVariable::new(k.clone(), v.clone())
                })
                .collect(),
        ),
    ));

    // Stderr tail + the raw initialize response line (for the MCP gate).
    // Both are touched from the sync `with_debug` callback — std Mutex.
    let stderr_tail = agent.stderr_tail.clone();
    let init_line: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let init_line_cb = init_line.clone();
    cfg = cfg.with_debug(move |line, direction| match direction {
        LineDirection::Stderr => {
            AgentProcess::tail_push(stderr_tail.clone(), line, 64 * 1024);
        }
        LineDirection::Stdout if line.contains("protocolVersion") => {
            let mut slot = init_line_cb.lock().unwrap();
            if slot.is_none() {
                *slot = Some(line.to_string());
            }
        }
        _ => {}
    });

    let ev_notif = events.clone();
    let ev_req = events.clone();
    let perms = permissions.clone();
    let client = Client
        .builder()
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                let _ = ev_notif
                    .send(AgentEvent::SessionUpdate {
                        session_id: notification.session_id.to_string(),
                        update: notification.update,
                    })
                    .await;
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _conn| {
                // The responder is not storable, so park a task that awaits
                // the decision oneshot and then answers. The registry is
                // FIFO per session (C4 drains it on cancel, normative
                // order: Cancelled BEFORE the CancelNotification).
                let allow_option_id = request
                    .options
                    .iter()
                    .find(|o| {
                        matches!(
                            o.kind,
                            PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                        )
                    })
                    .map(|o| o.option_id.clone());
                let (tx, rx) = oneshot::channel();
                perms
                    .push(
                        &request.session_id.to_string(),
                        PermissionPending {
                            tx,
                            allow_option_id,
                        },
                    )
                    .await;
                let _ = ev_req
                    .send(AgentEvent::PermissionRequest {
                        session_id: request.session_id.to_string(),
                        request: request.clone(),
                    })
                    .await;
                tokio::spawn(async move {
                    let outcome = rx.await.unwrap_or(RequestPermissionOutcome::Cancelled);
                    let _ = responder.respond(RequestPermissionResponse::new(outcome));
                });
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(cfg, |connection: ConnectionTo<Agent>| async move {
            let _init = connection
                .send_request(InitializeRequest::new(
                    agent_client_protocol::schema::ProtocolVersion::V1,
                ))
                .block_task()
                .await
                .map_err(|e| {
                    agent_client_protocol::util::internal_error(format!("initialize failed: {e}"))
                })?;

            // MCP capability gate (adapted to the resolved crate — see
            // `check_mcp_gate` for the protocol reality).
            if let Some(raw) = init_line.lock().unwrap().as_deref() {
                check_mcp_gate(raw).map_err(agent_client_protocol::util::internal_error)?;
            }

            while let Some(cmd) = cmds.recv().await {
                match cmd {
                    AgentCommand::NewSession {
                        cwd,
                        mcp_servers,
                        reply,
                    } => {
                        let mut req = NewSessionRequest::new(cwd);
                        if !mcp_servers.is_empty() {
                            req = req.mcp_servers(mcp_servers);
                        }
                        let sent = connection.send_request(req);
                        // Non-blocking: the crate resolves this callback on a background
                        // task; the command loop stays live (the block_task anti-footgun
                        // is what starved Cancel). A transport death never invokes the
                        // callback — the reply oneshot drops, which the caller maps to
                        // its "connection closed" error, and the task ends via
                        // `client.await` below.
                        let _ = sent.on_receiving_result(move |result| async move {
                            match result {
                                Ok(resp) => {
                                    let _ = reply.send(Ok(resp.session_id.to_string()));
                                }
                                Err(e) => {
                                    // Peer RPC error (agent refused the session): the
                                    // connection is fine — surface the message.
                                    let _ = reply.send(Err(e.to_string()));
                                }
                            }
                            Ok(())
                        });
                    }
                    AgentCommand::Prompt {
                        session_id,
                        text,
                        reply,
                    } => {
                        let req = PromptRequest::new(
                            session_id.clone(),
                            vec![ContentBlock::Text(TextContent::new(text))],
                        );
                        let sent = connection.send_request(req);
                        let _ = sent.on_receiving_result(move |result| async move {
                            match result {
                                Ok(resp) => {
                                    let _ = reply.send(Ok(PromptOutcome {
                                        stop_reason: resp.stop_reason,
                                        text: String::new(),
                                    }));
                                }
                                Err(e) => {
                                    // The agent answered with a JSON-RPC error (e.g.
                                    // "session not found"). Non-terminal: the connection
                                    // survives; the driver evicts the stale session.
                                    let _ = reply.send(Err(PromptError::Rpc(e.to_string())));
                                }
                            }
                            Ok(())
                        });
                    }
                    AgentCommand::Cancel { session_id } => {
                        let _ = connection.send_notification(CancelNotification::new(session_id));
                    }
                    AgentCommand::Shutdown => break,
                }
            }
            Ok(())
        });

    client
        .await
        .map_err(|e| format!("agent connection ended: {e}"))
}

/// MCP capability gate — see the doc comment below. The plan's original
/// check (`InitializeResponse.capabilities.session.mcp.stdio`) does not
/// exist in the resolved crate (agent-client-protocol 1.3.0 / schema 1.4.0)
/// nor in the current v1 spec, so the gate inspects the raw initialize
/// response line instead.
pub(crate) fn check_mcp_gate(init_line: &str) -> Result<(), String> {
    let v: serde_json::Value = match serde_json::from_str(init_line) {
        Ok(v) => v,
        Err(_) => return Ok(()), // unparseable -> no capability signal -> stdio mandate applies
    };
    let result = &v["result"];
    // Crate shape: agentCapabilities.mcpCapabilities (any object — stdio is
    // the mandatory baseline; the object's presence means the agent's SDK
    // models MCP servers).
    if result
        .get("agentCapabilities")
        .and_then(|c| c.get("mcpCapabilities"))
        .is_some()
    {
        return Ok(());
    }
    // Legacy pre-1.0 shape: capabilities.session.mcp (some agents still emit
    // this tree; the crate ignores it — the stub advertises it).
    if result
        .get("capabilities")
        .and_then(|c| c.get("session"))
        .and_then(|s| s.get("mcp"))
        .is_some()
    {
        return Ok(());
    }
    // Explicit legacy capability declaration WITHOUT any mcp surface: the
    // agent claims session capabilities but none usable by Lucent's bridge.
    if result
        .get("capabilities")
        .and_then(|c| c.get("session"))
        .is_some()
    {
        return Err(
            "This agent can't use Lucent's database tools (no MCP support). Pick another agent from Settings → Agents."
                .into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::acp::install::LaunchSpec;
    use crate::ai::acp::manager::AgentProcess;
    use std::collections::HashMap;
    use tempfile::tempdir;

    /// Locates the compiled stub-agent binary. `CARGO_BIN_EXE_*` is only set
    /// for integration tests, so for `--lib` runs we walk up from the test
    /// binary: `target/<profile>/deps/lucent-<hash>` -> `target/<profile>`.
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

    fn stub_process_with(env: HashMap<String, String>) -> Arc<AgentProcess> {
        Arc::new(AgentProcess {
            agent_id: "stub".into(),
            launch: LaunchSpec {
                cmd: stub_binary(),
                args: vec![],
                env,
            },
            stderr_tail: Arc::new(std::sync::Mutex::new(String::new())),
            spawns: Arc::new(std::sync::Mutex::new(Vec::new())),
        })
    }

    fn stub_process() -> Arc<AgentProcess> {
        stub_process_with(HashMap::new())
    }

    fn script_env(dir: &tempfile::TempDir) -> HashMap<String, String> {
        let mut env = HashMap::new();
        env.insert(
            "STUB_SCRIPT".to_string(),
            dir.path().join("script.json").to_string_lossy().into_owned(),
        );
        env
    }

    fn wire(
        proc: Arc<AgentProcess>,
    ) -> (
        mpsc::Sender<AgentCommand>,
        mpsc::Receiver<AgentEvent>,
        tokio::task::JoinHandle<Result<(), String>>,
    ) {
        let (cmds_tx, cmds_rx) = mpsc::channel(16);
        let (ev_tx, ev_rx) = mpsc::channel(64);
        let perms = Arc::new(PermissionRegistry::new());
        let handle = tokio::spawn(run_connection(proc, cmds_rx, ev_tx, perms));
        (cmds_tx, ev_rx, handle)
    }

    async fn new_session(
        cmds: &mpsc::Sender<AgentCommand>,
        cwd: PathBuf,
    ) -> Result<String, String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        cmds.send(AgentCommand::NewSession {
            cwd,
            mcp_servers: vec![],
            reply: reply_tx,
        })
        .await
        .map_err(|e| format!("send failed: {e}"))?;
        reply_rx.await.map_err(|e| format!("reply dropped: {e}"))?
    }

    async fn shutdown(cmds: &mpsc::Sender<AgentCommand>) {
        let _ = cmds.send(AgentCommand::Shutdown).await;
    }

    #[tokio::test]
    async fn connection_initializes_and_creates_session() {
        let proc = stub_process();
        let (cmds, _ev, handle) = wire(proc.clone());

        let tmp = tempdir().unwrap();
        let session_id = new_session(&cmds, tmp.path().to_path_buf())
            .await
            .expect("session/new round-trips against the stub");
        assert!(
            session_id.starts_with("stub-sess-"),
            "stub session id: {session_id}"
        );

        shutdown(&cmds).await;
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .expect("connection task finishes")
            .expect("task did not panic");
        assert!(result.is_ok(), "clean shutdown: {result:?}");
    }

    #[tokio::test]
    async fn connection_prompt_round_trips_stop_reason() {
        let proc = stub_process();
        let (cmds, _ev, handle) = wire(proc.clone());

        let tmp = tempdir().unwrap();
        let session_id = new_session(&cmds, tmp.path().to_path_buf())
            .await
            .expect("session/new ok");

        let (reply_tx, reply_rx) = oneshot::channel();
        cmds.send(AgentCommand::Prompt {
            session_id: session_id.clone(),
            text: "hi".into(),
            reply: reply_tx,
        })
        .await
        .unwrap();
        let outcome = reply_rx
            .await
            .expect("prompt reply arrives")
            .expect("prompt succeeds");
        assert!(
            matches!(outcome.stop_reason, StopReason::EndTurn),
            "stub defaults to end_turn: {:?}",
            outcome.stop_reason
        );

        shutdown(&cmds).await;
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .expect("connection task finishes")
            .expect("task did not panic");
        assert!(result.is_ok(), "clean shutdown: {result:?}");
    }

    #[tokio::test]
    async fn prompt_rpc_error_resolves_as_rpc_and_keeps_the_connection_alive() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("script.json"),
            r#"{"stopReason":"end_turn","steps":[{"rpcError":"session not found"}]}"#,
        )
        .unwrap();
        let proc = stub_process_with(script_env(&dir));
        let (cmds, _ev, handle) = wire(proc.clone());

        let tmp = tempdir().unwrap();
        let session_id = new_session(&cmds, tmp.path().to_path_buf())
            .await
            .expect("session/new ok");

        let (reply_tx, reply_rx) = oneshot::channel();
        cmds.send(AgentCommand::Prompt {
            session_id,
            text: "hi".into(),
            reply: reply_tx,
        })
        .await
        .unwrap();
        let err = reply_rx
            .await
            .expect("prompt reply arrives")
            .expect_err("the agent answered with an RPC error");
        assert!(
            matches!(&err, PromptError::Rpc(msg) if msg.contains("session not found")),
            "classified as a peer RPC error: {err:?}"
        );

        // The connection survived the error — a second session/new round-trips.
        let again = new_session(&cmds, tmp.path().to_path_buf())
            .await
            .expect("connection still alive after an RPC error");
        assert!(again.starts_with("stub-sess-"));

        shutdown(&cmds).await;
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .expect("task finishes")
            .expect("no panic");
        assert!(result.is_ok(), "clean shutdown: {result:?}");
    }

    #[tokio::test]
    async fn cancel_notification_is_delivered_while_the_prompt_is_in_flight() {
        // The stub streams two chunks 400ms apart; the cancel is sent at ~50ms
        // and the stub must SEE it mid-turn (it flags the turn cancelled). With
        // the old block_task loop the Cancel command would sit queued until the
        // prompt resolved and the stub would never see it before answering.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("script.json"),
            r#"{"stopReason":"end_turn","steps":[{"sleepMs":400},{"notify":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"mid"}}},{"sleepMs":400}]}"#,
        )
        .unwrap();
        let proc = stub_process_with(script_env(&dir));
        let (cmds, _ev, handle) = wire(proc.clone());

        let tmp = tempdir().unwrap();
        let session_id = new_session(&cmds, tmp.path().to_path_buf())
            .await
            .expect("session/new ok");

        let (reply_tx, reply_rx) = oneshot::channel();
        cmds.send(AgentCommand::Prompt {
            session_id: session_id.clone(),
            text: "hi".into(),
            reply: reply_tx,
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        cmds.send(AgentCommand::Cancel {
            session_id: session_id.clone(),
        })
        .await
        .unwrap();

        let outcome = reply_rx
            .await
            .expect("prompt reply arrives")
            .expect("prompt succeeds");
        assert!(
            matches!(outcome.stop_reason, StopReason::Cancelled),
            "the stub saw the cancel mid-turn: {:?}",
            outcome.stop_reason
        );
        assert!(
            proc.stderr_snippet().contains("cancelling current prompt"),
            "the stub logged the mid-turn cancel: {}",
            proc.stderr_snippet()
        );

        shutdown(&cmds).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .expect("task finishes");
    }

    #[tokio::test]
    async fn command_loop_stays_responsive_while_a_prompt_streams() {
        // A NewSession issued while a prompt is in flight must be serviced
        // before the prompt resolves. With block_task the command would sit in
        // the queue until the turn ended.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("script.json"),
            r#"{"stopReason":"end_turn","steps":[{"notify":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"a"}}},{"sleepMs":400},{"notify":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"b"}}},{"sleepMs":400}]}"#,
        )
        .unwrap();
        let proc = stub_process_with(script_env(&dir));
        let (cmds, _ev, handle) = wire(proc.clone());

        let tmp = tempdir().unwrap();
        let session_id = new_session(&cmds, tmp.path().to_path_buf())
            .await
            .expect("session/new ok");

        let (prompt_tx, mut prompt_rx) = oneshot::channel();
        cmds.send(AgentCommand::Prompt {
            session_id: session_id.clone(),
            text: "hi".into(),
            reply: prompt_tx,
        })
        .await
        .unwrap();

        let (ns_tx, ns_rx) = oneshot::channel();
        cmds.send(AgentCommand::NewSession {
            cwd: tmp.path().to_path_buf(),
            mcp_servers: vec![],
            reply: ns_tx,
        })
        .await
        .unwrap();

        let ns = tokio::time::timeout(std::time::Duration::from_secs(2), ns_rx)
            .await
            .expect("NewSession is serviced while the prompt is in flight")
            .expect("reply arrives")
            .expect("session/new ok");
        assert!(ns.starts_with("stub-sess-"));
        assert!(
            prompt_rx.try_recv().is_err(),
            "the prompt is still pending when the loop serviced NewSession"
        );

        let outcome = prompt_rx.await.expect("prompt eventually resolves").unwrap();
        assert!(matches!(outcome.stop_reason, StopReason::EndTurn));

        shutdown(&cmds).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .expect("task finishes");
    }

    #[tokio::test]
    async fn mcp_capability_gate_accepts_legacy_stdio_advertisement() {
        // Exactly what the stub sends on initialize.
        let line = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"capabilities":{"session":{"mcp":{"stdio":{}}}},"agentInfo":{"name":"lucent-acp-stub-agent","version":"0.1.0"}}}"#;
        assert!(check_mcp_gate(line).is_ok());
    }

    #[tokio::test]
    async fn mcp_capability_gate_accepts_crate_shape_and_absent_capabilities() {
        let crate_shape = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"mcpCapabilities":{"http":false,"sse":false}}}}"#;
        assert!(
            check_mcp_gate(crate_shape).is_ok(),
            "crate shape (stdio mandatory) passes"
        );
        let no_signal = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#;
        assert!(
            check_mcp_gate(no_signal).is_ok(),
            "no capability signal: stdio mandate applies"
        );
    }

    #[tokio::test]
    async fn mcp_capability_gate_rejects_legacy_tree_without_mcp() {
        let line = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"capabilities":{"session":{"load":true}}}}"#;
        let err = check_mcp_gate(line).expect_err("explicit session block without mcp fails");
        assert!(err.contains("no MCP support"), "spec §6 message: {err}");
    }

    #[test]
    fn stderr_lines_land_in_the_tail() {
        // Unit-level: the with_debug wiring is exercised by the live tests;
        // this pins the tail helper used by it.
        let proc = stub_process();
        let len = AgentProcess::tail_push(proc.stderr_tail.clone(), "boom\n", 64 * 1024);
        assert_eq!(len, 5);
        assert_eq!(proc.stderr_snippet(), "boom\n");
    }
}
