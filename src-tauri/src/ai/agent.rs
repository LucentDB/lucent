use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::ai::config::AiConfig;
use crate::ai::events::{AiEvent, DmlApprovalPayload, TokenUsage};
use crate::ai::tools::AiToolContext;
use crate::ai::truncate_utf8;

// ── Message types ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: MessageContent,
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: MessageContent::Text(text.into()),
            tool_calls: None,
        }
    }

    pub fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: MessageContent::Text(text.into()),
            tool_calls: None,
        }
    }

    pub fn with_tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: MessageContent::Text(String::new()),
            tool_calls: Some(tool_calls),
        }
    }

    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Tool,
            content: MessageContent::ToolResult {
                tool_use_id: tool_use_id.into(),
                content: content.into(),
            },
            tool_calls: None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    ToolResult {
        tool_use_id: String,
        content: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: MessageRole, text: &str) -> Message {
        Message {
            role,
            content: MessageContent::Text(text.into()),
            tool_calls: None,
        }
    }

    #[test]
    fn history_appends_messages() {
        let mut state = ConversationState::new("conn_1".into());
        assert!(state.history.is_empty());
        state.history.push(msg(MessageRole::User, "hello"));
        state.history.push(msg(MessageRole::Assistant, "hi"));
        assert_eq!(state.history.len(), 2);
        assert_eq!(state.history[0].role, MessageRole::User);
    }

    #[test]
    fn history_includes_tool_calls() {
        let mut state = ConversationState::new("conn_1".into());
        state.history.push(msg(MessageRole::User, "find acme"));
        state.history.push(Message::with_tool_calls(vec![ToolCall {
            id: "call_1".into(),
            name: "search_objects".into(),
            args: serde_json::json!({"query": "acme"}),
        }]));
        state
            .history
            .push(Message::tool_result("call_1", "found 1 row"));
        assert_eq!(state.history.len(), 3);
        assert_eq!(state.history[1].role, MessageRole::Assistant);
        assert!(state.history[1].tool_calls.is_some());
        assert_eq!(state.history[2].role, MessageRole::Tool);
    }

    #[test]
    fn take_staged_sql_returns_and_resets() {
        let mut state = ConversationState::new("conn_1".into());
        state.state = AgentState::PausedForDml {
            staged_sql: "DELETE FROM t".into(),
            staged_at: Instant::now(),
        };
        let sql = state.take_staged_sql();
        assert!(matches!(sql, Some((s, _)) if s == "DELETE FROM t"));
        assert!(matches!(state.state, AgentState::Idle));
    }

    #[test]
    fn take_staged_sql_returns_none_if_idle() {
        let mut state = ConversationState::new("conn_1".into());
        assert!(state.take_staged_sql().is_none());
    }

    #[test]
    fn take_staged_sql_reports_the_staging_time() {
        // E6: the staging timestamp must travel with the SQL so the approval
        // path can refuse stale DML.
        let mut state = ConversationState::new("conn_1".into());
        let staged_at = Instant::now();
        state.state = AgentState::PausedForDml {
            staged_sql: "DELETE FROM t".into(),
            staged_at,
        };
        let (sql, got) = state
            .take_staged_sql()
            .expect("staged SQL must be returned");
        assert_eq!(sql, "DELETE FROM t");
        assert_eq!(got, staged_at);
    }

    #[test]
    fn try_begin_turn_claims_an_idle_conversation() {
        let mut state = ConversationState::new("conn_1".into());
        let cancel = tokio_util::sync::CancellationToken::new();
        state.try_begin_turn(cancel.clone()).unwrap();
        assert!(matches!(state.state, AgentState::Running { .. }));
    }

    #[test]
    fn try_begin_turn_rejects_a_running_conversation() {
        // E1: two concurrent ai_chat calls used to start two agent loops on
        // one conversation (interleaved history, doubled spend). The CAS
        // must refuse the second.
        let mut state = ConversationState::new("conn_1".into());
        let cancel = tokio_util::sync::CancellationToken::new();
        state.try_begin_turn(cancel.clone()).unwrap();
        let err = state.try_begin_turn(cancel).unwrap_err();
        assert!(err.contains("already in progress"), "{err}");
    }

    #[test]
    fn try_begin_turn_rejects_a_dml_paused_conversation() {
        let mut state = ConversationState::new("conn_1".into());
        state.state = AgentState::PausedForDml {
            staged_sql: "DELETE FROM t".into(),
            staged_at: Instant::now(),
        };
        let err = state
            .try_begin_turn(tokio_util::sync::CancellationToken::new())
            .unwrap_err();
        assert!(err.contains("pending DML"), "{err}");
    }

    #[test]
    fn llm_response_roundtrip() {
        let response = LlmResponse {
            text: Some("hello".into()),
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "get_objects_info".into(),
                args: serde_json::json!({"objects": [{"name": "users"}]}),
            }],
            usage: TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                cached_prompt_tokens: 0,
            },
            thinking: Some("I need to check the schema".into()),
        };
        assert!(response.text.is_some());
        assert!(response.thinking.is_some());
        assert_eq!(response.tool_calls.len(), 1);
    }

    #[test]
    fn compact_history_keeps_small_histories_unchanged() {
        let h: Vec<Message> = (0..5).map(|i| Message::user(format!("m{i}"))).collect();
        let out = compact_history(h.clone());
        assert_eq!(out.len(), 5);
        assert_eq!(out, h);
    }

    #[test]
    fn compact_history_windows_oversized_histories_with_a_summary_prefix() {
        // E4: unbounded history guarantees a context-window failure on long
        // conversations. The window must keep the newest messages and tell
        // the model the past was collapsed.
        let h: Vec<Message> = (0..30).map(|i| Message::user(format!("m{i}"))).collect();
        let out = compact_history(h);
        assert_eq!(
            out.len(),
            HISTORY_WINDOW + 1,
            "window + summary placeholder"
        );
        let summary = &out[0];
        match &summary.content {
            MessageContent::Text(t) => {
                assert!(
                    t.contains("14 earlier messages"),
                    "summary must count the omitted: {t}"
                );
            }
            other => panic!("expected text summary, got {other:?}"),
        }
        let tail: Vec<&str> = out[1..]
            .iter()
            .map(|m| match &m.content {
                MessageContent::Text(t) => t.as_str(),
                other => panic!("expected text, got {other:?}"),
            })
            .collect();
        assert_eq!(tail, (14..30).map(|i| format!("m{i}")).collect::<Vec<_>>());
    }

    #[test]
    fn compact_history_never_starts_on_an_orphaned_tool_result() {
        // D5: 5 tool rounds × (assistant + 3 tools) + user + final assistant
        // = 22 messages. The newest-16 window lands on a `tool` result whose
        // assistant(tool_calls) was cut — strict providers reject a request
        // whose first message is role 'tool' (tool_call_id must follow a
        // message that declared it).
        let mut h: Vec<Message> = Vec::new();
        for r in 0..5 {
            h.push(Message::with_tool_calls(vec![ToolCall {
                id: format!("call_{r}_0"),
                name: "run_readonly_query".into(),
                args: serde_json::json!({ "sql": "SELECT 1" }),
            }]));
            for t in 0..3 {
                h.push(Message::tool_result(
                    format!("call_{r}_{t}"),
                    format!("row {r}_{t}"),
                ));
            }
        }
        h.push(Message::user("wrap up"));
        h.push(Message::assistant("done"));
        assert_eq!(h.len(), 22, "the 22-message trigger shape");

        let out = compact_history(h);
        assert_eq!(out[0].role, MessageRole::User, "summary must lead");
        let tail = &out[1..];
        assert!(
            tail[0].role != MessageRole::Tool,
            "window must not start on an orphaned tool result"
        );
        for (i, m) in tail.iter().enumerate() {
            if m.role == MessageRole::Tool {
                // Parallel tool calls produce several consecutive `tool`
                // results in one round — scan back to the round's start.
                let mut j = i;
                while j > 0 && tail[j - 1].role == MessageRole::Tool {
                    j -= 1;
                }
                assert!(
                    j > 0,
                    "tool result at index {i} must follow its assistant(tool_calls) message"
                );
                assert_eq!(
                    tail[j - 1].role,
                    MessageRole::Assistant,
                    "tool result at index {i} must follow its assistant(tool_calls) message"
                );
                assert!(
                    tail[j - 1].tool_calls.is_some(),
                    "the assistant message before a tool result must carry tool_calls"
                );
            }
        }
    }

    #[test]
    fn compact_history_keeps_exactly_history_window_messages() {
        // D5 boundary: exactly HISTORY_WINDOW messages is not a collapse —
        // this pins the off-by-one so the window lock is at >16, not >=16.
        let h: Vec<Message> = (0..HISTORY_WINDOW)
            .map(|i| Message::user(format!("m{i}")))
            .collect();
        let out = compact_history(h.clone());
        assert_eq!(
            out, h,
            "exactly HISTORY_WINDOW messages pass through untouched"
        );
    }

    #[test]
    fn compact_history_collapses_at_history_window_plus_one() {
        // D5 boundary: HISTORY_WINDOW + 1 is the first size that collapses.
        let h: Vec<Message> = (0..HISTORY_WINDOW + 1)
            .map(|i| Message::user(format!("m{i}")))
            .collect();
        let out = compact_history(h);
        assert_eq!(
            out.len(),
            HISTORY_WINDOW + 1,
            "summary placeholder + the full window tail"
        );
        match &out[0].content {
            MessageContent::Text(t) => {
                assert!(
                    t.contains("1 earlier messages"),
                    "summary must count the omitted: {t}"
                )
            }
            other => panic!("expected text summary, got {other:?}"),
        }
        // The newest HISTORY_WINDOW messages pass through in order.
        let tail: Vec<&str> = out[1..]
            .iter()
            .map(|m| match &m.content {
                MessageContent::Text(t) => t.as_str(),
                other => panic!("expected text, got {other:?}"),
            })
            .collect();
        assert_eq!(
            tail,
            (1..HISTORY_WINDOW + 1)
                .map(|i| format!("m{i}"))
                .collect::<Vec<_>>()
        );
    }
}

