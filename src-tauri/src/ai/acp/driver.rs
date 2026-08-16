//! `AcpChatDriver` — the ACP sibling of `DatabaseAgent::chat` (same
//! signature, same `AgentSink`/`CancellationToken` contract). Maps
//! `SessionUpdate` notifications into the existing `AiEvent` stream and
//! resolves the prompt's `stop_reason` into the `Done` event.
//!
//! The driver is stateless across turns: everything it needs lives in the
//! `AcpState` it holds (phase D3) — one connection task per agent process,
//! one ACP session per conversation, and the permission registry. Each
//! `chat()` acquires the process, gets-or-creates the conversation's
//! session, subscribes to the event fan-out, and runs one prompt turn.

use crate::ai::acp::connection::{AgentCommand, AgentEvent};
use crate::ai::acp::AcpState;
use crate::ai::agent::{AgentDriver, AgentSink, AgentState, ConversationState};
use crate::ai::config::{AcpAgentConfig, AiConfig};
use crate::ai::events::{
    AgentPermissionOption, AgentPermissionPayload, AiEvent, TokenUsage, ToolCallInfo,
};
use crate::ai::tools::AiToolContext;
use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, SessionUpdate, StopReason, ToolCallContent, ToolCallStatus,
    ToolCallUpdate, UsageUpdate,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::oneshot;

pub struct AcpChatDriver {
    /// The shared ACP subsystem: process manager, connection tasks, bridge
    /// handles, permission registry, session map (phase D3).
    pub acp_state: AcpState,
    pub acp: AcpAgentConfig,
    pub tool_ctx: AiToolContext,
}

impl AcpChatDriver {
    pub fn new(acp_state: AcpState, acp: AcpAgentConfig, tool_ctx: AiToolContext) -> Self {
        Self {
            acp_state,
            acp,
            tool_ctx,
        }
    }

