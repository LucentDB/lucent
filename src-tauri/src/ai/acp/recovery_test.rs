//! Phase F recovery capstones (no DB, run under plain `cargo test -p lucent
//! --lib`): the normative cancellation order with a pending permission
//! request, and crash recovery — the agent process dies mid-turn, chat()
//! errors surface the stderr tail, and the bounded restart budget (2 per 10
//! minutes, spec §4.3) eventually blocks with the budget message.
//!
//! Both drive the FULL stack (`AcpState` + `AcpManager` + connection task +
//! the real `lucent-acp-stub-agent` binary) — no mocks, no database. The
//! stub's scripted `permission` step genuinely blocks the turn until the
//! client answers, and its scripted `exit` step terminates the process
//! mid-turn (the crash).

use crate::ai::acp::driver::AcpChatDriver;
use crate::ai::acp::permissions::PermissionPending;
use crate::ai::acp::AcpState;
use crate::ai::agent::{AgentSink, ConversationState};
use crate::ai::config::{AcpAgentConfig, AiConfig};
use crate::ai::events::{AgentPermissionPayload, AiEvent};
use crate::ai::tools::AiToolContext;
use agent_client_protocol::schema::v1::RequestPermissionOutcome;
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

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

fn acp_cfg(script: Option<&std::path::Path>) -> AcpAgentConfig {
    let mut env = HashMap::new();
    if let Some(script) = script {
        env.insert(
            "STUB_SCRIPT".to_string(),
            script.to_string_lossy().into_owned(),
        );
    }
    AcpAgentConfig {
        agent_id: "stub".into(),
        command: Some(stub_binary()),
        env,
        auto_deny_permissions: false,
    }
}

/// Points the agent sandbox at a tempdir so session creation never writes
/// into the real ~/.lucent (kept alive for the test duration).
fn hermetic_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var(
        "LUCENT_ACP_WORKSPACE",
        dir.path().to_string_lossy().into_owned(),
    );
    dir
}

fn script_file(steps: serde_json::Value) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("script.json"),
        serde_json::to_string_pretty(&steps).unwrap(),
    )
    .unwrap();
    dir
}

fn conversation(conv_id: &str) -> Arc<tokio::sync::Mutex<ConversationState>> {
    Arc::new(tokio::sync::Mutex::new(ConversationState::new(
        conv_id.to_string(),
    )))
}

fn tool_ctx() -> AiToolContext {
    AiToolContext {
        db: Arc::new(AsyncMutex::new(None)),
        connection_id: None,
        capabilities: None,
        config: AiConfig::default(),
        schema_graph: Arc::new(AsyncMutex::new(None)),
        embedder: Arc::new(AsyncMutex::new(None)),
        reranker: Arc::new(AsyncMutex::new(None)),
    }
}

/// A sink that records both `AiEvent`s and `AgentPermissionPayload`s — the
/// capstone must observe the permission request the driver surfaces.
#[derive(Default)]
struct PermissionSink {
    events: std::sync::Mutex<Vec<AiEvent>>,
    permissions: std::sync::Mutex<Vec<AgentPermissionPayload>>,
}