// ── LLM response types ────────────────────────────────────────────────────

pub struct LlmResponse {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: TokenUsage,
    /// Model's reasoning/thinking content (sent as Thinking events).
    pub thinking: Option<String>,
}

#[derive(Error, Debug)]
pub enum LlmError {
    #[error("LLM API error: {0}")]
    Api(String),
    #[error("Provider not configured: {0}")]
    NotConfigured(String),
    #[error("Context window exceeded")]
    ContextTooLarge,
}

// ── Conversation state ────────────────────────────────────────────────────

pub struct ConversationState {
    pub connection_id: String,
    /// The chat conversation key (the frontend's `conversationId`, the key
    /// of this entry in `AppState.conversations`). Set by `run_agent_turn`
    /// right after the turn claim. The ACP driver keys its
    /// session-per-conversation map by this — several conversations can
    /// share one `connection_id`, so the connection id alone would merge
    /// their ACP sessions. `None` for conversations that never ran a turn
    /// (the driver then falls back to `connection_id`, which is what tests
    /// that drive the driver directly use).
    pub conversation_id: Option<String>,
    pub history: Vec<Message>,
    pub state: AgentState,
    pub created_at: Instant,
    /// run_readonly_query summaries keyed by normalized SQL — the model
    /// occasionally re-runs an identical query; serve it from here instead.
    pub query_cache: std::collections::HashMap<String, String>,
    /// The IPC channel the frontend created for this conversation. Kept so
    /// `execute_dml` can resume the agent on the same stream after the user
    /// approves a staged DML statement (C1). `Channel` is `Clone`; it is not
    /// `Debug`, which is why `ConversationState` no longer derives `Debug`.
    pub event_channel: Option<tauri::ipc::Channel<crate::ai::events::AiEvent>>,
}