    /// One ACP turn: acquire the agent process, get-or-create the
    /// conversation's session (spawning the DB-tools bridge on first use),
    /// stream `session/update` notifications into `AiEvent`s, surface
    /// `session/request_permission` requests through the sink, and resolve
    /// the prompt response into `Done`. Follow-up messages reuse the session
    /// — real multi-turn continuity, agent-side context (spec §3 D4).
    pub async fn chat(
        &self,
        message: String,
        _config: &AiConfig,
        system_prompt: String,
        conv_state: Arc<tokio::sync::Mutex<ConversationState>>,
        sink: Arc<dyn AgentSink>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), String> {
        let conversation_id = {
            let s = conv_state.lock().await;
            s.connection_id.clone()
        };
        // Session key = the conversation key (set by `run_agent_turn`).
        // Multiple conversations share one `connection_id`, so the connection
        // id alone would merge their ACP sessions; the fallback keeps
        // direct-driver tests (which never go through `run_agent_turn`)
        // working with the key they passed to `ConversationState::new`.
        let session_key = {
            let s = conv_state.lock().await;
            s.conversation_id
                .clone()
                .unwrap_or_else(|| s.connection_id.clone())
        };
        let process = self
            .acp_state
            .manager
            .ensure_process(&self.acp.agent_id, &self.acp)
            .await?;

        // Session-per-conversation: one ACP session for the conversation's
        // lifetime; follow-ups reuse it (the session map keys it by
        // conversation id). `session_for` also spawns the bridge listener
        // and delivers the bridge config via session/new's mcpServers.
        let session = self
            .acp_state
            .session_for(&session_key, &process, &self.tool_ctx, &sink)
            .await?;
        let conn = self.acp_state.ensure_connection(&process).await?;
        let mut events_rx = conn.events.subscribe();

        // v1 has no system-prompt param — the preamble is prepended to the
        // first user message of a session only (spec §4.4).
        // Lucent's 4 database tools (search_schema, get_objects_info, run_readonly_query, preview_dml)
        // are available via native MCP or through `./lucent-tool <tool> '<args>'`
        // in the session sandbox workspace.
        let first_prompt = session.first_prompt.swap(false, Ordering::SeqCst);
        let prompt_text = if first_prompt {
            let acp_tool_guidance = "\n\nDATABASE TOOLS IN ACP (via MCP):\n\
                 You have access to Lucent's database MCP server (lucent-db-tools) providing: search_schema, get_objects_info, run_readonly_query, preview_dml.\n\
                 CRITICAL INSTRUCTIONS:\n\
                 - ALWAYS use `run_readonly_query` to query the connected database, and `search_schema` / `get_objects_info` to inspect schemas and tables.\n\
                 - NEVER attempt to inspect or open local database files (such as .duckdb, .sqlite, .db) from disk using bash or python — all database queries must go through the database tools to reach the user's active database connection.\n\
                 - If your runtime lists MCP tools, invoke `search_schema`, `get_objects_info`, `run_readonly_query`, `preview_dml` as native tool calls. If your runtime runs in a bash-only harness, invoke `./lucent-tool <tool_name> '<json_arguments>'` from your current directory.\n\
                 Both methods execute directly against the live database through Lucent.".to_string();
            format!("{system_prompt}{acp_tool_guidance}\n\n{message}")
        } else {
            message
        };
        let (reply_tx, mut reply_rx) = oneshot::channel();
        conn.cmds
            .send(AgentCommand::Prompt {
                session_id: session.session_id.clone(),
                text: prompt_text,
                reply: reply_tx,
            })
            .await
            .map_err(|e| format!("agent connection closed before session/prompt: {e}"))?;

        let mut text_buf = String::new();
        let mut usage = TokenUsage::default();
        // ToolCall → ToolResult name tracking (ACP carries the name on the
        // ToolCall; ToolCallUpdate only carries the id).
        let mut tool_names: HashMap<String, String> = HashMap::new();
        let mut cancel_sent = false;
        // After cancel, the agent must answer within this deadline or the
        // process is killed (the error path drops the connection, which ends
        // the connection task and its process-group guard — the same
        // cancel-then-kill fallback the rig path uses).
        let mut cancel_deadline: Option<tokio::time::Instant> = None;

        let reply: Result<super::connection::PromptOutcome, String> = loop {
            tokio::select! {
                r = &mut reply_rx => {
                    break r.unwrap_or_else(|_| {
                        // Phase F: the agent process died mid-turn — carry
                        // its stderr tail in the error so the user sees the
                        // crash evidence (spec §4.3).
                        Err(format!(
                            "agent connection closed before the prompt resolved — last lines of agent stderr: {}",
                            process.stderr_snippet()
                        ))
                    });
                }
                ev = events_rx.recv() => {
                    match ev {
                        Ok(ev) => self.dispatch_event(
                            ev, &session.session_id, &session_key,
                            &mut text_buf, &mut usage, &mut tool_names, &sink,
                        ).await,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // Dropped events (slow consumer) — keep going;
                            // the prompt reply still carries the outcome.
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            return Err(format!(
                                "agent connection closed mid-turn — last lines of agent stderr: {}",
                                process.stderr_snippet()
                            ));
                        }
                    }
                }
                _ = cancel.cancelled(), if !cancel_sent => {
                    // Normative order (schema doc on
                    // RequestPermissionOutcome::Cancelled): resolve every
                    // pending permission request with Cancelled BEFORE the
                    // CancelNotification, or the agent can stay blocked
                    // awaiting permission resolution.
                    cancel_sent = true;
                    self.acp_state
                        .permissions
                        .drain_cancelled(&session.session_id)
                        .await;
                    let _ = conn
                        .cmds
                        .send(AgentCommand::Cancel {
                            session_id: session.session_id.clone(),
                        })
                        .await;
                    cancel_deadline = Some(
                        tokio::time::Instant::now() + std::time::Duration::from_secs(5),
                    );
                }
                _ = wait_until(cancel_deadline), if cancel_sent => {
                    return Err(
                        "Agent didn't respond to cancellation within 5s — killed it. Start a new conversation.".into()
                    );
                }
            }
        };

