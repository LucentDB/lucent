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
use crate::ai::acp::correlator::CorrelatorState;
use crate::ai::acp::AcpState;
use crate::ai::agent::{AgentDriver, AgentSink, AgentState, ConversationState};
use crate::ai::config::{AcpAgentConfig, AiConfig};
use crate::ai::events::{
    AgentPermissionOption, AgentPermissionPayload, AiEvent, TokenUsage, ToolCallInfo,
    ToolResultStatus,
};
use crate::ai::tools::AiToolContext;
use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, SessionUpdate, StopReason, ToolCallContent, ToolCallStatus,
    ToolCallUpdate, Usage, UsageUpdate,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
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
    #[allow(clippy::too_many_arguments)] // AgentDriver seam signature
    pub async fn chat(
        &self,
        message: String,
        _config: &AiConfig,
        system_prompt: String,
        conv_state: Arc<tokio::sync::Mutex<ConversationState>>,
        sink: Arc<dyn AgentSink>,
        cancel: tokio_util::sync::CancellationToken,
        applied_memory_count: usize,
    ) -> Result<(), String> {
        let conversation_id = {
            let s = conv_state.lock().await;
            s.conversation_id
                .clone()
                .unwrap_or_else(|| s.connection_id.clone())
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

        let first_prompt = session.first_prompt.swap(false, Ordering::SeqCst);
        // ACP delivers the system prompt — and therefore its memory block — only
        // on a session's first turn; a follow-up sends just the user message.
        // Report the count only when the rules were actually delivered (F-C2).
        let delivered_memory_count = if first_prompt {
            applied_memory_count
        } else {
            0
        };
        let mut notice: Option<String> = None;
        let prompt_text = if first_prompt {
            // Spec D4: the preamble only claims DB tools the agent actually
            // connected (ground truth = the bridge hello). Wait once, decide once —
            // v1 prepends the preamble to the first user message only.
            let tools_ok = if mcp_hopeless(&self.acp.agent_id) {
                // A curated-unsupported agent accepts `mcpServers` and drops it,
                // so the handshake will never arrive: waiting the full gate
                // stalls the first turn of every conversation for nothing.
                false
            } else {
                session.tools.wait_connected(tools_gate_timeout()).await
            };
            if !tools_ok && !session.tools_notice.swap(true, Ordering::SeqCst) {
                notice = Some(cli_bridge_notice());
            }
            first_prompt_text(
                &system_prompt,
                &message,
                &self.acp.agent_id,
                tools_ok,
                tool_helper_path(&self.acp.agent_id, &session_key),
            )
        } else {
            message
        };
        if let Some(content) = notice {
            sink.event(AiEvent::Notice { content });
        }
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

        let reply: Result<super::connection::PromptOutcome, super::connection::PromptError> = loop {
            tokio::select! {
                r = &mut reply_rx => {
                    break r.unwrap_or_else(|_| {
                        // Phase F: the agent process died mid-turn — carry
                        // its stderr tail in the error so the user sees the
                        // crash evidence (spec §4.3).
                        Err(super::connection::PromptError::Transport(format!(
                            "agent connection closed before the prompt resolved — last lines of agent stderr: {}",
                            process.stderr_snippet()
                        )))
                    });
                }
                ev = events_rx.recv() => {
                    match ev {
                        Ok(ev) => self.dispatch_event(
                            ev, &session.session_id, &session_key,
                            &mut text_buf, &mut usage, &mut tool_names, &session.correlator, &sink,
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
                    cancel_deadline = Some(tokio::time::Instant::now() + cancel_kill_timeout());
                }
                _ = wait_until(cancel_deadline), if cancel_sent => {
                    // The agent ignored session/cancel — kill it for real: abort the
                    // connection task (Client drop closes stdio; the agent exits on EOF),
                    // charge the restart budget, and evict its sessions.
                    self.acp_state.kill_agent(&self.acp.agent_id).await;
                    return Err(
                        "The agent didn't respond to cancellation — Lucent restarted it. Other conversations with this agent were interrupted; send your message again."
                            .into(),
                    );
                }
            }
        };

        // The stub emits every notification before the prompt response, so
        // anything still buffered belongs before `Done`.
        while let Ok(ev) = events_rx.try_recv() {
            self.dispatch_event(
                ev,
                &session.session_id,
                &session_key,
                &mut text_buf,
                &mut usage,
                &mut tool_names,
                &session.correlator,
                &sink,
            )
            .await;
        }

        let outcome = match reply {
            Ok(outcome) => outcome,
            Err(super::connection::PromptError::Rpc(msg)) => {
                // The agent rejected the turn (typically: it no longer knows this
                // conversation's session). Evict the session so the next turn
                // transparently recreates it, and tell the user the truth. The
                // connection itself survives (a peer error is not a crash).
                self.acp_state.drop_session(&session_key).await;
                return Err(format!(
                    "The agent lost this conversation's session ({msg}). Send your message again to start a fresh session."
                ));
            }
            Err(super::connection::PromptError::Transport(e)) => {
                return Err(format!(
                    "session/prompt failed: {e} — last lines of agent stderr: {}",
                    process.stderr_snippet()
                ));
            }
        };
        // The agent's end-of-turn usage is the authoritative prompt/completion
        // split; it lands here (after the last `usage_update` drained above) so
        // it wins over the occupancy fallback. Agents that report none keep the
        // fallback numbers.
        if let Some(turn_usage) = outcome.usage.as_ref() {
            if !end_turn_usage_is_empty(turn_usage) {
                apply_end_turn_usage(&mut usage, turn_usage);
            }
        }
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
            applied_memory_count: delivered_memory_count,
            cancelled: matches!(outcome.stop_reason, StopReason::Cancelled),
        });

        // Release the conversation claim (mirrors `DatabaseAgent::chat`'s
        // tail). The DML-hold precondition holds while the bridge keeps
        // `preview_dml` open: the prompt does not resolve, so the claim is
        // not released until the user answers.
        session.correlator.clear();
        conv_state.lock().await.state = AgentState::Idle;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // private fan-out helper; grouping would obscure the call sites
    async fn dispatch_event(
        &self,
        ev: AgentEvent,
        session_id: &str,
        conversation_id: &str,
        text_buf: &mut String,
        usage: &mut TokenUsage,
        tool_names: &mut HashMap<String, String>,
        correlator: &Arc<CorrelatorState>,
        sink: &Arc<dyn AgentSink>,
    ) {
        match ev {
            AgentEvent::SessionUpdate {
                session_id: sid,
                update,
            } if sid == session_id => {
                if let SessionUpdate::AgentMessageChunk(chunk) = &update {
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
                    if let AiEvent::ToolResult {
                        id,
                        tool,
                        summary,
                        output,
                        input,
                        ..
                    } = &mut event
                    {
                        if tool.is_empty() {
                            if let Some(name) = tool_names.get(id) {
                                *tool = name.clone();
                            }
                        }
                        // Bridge correlation (spec D3): the structured payload from the
                        // bridge round-trip (buffered under the MCP call id) is attached to
                        // the agent's own tool_call_id, so the UI card renders the grid.
                        if output.is_none() {
                            if let Some(buffered) = correlator.pop_for(tool) {
                                // The name the bridge executed is the truthful
                                // one. On the CLI path the agent's is a shell
                                // command, and consumers key off this field:
                                // the notebook reads `run_readonly_query`'s
                                // arguments from it to populate the AI Table.
                                *tool = buffered.tool;
                                // The bridge's summary wins over the agent's own:
                                // a CLI-path agent reports the tool's stdout — the
                                // Markdown row preview — and the structured payload
                                // below renders those same rows as a grid. The card
                                // header takes the short form ("10 rows"), never the
                                // preview text.
                                *summary = buffered.summary;
                                *output = Some(buffered.output);
                                // The agent's report of a CLI-path call carries no
                                // structured arguments (it ran a shell command), so
                                // the bridge's are the only real ones.
                                if input.is_none() {
                                    *input = buffered.input;
                                }
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
        applied_memory_count: usize,
    ) -> Result<(), String> {
        self.chat(
            message,
            config,
            system_prompt,
            conv_state,
            sink,
            cancel,
            applied_memory_count,
        )
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
        AgentMessageChunk(chunk) => chunk_text(chunk).map(|t| AiEvent::Text { content: t }),
        // The agent's echo of the user's own message (v1) — the client already
        // renders the user's text; it must not appear in the assistant stream.
        UserMessageChunk(_) => None,
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
                // Agents that revise the call's raw input on the update carry
                // it here; the rest are backfilled from the bridge in chat().
                input: tu.fields.raw_input.clone(),
                status: ToolResultStatus::Completed,
            }),
            // A failed call has no result payload in v1, but it MUST surface as a
            // failure — the card renders the error state (spec D7).
            Some(ToolCallStatus::Failed) => {
                let text = tool_result_text(tu);
                Some(AiEvent::ToolResult {
                    id: tu.tool_call_id.to_string(),
                    tool: String::new(),
                    summary: if text.is_empty() {
                        "Tool call failed".to_string()
                    } else {
                        text
                    },
                    output: None,
                    input: tu.fields.raw_input.clone(),
                    status: ToolResultStatus::Failed,
                })
            }
            // pending/in_progress re-renders: the frontend updates the card in place.
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

/// The first-prompt tools guidance, prepended when the agent connected
/// Lucent's DB-tool bridge (spec D4).
fn acp_tool_guidance() -> String {
    "\n\nDATABASE TOOLS IN ACP (via MCP):\n\
     You have access to Lucent's database MCP server (lucent-db-tools) providing: search_schema, get_objects_info, run_readonly_query, preview_dml.\n\
     CRITICAL INSTRUCTIONS:\n\
     - ALWAYS use `run_readonly_query` to query the connected database, and `search_schema` / `get_objects_info` to inspect schemas and tables.\n\
     - NEVER attempt to inspect or open local database files (such as .duckdb, .sqlite, .db) from disk using bash or python — all database queries must go through the database tools to reach the user's active database connection.\n\
     - If your runtime lists MCP tools, invoke `search_schema`, `get_objects_info`, `run_readonly_query`, `preview_dml` as native tool calls. If your runtime runs in a bash-only harness, invoke `./lucent-tool <tool_name> '<json_arguments>'` from your current directory.\n\
     Both methods execute directly against the live database through Lucent."
        .to_string()
}

/// The fallback guidance when the agent runtime has not connected MCP over
/// stdio. Informs the model that Lucent has provisioned the `lucent-tool` CLI
/// helper in its workspace, with the exact argument shape of every tool.
///
/// The schemas matter here in a way they don't on the MCP path: `tools/list`
/// carries them for a native client, but a CLI caller has nothing to read, and
/// an agent left to guess `{"sql": ...}` burns a turn finding out.
fn acp_fallback_tool_guidance(agent_id: &str, tool_path: Option<String>) -> String {
    // The `./`-relative form is the ergonomic one, but it breaks the moment the
    // agent changes directory — so name the absolute path too.
    let absolute = tool_path
        .map(|p| format!("Absolute path (works from any directory): `{p}`\n"))
        .unwrap_or_default();
    format!(
        "\n\nDATABASE TOOLS IN ACP:\n\
         Your agent runtime ({agent_id}) has not connected Lucent's MCP server over stdio, so Lucent has provisioned an equivalent CLI helper in your working directory. It reaches the same live database through the same guardrails:\n\
         - `./lucent-tool <tool_name> '<json_arguments>'` (or `lucent-tool.cmd` on Windows)\n\
         {absolute}\n\
         TOOLS AND THEIR ARGUMENTS:\n\
         - `./lucent-tool search_schema '{{\"query\":\"unpaid invoices\",\"mode\":\"hybrid\"}}'` — find tables/columns by meaning (`semantic`), by name (`keyword`), or both (`hybrid`, the default).\n\
         - `./lucent-tool get_objects_info '{{\"objects\":[{{\"schema\":\"public\",\"kind\":\"table\",\"name\":\"orders\"}}]}}'` — columns, types, constraints.\n\
         - `./lucent-tool run_readonly_query '{{\"sql\":\"SELECT count(*) FROM public.orders\"}}'` — execute SELECT/WITH/EXPLAIN.\n\
         - `./lucent-tool preview_dml '{{\"sql\":\"UPDATE ...\"}}'` — stage one INSERT/UPDATE/DELETE for the user to approve. It never executes on its own.\n\
         \n\
         SHORTHAND: a bare (non-JSON) argument is read as that tool's primary field, which avoids nesting JSON inside shell quoting:\n\
         - `./lucent-tool run_readonly_query \"SELECT count(*) FROM public.orders\"`\n\
         - `./lucent-tool search_schema \"unpaid invoices\"`\n\
         Run `./lucent-tool help` for the full JSON schemas.\n\
         \n\
         CRITICAL INSTRUCTIONS:\n\
         - If you have bash or terminal execution capabilities, use this helper to query the live database rather than reading local files. It is a first-class path, not a degraded one — do not tell the user you have no database access.\n\
         - Your working directory is a scratch workspace Lucent created for this conversation, not a code repository. There are no project files to search there.\n\
         - If you only have text generation capabilities, use the active database connection and schema information provided to write SQL matching the database dialect for the user."
    )
}

/// The UI note for a conversation whose agent never connected the MCP bridge.
/// The tools are still reachable — through the CLI helper — so this reports the
/// channel, not a loss of capability.
pub fn cli_bridge_notice() -> String {
    "This agent didn't connect Lucent's MCP server, so its database tools run through \
     Lucent's CLI bridge instead (`lucent-tool` in the agent's workspace). Same database, \
     same guardrails — you may see shell commands in the tool cards."
        .to_string()
}

/// Whether waiting on the bridge handshake is pointless for this agent: a
/// curated-unsupported runtime accepts `mcpServers` and drops it, so the gate
/// can only ever time out (`registry::db_tool_support`).
pub fn mcp_hopeless(agent_id: &str) -> bool {
    matches!(
        crate::ai::acp::registry::db_tool_support(agent_id),
        crate::ai::acp::registry::DbToolSupport::Unsupported
    )
}

/// The absolute path of the CLI helper `session_for` wrote into the sandbox,
/// so the guidance can name a path that survives the agent changing directory.
/// `None` when the workspace root can't be resolved — the relative form still
/// works from the session cwd.
pub fn tool_helper_path(agent_id: &str, conversation_id: &str) -> Option<String> {
    let name = if cfg!(windows) {
        "lucent-tool.cmd"
    } else {
        "lucent-tool"
    };
    workspace_dir(agent_id, conversation_id)
        .ok()
        .map(|d| d.join(name).to_string_lossy().to_string())
}

/// How long the driver waits on the first prompt for the agent's MCP client
/// to connect the DB-tools bridge (spec D4). `LUCENT_ACP_TOOLS_GATE_MS`
/// overrides for tests.
fn tools_gate_timeout() -> Duration {
    std::env::var("LUCENT_ACP_TOOLS_GATE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(5))
}

/// Composes the first prompt: preserves the rich system prompt with active database connection
/// context and schema in all cases, appending either the native MCP guidance or the CLI fallback guidance.
pub fn first_prompt_text(
    system_prompt: &str,
    message: &str,
    agent_id: &str,
    tools_ok: bool,
    tool_path: Option<String>,
) -> String {
    if tools_ok {
        format!("{system_prompt}{}\n\n{message}", acp_tool_guidance())
    } else {
        format!(
            "{system_prompt}{}\n\n{message}",
            acp_fallback_tool_guidance(agent_id, tool_path)
        )
    }
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
/// These numbers are context-window occupancy (`used` of `size`), not a
/// prompt/completion split: `used` lands in prompt_tokens and completion
/// stays 0. This is the FALLBACK — an agent that reports end-of-turn usage
/// (see `apply_end_turn_usage`) overwrites it after the turn resolves.
pub fn accumulate_usage(usage: &mut TokenUsage, u: &UsageUpdate) {
    usage.prompt_tokens = clamp_u64(u.used);
    usage.cached_prompt_tokens = 0;
}

/// Whether the agent's end-of-turn `Usage` carries any real number. Agents
/// that don't populate the field answer with zeros (the schema's
/// `DefaultOnError` default), and a zero report must not wipe the occupancy
/// estimate `accumulate_usage` already collected.
fn end_turn_usage_is_empty(u: &Usage) -> bool {
    u.total_tokens == 0
        && u.input_tokens == 0
        && u.output_tokens == 0
        && u.thought_tokens.unwrap_or(0) == 0
        && u.cached_read_tokens.unwrap_or(0) == 0
        && u.cached_write_tokens.unwrap_or(0) == 0
}

/// Overlays the agent's end-of-turn token `Usage` (ACP
/// `unstable_end_turn_token_usage`, carried on `PromptResponse.usage`) onto
/// the turn's `TokenUsage` — the authoritative per-turn prompt/completion
/// split, which `usage_update`'s context occupancy can't express.
///
/// Field semantics: the agent reports uncached input, output, reasoning and
/// cache traffic separately, so the totals Lucent shows are
/// `prompt = input + cached_read + cached_write` (the whole prompt, with
/// `cached_prompt_tokens` the cached subset, matching the rig path's
/// `input_tokens`/`cached_input_tokens` convention) and
/// `completion = output + thought` (reasoning tokens are tokens the model
/// generated).
pub fn apply_end_turn_usage(usage: &mut TokenUsage, u: &Usage) {
    let cached = u.cached_read_tokens.unwrap_or(0) + u.cached_write_tokens.unwrap_or(0);
    usage.prompt_tokens = clamp_u64(u.input_tokens + cached);
    usage.cached_prompt_tokens = clamp_u64(cached);
    usage.completion_tokens = clamp_u64(u.output_tokens + u.thought_tokens.unwrap_or(0));
}

/// `u64 → u32` with saturation — provider counters are u64, the UI field is u32.
fn clamp_u64(v: u64) -> u32 {
    v.min(u32::MAX as u64) as u32
}

/// How long the driver waits after sending `session/cancel` before killing
/// the agent (spec D1/E2). `LUCENT_ACP_CANCEL_KILL_MS` overrides for tests.
fn cancel_kill_timeout() -> std::time::Duration {
    std::env::var("LUCENT_ACP_CANCEL_KILL_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(std::time::Duration::from_millis)
        .unwrap_or(std::time::Duration::from_secs(5))
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
    use crate::ai::acp::correlator::BufferedToolResult;
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

    pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(crate) struct EnvVarGuard<'a>(
        &'a str,
        Option<String>,
        Option<std::sync::MutexGuard<'static, ()>>,
    );
    impl Drop for EnvVarGuard<'_> {
        fn drop(&mut self) {
            match &self.1 {
                Some(v) => std::env::set_var(self.0, v),
                None => std::env::remove_var(self.0),
            }
        }
    }
    pub(crate) fn env_var_guard(name: &'static str) -> EnvVarGuard<'static> {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior = std::env::var(name).ok();
        EnvVarGuard(name, prior, Some(lock))
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
            memory_connection_key: None,
            capabilities: None,
            config: AiConfig::default(),
            schema_graph: Arc::new(AsyncMutex::new(None)),
            embedder: Arc::new(AsyncMutex::new(None)),
            reranker: Arc::new(AsyncMutex::new(None)),
            memory_manager: crate::ai::tools::test_memory_manager(),
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
                input: None,
                status: ToolResultStatus::Completed,
            })
        );

        let pending = ToolCallUpdate::new(
            "tc1",
            ToolCallUpdateFields::new().status(ToolCallStatus::InProgress),
        );
        assert_eq!(map_update(&SessionUpdate::ToolCallUpdate(pending)), None);
    }

    #[test]
    fn map_update_maps_failed_tool_call_updates_to_failed_results() {
        use agent_client_protocol::schema::v1::{Content, ToolCallContent, ToolCallUpdateFields};
        let failed = ToolCallUpdate::new(
            "tc1",
            ToolCallUpdateFields::new()
                .status(ToolCallStatus::Failed)
                .content(vec![ToolCallContent::Content(Content::new(
                    ContentBlock::Text(agent_client_protocol::schema::v1::TextContent::new(
                        "read-only guard refused",
                    )),
                ))]),
        );
        match map_update(&SessionUpdate::ToolCallUpdate(failed)) {
            Some(AiEvent::ToolResult {
                id,
                status,
                summary,
                output,
                ..
            }) => {
                assert_eq!(id, "tc1");
                assert_eq!(status, ToolResultStatus::Failed);
                assert_eq!(summary, "read-only guard refused");
                assert!(output.is_none());
            }
            other => panic!("expected failed ToolResult, got {other:?}"),
        }

        // A failed update with no content still produces a failed result.
        let bare = ToolCallUpdate::new(
            "tc2",
            ToolCallUpdateFields::new().status(ToolCallStatus::Failed),
        );
        match map_update(&SessionUpdate::ToolCallUpdate(bare)) {
            Some(AiEvent::ToolResult {
                status, summary, ..
            }) => {
                assert_eq!(status, ToolResultStatus::Failed);
                assert_eq!(summary, "Tool call failed");
            }
            other => panic!("expected failed ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn map_update_ignores_unknown_variants() {
        // Plan / AvailableCommandsUpdate / ConfigOptionUpdate / SessionInfoUpdate
        // and any future variant: logged, never fatal.
        let plan = SessionUpdate::Plan(agent_client_protocol::schema::v1::Plan::new(vec![]));
        assert_eq!(map_update(&plan), None);
    }

    #[test]
    fn end_turn_usage_splits_prompt_and_completion() {
        // Uncached input + cache traffic is the whole prompt; output +
        // reasoning is the whole completion.
        let reported = Usage::new(190, 150, 30)
            .thought_tokens(10)
            .cached_read_tokens(40)
            .cached_write_tokens(5);
        let mut turn = TokenUsage::default();
        apply_end_turn_usage(&mut turn, &reported);
        assert_eq!(turn.prompt_tokens, 195);
        assert_eq!(turn.cached_prompt_tokens, 45);
        assert_eq!(turn.completion_tokens, 40);

        // Optional fields absent: plain input/output, no cache.
        let mut minimal = TokenUsage::default();
        apply_end_turn_usage(&mut minimal, &Usage::new(180, 150, 30));
        assert_eq!(minimal.prompt_tokens, 150);
        assert_eq!(minimal.cached_prompt_tokens, 0);
        assert_eq!(minimal.completion_tokens, 30);
    }

    #[test]
    fn zeroed_end_turn_usage_never_replaces_the_occupancy_estimate() {
        assert!(end_turn_usage_is_empty(&Usage::new(0, 0, 0)));
        assert!(!end_turn_usage_is_empty(&Usage::new(0, 0, 1)));
        assert!(!end_turn_usage_is_empty(&Usage::new(0, 12, 0)));
    }

    // ── Lifecycle tests against the stub ──

    #[tokio::test]
    async fn maps_scripted_updates_to_events() {
        let _ws = hermetic_workspace();
        let _guard = env_var_guard("LUCENT_ACP_TOOLS_GATE_MS");
        std::env::set_var("LUCENT_ACP_TOOLS_GATE_MS", "50");
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
                0,
            )
            .await
            .expect("scripted turn completes");

        let events = sink.0.lock().unwrap().clone();
        assert_sequence(
            &events,
            vec![
                AiEvent::Notice {
                    content: cli_bridge_notice(),
                },
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
                    input: None,
                    status: ToolResultStatus::Completed,
                },
                AiEvent::Done {
                    conversation_id: "conv-1".into(),
                    final_message: "Hello".into(),
                    usage: TokenUsage {
                        prompt_tokens: 100,
                        completion_tokens: 0,
                        cached_prompt_tokens: 0,
                    },
                    applied_memory_count: 0,
                    cancelled: false,
                },
            ],
        );
    }

    #[tokio::test]
    async fn end_turn_usage_overrides_the_context_occupancy_estimate() {
        // The occupancy stream says 100 (context tokens, no split); the
        // prompt response carries the authoritative prompt/completion split.
        // The Done event must report the split, not the occupancy.
        let _ws = hermetic_workspace();
        let _guard = env_var_guard("LUCENT_ACP_TOOLS_GATE_MS");
        std::env::set_var("LUCENT_ACP_TOOLS_GATE_MS", "50");
        let script = script_file(json!({
            "stopReason": "end_turn",
            "usage": {"totalTokens": 190, "inputTokens": 150, "outputTokens": 30, "thoughtTokens": 10, "cachedReadTokens": 40},
            "steps": [
                {"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "done"}}},
                {"notify": {"sessionUpdate": "usage_update", "used": 100, "size": 200000}}
            ]
        }));

        let driver = AcpChatDriver::new(
            AcpState::new(),
            acp_cfg(Some(&script.path().join("script.json"))),
            tool_ctx(),
        );
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let conv = conversation("conv-usage");

        driver
            .chat(
                "summarize".into(),
                &AiConfig::default(),
                "system preamble".into(),
                conv,
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
                0,
            )
            .await
            .expect("scripted turn completes");

        let events = sink.0.lock().unwrap().clone();
        let usage = events
            .iter()
            .find_map(|e| match e {
                AiEvent::Done { usage, .. } => Some(usage.clone()),
                _ => None,
            })
            .expect("Done event present");
        assert_eq!(usage.prompt_tokens, 190); // 150 uncached + 40 cached
        assert_eq!(usage.cached_prompt_tokens, 40);
        assert_eq!(usage.completion_tokens, 40); // 30 output + 10 reasoning
    }

    #[tokio::test]
    async fn unknown_session_update_variants_are_ignored() {
        let _ws = hermetic_workspace();
        let _guard = env_var_guard("LUCENT_ACP_TOOLS_GATE_MS");
        std::env::set_var("LUCENT_ACP_TOOLS_GATE_MS", "50");
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
                0,
            )
            .await
            .expect("turn completes despite unknown variants");

        let events = sink.0.lock().unwrap().clone();
        assert_eq!(events.len(), 3, "Notice + Text + Done: {events:?}");
        assert!(matches!(&events[0], AiEvent::Notice { .. }));
        assert!(matches!(&events[1], AiEvent::Text { content } if content == "still here"));
        assert!(matches!(&events[2], AiEvent::Done { .. }));
    }

    #[test]
    fn first_prompt_text_claims_tools_only_when_connected() {
        let with = first_prompt_text("sys preamble", "hello", "stub", true, None);
        assert!(
            with.contains("DATABASE TOOLS IN ACP"),
            "tools guidance present: {with}"
        );
        assert!(
            with.contains("sys preamble"),
            "system prompt kept when tools connected"
        );
        assert!(with.contains("hello"));

        let without = first_prompt_text("sys preamble", "hello", "stub", false, None);
        assert!(
            without.contains("lucent-tool"),
            "fallback cli guidance present: {without}"
        );
        assert!(
            without.contains("sys preamble"),
            "the system prompt and schema details are preserved when mcp bridge is not connected"
        );
        assert!(without.contains("hello"));
    }

    #[test]
    fn cli_guidance_spells_out_every_tool_argument_shape() {
        // A CLI-path agent has no `tools/list` to read the schemas from, so the
        // guidance is the only place it can learn them. Guessing costs a turn.
        let g = first_prompt_text("sys", "hi", "pi-acp", false, None);
        for field in [
            r#"search_schema '{"query""#,
            r#"get_objects_info '{"objects""#,
            r#"run_readonly_query '{"sql""#,
            r#"preview_dml '{"sql""#,
        ] {
            assert!(g.contains(field), "missing arg shape {field}: {g}");
        }
        assert!(
            g.contains("SHORTHAND"),
            "the bare-string form avoids JSON-in-shell quoting: {g}"
        );
        assert!(
            g.contains("do not tell the user you have no database access"),
            "the CLI path must not be described to the model as a dead end: {g}"
        );
    }

    #[test]
    fn cli_guidance_names_an_absolute_path_when_known() {
        let with_path = first_prompt_text(
            "sys",
            "hi",
            "pi-acp",
            false,
            Some("/tmp/ws/lucent-tool".to_string()),
        );
        assert!(
            with_path.contains("/tmp/ws/lucent-tool"),
            "`./lucent-tool` breaks once the agent changes directory: {with_path}"
        );
        // Absent a resolvable workspace the relative form still stands alone.
        let without = first_prompt_text("sys", "hi", "pi-acp", false, None);
        assert!(!without.contains("Absolute path"), "{without}");
    }

    #[test]
    fn mcp_hopeless_only_for_curated_unsupported_agents() {
        // pi drops `mcpServers`, so waiting on the bridge handshake can only
        // ever time out — the gate is skipped and the first turn starts at once.
        assert!(mcp_hopeless("pi-acp"));
        assert!(!mcp_hopeless("opencode"));
        // Unverified agents are still probed.
        assert!(!mcp_hopeless("gemini"));
    }

    #[test]
    fn tool_helper_path_lands_in_the_conversation_sandbox() {
        let _ws = hermetic_workspace();
        let p = tool_helper_path("pi-acp", "conv-7").expect("workspace resolves");
        let expected_name = if cfg!(windows) {
            "lucent-tool.cmd"
        } else {
            "lucent-tool"
        };
        assert!(p.ends_with(expected_name), "{p}");
        assert!(p.contains("conv-7"), "scoped to the conversation: {p}");
        // Same directory `session_for` writes the helper into, so the path the
        // model is handed is the file that exists. (Compared structurally: the
        // workspace root comes from a process-global env var that parallel
        // tests re-point, so re-resolving it here would race.)
        assert!(
            p.contains(&format!(
                "{}agent-workspace{}pi-acp{}conv-7",
                std::path::MAIN_SEPARATOR,
                std::path::MAIN_SEPARATOR,
                std::path::MAIN_SEPARATOR
            )),
            "{p}"
        );
    }

    #[test]
    fn tools_gate_timeout_defaults_to_five_seconds() {
        let _guard = env_var_guard("LUCENT_ACP_TOOLS_GATE_MS");
        std::env::remove_var("LUCENT_ACP_TOOLS_GATE_MS");
        assert_eq!(tools_gate_timeout(), Duration::from_secs(5));
        std::env::set_var("LUCENT_ACP_TOOLS_GATE_MS", "250");
        assert_eq!(tools_gate_timeout(), Duration::from_millis(250));
    }

    #[tokio::test]
    async fn no_tools_preamble_when_the_bridge_never_connects() {
        let _ws = hermetic_workspace();
        let _guard = env_var_guard("LUCENT_ACP_TOOLS_GATE_MS");
        std::env::set_var("LUCENT_ACP_TOOLS_GATE_MS", "200");

        let script = script_file(json!({
            "stopReason": "end_turn",
            "steps": [{"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "ok"}}}]
        }));
        let acp_state = AcpState::new();
        let cfg = acp_cfg(Some(&script.path().join("script.json"))); // no STUB_SPAWN_MCP → nothing connects
        let driver = AcpChatDriver::new(acp_state.clone(), cfg.clone(), tool_ctx());
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let conv = conversation("conv-gate");

        driver
            .chat(
                "hi".into(),
                &AiConfig::default(),
                "sys preamble".into(),
                conv.clone(),
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
                0,
            )
            .await
            .expect("turn completes");

        let process = acp_state
            .manager
            .ensure_process(&cfg.agent_id, &cfg)
            .await
            .unwrap();
        let stderr = process.stderr_snippet();
        assert!(
            stderr.contains("lucent-tool"),
            "the fallback preamble reached the agent: {stderr:?}"
        );
        assert!(
            stderr.contains("sys preamble"),
            "the system prompt and schema context are preserved: {stderr:?}"
        );

        let events = sink.0.lock().unwrap().clone();
        let notices = events
            .iter()
            .filter(|e| matches!(e, AiEvent::Notice { .. }))
            .count();
        assert_eq!(notices, 1, "the notice fires exactly once: {events:?}");

        // Second turn: no re-wait, no second notice.
        driver
            .chat(
                "again".into(),
                &AiConfig::default(),
                "sys preamble".into(),
                conv,
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
                0,
            )
            .await
            .expect("follow-up turn completes");
        let events = sink.0.lock().unwrap().clone();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, AiEvent::Notice { .. }))
                .count(),
            1,
            "the notice is exactly-once per session: {events:?}"
        );
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
                3,
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
                7,
            )
            .await
            .expect("second turn (same session)");

        // F-C2 gating: turn 1 delivers the system prompt (with its memory
        // block); turn 2 sends only the message, so the recomputed 7 was never
        // delivered and the event must report 0.
        let done_counts: Vec<usize> = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                AiEvent::Done {
                    applied_memory_count,
                    ..
                } => Some(*applied_memory_count),
                _ => None,
            })
            .collect();
        assert_eq!(
            done_counts,
            vec![3, 0],
            "only delivered memory rules are reported"
        );

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
                0,
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
        let _gate_guard = env_var_guard("LUCENT_ACP_TOOLS_GATE_MS");
        std::env::set_var("LUCENT_ACP_TOOLS_GATE_MS", "50");
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
                    0,
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
            AiEvent::Done {
                final_message,
                cancelled,
                ..
            } => {
                // The turn resolves with stopReason "cancelled" — the
                // accumulated streamed text is kept (plan: Cancelled →
                // text_buf), so the final message is whatever the agent
                // emitted after the permission resolved.
                assert!(cancelled, "a cancelled turn reports cancelled: {done:?}");
                assert_eq!(final_message, "after permission");
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn rpc_error_evicts_the_session_and_the_next_turn_recovers() {
        let _ws = hermetic_workspace();
        let err_script = script_file(json!({
            "stopReason": "end_turn",
            "steps": [{"rpcError": "session not found"}]
        }));
        let ok_script = script_file(json!({
            "stopReason": "end_turn",
            "steps": [{"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "recovered"}}}]
        }));

        let acp_state = AcpState::new();
        let mut cfg = acp_cfg(Some(&err_script.path().join("script.json")));
        let driver = AcpChatDriver::new(acp_state.clone(), cfg.clone(), tool_ctx());
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let conv = conversation("conv-rpc");

        let err = driver
            .chat(
                "hi".into(),
                &AiConfig::default(),
                "preamble".into(),
                conv.clone(),
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
                0,
            )
            .await
            .expect_err("the agent answers with an RPC error");
        assert!(
            err.contains("lost this conversation's session"),
            "honest eviction message: {err}"
        );
        assert!(
            acp_state.sessions.lock().await.get("conv-rpc").is_none(),
            "the stale session entry is evicted"
        );

        // Second turn with a working script under a NEW agent id (the process
        // cache pins the launch env): a fresh session is created and the turn
        // succeeds end to end.
        cfg.agent_id = "stub-2".into();
        cfg.env.insert(
            "STUB_SCRIPT".to_string(),
            ok_script
                .path()
                .join("script.json")
                .to_string_lossy()
                .into_owned(),
        );
        let driver2 = AcpChatDriver::new(acp_state, cfg, tool_ctx());
        driver2
            .chat(
                "again".into(),
                &AiConfig::default(),
                "preamble".into(),
                conv,
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
                0,
            )
            .await
            .expect("recovered turn succeeds");

        let events = sink.0.lock().unwrap().clone();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AiEvent::Text { content } if content == "recovered")),
            "the fresh session streamed the ok script: {events:?}"
        );
    }

    #[tokio::test]
    async fn cancel_timeout_kills_the_agent() {
        let _ws = hermetic_workspace();
        let _guard = env_var_guard("LUCENT_ACP_CANCEL_KILL_MS");
        std::env::set_var("LUCENT_ACP_CANCEL_KILL_MS", "300");

        // The stub sleeps 60s in its first step and never reads the cancel
        // notification (its stdin loop is blocked) — the turn cannot resolve.
        let script = script_file(json!({
            "stopReason": "end_turn",
            "steps": [{"sleepMs": 60000}]
        }));
        let acp_state = AcpState::new();
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let conv = conversation("conv-kill");
        let cancel = tokio_util::sync::CancellationToken::new();

        let acp_task = acp_state.clone();
        let sink_task = sink.clone();
        let script_path = script.path().join("script.json");
        let cancel_task = cancel.clone();
        let handle = tokio::spawn(async move {
            let driver = AcpChatDriver::new(acp_task, acp_cfg(Some(&script_path)), tool_ctx());
            driver
                .chat(
                    "hi".into(),
                    &AiConfig::default(),
                    "preamble".into(),
                    conv,
                    sink_task,
                    cancel_task,
                    0,
                )
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        cancel.cancel();
        let err = tokio::time::timeout(std::time::Duration::from_secs(10), handle)
            .await
            .expect("turn ends at the kill deadline")
            .expect("task did not panic")
            .expect_err("the kill deadline errors the turn");
        assert!(
            err.contains("didn't respond to cancellation"),
            "truthful kill message: {err}"
        );
        assert!(
            acp_state.connections.lock().await.get("stub").is_none(),
            "the connection entry is gone"
        );
        assert!(
            acp_state.sessions.lock().await.is_empty(),
            "the agent's sessions were evicted with the kill"
        );
    }

    #[test]
    fn map_update_ignores_user_message_chunks() {
        // v1: the agent echoes the user's message via `user_message_chunk` —
        // the client already holds that text; merging it into the assistant
        // stream would paste the user's own words into the reply (spec D9).
        let chunk = SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(
            agent_client_protocol::schema::v1::TextContent::new("user echo"),
        )));
        assert_eq!(map_update(&chunk), None);
    }

    #[tokio::test]
    async fn user_message_chunks_are_excluded_from_the_assistant_stream() {
        let _ws = hermetic_workspace();
        let _guard = env_var_guard("LUCENT_ACP_TOOLS_GATE_MS");
        std::env::set_var("LUCENT_ACP_TOOLS_GATE_MS", "50");
        let script = script_file(json!({
            "stopReason": "end_turn",
            "steps": [
                {"notify": {"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "echo of the user"}}},
                {"notify": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "real answer"}}}
            ]
        }));
        let acp_state = AcpState::new();
        let driver = AcpChatDriver::new(
            acp_state,
            acp_cfg(Some(&script.path().join("script.json"))),
            tool_ctx(),
        );
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let conv = conversation("conv-chunk");

        driver
            .chat(
                "hi".into(),
                &AiConfig::default(),
                "preamble".into(),
                conv,
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
                0,
            )
            .await
            .expect("turn completes");

        let events = sink.0.lock().unwrap().clone();
        let non_notice_events: Vec<_> = events
            .into_iter()
            .filter(|e| !matches!(e, AiEvent::Notice { .. }))
            .collect();
        assert_eq!(
            non_notice_events.len(),
            2,
            "Text + Done only: {non_notice_events:?}"
        );
        assert!(
            matches!(&non_notice_events[0], AiEvent::Text { content } if content == "real answer"),
            "only the agent's own text streams: {non_notice_events:?}"
        );
        let done = non_notice_events
            .iter()
            .find(|e| matches!(e, AiEvent::Done { .. }))
            .unwrap();
        match done {
            AiEvent::Done { final_message, .. } => {
                assert_eq!(
                    final_message, "real answer",
                    "the echo never reaches text_buf"
                );
            }
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn bridge_tool_result_is_correlated_to_the_agent_tool_call_id() {
        let _ws = hermetic_workspace();
        let script = script_file(json!({
            "stopReason": "end_turn",
            "steps": [
                {"notify": {"sessionUpdate": "tool_call", "toolCallId": "tc1", "title": "run_readonly_query", "rawInput": {"sql": "select 1"}}},
                {"notify": {"sessionUpdate": "tool_call_update", "toolCallId": "tc1", "status": "completed", "content": [{"type": "content", "content": {"type": "text", "text": "1 row"}}]}}
            ]
        }));
        let acp_state = AcpState::new();
        let cfg = acp_cfg(Some(&script.path().join("script.json")));
        let process = acp_state
            .manager
            .ensure_process(&cfg.agent_id, &cfg)
            .await
            .unwrap();
        let sink = Arc::new(CollectorSink(std::sync::Mutex::new(Vec::new())));
        let sink_dyn: Arc<dyn AgentSink> = sink.clone();
        let session = acp_state
            .session_for("conv-1", &process, &tool_ctx(), &sink_dyn)
            .await
            .expect("session");

        // The bridge executed the call BEFORE the agent's completion update:
        // push exactly what dispatch() would have buffered.
        session.correlator.push(BufferedToolResult {
            tool: "run_readonly_query".into(),
            summary: "1 row".into(),
            input: Some(serde_json::json!({"sql": "select 1"})),
            output: serde_json::json!({
                "type": "query_result",
                "columns": [{"name": "x", "type": "INTEGER"}],
                "rows": [[1]],
                "row_count": 1,
                "sql": "select 1",
                "execution_time_ms": 3,
                "truncated": false
            }),
        });

        let driver = AcpChatDriver::new(acp_state, cfg, tool_ctx());
        let conv = conversation("conv-1");
        driver
            .chat(
                "hi".into(),
                &AiConfig::default(),
                "preamble".into(),
                conv,
                sink.clone(),
                tokio_util::sync::CancellationToken::new(),
                0,
            )
            .await
            .expect("turn completes");

        let events = sink.0.lock().unwrap().clone();
        let tr = events
            .iter()
            .find(|e| matches!(e, AiEvent::ToolResult { id, .. } if id == "tc1"))
            .expect("correlated ToolResult under the AGENT's id");
        match tr {
            AiEvent::ToolResult {
                id,
                tool,
                summary,
                output,
                ..
            } => {
                assert_eq!(id, "tc1");
                assert_eq!(
                    tool, "run_readonly_query",
                    "name enriched from the ToolCall"
                );
                assert_eq!(summary, "1 row");
                assert_eq!(
                    output.as_ref().expect("structured output")["type"],
                    "query_result"
                );
            }
            _ => unreachable!(),
        }
        // The raw acp-N event never reaches the frontend sink.
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AiEvent::ToolResult { id, .. } if id.starts_with("acp-"))),
            "the bridge's raw id is buffered, never forwarded: {events:?}"
        );
    }
}