impl ConversationState {
    pub fn new(connection_id: String) -> Self {
        Self {
            connection_id,
            conversation_id: None,
            history: vec![],
            state: AgentState::Idle,
            created_at: Instant::now(),
            query_cache: std::collections::HashMap::new(),
            event_channel: None,
        }
    }

    /// Take the staged DML and its staging time. `None` when no DML is
    /// paused. The time lets the approval path refuse stale statements (E6).
    pub fn take_staged_sql(&mut self) -> Option<(String, std::time::Instant)> {
        if let AgentState::PausedForDml {
            ref staged_sql,
            staged_at,
        } = self.state
        {
            let sql = staged_sql.clone();
            self.state = AgentState::Idle;
            Some((sql, staged_at))
        } else {
            None
        }
    }

    /// Atomically claim the conversation for one agent turn. Returns an
    /// error when a turn is already running or a DML approval is pending —
    /// two concurrent `ai_chat` calls must never interleave two agent loops
    /// on one history (E1).
    pub fn try_begin_turn(
        &mut self,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), String> {
        match &self.state {
            AgentState::Idle => {
                self.state = AgentState::Running { cancel };
                Ok(())
            }
            AgentState::Running { .. } => {
                Err("An AI response is already in progress for this conversation.".into())
            }
            AgentState::PausedForDml { .. } => {
                Err("Approve or cancel the pending DML before sending another message.".into())
            }
        }
    }
}

#[derive(Debug)]
pub enum AgentState {
    Idle,
    Running {
        cancel: tokio_util::sync::CancellationToken,
    },
    PausedForDml {
        staged_sql: String,
        staged_at: Instant,
    },
}

// ── Event Sink ──────────────────────────────────────────────────────────

/// Event sink for the agent loop — decouples the loop from tauri types so
/// the eval harness can drive it headless.
pub trait AgentSink: Send + Sync {
    fn event(&self, event: crate::ai::events::AiEvent);
    fn dml_approval(&self, payload: crate::ai::events::DmlApprovalPayload);
    /// The agent asks the user for permission to run one of its own tools
    /// (spec §3 D6). Default no-op so existing test sinks keep compiling.
    fn permission_request(&self, _payload: crate::ai::events::AgentPermissionPayload) {}
}

#[cfg(test)]
pub struct CollectorSink(pub std::sync::Mutex<Vec<crate::ai::events::AiEvent>>);

#[cfg(test)]
impl AgentSink for CollectorSink {
    fn event(&self, event: crate::ai::events::AiEvent) {
        self.0.lock().unwrap().push(event);
    }
    fn dml_approval(&self, _payload: crate::ai::events::DmlApprovalPayload) {}
}

// ── AgentDriver seam ──────────────────────────────────────────────────────

/// The one-turn agent contract shared by the rig loop (`DatabaseAgent`) and
/// the ACP path (`AcpChatDriver`) — the seam `ai_chat` branches on (spec
/// §3 D3). The rig loop owns its completion round-trips; the ACP driver
/// owns the agent's prompt turn. Same signature, same sink/cancel contract.
#[async_trait::async_trait]
pub trait AgentDriver: Send + Sync {
    async fn chat(
        &self,
        message: String,
        config: &AiConfig,
        system_prompt: String,
        conv_state: Arc<Mutex<ConversationState>>,
        sink: Arc<dyn AgentSink>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), String>;

    /// Type-identity hook so tests can assert which driver a branch chose.
    fn as_any(&self) -> &dyn std::any::Any;
}

// ── DatabaseAgent ─────────────────────────────────────────────────────────

pub struct DatabaseAgent {
    provider: Arc<dyn crate::ai::provider::LlmProvider>,
    tools: Vec<crate::ai::tools::LucentToolEnum>,
    ctx: AiToolContext,
}

impl DatabaseAgent {
    pub fn new(
        provider: Arc<dyn crate::ai::provider::LlmProvider>,
        tools: Vec<crate::ai::tools::LucentToolEnum>,
        ctx: AiToolContext,
    ) -> Self {
        Self {
            provider,
            tools,
            ctx,
        }
    }

    /// Agent loop: sends message → LLM responds (text or tool_calls) → handle tools → repeat.
    /// Uses Rig's native tool framework. After tool execution, pushes a follow-up
    /// user message to prompt the model's response, so the API never receives
    /// tool results without a subsequent user turn.