        // The stub emits every notification before the prompt response, so
        // anything still buffered belongs before `Done`.
        loop {
            match events_rx.try_recv() {
                Ok(ev) => {
                    self.dispatch_event(
                        ev,
                        &session.session_id,
                        &session_key,
                        &mut text_buf,
                        &mut usage,
                        &mut tool_names,
                        &sink,
                    )
                    .await;
                }
                Err(_) => break,
            }
        }

        let outcome = reply.map_err(|e| {
            // Phase F: surface the agent's stderr tail with the prompt error
            // (the connection task ends on a transport error, so this is the
            // common crash path).
            format!(
                "session/prompt failed: {e} — last lines of agent stderr: {}",
                process.stderr_snippet()
            )
        })?;
        let final_message = match outcome.stop_reason {
            StopReason::EndTurn => text_buf.clone(),
            StopReason::MaxTokens | StopReason::MaxTurnRequests => {
                "Reached maximum turns.".to_string()
            }
            // Refusal and Cancelled both end the turn with whatever was
            // streamed; unknown (non_exhaustive) reasons fall back the same
            // way — never fatal.
            StopReason::Refusal | StopReason::Cancelled => text_buf.clone(),
            _ => text_buf.clone(),
        };
        sink.event(AiEvent::Done {
            conversation_id: conversation_id.clone(),
            final_message,
            usage,
        });

        // Release the conversation claim (mirrors `DatabaseAgent::chat`'s
        // tail). The DML-hold precondition holds while the bridge keeps
        // `preview_dml` open: the prompt does not resolve, so the claim is
        // not released until the user answers.
        conv_state.lock().await.state = AgentState::Idle;
        Ok(())
    }

    async fn dispatch_event(
        &self,
        ev: AgentEvent,
        session_id: &str,
        conversation_id: &str,
        text_buf: &mut String,
        usage: &mut TokenUsage,
        tool_names: &mut HashMap<String, String>,
        sink: &Arc<dyn AgentSink>,
    ) {
        match ev {
            AgentEvent::SessionUpdate {
                session_id: sid,
                update,
            } if sid == session_id => {
                if let SessionUpdate::AgentMessageChunk(chunk)
                | SessionUpdate::UserMessageChunk(chunk) = &update
                {
                    if let Some(t) = chunk_text(chunk) {
                        text_buf.push_str(&t);
                    }
                }
                if let SessionUpdate::UsageUpdate(u) = &update {
                    accumulate_usage(usage, u);
                }
                if let Some(mut event) = map_update(&update) {
                    // Enrich ToolResult with the name tracked from the
                    // ToolCall (the rig path always fills it; the frontend
                    // keys cards by id, but the payload contract matches).
                    if let AiEvent::ToolResult { id, tool, .. } = &mut event {
                        if tool.is_empty() {
                            if let Some(name) = tool_names.get(id) {
                                *tool = name.clone();
                            }
                        }
                    }
                    if let AiEvent::ToolCalls { tools } = &event {
                        for t in tools {
                            tool_names.insert(t.id.clone(), t.name.clone());
                        }
                    }
                    sink.event(event);
                }
            }
            AgentEvent::PermissionRequest {
                session_id: sid,
                request,
            } if sid == session_id => {
                // Spec §3 D6: agent permission requests are always surfaced,
                // never auto-granted. `auto_deny_permissions` rejects without
                // a dialog. The connection task's responder stays parked on
                // the registry FIFO until `respond_agent_permission` (or the
                // cancel drain) resolves it.
                let payload = AgentPermissionPayload {
                    conversation_id: conversation_id.to_string(),
                    title: request.tool_call.fields.title.clone().unwrap_or_else(|| {
                        request
                            .tool_call
                            .fields
                            .kind
                            .as_ref()
                            .map(|k| format!("{k:?}"))
                            .unwrap_or_else(|| "Permission requested".into())
                    }),
                    description: tool_result_text(&request.tool_call),
                    options: request
                        .options
                        .iter()
                        .map(|o| AgentPermissionOption {
                            id: o.option_id.to_string(),
                            name: o.name.clone(),
                        })
                        .collect(),
                };
                if self.acp.auto_deny_permissions {
                    let _ = self.acp_state.permissions.respond(&sid, false).await;
                } else {
                    sink.permission_request(payload);
                }
            }
            _ => {}
        }
    }
}

