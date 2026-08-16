//! D5 capstone: the full `ai_chat` seam driven through `run_agent_turn` with
//! the scripted stub agent — config → branch → `AcpChatDriver` → stub over
//! real ACP → `AiEvent`s on the IPC channel. No database: the stub never
//! calls tools (the bridge is still spawned by `session_for`, proving the
//! session wiring doesn't depend on a live tool call).
//!
//! Uses `tauri::test::mock_app` (dev-dependency `test` feature) for a real
//! `AppHandle` + managed `AppState` without any windows.

use crate::ai::agent::{AgentState, ConversationState};
use crate::ai::config::{AcpAgentConfig, AiConfig};
use crate::ai::events::AiEvent;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::Manager;
use tokio::sync::Mutex;

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
async fn full_turn_through_run_agent_turn_with_stub_agent() {
    // Hermetic sandbox + scripted stub behavior (thought chunk → message
    // chunk → end_turn).
    let _ws = tempfile::tempdir().unwrap();
    std::env::set_var(
        "LUCENT_ACP_WORKSPACE",
        _ws.path().to_string_lossy().into_owned(),
    );
    let script_dir = tempfile::tempdir().unwrap();
    let script_path = script_dir.path().join("script.json");
    std::fs::write(
        &script_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "stopReason": "end_turn",
            "steps": [
                {"notify": {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "thinking…"}}},
                {"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Hello"}}}
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let cfg = AiConfig {
        acp: Some(AcpAgentConfig {
            agent_id: "stub".into(),
            command: Some(stub_binary()),
            env: HashMap::from([(
                "STUB_SCRIPT".into(),
                script_path.to_string_lossy().into_owned(),
            )]),
            auto_deny_permissions: false,
        }),
        ..AiConfig::default()
    };

    // Tauri mock runtime: real AppHandle + managed AppState, no windows.
    let app = tauri::test::mock_app();
    let state = crate::commands::AppState::new();
    *state.ai_config.write().await = cfg;
    state.conversations.insert(
        "conv-1".into(),
        Arc::new(Mutex::new(ConversationState::new("conn-1".into()))),
    );
    app.manage(state);
    let state = app.state::<crate::commands::AppState>();

    // The IPC channel: `TauriSink` forwards every `AiEvent` here, exactly as
    // it would to the frontend.
    let received: Arc<std::sync::Mutex<Vec<AiEvent>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recv_task = received.clone();
    let channel: Channel<AiEvent> = Channel::new(move |body| {
        if let Ok(ev) = body.deserialize::<AiEvent>() {
            recv_task.lock().unwrap().push(ev);
        }
        Ok(())
    });

    crate::commands::run_agent_turn(
        &state,
        app.handle(),
        channel,
        "conv-1".into(),
        "hi".into(),
        "system preamble".into(),
    )
    .await
    .expect("the full ACP turn completes");

    // The exact event sequence the rig path would emit for this script
    // (Thinking + Text + Done).
    let events = received.lock().unwrap().clone();
    assert_eq!(events.len(), 3, "Thinking + Text + Done: {events:?}");
    assert!(matches!(&events[0], AiEvent::Thinking { content } if content == "thinking…"));
    assert!(matches!(&events[1], AiEvent::Text { content } if content == "Hello"));
    match &events[2] {
        AiEvent::Done {
            conversation_id,
            final_message,
            ..
        } => {
            // Mirrors the rig path: `DatabaseAgent::chat` keys the Done
            // event by `ConversationState.connection_id` — the ACP driver
            // keeps the same contract.
            assert_eq!(conversation_id, "conn-1");
            assert_eq!(final_message, "Hello");
        }
        other => panic!("expected Done, got {other:?}"),
    }

    // The turn released the conversation claim — follow-up messages can
    // begin (the DML-hold precondition only applies while the prompt is
    // unresolved).
    let conv = state
        .conversations
        .get("conv-1")
        .expect("conversation present");
    assert!(
        matches!(conv.lock().await.state, AgentState::Idle),
        "conversation returns to Idle after the turn"
    );

    // Session-per-conversation state is live: the session and the connection
    // task both exist for the next turn.
    assert!(
        state.acp.sessions.lock().await.contains_key("conv-1"),
        "session cached for the conversation"
    );
    assert!(
        state.acp.connections.lock().await.contains_key("stub"),
        "connection task running for the agent"
    );
}