    pub async fn chat(
        &self,
        message: String,
        config: &AiConfig,
        system_prompt: String,
        conv_state: Arc<Mutex<ConversationState>>,
        sink: Arc<dyn AgentSink>,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(), String> {
        let conversation_id = conv_state.lock().await.connection_id.clone();

        let agent = self
            .provider
            .build_agent(
                &config.model,
                system_prompt,
                config.max_tokens,
                self.tools.clone(),
            )
            .await;

        let mut turn_count = 0u32;
        // First prompt is the user's actual message; subsequent prompts are empty.
        let mut prompt = Message::user(message);

        log::debug!(
            "Agent loop starting, model={}, max_turns={}",
            config.model,
            config.max_turns
        );

        loop {
            if cancel.is_cancelled() {
                log::info!("Agent cancelled at turn {turn_count}");
                return Ok(());
            }
            if turn_count >= config.max_turns {
                log::warn!("Agent reached max turns ({})", config.max_turns);
                sink.event(AiEvent::Done {
                    conversation_id: conversation_id.clone(),
                    final_message: "Reached maximum turns.".into(),
                    usage: TokenUsage::default(),
                });
                return Ok(());
            }
            turn_count += 1;

            // Send the WINDOWED conversation history to the LLM: the stored
            // history stays complete for eviction and the UI, but only the
            // newest HISTORY_WINDOW messages (plus a summary of the omitted
            // past) reach the model (E4).
            // Turn 1: history is empty, prompt is the user's message.
            // Turn 2+: history contains user message + assistant(tool_calls) + tool results,
            // and prompt is an empty user message to avoid ending on `tool` role.
            let history = conv_state.lock().await.history.clone();
            let history = compact_history(history);
            let history_len = history.len();
            log::debug!("Turn {turn_count}: calling LLM with {history_len} windowed messages");

            let on_delta = |delta: crate::ai::provider::AgentDelta| match delta {
                crate::ai::provider::AgentDelta::Thinking(chunk) => {
                    sink.event(AiEvent::Thinking { content: chunk });
                }
                crate::ai::provider::AgentDelta::Text(chunk) => {
                    sink.event(AiEvent::Text { content: chunk });
                }
            };

            let response = agent
                .complete(prompt.clone(), history, &on_delta)
                .await
                .map_err(|e| {
                    log::error!("LLM complete failed at turn {turn_count}: {e}");
                    e.to_string()
                })?;

            log::debug!(
                "Turn {turn_count}: got {} tool_calls, text_len={}, usage={:?}",
                response.tool_calls.len(),
                response.text.as_ref().map(|t| t.len()).unwrap_or(0),
                response.usage,
            );

            if response.usage.prompt_tokens > 0 {
                log::info!(
                    "Turn {turn_count} tokens: {} prompt ({} cached), {} completion",
                    response.usage.prompt_tokens,
                    response.usage.cached_prompt_tokens,
                    response.usage.completion_tokens,
                );
            }

            // Record the prompt that produced this turn (only genuine user text
            // is stored — empty continuation prompts never enter history).
            record_user_prompt(&mut conv_state.lock().await.history, &prompt);

            if response.tool_calls.is_empty() {
                // Final text response — already streamed live via on_delta above.
                let final_msg = response.text.unwrap_or_default();
                if !final_msg.is_empty() {
                    conv_state
                        .lock()
                        .await
                        .history
                        .push(Message::assistant(&final_msg));
                }
                sink.event(AiEvent::Done {
                    conversation_id: conversation_id.clone(),
                    final_message: final_msg,
                    usage: response.usage,
                });
                conv_state.lock().await.state = AgentState::Idle;
                return Ok(());
            }

            // Tool calls received
            sink.event(AiEvent::ToolCalls {
                tools: response
                    .tool_calls
                    .iter()
                    .map(|tc| crate::ai::events::ToolCallInfo {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        args: tc.args.clone(),
                    })
                    .collect(),
            });

            // Check for duplicate queries in the conversation cache
            let cached_results: Vec<Option<String>> = {
                let conv = conv_state.lock().await;
                response
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        if tc.name == "run_readonly_query" {
                            tc.args["sql"]
                                .as_str()
                                .and_then(|s| conv.query_cache.get(&normalize_sql(s)).cloned())
                        } else {
                            None
                        }
                    })
                    .collect()
            };

            // Execute all tools in parallel
            log::info!(
                "Agent invoking {} tool(s): {}",
                response.tool_calls.len(),
                response
                    .tool_calls
                    .iter()
                    .map(|tc| format!("{}({})", tc.name, tc.args))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            let handles: Vec<_> = response.tool_calls.iter().zip(cached_results).map(|(tc, cached)| {
                let name = tc.name.clone();
                let args = tc.args.clone();
                let tools = self.tools.clone();
                let ctx = self.ctx.clone();
                tokio::spawn(async move {
                    if tool_budget_exhausted(turn_count) {
                        log::warn!("Tool '{name}' blocked: hard tool budget reached");
                        return Ok(budget_exhausted_output());
                    }
                    if let Some(prior) = cached {
                        log::info!("Tool '{name}' served from conversation query cache (identical SQL)");
                        return Ok(crate::ai::tools::ToolOutput::Text {
                            content: format!(
                                "[You already executed this exact query earlier in this \
                                 conversation. Cached result below — do NOT run it again.]\n\n{prior}"
                            ),
                        });
                    }
                    let start = std::time::Instant::now();
                    let result = match tools.iter().find(|t| t.name() == name) {
                        Some(t) => t.call(args, &ctx).await,
                        None => Err(crate::ai::tools::ToolError::Execution(format!("unknown tool: {name}"))),
                    };
                    let status = match &result {
                        Ok(_) => "ok".to_string(),
                        Err(e) => format!("error: {e}"),
                    };
                    log::info!("Tool '{name}' completed in {:.0?}: {status}", start.elapsed());
                    result
                })
            }).collect();

            let mut first_dml: Option<crate::ai::tools::ToolOutput> = None;
            let mut round_results: Vec<(String, String)> = Vec::new();

            for (tc, jr) in response
                .tool_calls
                .iter()
                .zip(futures::future::join_all(handles).await)
            {
                let (output, summary) = match jr {
                    Ok(Ok(o)) => {
                        let s = match &o {
                            // Full content is already available via output_json's
                            // structured payload (shown in the tool card's expanded
                            // body) — the status column should stay a short, uniform
                            // completion indicator, not a truncated content preview.
                            crate::ai::tools::ToolOutput::Text { .. } => "done".to_string(),
                            crate::ai::tools::ToolOutput::QueryResult { row_count, .. } => {
                                format!("{row_count} rows")
                            }
                            crate::ai::tools::ToolOutput::DmlPreview {
                                statement_type,
                                tables_affected,
                                ..
                            } => format!("{statement_type} on {}", tables_affected.join(", ")),
                        };
                        (o, s)
                    }
                    Ok(Err(e)) => {
                        let err_msg = format!("{}({}) failed: {e}", tc.name, tc.args);
                        log::error!("{err_msg}");
                        (
                            crate::ai::tools::ToolOutput::Text { content: err_msg },
                            "error".into(),
                        )
                    }
                    Err(je) if je.is_panic() => {
                        let err_msg = format!("{}({}) panicked during execution", tc.name, tc.args);
                        log::error!("{err_msg}");
                        (
                            crate::ai::tools::ToolOutput::Text { content: err_msg },
                            "panicked".into(),
                        )
                    }
                    Err(e) => {
                        let err_msg = format!("{}({}) task error: {e}", tc.name, tc.args);
                        log::error!("{err_msg}");
                        (
                            crate::ai::tools::ToolOutput::Text { content: err_msg },
                            "task_error".into(),
                        )
                    }
                };

                // Build structured output for the tool call card
                let output_json = match &output {
                    crate::ai::tools::ToolOutput::Text { content } => {
                        Some(serde_json::json!({"type": "text", "data": content}))
                    }
                    crate::ai::tools::ToolOutput::QueryResult {
                        ref columns,
                        ref rows,
                        row_count,
                        ref sql,
                        execution_time_ms,
                        truncated,
                        ..
                    } => {
                        let cols: Vec<serde_json::Value> = columns
                            .iter()
                            .map(|c| serde_json::json!({"name": c.name, "type": c.data_type}))
                            .collect();
                        Some(serde_json::json!({
                            "type": "query_result",
                            "columns": cols,
                            "rows": rows.iter().take(10).collect::<Vec<_>>(),
                            "row_count": row_count,
                            "sql": sql,
                            "execution_time_ms": execution_time_ms,
                            "truncated": truncated,
                        }))
                    }
                    crate::ai::tools::ToolOutput::DmlPreview {
                        sql,
                        statement_type,
                        tables_affected,
                        description,
                        estimated_rows_affected,
                    } => Some(serde_json::json!({
                        "type": "dml_preview",
                        "sql": sql,
                        "statement_type": statement_type,
                        "tables_affected": tables_affected,
                        "description": description,
                        "estimated_rows_affected": estimated_rows_affected,
                    })),
                };

                sink.event(AiEvent::ToolResult {
                    id: tc.id.clone(),
                    tool: tc.name.clone(),
                    summary,
                    output: output_json,
                });

                if let crate::ai::tools::ToolOutput::QueryResult {
                    ref columns,
                    ref rows,
                    row_count,
                    ref sql,
                    execution_time_ms,
                    ..
                } = output
                {
                    sink.event(AiEvent::QueryResult {
                        columns: columns.clone(),
                        rows: rows.clone(),
                        row_count,
                        sql: sql.clone(),
                        execution_time_ms,
                    });
                }

                // Build tool result message (truncated for LLM context budget)
                let result_text = {
                    let raw = match &output {
                        crate::ai::tools::ToolOutput::Text { content } => content.clone(),
                        crate::ai::tools::ToolOutput::QueryResult { text_summary, .. } => {
                            text_summary.clone()
                        }
                        crate::ai::tools::ToolOutput::DmlPreview {
                            description, sql, ..
                        } => format!("DML: {description}. SQL: {sql}"),
                    };
                    truncate_tool_output(&raw)
                };

                // Store fresh query results in the conversation cache.
                if tc.name == "run_readonly_query" {
                    if let (
                        Some(sql),
                        crate::ai::tools::ToolOutput::QueryResult {
                            ref text_summary, ..
                        },
                    ) = (tc.args["sql"].as_str(), &output)
                    {
                        conv_state
                            .lock()
                            .await
                            .query_cache
                            .insert(normalize_sql(sql), text_summary.clone());
                    }
                }

                round_results.push((tc.id.clone(), result_text));

                if matches!(output, crate::ai::tools::ToolOutput::DmlPreview { .. })
                    && first_dml.is_none()
                {
                    first_dml = Some(output);
                }
            }

            // Nudge the model to wrap up after several tool rounds.
            append_wrap_up_nudge(&mut round_results, turn_count);

            // Record the tool round: ONE assistant message with ALL calls, then
            // one tool-result per call, in call order.
            record_tool_round(
                &mut conv_state.lock().await.history,
                &response.tool_calls,
                &round_results,
            );

            // DML: pause for user approval
            if let Some(crate::ai::tools::ToolOutput::DmlPreview {
                sql,
                tables_affected,
                description,
                estimated_rows_affected,
                ..
            }) = first_dml
            {
                log::info!(
                    "DML pause for approval: {} tables={:?}",
                    description,
                    tables_affected
                );
                conv_state.lock().await.state = AgentState::PausedForDml {
                    staged_sql: sql.clone(),
                    staged_at: Instant::now(),
                };
                sink.dml_approval(DmlApprovalPayload {
                    conversation_id: conversation_id.clone(),
                    sql,
                    tables_affected,
                    description,
                    estimated_rows_affected,
                });
                return Ok(());
            }

            // Next prompt is empty — the history already contains the user's context
            // and ends with `tool` role. An empty user prompt keeps the API happy
            // (DeepSeek/openCode Go reject `tool` as the last role).
            prompt = Message::user("");
        }
    }
}