#[async_trait::async_trait]
impl AgentDriver for AcpChatDriver {
    /// The seam `pick_driver` returns — same signature as `DatabaseAgent`,
    /// same sink/cancel contract (spec §3 D3). Pure delegation to `chat`.
    async fn chat(
        &self,
        message: String,
        config: &AiConfig,
        system_prompt: String,
        conv_state: Arc<tokio::sync::Mutex<ConversationState>>,
        sink: Arc<dyn AgentSink>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), String> {
        self.chat(message, config, system_prompt, conv_state, sink, cancel)
            .await
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Maps one `SessionUpdate` to the `AiEvent` it should emit. `None` for
/// variants that don't produce an event (pending/in-progress tool re-renders
/// update in place on the frontend; usage is accumulated separately; unknown
/// variants are logged and ignored — `SessionUpdate` is `#[non_exhaustive]`).
pub fn map_update(update: &SessionUpdate) -> Option<AiEvent> {
    use SessionUpdate::*;
    match update {
        AgentThoughtChunk(chunk) => chunk_text(chunk).map(|t| AiEvent::Thinking { content: t }),
        AgentMessageChunk(chunk) | UserMessageChunk(chunk) => {
            chunk_text(chunk).map(|t| AiEvent::Text { content: t })
        }
        ToolCall(tc) => Some(AiEvent::ToolCalls {
            tools: vec![ToolCallInfo {
                id: tc.tool_call_id.to_string(),
                // ACP has no tool "name" on ToolCall — the title is the
                // human-readable name the frontend renders (schema 1.4.0).
                name: tc.title.clone(),
                args: tc.raw_input.clone().unwrap_or(serde_json::Value::Null),
            }],
        }),
        ToolCallUpdate(tu) => match tu.fields.status {
            Some(ToolCallStatus::Completed) => Some(AiEvent::ToolResult {
                id: tu.tool_call_id.to_string(),
                tool: String::new(), // filled by chat()'s name tracking
                summary: tool_result_text(tu),
                output: None,
            }),
            // pending/in_progress re-renders: the frontend updates the card
            // in place; a failed call has no result payload in v1.
            _ => None,
        },
        _ => {
            log::debug!("ignoring session update variant: {update:?}");
            None
        }
    }
}

/// Extracts the text of a content chunk, `None` for non-text blocks
/// (images, audio, resources).
pub fn chunk_text(chunk: &ContentChunk) -> Option<String> {
    match &chunk.content {
        ContentBlock::Text(t) => Some(t.text.clone()),
        _ => None,
    }
}

/// The text summary of a completed tool call: concatenated text content
/// blocks. Non-text content (diffs, terminals) is skipped.
pub fn tool_result_text(tu: &ToolCallUpdate) -> String {
    let mut parts = Vec::new();
    if let Some(contents) = &tu.fields.content {
        for c in contents {
            if let ToolCallContent::Content(content) = c {
                if let ContentBlock::Text(t) = &content.content {
                    if !t.text.is_empty() {
                        parts.push(t.text.clone());
                    }
                }
            }
        }
    }
    parts.join("\n")
}

/// The honest first-prompt preamble when the agent never connected Lucent's
/// DB-tool bridge: no tool claims (the model must not promise tools it
/// doesn't have) and a graceful fallback — SQL the user can run in Lucent's
/// query editor. Replaces the real system prompt for the first turn.
pub fn no_tools_preamble(agent_id: &str) -> String {
    format!(
        "You are connected to a database through Lucent, a database client.\n\n\
         Lucent provides four database tools (search_schema, get_objects_info, \
         run_readonly_query, preview_dml) through its own MCP tool server. Your agent \
         runtime ({agent_id}) did not connect to that server, so THOSE TOOLS ARE NOT \
         AVAILABLE in this session — they are not in your toolset and you cannot call them.\n\n\
         RULES:\n\
         - Do not claim to have these tools, and do not attempt to call them.\n\
         - If the user asks about the database, write SQL they can run in Lucent's query \
           editor and explain what it does; describe which queries would answer their question.\n\
         - Never fabricate query results or schema details you have not seen."
    )
}

/// The agent's sandbox root for a conversation:
/// `~/.lucent/agent-workspace/<agent>/<conversation>/` (spec §D7).
/// `LUCENT_ACP_WORKSPACE` overrides the base so tests stay hermetic.
pub fn workspace_dir(agent_id: &str, conversation_id: &str) -> Result<PathBuf, String> {
    let base = std::env::var("LUCENT_ACP_WORKSPACE")
        .or_else(|_| std::env::var("HOME").map_err(|_| "HOME not set".to_string()))?;
    Ok(PathBuf::from(base)
        .join(".lucent")
        .join("agent-workspace")
        .join(sanitize_segment(agent_id))
        .join(sanitize_segment(conversation_id)))
}

/// Path-segment sanitizer: conversation ids come from the frontend and
/// agent ids from the registry — keep only safe characters so a hostile id
/// can't escape the sandbox root.
fn sanitize_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Accumulates a `UsageUpdate` into the `TokenUsage` the Done event carries.
/// v1's numbers are context-window occupancy (`used` of `size`) — the
/// prompt/completion split is not observable, so `used` lands in
/// prompt_tokens and completion stays 0; documented as approximate in the
/// UI tooltip.
pub fn accumulate_usage(usage: &mut TokenUsage, u: &UsageUpdate) {
    usage.prompt_tokens = u.used.min(u32::MAX as u64) as u32;
    usage.cached_prompt_tokens = 0;
}

/// Pending-forever future until `deadline` (used by the post-cancel kill
/// deadline in `chat()`); immediately ready when there is no deadline.
async fn wait_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::agent::CollectorSink;
    use agent_client_protocol::schema::v1::{
        ContentBlock, ToolCall, ToolCallStatus, ToolCallUpdate,
    };
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::sync::Mutex as AsyncMutex;

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

    fn hermetic_workspace() -> tempfile::TempDir {
        // Point the agent sandbox at a tempdir so tests never write into
        // the real ~/.lucent (create_dir_all recreates it on demand). Kept
        // alive for the test duration by the binding.
        let dir = tempdir().unwrap();
        std::env::set_var(
            "LUCENT_ACP_WORKSPACE",
            dir.path().to_string_lossy().into_owned(),
        );
        dir
    }

    fn script_file(steps: serde_json::Value) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
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

    fn assert_sequence(events: &[AiEvent], expected: Vec<AiEvent>) {
        assert_eq!(events.len(), expected.len(), "event count: {events:?}");
        for (got, want) in events.iter().zip(expected.iter()) {
            assert_eq!(got, want);
        }
    }

    // ── Pure mapping tests ──

    #[test]
    fn map_update_maps_thought_and_message_chunks() {
        let thought = SessionUpdate::AgentThoughtChunk(ContentChunk::new(ContentBlock::Text(
            agent_client_protocol::schema::v1::TextContent::new("thinking…"),
        )));
        assert_eq!(
            map_update(&thought),
            Some(AiEvent::Thinking {
                content: "thinking…".into()
            })
        );

        let msg = SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
            agent_client_protocol::schema::v1::TextContent::new("Hel"),
        )));
        assert_eq!(
            map_update(&msg),
            Some(AiEvent::Text {
                content: "Hel".into()
            })
        );
    }

    #[test]
    fn map_update_maps_tool_call_to_tool_calls() {
        let tc = ToolCall::new("tc1", "search_schema").raw_input(json!({"query": "users"}));
        let got = map_update(&SessionUpdate::ToolCall(tc)).expect("ToolCall maps");
        match got {
            AiEvent::ToolCalls { tools } => {
                assert_eq!(tools.len(), 1);
                assert_eq!(tools[0].id, "tc1");
                assert_eq!(tools[0].name, "search_schema");
                assert_eq!(tools[0].args, json!({"query": "users"}));
            }
            other => panic!("expected ToolCalls, got {other:?}"),
        }
    }

    #[test]
    fn map_update_maps_completed_tool_call_update_only() {
        use agent_client_protocol::schema::v1::{Content, ToolCallContent, ToolCallUpdateFields};
        let completed = ToolCallUpdate::new(
            "tc1",
            ToolCallUpdateFields::new()
                .status(ToolCallStatus::Completed)
                .content(vec![ToolCallContent::Content(Content::new(
                    ContentBlock::Text(agent_client_protocol::schema::v1::TextContent::new(
                        "found 2 tables",
                    )),
                ))]),
        );
        assert_eq!(
            map_update(&SessionUpdate::ToolCallUpdate(completed)),
            Some(AiEvent::ToolResult {
                id: "tc1".into(),
                tool: String::new(),
                summary: "found 2 tables".into(),
                output: None,
            })
        );

        let pending = ToolCallUpdate::new(
            "tc1",
            ToolCallUpdateFields::new().status(ToolCallStatus::InProgress),
        );
        assert_eq!(map_update(&SessionUpdate::ToolCallUpdate(pending)), None);
    }

    #[test]
    fn map_update_ignores_unknown_variants() {
        // Plan / AvailableCommandsUpdate / ConfigOptionUpdate / SessionInfoUpdate
        // and any future variant: logged, never fatal.
        let plan = SessionUpdate::Plan(agent_client_protocol::schema::v1::Plan::new(vec![]));
        assert_eq!(map_update(&plan), None);
    }

    // ── Lifecycle tests against the stub ──

    #[tokio::test]
    async fn maps_scripted_updates_to_events() {
        let _ws = hermetic_workspace();
        let script = script_file(json!({
            "stopReason": "end_turn",
            "steps": [
                {"notify": {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "thinking…"}}},
                {"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Hel"}}},
                {"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "lo"}}},
                {"notify": {"sessionUpdate": "tool_call", "toolCallId": "tc1", "title": "search_schema", "rawInput": {"query": "users"}}},
                {"notify": {"sessionUpdate": "tool_call_update", "toolCallId": "tc1", "status": "completed", "content": [{"type": "content", "content": {"type": "text", "text": "found 2 tables"}}]}},
                {"notify": {"sessionUpdate": "usage_update", "used": 100, "size": 200000}}
            ]
        }));

        let acp_state = AcpState::new();
        let driver = AcpChatDriver::new(
            acp_state,
            acp_cfg(Some(&script.path().join("script.json"))),
            tool_ctx(),
        );
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let conv = conversation("conv-1");

        driver
            .chat(
                "find user tables".into(),
                &AiConfig::default(),
                "system preamble".into(),
                conv.clone(),
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("scripted turn completes");

        let events = sink.0.lock().unwrap().clone();
        assert_sequence(
            &events,
            vec![
                AiEvent::Thinking {
                    content: "thinking…".into(),
                },
                AiEvent::Text {
                    content: "Hel".into(),
                },
                AiEvent::Text {
                    content: "lo".into(),
                },
                AiEvent::ToolCalls {
                    tools: vec![ToolCallInfo {
                        id: "tc1".into(),
                        name: "search_schema".into(),
                        args: json!({"query": "users"}),
                    }],
                },
                AiEvent::ToolResult {
                    id: "tc1".into(),
                    tool: "search_schema".into(), // enriched from the ToolCall
                    summary: "found 2 tables".into(),
                    output: None,
                },
                AiEvent::Done {
                    conversation_id: "conv-1".into(),
                    final_message: "Hello".into(),
                    usage: TokenUsage {
                        prompt_tokens: 100,
                        completion_tokens: 0,
                        cached_prompt_tokens: 0,
                    },
                },
            ],
        );
    }

    #[tokio::test]
    async fn unknown_session_update_variants_are_ignored() {
        let _ws = hermetic_workspace();
        let script = script_file(json!({
            "stopReason": "end_turn",
            "steps": [
                {"notify": {"sessionUpdate": "plan"}},
                {"notify": {"sessionUpdate": "available_commands_update", "commands": []}},
                {"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "still here"}}}
            ]
        }));

        let acp_state = AcpState::new();
        let driver = AcpChatDriver::new(
            acp_state,
            acp_cfg(Some(&script.path().join("script.json"))),
            tool_ctx(),
        );
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let conv = conversation("conv-2");

        driver
            .chat(
                "hi".into(),
                &AiConfig::default(),
                "preamble".into(),
                conv.clone(),
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("turn completes despite unknown variants");

        let events = sink.0.lock().unwrap().clone();
        assert_eq!(events.len(), 2, "Text + Done: {events:?}");
        assert!(matches!(&events[0], AiEvent::Text { content } if content == "still here"));
        assert!(matches!(&events[1], AiEvent::Done { .. }));
    }

    #[tokio::test]
    async fn first_turn_prompt_carries_acp_tool_guidance() {
        let _ws = hermetic_workspace();
        let script = script_file(json!({
            "stopReason": "end_turn",
            "steps": [
                {"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "ok"}}}
            ]
        }));

        let acp_state = AcpState::new();
        let mut cfg = acp_cfg(Some(&script.path().join("script.json")));
        cfg.agent_id = "pi-acp".into();
        let driver = AcpChatDriver::new(acp_state.clone(), cfg.clone(), tool_ctx());
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let conv = conversation("conv-4");

        driver
            .chat(
                "hi".into(),
                &AiConfig::default(),
                "system preamble with schema context".into(),
                conv,
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("turn completes");

        let process = acp_state
            .manager
            .ensure_process(&cfg.agent_id, &cfg)
            .await
            .expect("cached process");
        let stderr = process.stderr_snippet();
        assert!(
            stderr.contains("DATABASE TOOLS IN ACP"),
            "ACP tool guidance reached the agent: {stderr:?}"
        );
        assert!(
            stderr.contains("system preamble with schema context"),
            "the system prompt reached the agent: {stderr:?}"
        );

        let events = sink.0.lock().unwrap().clone();
        assert_eq!(events.len(), 2, "Text + Done: {events:?}");
        assert!(matches!(&events[0], AiEvent::Text { content } if content == "ok"));
        assert!(matches!(&events[1], AiEvent::Done { .. }));
    }

    #[tokio::test]
    async fn followup_prompts_send_message_directly() {
        let _ws = hermetic_workspace();
        let script = script_file(json!({
            "stopReason": "end_turn",
            "steps": [
                {"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "first"}}}
            ]
        }));

        let acp_state = AcpState::new();
        let mut cfg = acp_cfg(Some(&script.path().join("script.json")));
        cfg.agent_id = "pi-acp".into();
        let driver = AcpChatDriver::new(acp_state.clone(), cfg.clone(), tool_ctx());
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let conv = conversation("conv-5");

        driver
            .chat(
                "first msg".into(),
                &AiConfig::default(),
                "system preamble".into(),
                conv.clone(),
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("first turn");
        driver
            .chat(
                "second msg".into(),
                &AiConfig::default(),
                "system preamble".into(),
                conv.clone(),
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("second turn (same session)");

        let process = acp_state
            .manager
            .ensure_process(&cfg.agent_id, &cfg)
            .await
            .expect("cached process");
        let stderr = process.stderr_snippet();
        assert!(
            stderr.contains("second msg"),
            "follow-up message reached the agent: {stderr:?}"
        );
    }

    #[tokio::test]
    async fn stop_reason_max_turn_requests_maps_to_done_message() {
        let _ws = hermetic_workspace();
        let script = script_file(json!({
            "stopReason": "max_turn_requests",
            "steps": [
                {"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "partial"}}}
            ]
        }));

        let acp_state = AcpState::new();
        let driver = AcpChatDriver::new(
            acp_state,
            acp_cfg(Some(&script.path().join("script.json"))),
            tool_ctx(),
        );
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let conv = conversation("conv-3");

        driver
            .chat(
                "hi".into(),
                &AiConfig::default(),
                "preamble".into(),
                conv,
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
            )
            .await
            .expect("turn completes");

        let events = sink.0.lock().unwrap().clone();
        let done = events
            .iter()
            .find(|e| matches!(e, AiEvent::Done { .. }))
            .expect("Done present");
        match done {
            AiEvent::Done { final_message, .. } => {
                assert_eq!(final_message, "Reached maximum turns.");
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn cancellation_resolves_pending_permissions_before_cancel_notification() {
        let _ws = hermetic_workspace();
        // The stub emits a permission request mid-turn and then waits for
        // the client's response before finishing. The client (driver) must
        // answer with Cancelled (normative order) when the token cancels.
        let script = script_file(json!({
            "stopReason": "end_turn",
            "steps": [
                {"permission": {"title": "Read ~/.zshrc", "options": [{"optionId": "allow_once", "name": "Allow once", "kind": "allow_once"}]}},
                {"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "after permission"}}}
            ]
        }));

        let acp_state = AcpState::new();
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let conv = conversation("conv-4");
        let cancel = tokio_util::sync::CancellationToken::new();

        // Cancel shortly after the turn starts — the stub's permission
        // request parks the connection task's responder until the driver
        // resolves it. The driver is built inside the task (it borrows the
        // script dir, which is moved in).
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
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        cancel.cancel();
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .expect("turn finishes after cancel")
            .expect("task did not panic");

        // The stub responds with stopReason "cancelled" after the pending
        // permission is answered — if the driver sent CancelNotification
        // before resolving it, the stub would still be blocked waiting.
        assert!(result.is_ok(), "turn resolves: {result:?}");

        // Normative contract, verified on the wire: the pending permission
        // was resolved with `Cancelled` (the stub logs the outcome it
        // received on stderr, which lands in the process stderr tail).
        let stderr = acp_state
            .manager
            .processes
            .lock()
            .unwrap()
            .get("stub")
            .expect("process record exists")
            .stderr_snippet();
        assert!(
            stderr.contains("permission outcome") && stderr.contains("cancelled"),
            "permission resolved with Cancelled before cancel: {stderr:?}"
        );

        let events = sink.0.lock().unwrap().clone();
        let done = events
            .iter()
            .find(|e| matches!(e, AiEvent::Done { .. }))
            .expect("Done present after cancel");
        match done {
            AiEvent::Done { final_message, .. } => {
                // The turn resolves with stopReason "cancelled" — the
                // accumulated streamed text is kept (plan: Cancelled →
                // text_buf), so the final message is whatever the agent
                // emitted after the permission resolved.
                assert_eq!(final_message, "after permission");
            }
            _ => unreachable!(),
        }
    }
}