impl AgentSink for PermissionSink {
    fn event(&self, event: AiEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn dml_approval(&self, _payload: crate::ai::events::DmlApprovalPayload) {}
    fn permission_request(&self, payload: AgentPermissionPayload) {
        self.permissions.lock().unwrap().push(payload);
    }
}

/// Cancellation with a pending permission request, through the full stack:
/// the stub emits a request_permission and holds the prompt open until the
/// client answers. The driver must resolve every pending permission with
/// `RequestPermissionOutcome::Cancelled` BEFORE sending the
/// session/cancel notification (normative MUST, schema doc on
/// `RequestPermissionOutcome::Cancelled`) — the stub stays blocked
/// otherwise, and the turn never resolves.
#[tokio::test]
async fn cancel_resolves_pending_permission_then_cancels() {
    let _ws = hermetic_workspace();
    let script = script_file(json!({
        "stopReason": "end_turn",
        "steps": [
            {"permission": {"title": "Read ~/.zshrc", "options": [{"optionId": "allow_once", "name": "Allow once", "kind": "allow_once"}]}},
            {"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "after permission"}}}
        ]
    }));

    let acp_state = AcpState::new();
    let sink = Arc::new(PermissionSink::default());
    let conv = conversation("conv-4");
    let cancel = tokio_util::sync::CancellationToken::new();

    let cancel_for_task = cancel.clone();
    let script_path = script.path().to_path_buf();
    let sink_task = sink.clone();
    let acp_task = acp_state.clone();
    let handle = tokio::spawn(async move {
        let driver = AcpChatDriver::new(
            acp_task,
            acp_cfg(Some(&script_path.join("script.json"))),
            tool_ctx(),
        );
        driver
            .chat(
                "hi".into(),
                &AiConfig::default(),
                "preamble".into(),
                conv,
                sink_task,
                cancel_for_task,
            )
            .await
    });

    // The surfaced permission request is the deterministic signal that the
    // stub's turn is blocked on the client's decision.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let payload = loop {
        if let Some(p) = sink.permissions.lock().unwrap().first().cloned() {
            break p;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "permission payload never arrived on the sink"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(payload.conversation_id, "conv-4");
    assert_eq!(payload.title, "Read ~/.zshrc");

    // Probe the registry: a pending permission the TEST owns, so the
    // Cancelled outcome is asserted directly. The connection task's own
    // pending sits in the same FIFO queue and is drained alongside it.
    let session_id = acp_state
        .sessions
        .lock()
        .await
        .get("conv-4")
        .expect("session cached for the conversation")
        .session_id
        .clone();
    let (probe_tx, probe_rx) = tokio::sync::oneshot::channel();
    acp_state
        .permissions
        .push(
            &session_id,
            PermissionPending {
                tx: probe_tx,
                allow_option_id: None,
            },
        )
        .await;

    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(10), handle)
        .await
        .expect("turn finishes after cancel")
        .expect("task did not panic");
    assert!(result.is_ok(), "chat resolves after cancel: {result:?}");

    // Normative contract: the pending permission resolved with Cancelled.
    let outcome = probe_rx.await.expect("probe pending permission resolves");
    assert!(
        matches!(outcome, RequestPermissionOutcome::Cancelled),
        "pending permission resolved with Cancelled: {outcome:?}"
    );

    // The session/cancel notification reached the stub (it logs it on
    // stderr, which lands in the process stderr tail). The permission
    // resolution unblocks the stub, so the turn ends before the connection
    // task processes the queued Cancel command — poll briefly.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let stderr = loop {
        let tail = acp_state
            .manager
            .processes
            .lock()
            .unwrap()
            .get("stub")
            .expect("process record exists")
            .stderr_snippet();
        if tail.contains("session/cancel") {
            break tail;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "session/cancel never reached the stub: {tail:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    // The prompt reply arrived with stop_reason "cancelled" — the stub
    // answers the permission with Cancelled (which it treats as the cancel
    // signal), streams the remaining text, then responds cancelled. The
    // driver keeps the accumulated text in the Done event.
    let events = sink.events.lock().unwrap().clone();
    let done = events
        .iter()
        .find(|e| matches!(e, AiEvent::Done { .. }))
        .expect("Done present after cancel");
    match done {
        AiEvent::Done { final_message, .. } => {
            assert_eq!(
                final_message, "after permission",
                "streamed text survives the cancelled stop_reason"
            );
        }
        _ => unreachable!(),
    }
}

/// Crash recovery: the stub terminates itself on every prompt (scripted
/// `exit` step), so every chat() attempt dies mid-turn. The stack must (a)
/// surface the agent's stderr tail in the errors, and (b) charge each crash
/// against the restart budget (2 per 10 min per agent, spec §4.3) until a
/// further attempt fails with the budget message.
#[tokio::test]
async fn agent_crash_surfaces_stderr_tail_and_budget_blocks_restart() {
    let _ws = hermetic_workspace();
    let script = script_file(json!({
        "stopReason": "end_turn",
        "steps": [{"exit": true}]
    }));

    let acp_state = AcpState::new();
    let driver = AcpChatDriver::new(
        acp_state.clone(),
        acp_cfg(Some(&script.path().join("script.json"))),
        tool_ctx(),
    );
    let conv = conversation("conv-5");

    let mut errors: Vec<String> = Vec::new();
    // Fixed attempt count (no early break): the budget gate must hold AFTER
    // the first block too — a blocked reap must not be followed by an
    // ungated fresh spawn (the alternation bug: block → spawn+crash → block
    // → … would show up as a non-budget error after the first "too many").
    for attempt in 0..10 {
        let sink: Arc<dyn AgentSink> = Arc::new(crate::ai::agent::CollectorSink(
            std::sync::Mutex::new(Vec::new()),
        ));
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            driver.chat(
                "hi".into(),
                &AiConfig::default(),
                "preamble".into(),
                conv.clone(),
                sink,
                tokio_util::sync::CancellationToken::new(),
            ),
        )
        .await;
        match result {
            Ok(Ok(())) => errors.push(format!("attempt {attempt} unexpectedly succeeded")),
            Ok(Err(e)) => errors.push(e),
            Err(_) => panic!("chat attempt {attempt} hung — the crash was not detected"),
        }
    }

    let all = errors.join("\n");
    assert!(
        all.contains("STUB exit step"),
        "an error surfaces the agent's stderr tail: {all}"
    );
    let first_block = errors
        .iter()
        .position(|e| e.contains("too many"))
        .expect("the restart budget eventually blocks with its message");
    assert!(
        errors[first_block..].iter().all(|e| e.contains("too many")),
        "the budget gate holds after the first block — no ungated fresh spawn: {all}"
    );
}