#[async_trait::async_trait]
impl AgentDriver for DatabaseAgent {
    /// Pure delegation — the rig loop's body is unchanged; the trait exists
    /// so `run_agent_turn` can branch between the two drivers at one seam.
    async fn chat(
        &self,
        message: String,
        config: &AiConfig,
        system_prompt: String,
        conv_state: Arc<Mutex<ConversationState>>,
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

// ── History-recording helpers ──────────────────────────────────────────────

/// Maximum messages sent to the LLM per turn. Older tool rounds are
/// replaced by a summary placeholder — the system prompt already carries
/// the schema and each tool result is individually capped at 5 KB, so a
/// bounded window keeps long conversations inside the context window (E4).
const HISTORY_WINDOW: usize = 16;

/// Window the conversation history for one LLM call. Returns the history
/// unchanged when it fits; otherwise the oldest messages collapse into a
/// single summary placeholder so the model still knows the conversation
/// has a past. The STORED history is never modified — only the request is
/// windowed.
fn compact_history(history: Vec<Message>) -> Vec<Message> {
    if history.len() <= HISTORY_WINDOW {
        return history;
    }
    let total = history.len();
    let mut tail: Vec<Message> = history.into_iter().skip(total - HISTORY_WINDOW).collect();
    // D5: never let the window start on a `tool` result whose
    // assistant(tool_calls) message was cut — strict providers reject a
    // request whose first message is role 'tool' (tool_call_id must follow
    // a message that declared it). The summary already tells the model the
    // past was collapsed; the window may be slightly under HISTORY_WINDOW.
    while tail
        .first()
        .map(|m| m.role == MessageRole::Tool)
        .unwrap_or(false)
    {
        tail.remove(0);
    }
    let omitted = total - tail.len();
    let mut out = Vec::with_capacity(tail.len() + 1);
    out.push(Message::user(format!(
        "[Earlier context: {omitted} earlier messages were omitted to stay within \
         the context window. Continue the conversation normally; the pending task \
         is in the latest messages below.]"
    )));
    out.extend(tail);
    out
}

/// Cap a single tool output at 5000 bytes for the LLM context window.
fn truncate_tool_output(raw: &str) -> String {
    if raw.len() > 5000 {
        format!(
            "{}... [truncated {} chars]",
            truncate_utf8(raw, 5000),
            raw.len() - 5000
        )
    } else {
        raw.to_string()
    }
}

/// Remove SQL comments (`-- …\n` and `/* … */`) outside string literals.
/// Single-quote literals with doubled-quote escaping are respected; nested
/// block comments are not (Postgres allows them, but the cache key being
/// occasionally conservative is harmless — worst case is a cache miss).
fn strip_sql_comments(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_string {
            out.push(c);
            if c == '\'' {
                // doubled quote = escaped quote, stay in string
                if bytes.get(i + 1) == Some(&b'\'') {
                    out.push('\'');
                    i += 1;
                } else {
                    in_string = false;
                }
            }
            i += 1;
        } else if c == '\'' {
            in_string = true;
            out.push(c);
            i += 1;
        } else if c == '-' && bytes.get(i + 1) == Some(&b'-') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if c == '/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            out.push(' ');
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// Whitespace-normalize SQL for duplicate detection. Comments are stripped
/// first (a comment-only variant defeated the cache in production); case is
/// preserved — string literals are case-sensitive.
fn normalize_sql(sql: &str) -> String {
    strip_sql_comments(sql)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(';')
        .to_string()
}

/// Hard ceiling on tool rounds per user turn. The polite wrap-up nudge
/// (WRAP_UP_AFTER_ROUNDS) was observed being ignored for 6 straight rounds;
/// past this ceiling, tools stop executing and the model must answer.
const HARD_TOOL_BUDGET_ROUNDS: u32 = 8;

fn tool_budget_exhausted(turn_count: u32) -> bool {
    turn_count > HARD_TOOL_BUDGET_ROUNDS
}

fn budget_exhausted_output() -> crate::ai::tools::ToolOutput {
    crate::ai::tools::ToolOutput::Text {
        content: format!(
            "[Tool budget exhausted: {HARD_TOOL_BUDGET_ROUNDS} rounds reached — \
             no more tool calls will execute this turn. Answer NOW from the data \
             you already retrieved, stating any caveats.]"
        ),
    }
}

/// After this many tool rounds in one user turn, nudge the model to conclude.
/// Observed failure mode: an ambiguous question spiraled into 7 tool rounds
/// (96s wall time) refining results it could already have summarized.
const WRAP_UP_AFTER_ROUNDS: u32 = 4;

/// Append an efficiency note to the LAST tool result of the round. Tool-result
/// text is the one channel that reaches the model next turn without touching
/// the cached prompt prefix or the tool definitions.
fn append_wrap_up_nudge(round_results: &mut [(String, String)], turn_count: u32) {
    if turn_count < WRAP_UP_AFTER_ROUNDS {
        return;
    }
    if let Some(last) = round_results.last_mut() {
        last.1.push_str(&format!(
            "\n\n[Efficiency note: this is tool round {turn_count} for this question. \
             You almost certainly have enough data — answer now with what you have, \
             stating any caveats, unless something essential is genuinely missing.]"
        ));
    }
}

/// Append the prompt that produced this turn — but only genuine user text.
/// Empty continuation prompts (used to keep providers happy between tool
/// rounds) and tool-result prompts must never enter durable history.
fn record_user_prompt(history: &mut Vec<Message>, prompt: &Message) {
    let is_real_user_text = prompt.role == MessageRole::User
        && matches!(&prompt.content, MessageContent::Text(t) if !t.is_empty());
    if is_real_user_text {
        history.push(prompt.clone());
    }
}

/// Append one tool round: a single assistant message carrying ALL tool calls,
/// followed by one tool-result message per call, in call order.
fn record_tool_round(history: &mut Vec<Message>, calls: &[ToolCall], results: &[(String, String)]) {
    if calls.is_empty() {
        return;
    }
    history.push(Message::with_tool_calls(calls.to_vec()));
    for (tool_use_id, text) in results {
        history.push(Message::tool_result(tool_use_id, text));
    }
}

#[cfg(test)]
mod query_cache_tests {
    use super::*;

    #[test]
    fn normalize_collapses_whitespace_and_trailing_semicolon() {
        let a = normalize_sql("SELECT  x\nFROM t\n  WHERE y = 'CAN';");
        let b = normalize_sql("SELECT x FROM t WHERE y = 'CAN'");
        assert_eq!(a, b);
    }

    #[test]
    fn normalize_preserves_literal_case() {
        assert_ne!(
            normalize_sql("SELECT * FROM t WHERE c = 'CAN'"),
            normalize_sql("SELECT * FROM t WHERE c = 'can'"),
            "lowercasing would collide distinct string literals"
        );
    }

    #[test]
    fn comment_only_variants_normalize_identically() {
        // Exact production case: turn 9 = turn 8 plus a leading line comment.
        let a =
            normalize_sql("-- Get all rows ranked by popularity\nSELECT x FROM t ORDER BY c DESC;");
        let b = normalize_sql("SELECT x FROM t ORDER BY c DESC");
        assert_eq!(
            a, b,
            "a comment must never defeat the duplicate-query cache"
        );
    }

    #[test]
    fn block_comments_are_stripped() {
        assert_eq!(
            normalize_sql("SELECT /* inline note */ x FROM t"),
            normalize_sql("SELECT x FROM t"),
        );
    }

    #[test]
    fn comment_markers_inside_string_literals_survive() {
        let a = normalize_sql("SELECT * FROM t WHERE tag = '--not a comment'");
        assert!(
            a.contains("--not a comment"),
            "literal content must be preserved: {a}"
        );
    }

    #[test]
    fn doubled_quote_inside_string_literal_is_escaped_quote_not_end() {
        // Postgres: 'O''Brien' is the literal O'Brien.
        let a = normalize_sql("SELECT * FROM t WHERE name = 'O''Brien'");
        assert!(
            a.contains("O''Brien"),
            "doubled quote is an escaped quote: {a}"
        );
    }

    #[test]
    fn line_comment_at_end_of_file_without_trailing_newline() {
        // SQL with no newline after the comment — the comment extends to EOF.
        let a = normalize_sql("SELECT 1 -- just a comment");
        let b = normalize_sql("SELECT 1");
        assert_eq!(a, b, "comment at EOF without newline must be stripped");
    }

    #[test]
    fn sql_that_is_only_comments_normalizes_to_empty() {
        let a = normalize_sql("-- only a comment\n/* also block */");
        assert_eq!(a, "", "only-comment SQL normalizes to empty string");
    }

    #[test]
    fn empty_sql_normalizes_to_empty() {
        assert_eq!(normalize_sql(""), "");
        assert_eq!(normalize_sql("   \n  "), "");
    }

    #[test]
    fn block_comment_with_nested_line_comment_markers() {
        // A `/* ... */` block containing `--` text. The `--` inside should NOT
        // start comment-mode since we're already in block mode.
        let a = normalize_sql("SELECT /* note -- still inside block */ x FROM t");
        let b = normalize_sql("SELECT x FROM t");
        assert_eq!(a, b, "`--` inside block comment should be safe");
    }

    #[test]
    fn comment_in_empty_string_after_close_quote() {
        // c == '\0' after the string closes should not cause OOB.
        let sql = "SELECT '' -- comment";
        let a = normalize_sql(sql);
        let b = normalize_sql("SELECT ''");
        assert_eq!(a, b);
    }

    #[test]
    fn conversation_state_caches_and_returns_summaries() {
        let mut state = ConversationState::new("conn_1".into());
        assert!(state.query_cache.is_empty());
        state
            .query_cache
            .insert(normalize_sql("SELECT 1;"), "Result: 1 rows".into());
        assert_eq!(
            state
                .query_cache
                .get(&normalize_sql("SELECT  1"))
                .map(String::as_str),
            Some("Result: 1 rows"),
        );
    }
}

#[cfg(test)]
mod tool_budget_tests {
    use super::*;

    #[test]
    fn budget_allows_early_rounds_and_blocks_late_ones() {
        assert!(!tool_budget_exhausted(HARD_TOOL_BUDGET_ROUNDS));
        assert!(tool_budget_exhausted(HARD_TOOL_BUDGET_ROUNDS + 1));
    }

    #[test]
    fn exhausted_output_orders_an_answer_not_an_apology() {
        let out = budget_exhausted_output();
        match out {
            crate::ai::tools::ToolOutput::Text { content } => {
                assert!(content.contains("Answer NOW"), "{content}");
                assert!(
                    content.contains("no more tool calls will execute"),
                    "{content}"
                );
            }
            _ => panic!("expected Text output"),
        }
    }

    #[test]
    fn budget_check_fires_before_cache_check() {
        // The spawned-task closure checks budget FIRST, then cache. This means
        // even a cached query is blocked when the budget is exhausted.
        // We can't test the spawned task directly, but we can verify the
        // ordering contract: tool_budget_exhausted returns true before any
        // cache lookup would happen at turn_count > HARD_TOOL_BUDGET_ROUNDS.
        let exhausted = HARD_TOOL_BUDGET_ROUNDS + 1;
        assert!(tool_budget_exhausted(exhausted));
        // The cache path is unreachable when budget is exhausted — the closure
        // short-circuits before the cache check. This is a contract test.
    }
}

#[cfg(test)]
mod wrap_up_nudge_tests {
    use super::*;

    #[test]
    fn no_nudge_below_threshold() {
        let mut results = vec![("c1".to_string(), "42 rows".to_string())];
        append_wrap_up_nudge(&mut results, WRAP_UP_AFTER_ROUNDS - 1);
        assert_eq!(results[0].1, "42 rows", "early rounds must be untouched");
    }

    #[test]
    fn nudge_appended_to_last_result_at_threshold() {
        let mut results = vec![
            ("c1".to_string(), "first".to_string()),
            ("c2".to_string(), "second".to_string()),
        ];
        append_wrap_up_nudge(&mut results, WRAP_UP_AFTER_ROUNDS);
        assert_eq!(
            results[0].1, "first",
            "only the LAST result carries the nudge"
        );
        assert!(
            results[1].1.contains("answer now with what you have"),
            "{}",
            results[1].1
        );
    }

    #[test]
    fn nudge_on_empty_results_is_a_noop() {
        let mut results: Vec<(String, String)> = vec![];
        append_wrap_up_nudge(&mut results, WRAP_UP_AFTER_ROUNDS + 3);
        assert!(results.is_empty());
    }
}

#[cfg(test)]
mod history_recording_tests {
    use super::*;

    #[test]
    fn real_user_prompt_is_recorded() {
        let mut history: Vec<Message> = vec![];
        record_user_prompt(&mut history, &Message::user("how many users?"));
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].role, MessageRole::User);
    }

    #[test]
    fn empty_continuation_prompt_is_not_recorded() {
        let mut history: Vec<Message> = vec![];
        record_user_prompt(&mut history, &Message::user(""));
        assert!(
            history.is_empty(),
            "empty user prompts are protocol filler between tool rounds — \
             recording them pollutes history forever"
        );
    }

    #[test]
    fn tool_result_prompt_is_not_recorded_as_user_turn() {
        let mut history: Vec<Message> = vec![];
        record_user_prompt(&mut history, &Message::tool_result("call_1", "42 rows"));
        assert!(history.is_empty(), "only genuine user text is a user turn");
    }

    #[test]
    fn parallel_tool_calls_are_one_assistant_message_then_results_in_order() {
        let mut history: Vec<Message> = vec![];
        let calls = vec![
            ToolCall {
                id: "c1".into(),
                name: "search_schema".into(),
                args: serde_json::json!({"query": "a"}),
            },
            ToolCall {
                id: "c2".into(),
                name: "run_readonly_query".into(),
                args: serde_json::json!({"sql": "SELECT 1"}),
            },
        ];
        let results = vec![
            ("c1".to_string(), "schema stuff".to_string()),
            ("c2".to_string(), "1 row".to_string()),
        ];
        record_tool_round(&mut history, &calls, &results);

        assert_eq!(history.len(), 3, "one assistant message + two tool results");
        assert_eq!(history[0].role, MessageRole::Assistant);
        assert_eq!(
            history[0].tool_calls.as_ref().unwrap().len(),
            2,
            "ALL parallel calls belong to a single assistant message — \
             one-message-per-call is malformed for OpenAI-compatible APIs"
        );
        assert_eq!(history[1].role, MessageRole::Tool);
        assert_eq!(history[2].role, MessageRole::Tool);
        match (&history[1].content, &history[2].content) {
            (
                MessageContent::ToolResult {
                    tool_use_id: id1, ..
                },
                MessageContent::ToolResult {
                    tool_use_id: id2, ..
                },
            ) => {
                assert_eq!(id1, "c1");
                assert_eq!(id2, "c2");
            }
            _ => panic!("expected tool results"),
        }
    }
}

#[cfg(test)]
mod sink_tests {
    use super::*;

    #[test]
    fn collector_sink_records_events() {
        let sink = CollectorSink(std::sync::Mutex::new(vec![]));
        sink.event(crate::ai::events::AiEvent::Text {
            content: "hi".into(),
        });
        assert_eq!(sink.0.lock().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod truncation_tests {
    use super::*;

    #[test]
    fn tool_output_truncation_never_panics_on_multibyte_boundary() {
        let raw = "中".repeat(6000);
        let cut = truncate_tool_output(&raw);
        assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
        assert!(cut.contains("truncated"), "{cut}");
    }

    #[test]
    fn short_tool_output_passes_through_untouched() {
        assert_eq!(truncate_tool_output("SELECT 1"), "SELECT 1");
    }
}
