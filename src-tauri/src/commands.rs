use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use lucent_protocol::{ConnectionConfig, ConnectionId, QueryId};
use serde::Serialize;
use tauri::{Emitter, Manager, State};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::ai::acp::install::InstalledAgent;
use crate::ai::agent::{AgentDriver, AgentSink, AgentState, ConversationState, DatabaseAgent};
use crate::ai::config::{keychain_account, AiConfig, AiProvider, KEYCHAIN_SERVICE};
use crate::ai::context::SchemaCache;
use crate::ai::events::{AiErrorPayload, AiEvent, TokenUsage};
use crate::ai::provider::LlmProvider;
use crate::ai::providers::rig::RigProvider;
use crate::ai::tools::AiToolContext;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tracing::Instrument;

use crate::client::{ConnectorClient, ExecuteResult};
use crate::connections::ConnectionProfileRepository;
use crate::supervisor::Supervisor;

/// Tauri-side sink bridging the agent loop to IPC events. Generic over the
/// runtime so tests can drive `run_agent_turn` with `tauri::test::mock_app`
/// (production uses the default `Wry`).
pub(crate) struct TauriSink<R: tauri::Runtime> {
    channel: tauri::ipc::Channel<crate::ai::events::AiEvent>,
    app_handle: tauri::AppHandle<R>,
    conversation_id: String,
    assistant_message_id: Option<String>,
    turn_text: Arc<std::sync::Mutex<String>>,
    turn_segments: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    turn_start_ms: i64,
}

fn finalize_active_thinking(segs: &mut [serde_json::Value]) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    if let Some(last) = segs.last_mut() {
        if last.get("type").and_then(|v| v.as_str()) == Some("thinking")
            && last
                .get("streaming")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        {
            last["streaming"] = serde_json::Value::Bool(false);
            if let Some(started) = last.get("startedAt").and_then(|v| v.as_i64()) {
                last["durationMs"] =
                    serde_json::Value::Number(serde_json::Number::from((now_ms - started).max(0)));
            }
        }
    }
}

impl<R: tauri::Runtime> AgentSink for TauriSink<R> {
    fn event(&self, event: crate::ai::events::AiEvent) {
        match &event {
            crate::ai::events::AiEvent::Thinking { content } => {
                let mut segs = self.turn_segments.lock().unwrap();
                let now_ms = chrono::Utc::now().timestamp_millis();
                if let Some(last) = segs.last_mut() {
                    if last.get("type").and_then(|v| v.as_str()) == Some("thinking")
                        && last
                            .get("streaming")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    {
                        if let Some(c) = last.get_mut("content") {
                            if let Some(s) = c.as_str() {
                                *c = serde_json::Value::String(format!("{s}{content}"));
                            }
                        }
                    } else {
                        segs.push(serde_json::json!({
                            "type": "thinking",
                            "content": content,
                            "streaming": true,
                            "startedAt": now_ms,
                        }));
                    }
                } else {
                    segs.push(serde_json::json!({
                        "type": "thinking",
                        "content": content,
                        "streaming": true,
                        "startedAt": now_ms,
                    }));
                }
            }
            crate::ai::events::AiEvent::Text { content } => {
                let mut segs = self.turn_segments.lock().unwrap();
                finalize_active_thinking(&mut segs);
                let mut text = self.turn_text.lock().unwrap();
                text.push_str(content);
            }
            crate::ai::events::AiEvent::Notice { content } => {
                let mut segs = self.turn_segments.lock().unwrap();
                finalize_active_thinking(&mut segs);
                segs.push(serde_json::json!({
                    "type": "note",
                    "content": content,
                }));
            }
            crate::ai::events::AiEvent::ToolCalls { tools } => {
                let mut segs = self.turn_segments.lock().unwrap();
                finalize_active_thinking(&mut segs);
                for t in tools {
                    segs.push(serde_json::json!({
                        "type": "tool_call",
                        "call": {
                            "id": t.id,
                            "name": t.name,
                            "args": t.args,
                            "summary": serde_json::Value::Null,
                            "status": "running",
                        }
                    }));
                }
            }
            crate::ai::events::AiEvent::ToolResult {
                id,
                summary,
                output,
                input,
                status,
                ..
            } => {
                let mut segs = self.turn_segments.lock().unwrap();
                for seg in segs.iter_mut() {
                    if seg.get("type").and_then(|v| v.as_str()) == Some("tool_call") {
                        if let Some(call) = seg.get_mut("call") {
                            if call.get("id").and_then(|v| v.as_str()) == Some(id) {
                                call["summary"] = serde_json::Value::String(summary.clone());
                                if let Some(out) = output {
                                    call["output"] = out.clone();
                                }
                                if let Some(inp) = input {
                                    call["args"] = inp.clone();
                                }
                                call["status"] = serde_json::Value::String(match status {
                                    crate::ai::events::ToolResultStatus::Completed => {
                                        "completed".into()
                                    }
                                    crate::ai::events::ToolResultStatus::Failed => "failed".into(),
                                });
                            }
                        }
                    }
                }
            }
            crate::ai::events::AiEvent::Done {
                usage,
                final_message,
                cancelled,
                ..
            } => {
                let mut segs = self.turn_segments.lock().unwrap();
                finalize_active_thinking(&mut segs);
                if *cancelled {
                    for seg in segs.iter_mut() {
                        if seg.get("type").and_then(|v| v.as_str()) == Some("tool_call") {
                            if let Some(call) = seg.get_mut("call") {
                                if call.get("status").and_then(|v| v.as_str()) == Some("running") {
                                    call["status"] = serde_json::Value::String("stopped".into());
                                }
                            }
                        }
                    }
                }

                let state = self.app_handle.state::<AppState>();
                let mut entry = state
                    .llm_usage
                    .entry(self.conversation_id.clone())
                    .or_default();
                let accumulated = accumulate_usage(&entry, usage);
                *entry = accumulated;

                let streamed_text = self.turn_text.lock().unwrap().clone();
                let content = if !final_message.is_empty() {
                    final_message.clone()
                } else {
                    streamed_text
                };

                let session_json = if !segs.is_empty() {
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    Some(
                        serde_json::json!({
                            "segments": *segs,
                            "startedAt": self.turn_start_ms,
                            "durationMs": (now_ms - self.turn_start_ms).max(0),
                            "active": false,
                        })
                        .to_string(),
                    )
                } else {
                    None
                };

                if !content.is_empty() || session_json.is_some() {
                    let mem_mgr = state.memory_manager.clone();
                    let conv_id = self.conversation_id.clone();
                    let msg_id = self
                        .assistant_message_id
                        .clone()
                        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                    let now = chrono::Utc::now().timestamp();
                    tauri::async_runtime::spawn(async move {
                        let msg = crate::ai::memory::ChatMessage {
                            id: msg_id,
                            conversation_id: conv_id,
                            role: "assistant".into(),
                            content,
                            session_json,
                            created_at: now,
                        };
                        if let Err(e) = mem_mgr.save_message(msg).await {
                            log::error!("Failed to save assistant chat message: {e}");
                        }
                    });
                }
            }
            _ => {}
        }
        let _ = self.channel.send(event);
    }
    fn dml_approval(&self, payload: crate::ai::events::DmlApprovalPayload) {
        let mut segs = self.turn_segments.lock().unwrap();
        finalize_active_thinking(&mut segs);
        let _ = self.app_handle.emit("ai:dml_approval", payload);
    }
    fn permission_request(&self, payload: crate::ai::events::AgentPermissionPayload) {
        let mut segs = self.turn_segments.lock().unwrap();
        finalize_active_thinking(&mut segs);
        let _ = self.app_handle.emit("ai:agent_permission", payload);
    }
}

/// Pure accumulation of one run's usage into a conversation's totals.
pub(crate) fn accumulate_usage(existing: &TokenUsage, new: &TokenUsage) -> TokenUsage {
    TokenUsage {
        prompt_tokens: existing.prompt_tokens.saturating_add(new.prompt_tokens),
        completion_tokens: existing
            .completion_tokens
            .saturating_add(new.completion_tokens),
        cached_prompt_tokens: existing
            .cached_prompt_tokens
            .saturating_add(new.cached_prompt_tokens),
    }
}

#[derive(Debug, Serialize)]
pub struct CommandError {
    pub kind: String,
    pub message: String,
}

impl CommandError {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

/// Test-only parking gate for the memory embedder's lazy initialization.
/// `get_or_init_memory_embedder` records each arrival and then parks until the
/// owning test aborts the task, so a prompt build can be observed *inside* the
/// memory block without ever loading the real ONNX model.
#[cfg(test)]
pub struct MemoryEmbedderTestGate {
    entered: std::sync::atomic::AtomicUsize,
    parked: tokio::sync::Semaphore,
    /// When set, embedder initialization reports "unavailable" immediately —
    /// as if the ONNX model failed to load — so a test can exercise the
    /// lexical-only memory block without downloading the 130 MB model.
    unavailable: bool,
}

#[cfg(test)]
impl MemoryEmbedderTestGate {
    pub fn new() -> Self {
        Self {
            entered: std::sync::atomic::AtomicUsize::new(0),
            // Zero permits: `acquire` never resolves, so parked builders only
            // leave via task abort (and the model is never downloaded).
            parked: tokio::sync::Semaphore::new(0),
            unavailable: false,
        }
    }

    /// A gate that makes embedder initialization fail fast (return `None`),
    /// keeping the memory block's lexical retrieval path testable offline.
    pub fn unavailable() -> Self {
        Self {
            entered: std::sync::atomic::AtomicUsize::new(0),
            parked: tokio::sync::Semaphore::new(0),
            unavailable: true,
        }
    }

    /// How many builders have reached embedder initialization.
    pub fn entered(&self) -> usize {
        self.entered.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn is_unavailable(&self) -> bool {
        self.unavailable
    }

    async fn arrive_and_park(&self) {
        self.entered
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let _ = self.parked.acquire().await;
    }
}

#[cfg(test)]
impl Default for MemoryEmbedderTestGate {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AppState {
    /// Connection profiles and SSH configs repository.
    pub repo: Arc<ConnectionProfileRepository>,
    pub supervisor: Mutex<Option<Supervisor>>,
    /// API key cache: keychain access can stall seconds on first touch, and
    /// we otherwise hit it on every single message.
    pub api_key_cache: Arc<RwLock<Option<(crate::ai::config::AiProvider, String)>>>,
    /// Decrypted connection secrets (profile_id → password), cached so the
    /// keychain is read at most once per launch. Keychain access can stall for
    /// seconds and — whenever the binary's code signature changes (every dev
    /// rebuild) — macOS prompts for the login password on EVERY access.
    /// Connect, test-connection, AI chat, and notebooks all need the same
    /// secret; this keeps them to a single keychain read.
    pub password_cache: Arc<RwLock<HashMap<String, String>>>,
    /// Hold this lock only to clone the Arc. Never hold it across `.execute().await` — a long query would serialize every other query and deadlock `cancel`.
    pub client: Arc<Mutex<Option<ConnectorClient>>>,
    /// The ConnectionId assigned by the worker for the current connection.
    /// `Arc` so the background indexer can hold a clone to check ownership of
    /// the shared schema-graph slot (swap guard) without touching the
    /// AppState struct.
    pub current_connection_id: Arc<Mutex<Option<ConnectionId>>>,
    /// Postgres session dedicated to AI tools, preflight, and DML — so BEGIN/
    /// ROLLBACK and statement_timeout on the AI path can never touch the
    /// editor's session (the same worker socket, a different ConnectionId).
    pub ai_connection_id: Mutex<Option<ConnectionId>>,
    /// The in-flight editor query (if any) — used by cancel_query. Set before
    /// execute().await, cleared after; holds the newest in-flight editor query;
    /// older completions must not clear a newer registration.
    pub editor_query: Mutex<Option<(QueryId, ConnectionId)>>,
    pub current_database: Mutex<Option<String>>,
    pub current_connection_config: Mutex<Option<lucent_protocol::ConnectionConfig>>,
    /// The connection key the memory subsystem actually uses for the live
    /// connection: the profile id when a profile is connected, else the
    /// frontend's inline `host:port/database` key (App.svelte's
    /// `activeConnectionId`). Distinct from the vector-cache `connection_key`
    /// (`cache_store::connection_key_for`); used to scope the Gate 1 drift
    /// cascade to the keys `memories.connection_key` actually holds. `None`
    /// when disconnected.
    pub memory_connection_key: Mutex<Option<String>>,
    // AI module state
    pub conversations: DashMap<String, Arc<Mutex<ConversationState>>>,
    pub ai_config: Arc<RwLock<AiConfig>>,
    pub schema_cache: Arc<SchemaCache>,
    pub schema_graph: Arc<Mutex<Option<crate::ai::schema_graph::SchemaGraph>>>,
    pub embedder: Arc<Mutex<Option<crate::ai::embed::Embedder>>>,
    /// Lazily-initialized 512-token memory embedder. `OnceCell` (not a
    /// `Mutex<Option<_>>`) so no lock is held across the 130 MB model
    /// download/load: concurrent readers of an already-initialized embedder
    /// are never blocked, and a failed init leaves the cell empty so a later
    /// call retries (see B-I9).
    pub memory_embedder: Arc<tokio::sync::OnceCell<crate::ai::embed::Embedder>>,
    /// Test-only seam: when set, `get_or_init_memory_embedder` parks at the
    /// top before any lock or model work. The lock-scope (α) and concurrency
    /// (ζ) regression tests use it to hold a prompt build inside the memory
    /// block without triggering a real 130 MB model download. Always `None`
    /// in production.
    #[cfg(test)]
    pub memory_embedder_test_gate: Option<Arc<MemoryEmbedderTestGate>>,
    pub reranker: Arc<Mutex<Option<crate::ai::rerank::Reranker>>>,
    pub memory_manager: Arc<crate::ai::memory::MemoryManager>,
    /// Paths the user explicitly chose in a native save dialog this session or
    /// a previous one (persisted to `<config_dir>/lucent/approved_save_paths.json`).
    /// Write commands only ever touch paths in this set — the frontend is an
    /// untrusted boundary, so raw IPC paths are never written directly.
    pub approved_save_paths: Arc<Mutex<std::collections::HashSet<std::path::PathBuf>>>,
    pub notebook_sessions: DashMap<String, crate::notebook::session::NotebookSession>,
    /// Ring buffer of worker stderr lines for the in-app Logs drawer: the
    /// supervisor's drain task appends; `get_logs` tails from the frontend.
    pub logs: crate::supervisor::LogBuffer,
    /// Per-conversation accumulated LLM token usage, keyed by conversation id.
    /// Fed by `TauriSink` on every `AiEvent::Done`; read by `get_ai_usage`.
    pub llm_usage: DashMap<String, TokenUsage>,
    /// Session-frozen profile snapshots, keyed by conversation id. Built once
    /// on the conversation's first memory-enabled turn and reused unchanged
    /// for the rest of the session (rules learned mid-conversation do not
    /// retroactively alter a prompt already in flight).
    pub profile_snapshots: Arc<DashMap<String, crate::ai::memory::ProfileSnapshot>>,
    /// Shared idle clock for the sleep-time compute daemon. Query execution and
    /// AI chat `touch()` it; the daemon in `lib.rs` reads it to decide when the
    /// app has been idle long enough to run consolidation. Owned here (rather
    /// than managed separately by Tauri) so both the hot paths and the daemon
    /// see one tracker.
    pub idle_tracker: Arc<crate::ai::memory::IdleTracker>,
    /// Capabilities of the connected driver. `None` when disconnected.
    /// Phase 2 moves this onto `LiveConnection`; `capabilities()` is the seam
    /// that keeps that a small change.
    pub driver_capabilities: Mutex<Option<lucent_protocol::DriverCapabilities>>,
    /// Background schema indexing manager. Holds per-connection indexing tasks,
    /// persistent BLAKE3 cache store, and telemetry emitter.
    pub indexing: crate::ai::indexer::IndexingManager,
    /// HTTP client for the ACP agent registry (fetch + binary downloads),
    /// rustls-only. Built once here so tests can construct AppState without a
    /// Tauri runtime; the 60s timeout bounds both feed fetches and downloads.
    pub acp_http: reqwest::Client,
    /// The shared ACP subsystem: process manager, per-conversation bridge
    /// handles, permission registry, connection/session state (phase D).
    /// `Clone` is cheap — the chat driver takes a copy per turn.
    pub acp: crate::ai::acp::AcpState,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self::with_indexing_sink(Arc::new(crate::ai::indexer::LoggingSink))
    }

    /// Build the app state with a specific indexing telemetry sink. Production
    /// passes the Tauri emitter (wired in lib.rs setup); tests use the default
    /// logging sink so they need no Tauri runtime.
    pub fn with_indexing_sink(sink: Arc<dyn crate::ai::indexer::IndexingEventSink>) -> Self {
        let ai_config = crate::ai::config::load_config_from_disk();
        let cache =
            crate::ai::cache_store::PersistentVectorCache::open_default().unwrap_or_else(|e| {
                log::warn!("vector cache unavailable: {e}");
                let tmp = std::env::temp_dir()
                    .join(format!("lucent-cache-fallback-{}", std::process::id()));
                crate::ai::cache_store::PersistentVectorCache::open_at(tmp)
                    .expect("fallback cache opens")
            });
        let indexing = crate::ai::indexer::IndexingManager::new(cache, sink);
        Self {
            repo: Arc::new(ConnectionProfileRepository::load()),
            supervisor: Mutex::new(None),
            client: Arc::new(Mutex::new(None)),
            current_connection_id: Arc::new(Mutex::new(None)),
            ai_connection_id: Mutex::new(None),
            editor_query: Mutex::new(None),
            current_database: Mutex::new(None),
            current_connection_config: Mutex::new(None),
            memory_connection_key: Mutex::new(None),
            driver_capabilities: Mutex::new(None),
            conversations: DashMap::new(),
            schema_cache: Arc::new(SchemaCache::new(ai_config.schema_cache_ttl_secs)),
            ai_config: Arc::new(RwLock::new(ai_config)),
            schema_graph: Arc::new(Mutex::new(None)),
            embedder: Arc::new(Mutex::new(None)),
            memory_embedder: Arc::new(tokio::sync::OnceCell::new()),
            #[cfg(test)]
            memory_embedder_test_gate: None,
            reranker: Arc::new(Mutex::new(None)),
            memory_manager: Arc::new(
                crate::ai::memory::MemoryManager::open_default().unwrap_or_else(|e| {
                    log::warn!("memory db unavailable, falling back to in-memory: {e}");
                    crate::ai::memory::MemoryManager::open_in_memory()
                        .expect("in-memory memory db opens")
                }),
            ),
            approved_save_paths: Arc::new(Mutex::new(load_approved_paths())),
            api_key_cache: Arc::new(RwLock::new(None)),
            password_cache: Arc::new(RwLock::new(HashMap::new())),
            notebook_sessions: DashMap::new(),
            logs: crate::supervisor::new_log_buffer(),
            llm_usage: DashMap::new(),
            profile_snapshots: Arc::new(DashMap::new()),
            idle_tracker: Arc::new(crate::ai::memory::IdleTracker::new()),
            indexing,
            acp_http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .expect("reqwest client for the ACP registry builds"),
            acp: crate::ai::acp::AcpState::new(),
        }
    }

    /// Clone the active ConnectorClient out of the mutex. Hold this lock only
    /// for the clone — never across `execute().await`: a long query would
    /// serialize every other query and deadlock `cancel`. Every query site
    /// must go through this (or the same clone-then-drop pattern).
    pub async fn client_handle(&self) -> Option<crate::client::ConnectorClient> {
        self.client.lock().await.clone()
    }

    /// Clone the connected driver's capabilities. `None` when disconnected.
    pub async fn capabilities(&self) -> Option<lucent_protocol::DriverCapabilities> {
        self.driver_capabilities.lock().await.clone()
    }

    /// Returns or lazily initializes the 512-token memory embedder.
    ///
    /// The data lock is never held across the model load: `OnceCell` lets
    /// concurrent callers share one initialization without any caller
    /// blocking a reader of an already-loaded embedder (B-I9). A failed init
    /// leaves the cell uninitialized, so a later call retries.
    pub async fn get_or_init_memory_embedder(&self) -> Option<crate::ai::embed::Embedder> {
        #[cfg(test)]
        {
            if let Some(gate) = &self.memory_embedder_test_gate {
                if gate.is_unavailable() {
                    return None;
                }
                gate.arrive_and_park().await;
            }
        }
        match self
            .memory_embedder
            .get_or_try_init(|| async {
                match tokio::task::spawn_blocking(crate::ai::embed::Embedder::new_memory).await {
                    Ok(Ok(model)) => Ok(model),
                    Ok(Err(e)) => {
                        log::warn!("failed to init memory embedder: {e}");
                        Err(())
                    }
                    Err(e) => {
                        log::warn!("panic in memory embedder init: {e}");
                        Err(())
                    }
                }
            })
            .await
        {
            Ok(embedder) => Some(embedder.clone()),
            Err(()) => None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ConnectResult {
    pub server_version: String,
    pub database: String,
}

#[cfg(test)]
mod capability_state_tests {
    use lucent_protocol::{ReadOnlyMode, SqlDialect};

    use super::CapabilityView;

    #[test]
    fn the_view_the_frontend_gets_names_the_enforcement_level() {
        let strong = CapabilityView::from(&lucent_driver_postgres_caps(SqlDialect::PostgreSql));
        assert_eq!(strong.driver, "postgres");
        assert_eq!(strong.display_name, "PostgreSQL");
        assert!(strong.engine_enforced_readonly);
        assert!(
            strong.readonly_disclosure.is_none(),
            "an intact guarantee must produce no note, not a reassuring one"
        );
    }

    #[test]
    fn a_guard_only_driver_ships_its_disclosure_to_the_ui() {
        let mut caps = lucent_driver_postgres_caps(SqlDialect::PostgreSql);
        caps.readonly = ReadOnlyMode::GuardOnly;
        let view = CapabilityView::from(&caps);
        assert!(!view.engine_enforced_readonly);
        let note = view.readonly_disclosure.expect("must disclose");
        assert!(note.to_lowercase().contains("not enforced"), "{note}");
    }

    #[test]
    fn forwards_the_driver_s_sql_dialect() {
        let view = CapabilityView::from(&lucent_driver_postgres_caps(SqlDialect::DuckDb));
        assert_eq!(view.dialect, SqlDialect::DuckDb);
    }

    fn lucent_driver_postgres_caps(dialect: SqlDialect) -> lucent_protocol::DriverCapabilities {
        lucent_protocol::DriverCapabilities {
            id: "postgres".into(),
            display_name: "PostgreSQL".into(),
            sql_dialect: dialect,
            namespace_model: lucent_protocol::NamespaceModel::DbSchemaObject,
            readonly: ReadOnlyMode::TransactionScoped,
            statement_timeout: lucent_protocol::TimeoutSupport::Statement,
            cancel: lucent_protocol::CancelMode::Native,
            paging: lucent_protocol::PagingStyle::LimitOffset,
            identifier_quote: '"',
            string_literal: lucent_protocol::StringLiteralStyle::StandardConforming,
            auth: lucent_protocol::AuthModel::UserPassword,
        }
    }
}

/// The capability facts the UI needs, flattened for the frontend.
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityView {
    pub driver: String,
    pub display_name: String,
    /// False when the AST guard is the only read-only protection.
    pub engine_enforced_readonly: bool,
    /// Present only when the guarantee is weakened. The badge renders it as a
    /// warning; absence means "nothing to say", not "everything is fine".
    pub readonly_disclosure: Option<String>,
    /// Which SQL dialect the editor should assume for autocomplete and paging.
    pub dialect: lucent_protocol::SqlDialect,
}

impl From<&lucent_protocol::DriverCapabilities> for CapabilityView {
    fn from(c: &lucent_protocol::DriverCapabilities) -> Self {
        Self {
            driver: c.id.clone(),
            display_name: c.display_name.clone(),
            engine_enforced_readonly: c.readonly.is_engine_enforced(),
            readonly_disclosure: c.readonly.disclosure().map(str::to_string),
            dialect: c.sql_dialect,
        }
    }
}

#[tauri::command]
pub async fn connection_capabilities(
    state: State<'_, AppState>,
) -> Result<Option<CapabilityView>, CommandError> {
    Ok(state
        .capabilities()
        .await
        .as_ref()
        .map(CapabilityView::from))
}

#[derive(Debug, Serialize)]
pub struct DisconnectResult {
    pub ok: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct SchemaObject {
    pub name: String,
    pub kind: String,
    pub row_count: Option<i64>,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct SchemaInfo {
    /// Dotted display name (`catalog.schema` for DuckDB, `schema` for
    /// Postgres). For display only — never round-tripped back into a
    /// namespace: use [`SchemaInfo::path`] for that.
    pub name: String,
    /// The namespace path segments. The sidebar lists objects by passing
    /// these through — a dotted string would be misread as one segment.
    pub path: Vec<String>,
    pub object_count: i64,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct EditorColumn {
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct EditorTable {
    /// `NamespacePath` joined with '.'. Single-segment for Postgres today.
    pub schema: String,
    pub name: String,
    pub columns: Vec<EditorColumn>,
}

fn object_details_to_editor_tables(
    details: Vec<lucent_protocol::ObjectDetail>,
) -> Vec<EditorTable> {
    details
        .into_iter()
        .map(|d| EditorTable {
            schema: d.reference.namespace.join("."),
            name: d.reference.name,
            columns: d
                .columns
                .into_iter()
                .map(|c| EditorColumn {
                    name: c.name,
                    type_name: c.type_name,
                })
                .collect(),
        })
        .collect()
}

/// Normalized namespaces → the flat `SchemaInfo` the sidebar consumes.
///
/// Postgres emits one path segment, so this reproduces today's schema names
/// exactly. A driver with deeper namespaces renders dotted.
pub fn namespaces_to_schema_info(namespaces: Vec<lucent_protocol::Namespace>) -> Vec<SchemaInfo> {
    namespaces
        .into_iter()
        .map(|n| SchemaInfo {
            name: n.display(),
            path: n.path.clone(),
            // The frontend field is a plain i64. Until it learns to render
            // "unknown", collapse None here — in exactly one place.
            object_count: n.object_count.unwrap_or(0) as i64,
        })
        .collect()
}

/// Normalized object summaries → the sidebar's `SchemaObject` list.
///
/// Partition children are dropped: a table with 84 partitions would otherwise
/// bury every other object in the tree.
fn summaries_to_schema_objects(
    summaries: Vec<lucent_protocol::ObjectSummary>,
) -> Vec<SchemaObject> {
    summaries
        .into_iter()
        .filter(|s| !s.is_partition_child)
        .map(|s| SchemaObject {
            name: s.reference.name,
            kind: s.reference.kind.as_str().to_string(),
            row_count: s.est_rows.map(|n| n as i64),
        })
        .collect()
}

#[cfg(test)]
mod editor_schema_mapping_tests {
    use super::{object_details_to_editor_tables, EditorColumn, EditorTable};
    use lucent_protocol::{ColumnDetail, ObjectDetail, ObjectKind, ObjectRef};

    fn column(name: &str, type_name: &str) -> ColumnDetail {
        ColumnDetail {
            name: name.to_string(),
            type_name: type_name.to_string(),
            nullable: true,
            is_primary_key: false,
            ordinal: 1,
            default: None,
            comment: None,
            foreign_key: None,
        }
    }

    fn detail(schema: &str, table: &str, columns: Vec<ColumnDetail>) -> ObjectDetail {
        ObjectDetail {
            reference: ObjectRef {
                namespace: vec![schema.to_string()],
                name: table.to_string(),
                kind: ObjectKind::Table,
            },
            columns,
            comment: None,
        }
    }

    #[test]
    fn maps_schema_table_and_columns() {
        let details = vec![detail(
            "public",
            "customers",
            vec![column("id", "int4"), column("name", "text")],
        )];
        let tables = object_details_to_editor_tables(details);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].schema, "public");
        assert_eq!(tables[0].name, "customers");
        assert_eq!(
            tables[0].columns,
            vec![
                EditorColumn {
                    name: "id".into(),
                    type_name: "int4".into()
                },
                EditorColumn {
                    name: "name".into(),
                    type_name: "text".into()
                },
            ]
        );
    }

    #[test]
    fn a_table_with_no_columns_maps_to_an_empty_list_not_an_error() {
        let details = vec![detail("public", "empty_view", vec![])];
        let tables = object_details_to_editor_tables(details);
        assert_eq!(tables[0].columns, Vec::<EditorColumn>::new());
    }

    #[test]
    fn empty_input_maps_to_empty_output() {
        assert_eq!(
            object_details_to_editor_tables(vec![]),
            Vec::<EditorTable>::new()
        );
    }
}

#[tauri::command]
pub async fn get_schemas(state: State<'_, AppState>) -> Result<Vec<SchemaInfo>, CommandError> {
    let conn_id = (*state.current_connection_id.lock().await)
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let namespaces = client
        .list_namespaces(conn_id)
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    Ok(namespaces_to_schema_info(namespaces))
}

#[derive(Debug, Serialize, Clone)]
pub struct SchemaObjectsResult {
    pub objects: Vec<SchemaObject>,
}

#[derive(Debug, Serialize, Clone)]
pub struct DatabaseInfo {
    pub name: String,
    pub is_current: bool,
}

#[tauri::command]
pub async fn connect(
    state: State<'_, AppState>,
    connection_id: Option<String>,
    config: Option<ConnectionConfig>,
) -> Result<ConnectResult, CommandError> {
    // Resolve connection config from profile ID or inline config
    let resolved = if let Some(prof_id) = &connection_id {
        let profile = state
            .repo
            .get_profile(prof_id)
            .await
            .ok_or_else(|| CommandError::new("NotFound", "connection profile not found"))?;

        // Mark last_used
        state.repo.mark_used(prof_id).await.ok();

        profile_to_config(&state, &profile, prof_id).await?
    } else if let Some(cfg) = config {
        cfg
    } else {
        return Err(CommandError::new(
            "InvalidArgs",
            "provide connection_id or config",
        ));
    };

    // Correlate everything below (including bridged `log::` lines) under the
    // `connect` span. No await inside the span construction.
    let span = crate::trace::connect_span(
        resolved.get("host").unwrap_or(""),
        resolved.port().unwrap_or(0),
        resolved.get("database").unwrap_or(""),
    );
    connect_impl(state, resolved, connection_id.as_deref())
        .instrument(span)
        .await
}

/// Build a driver config from a saved profile plus its keychain secret.
async fn profile_to_config(
    state: &AppState,
    profile: &crate::connections::ConnectionProfile,
    profile_id: &str,
) -> Result<ConnectionConfig, CommandError> {
    let mut config = ConnectionConfig::new(profile.driver.clone());
    for (key, value) in &profile.params {
        config = config.with(key.clone(), value.clone());
    }

    // The secret lives in the keychain, never in connections.json.
    match cached_password(state, profile_id).await {
        Ok(secret) => config = config.with_secret(secret),
        // A driver with AuthModel::FilePath or None has no secret to fetch.
        Err(crate::connections::KeychainError::NotFound) => {}
        Err(e) => return Err(CommandError::new("KeychainError", e.to_string())),
    }
    Ok(config)
}

/// Read a profile's keychain secret at most once per launch; later reads for
/// the same profile come from memory.
pub(crate) async fn cached_password(
    state: &AppState,
    profile_id: &str,
) -> Result<String, crate::connections::KeychainError> {
    if let Some(pw) = state.password_cache.read().await.get(profile_id) {
        return Ok(pw.clone());
    }
    let pw = crate::connections::get_password(profile_id)?;
    state
        .password_cache
        .write()
        .await
        .insert(profile_id.to_string(), pw.clone());
    Ok(pw)
}

async fn connect_impl(
    state: State<'_, AppState>,
    resolved: ConnectionConfig,
    profile_id: Option<&str>,
) -> Result<ConnectResult, CommandError> {
    log::info!(
        "Connecting to database {:?}@{}/{}",
        resolved.get("user").unwrap_or(""),
        resolved.get("host").unwrap_or(""),
        resolved.get("database").unwrap_or("")
    );

    // Disconnect previous connection if any
    {
        let prev_id = *state.current_connection_id.lock().await;
        // Abort the previous connection's background indexer so it cannot swap
        // its (socket-independent) Tier-2 graph into the shared slot after this
        // connect starts a new one. The swap guard in indexer.rs is the
        // defense-in-depth; stopping here removes the task promptly.
        if let Some(prev_id) = prev_id {
            state.indexing.stop(prev_id).await;
        }
        let mut client_lock = state.client.lock().await;
        if let Some(ref mut old_client) = *client_lock {
            log::info!("Disconnecting previous connection before new connect");
            let _ = old_client.shutdown().await;
        }
        // Clear before connecting
        *client_lock = None;
        // The AI session dies with the old client.
        *state.ai_connection_id.lock().await = None;
        // The memory key names the old connection; a failed reconnect must not
        // leave it pointing at stale memories.
        *state.memory_connection_key.lock().await = None;
    }

    // Invalidate schema cache for old connection
    state.schema_cache.clear();

    // Retry loop: after disconnecting the old client, the worker may still
    // be exiting. `ensure_running` now respawns on exit, but there's a race
    // where the socket file exists while the old worker is shutting down.
    let mut last_connect_err: Option<String> = None;
    let mut client: Option<ConnectorClient> = None;
    let mut connect_id: Option<ConnectionId> = None;
    for attempt in 0..3 {
        let mut supervisor_lock = state.supervisor.lock().await;
        // One worker process per driver TYPE. Switching drivers replaces the
        // supervisor; Phase 2 turns this into a refcounted pool so several
        // drivers run at once (spec §6.1).
        let needs_replacement = supervisor_lock
            .as_ref()
            .is_some_and(|s| s.driver_id() != resolved.driver);
        if needs_replacement {
            if let Some(mut old) = supervisor_lock.take() {
                log::info!(
                    "Switching driver from {} to {}; stopping the old worker",
                    old.driver_id(),
                    resolved.driver
                );
                old.shutdown().await.ok();
            }
        }
        let sup = supervisor_lock
            .get_or_insert_with(|| Supervisor::for_driver(&resolved.driver, state.logs.clone()));
        let sp = match sup.ensure_running().await {
            Ok(()) => sup.endpoint().to_string(),
            Err(e) => {
                log::error!("Worker startup failed (attempt {}): {e}", attempt + 1);
                last_connect_err = Some(e);
                continue;
            }
        };
        let tk = sup.handshake_token().to_string();
        log::debug!("Worker endpoint at {sp:?}, token={tk}");
        drop(supervisor_lock);

        match ConnectorClient::connect(&sp, &tk, resolved.clone()).await {
            Ok((c, cid)) => {
                client = Some(c);
                connect_id = Some(cid);
                break;
            }
            Err(e) => {
                log::error!(
                    "ConnectorClient::connect failed (attempt {}): {e}",
                    attempt + 1
                );
                last_connect_err = Some(e);
                // Brief backoff before retrying — the worker may have just
                // exited and need time for the new one to bind the socket.
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
    let client = client.ok_or_else(|| {
        let msg = last_connect_err.unwrap_or_else(|| "unable to start worker".to_string());
        CommandError::new("ConnectError", msg)
    })?;
    let worker_conn_id = connect_id
        .ok_or_else(|| CommandError::new("ConnectError", "connection ID not available"))?;
    let server_version = client
        .server_info
        .as_ref()
        .map(|s| s.version.clone())
        .unwrap_or_default();
    let database = display_database(&resolved);
    log::info!(
        "Connected to {database} ({} {server_version})",
        resolved.driver
    );

    let capabilities = client.server_info.as_ref().map(|s| s.capabilities.clone());
    if let Some(caps) = &capabilities {
        if !caps.readonly.is_engine_enforced() {
            // Loud on purpose: this is the moment the two-layer read-only
            // guarantee in the README stops holding for this connection.
            log::warn!(
                "Connected to a {} database with NO engine-enforced read-only. \
                 The AI's SQL guard is the only protection.",
                caps.display_name
            );
        }
    }
    *state.driver_capabilities.lock().await = capabilities;

    // Refresh schema cache BEFORE storing client (no lock needed)
    // Driver-aware key: two DuckDB files (or a DuckDB and a Postgres
    // connection) must not collide on a bare host:port/db label.
    let conn_id = format!(
        "{}://{}/{}",
        resolved.driver,
        resolved
            .get("host")
            .or_else(|| resolved.get("path"))
            .unwrap_or(""),
        resolved.get("database").unwrap_or("")
    );
    if let Ok(tree) = state
        .schema_cache
        .refresh(conn_id.clone(), &client, worker_conn_id)
        .await
    {
        if let Some(pid) = profile_id {
            state.schema_cache.set(pid.to_string(), tree);
        }
    }

    // Build semantic schema index — Tier-1 harvest inline (~5–50ms), Tier-2
    // enriched in the background by IndexingManager. Non-blocking failure,
    // never fails connect() (per the design's "Fast Mode" requirement). The
    // harvest stores the Tier-1 graph immediately; the background start is
    // deferred until session B exists below, because the sampling connection
    // must be B (spec §B.5) — sampling on the editor session would run
    // `SET statement_timeout = 3000` on a session the user is actively
    // querying.
    *state.schema_graph.lock().await = None;
    let mut pending_index: Option<(
        crate::ai::schema_graph::SchemaGraph,
        crate::ai::schema_graph::CatalogSnapshot,
        Arc<AiConfig>,
        String,
        lucent_protocol::DriverCapabilities,
    )> = None;
    {
        let ai_cfg = state.ai_config.read().await.clone();
        if ai_cfg.enable_semantic_index {
            let capabilities = state.capabilities().await;
            if let Some(capabilities) = capabilities {
                match crate::ai::schema_graph::SchemaGraph::from_catalog(
                    worker_conn_id,
                    &client,
                    &capabilities,
                )
                .await
                {
                    Ok((graph, snapshot)) => {
                        *state.schema_graph.lock().await = Some(graph.clone());
                        let config = Arc::new(ai_cfg);
                        let connection_key = crate::ai::cache_store::connection_key_for(&resolved);
                        pending_index =
                            Some((graph, snapshot, config, connection_key, capabilities));
                    }
                    Err(e) => {
                        log::warn!("Tier-1 schema harvest failed, continuing without it: {e}");
                    }
                }
            }
        }
    }

    *state.current_connection_id.lock().await = Some(worker_conn_id);

    // Dedicated AI session on the same worker socket. Failure is non-fatal —
    // AI tools fall back to the editor session only if B is absent (see
    // Task 2.2); the connect command must not fail because of it.
    let ai_conn_id = ConnectionId(Uuid::new_v4());
    let ai_cfg = resolved.clone();
    let ai_result = client.connect_with_id(ai_conn_id, ai_cfg).await;
    match ai_result {
        Ok(_) => {
            log::info!("AI session B established");
            *state.ai_connection_id.lock().await = Some(ai_conn_id);
        }
        Err(e) => {
            log::warn!("AI session B failed to open ({e}); AI tools will use the editor session");
            *state.ai_connection_id.lock().await = None;
        }
    }

    // Start the background indexer now that session B exists: the sampling
    // connection id captured here is real (Some on success, None on B-failure
    // fallback), so enrich samples on B — never on the editor session.
    //
    // The memory subsystem keys memories by the frontend-visible connection id
    // (profile id, else host:port/database), NOT the vector-cache hash. Store
    // it for sync_schema_indexing and pass it to the indexer so the Gate 1
    // drift cascade scopes to the keys memories are actually stored under.
    let memory_connection_key = memory_connection_key_for(profile_id, &resolved);
    *state.memory_connection_key.lock().await = Some(memory_connection_key.clone());

    if let Some((graph, snapshot, config, connection_key, capabilities)) = pending_index {
        let sampling_conn = *state.ai_connection_id.lock().await;
        state
            .indexing
            .start(
                worker_conn_id,
                Some(client.clone()),
                sampling_conn,
                state.schema_graph.clone(),
                // Swap guard: the indexer only publishes its Tier-2 graph while
                // this connection is still the current one (stale-task safety).
                state.current_connection_id.clone(),
                (graph, snapshot),
                state.embedder.clone(),
                None, // no override in production
                config,
                connection_key,
                memory_connection_key,
                capabilities,
                state.memory_manager.clone(),
            )
            .await;
    }

    *state.client.lock().await = Some(client);

    *state.current_database.lock().await = Some(database.clone());
    *state.current_connection_config.lock().await = Some(resolved.clone());

    Ok(ConnectResult {
        server_version,
        database,
    })
}

// ─── Connection Profile Commands ─────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct TestConnectionResult {
    pub success: bool,
    pub message: String,
    pub server_version: Option<String>,
}

#[tauri::command]
pub async fn list_connections(
    state: State<'_, AppState>,
) -> Result<Vec<crate::connections::ConnectionProfile>, CommandError> {
    Ok(state.repo.list_profiles().await)
}

#[tauri::command]
pub async fn get_connection(
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::connections::ConnectionProfile, CommandError> {
    state
        .repo
        .get_profile(&id)
        .await
        .ok_or_else(|| CommandError::new("NotFound", "connection profile not found"))
}

#[tauri::command]
pub async fn save_connection(
    state: State<'_, AppState>,
    profile: crate::connections::ConnectionProfile,
    password: Option<String>,
) -> Result<crate::connections::ConnectionProfile, CommandError> {
    let mut profile = profile;
    profile.updated_at = chrono::Utc::now().to_rfc3339();

    // Defense-in-depth: generate a UUID if the frontend didn't provide one.
    // The keychain backend rejects empty "user" attributes.
    if profile.id.is_empty() {
        profile.id = uuid::Uuid::new_v4().to_string();
    }

    // Derive the @mention alias from the name when the frontend didn't supply
    // one, so every profile is addressable in AI chat. An empty slug is stored
    // as `None` rather than an empty alias, which would match every mention.
    if profile.alias.is_none() {
        let slug = crate::connections::slugify_alias(&profile.name);
        profile.alias = (!slug.is_empty()).then_some(slug);
    }

    if let Some(ref pw) = password {
        crate::connections::set_password(&profile.id, pw)
            .map_err(|e| CommandError::new("KeychainError", e.to_string()))?;
        state
            .password_cache
            .write()
            .await
            .insert(profile.id.clone(), pw.clone());
    }

    let is_new = state.repo.get_profile(&profile.id).await.is_none();
    if is_new {
        profile.created_at = chrono::Utc::now().to_rfc3339();
    }

    state
        .repo
        .save_profile(profile.clone())
        .await
        .map_err(|e| CommandError::new("FileError", e))?;

    Ok(profile)
}

#[tauri::command]
pub async fn delete_connection(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    state.password_cache.write().await.remove(&id);
    state
        .repo
        .delete_profile(&id)
        .await
        .map_err(|e| CommandError::new("FileError", e))
}

#[tauri::command]
pub async fn duplicate_connection(
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::connections::ConnectionProfile, CommandError> {
    let original = state
        .repo
        .get_profile(&id)
        .await
        .ok_or_else(|| CommandError::new("NotFound", "profile not found"))?;

    let mut copy = original.clone();
    copy.id = uuid::Uuid::new_v4().to_string();
    copy.name = format!("{} (copy)", copy.name);
    // The copy must not inherit the original's @mention handle — two profiles
    // sharing an alias would make every mention of it ambiguous.
    let slug = crate::connections::slugify_alias(&copy.name);
    copy.alias = (!slug.is_empty()).then_some(slug);
    let now = chrono::Utc::now().to_rfc3339();
    copy.created_at = now.clone();
    copy.updated_at = now;
    copy.last_used = None;

    // Copy password if exists (best-effort)
    if let Ok(pw) = crate::connections::get_password(&id) {
        crate::connections::set_password(&copy.id, &pw).ok();
        state
            .password_cache
            .write()
            .await
            .insert(copy.id.clone(), pw);
    }

    state
        .repo
        .save_profile(copy.clone())
        .await
        .map_err(|e| CommandError::new("FileError", e))?;

    Ok(copy)
}

#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    id: String,
) -> Result<TestConnectionResult, CommandError> {
    let profile = state
        .repo
        .get_profile(&id)
        .await
        .ok_or_else(|| CommandError::new("NotFound", "profile not found"))?;

    let config = profile_to_config(&state, &profile, &id).await?;

    probe_connection(config, profile.driver.clone()).await
}

/// The database label shown in the UI for a connection config.
///
/// Host-based drivers (Postgres) name themselves by the `database` param;
/// file-based drivers (DuckDB) by the `path` param. A driver-agnostic
/// fallback keeps the explorer, the connect log, and the schema-cache key
/// meaningful for either — an empty label makes the sidebar show the
/// disconnected empty state while connected.
pub fn display_database(config: &lucent_protocol::ConnectionConfig) -> String {
    config
        .get("database")
        .or_else(|| config.get("path"))
        .unwrap_or("")
        .to_string()
}

/// The connection key the memory subsystem stores and retrieves under: the
/// profile id when a profile is connected, else the frontend's inline
/// `host:port/database` key (App.svelte's `activeConnectionId`, built from the
/// connection form's fields). This is deliberately NOT the vector-cache
/// `connection_key_for` hash — the Gate 1 drift cascade must match the keys
/// `memories.connection_key` actually holds, or it silently invalidates
/// nothing.
pub fn memory_connection_key_for(profile_id: Option<&str>, config: &ConnectionConfig) -> String {
    if let Some(id) = profile_id {
        return id.to_string();
    }
    format!(
        "{}:{}/{}",
        config.get("host").unwrap_or(""),
        config.get("port").unwrap_or(""),
        config.get("database").unwrap_or("")
    )
}

/// Probe a connection config through a dedicated, short-lived worker process.
///
/// A throwaway worker is required, not an optimization: the app's real worker
/// serves exactly one socket — the live connection's — so a second probe socket
/// would sit unaccepted in the backlog and time out after 15s, reporting a
/// healthy database as unreachable. A fresh worker per probe leaves the live
/// connection untouched and exercises the real seam.
///
/// The probe worker must match the profile's driver: a Postgres worker would
/// interpret a DuckDB config as Postgres credentials (and vice versa).
pub async fn probe_connection(
    config: ConnectionConfig,
    display_fallback: String,
) -> Result<TestConnectionResult, CommandError> {
    // The probe worker must match the config's driver, not the app default.
    let mut supervisor =
        Supervisor::for_driver(config.driver.as_str(), crate::supervisor::new_log_buffer());

    let socket_and_token = match supervisor.ensure_running().await {
        Ok(()) => (
            supervisor.endpoint().to_string(),
            supervisor.handshake_token().to_string(),
        ),
        Err(e) => {
            let _ = supervisor.shutdown().await;
            return Err(CommandError::new("ConnectError", e));
        }
    };

    let outcome = ConnectorClient::connect(&socket_and_token.0, &socket_and_token.1, config).await;
    let result = match outcome {
        Ok((mut client, cid)) => {
            let version = client
                .server_info
                .as_ref()
                .map(|s| s.version.clone())
                .unwrap_or_default();
            let display = client
                .server_info
                .as_ref()
                .map(|s| s.capabilities.display_name.clone())
                .unwrap_or(display_fallback);
            let _ = client.disconnect_id(cid).await;
            let _ = client.shutdown().await;
            TestConnectionResult {
                success: true,
                message: format!("Connected to {display} {version}"),
                server_version: Some(version),
            }
        }
        Err(e) => {
            let _ = supervisor.shutdown().await;
            return Err(CommandError::new("ConnectionFailed", e));
        }
    };

    let _ = supervisor.shutdown().await;
    Ok(result)
}

// ─── SSH Config Commands ────────────────────────────────────────────────

#[tauri::command]
pub async fn save_ssh_config(
    state: State<'_, AppState>,
    config: crate::ssh::SshConfig,
    secret: Option<String>,
) -> Result<(), CommandError> {
    if let Some(s) = &secret {
        crate::connections::set_ssh_secret(&config.id, s)
            .map_err(|e| CommandError::new("KeychainError", e.to_string()))?;
    }
    state
        .repo
        .save_ssh_config(config)
        .await
        .map_err(|e| CommandError::new("FileError", e))
}

#[tauri::command]
pub async fn list_ssh_configs(
    state: State<'_, AppState>,
) -> Result<Vec<crate::ssh::SshConfig>, CommandError> {
    Ok(state.repo.list_ssh_configs().await)
}

#[tauri::command]
pub async fn delete_ssh_config(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
    state
        .repo
        .delete_ssh_config(&id)
        .await
        .map_err(|e| CommandError::new("FileError", e))
}

// ─── Query History Commands ────────────────────────────────────────────

// camelCase to match the frontend's `HistoryEntry` interface, which every
// consumer already reads (`entry.rowCount`, `entry.executedAt`,
// `entry.dateGroup`). Without this the fields arrived snake_case and silently
// read as `undefined` — rendering "NaN rows", blank timestamps, and collapsing
// the history panel's date grouping into a single unlabelled group.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntryResult {
    pub id: String,
    pub connection_id: String,
    pub connection_name: String,
    pub database: String,
    pub sql: String,
    pub duration_ms: u64,
    pub row_count: Option<u64>,
    pub status: String,
    pub error: Option<String>,
    pub executed_at: String,
    pub favorite: bool,
    pub date_group: String,
}

impl From<crate::query_history::QueryHistoryEntry> for HistoryEntryResult {
    fn from(e: crate::query_history::QueryHistoryEntry) -> Self {
        let date_group = crate::query_history::date_group(&e.executed_at);
        Self {
            id: e.id,
            connection_id: e.connection_id,
            connection_name: e.connection_name,
            database: e.database,
            sql: e.sql,
            duration_ms: e.duration_ms,
            row_count: e.row_count,
            status: e.status,
            error: e.error,
            executed_at: e.executed_at,
            favorite: e.favorite,
            date_group,
        }
    }
}

#[tauri::command]
pub async fn list_history(
    connection_id: Option<String>,
    search: Option<String>,
    favorite_only: Option<bool>,
) -> Result<Vec<HistoryEntryResult>, CommandError> {
    let entries = crate::query_history::search_entries(
        search.as_deref(),
        connection_id.as_deref(),
        favorite_only.unwrap_or(false),
    );
    Ok(entries.into_iter().map(HistoryEntryResult::from).collect())
}

#[tauri::command]
pub async fn toggle_history_favorite(id: String) -> Result<(), CommandError> {
    crate::query_history::toggle_favorite(&id).map_err(|e| CommandError::new("FileError", e))
}

#[tauri::command]
pub async fn delete_history_entry(id: String) -> Result<(), CommandError> {
    crate::query_history::delete_entry(&id).map_err(|e| CommandError::new("FileError", e))
}

#[tauri::command]
pub async fn clear_history() -> Result<(), CommandError> {
    crate::query_history::clear_history().map_err(|e| CommandError::new("FileError", e))
}

// ─── Export Commands ───────────────────────────────────────────────────

#[tauri::command]
pub async fn export_results(
    state: State<'_, AppState>,
    columns: Vec<crate::export::ColumnMeta>,
    rows: Vec<Vec<serde_json::Value>>,
    format: crate::export::ExportFormat,
    options: crate::export::ExportOptions,
    path: String,
) -> Result<u64, CommandError> {
    let formatted = match format {
        crate::export::ExportFormat::Csv => crate::export::format_csv(&columns, &rows, &options),
        crate::export::ExportFormat::Json => crate::export::format_json(&columns, &rows, &options),
        crate::export::ExportFormat::SqlInsert => {
            let table = options.table_name.as_deref().ok_or_else(|| {
                CommandError::new("InvalidArgs", "table_name required for INSERT format")
            })?;
            crate::export::format_inserts(table, &columns, &rows, &options)
        }
    };
    let bytes = formatted.len() as u64;
    let approved = state.approved_save_paths.lock().await.clone();
    let canonical = validate_approved(std::path::Path::new(&path), &approved)?;
    std::fs::write(&canonical, formatted.as_bytes())
        .map_err(|e| CommandError::new("PathError", format!("write failed: {e}")))?;
    Ok(bytes)
}

#[tauri::command]
pub async fn copy_results(
    app_handle: tauri::AppHandle,
    columns: Vec<crate::export::ColumnMeta>,
    rows: Vec<Vec<serde_json::Value>>,
    format: crate::export::ExportFormat,
    options: crate::export::ExportOptions,
) -> Result<(), CommandError> {
    let formatted = match format {
        crate::export::ExportFormat::Csv => crate::export::format_csv(&columns, &rows, &options),
        crate::export::ExportFormat::Json => crate::export::format_json(&columns, &rows, &options),
        crate::export::ExportFormat::SqlInsert => {
            let table = options.table_name.as_deref().ok_or_else(|| {
                CommandError::new("InvalidArgs", "table_name required for INSERT format")
            })?;
            crate::export::format_inserts(table, &columns, &rows, &options)
        }
    };
    app_handle
        .clipboard()
        .write_text(formatted)
        .map_err(|e| CommandError::new("ClipboardError", e.to_string()))
}

/// Canonicalize `path`, tolerating a not-yet-existing final component
/// (new files) by canonicalizing the parent instead. Resolves symlinks, so
/// a symlink pointing outside an approved directory is caught by the caller.
pub fn canonicalize_allow_missing(
    path: &std::path::Path,
) -> Result<std::path::PathBuf, CommandError> {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return Ok(canonical);
    }
    let parent = path
        .parent()
        .ok_or_else(|| CommandError::new("PathError", "invalid destination path"))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| CommandError::new("PathError", "invalid destination path"))?;
    let canonical_parent = std::fs::canonicalize(parent)
        .map_err(|e| CommandError::new("PathError", format!("cannot resolve parent: {e}")))?;
    Ok(canonical_parent.join(file_name))
}

/// A path may only be written if it was chosen by the user in a native save
/// dialog this session (or a previous one — the approved set is persisted).
pub fn validate_approved(
    path: &std::path::Path,
    approved: &std::collections::HashSet<std::path::PathBuf>,
) -> Result<std::path::PathBuf, CommandError> {
    let canonical = canonicalize_allow_missing(path)?;
    if !approved.contains(&canonical) {
        return Err(CommandError::new(
            "PathError",
            "destination path was not chosen in a native save dialog",
        ));
    }
    Ok(canonical)
}

/// `<config_dir>/lucent/approved_save_paths.json` — the persisted set of
/// paths the user explicitly chose in a native dialog. Only ever grows via
/// `choose_path_via_dialog`; nothing else writes it.
fn approved_paths_file() -> Result<std::path::PathBuf, CommandError> {
    let mut dir =
        dirs::config_dir().ok_or_else(|| CommandError::new("PathError", "no config dir"))?;
    dir.push("lucent");
    std::fs::create_dir_all(&dir).map_err(|e| CommandError::new("PathError", e.to_string()))?;
    Ok(dir.join("approved_save_paths.json"))
}

/// Load the persisted approved-path set. Missing/corrupt files degrade to
/// empty — the set is a convenience cache, not a security boundary by itself
/// (a fresh dialog re-approves on demand).
fn load_approved_paths() -> std::collections::HashSet<std::path::PathBuf> {
    let Ok(file) = approved_paths_file() else {
        return Default::default();
    };
    std::fs::read_to_string(file)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .map(|v| v.into_iter().map(std::path::PathBuf::from).collect())
        .unwrap_or_default()
}

/// Write the approved-path set back to disk. Best-effort: a persistence
/// failure must not fail the dialog flow the user already completed.
async fn persist_approved_paths(state: &AppState) -> Result<(), CommandError> {
    let paths = state.approved_save_paths.lock().await.clone();
    let file = approved_paths_file()?;
    let list: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    std::fs::write(
        &file,
        serde_json::to_string(&list).unwrap_or_else(|_| "[]".into()),
    )
    .map_err(|e| CommandError::new("PathError", format!("cannot persist approved paths: {e}")))
}

/// Shared body for `choose_save_path` / `choose_export_path`: show a native
/// save dialog, canonicalize the picked path, record it in the approved set
/// (persisted), and return the canonical path to the frontend.
async fn choose_path_via_dialog(
    app_handle: &tauri::AppHandle,
    default_name: String,
    filter_name: String,
    extensions: Vec<String>,
    state: &AppState,
) -> Result<Option<String>, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let ext_refs: Vec<&str> = extensions.iter().map(String::as_str).collect();
    tauri_plugin_dialog::FileDialogBuilder::new(app_handle.dialog().clone())
        .set_file_name(&default_name)
        .add_filter(&filter_name, &ext_refs)
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let picked = rx
        .await
        .map_err(|_| CommandError::new("PathError", "save dialog channel closed"))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| CommandError::new("PathError", format!("dialog returned a URL: {e}")))?;
    let canonical = canonicalize_allow_missing(&path)?;
    state
        .approved_save_paths
        .lock()
        .await
        .insert(canonical.clone());
    // Best-effort persistence: a read-only config dir must not fail the
    // dialog flow the user already completed (the in-memory set still gates
    // writes this session).
    if let Err(e) = persist_approved_paths(state).await {
        log::warn!("could not persist approved save paths: {e}");
    }
    Ok(Some(canonical.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn choose_save_path(
    app_handle: tauri::AppHandle,
    default_name: String,
    filter_name: String,
    extensions: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Option<String>, CommandError> {
    choose_path_via_dialog(&app_handle, default_name, filter_name, extensions, &state).await
}

#[tauri::command]
pub async fn choose_export_path(
    app_handle: tauri::AppHandle,
    default_name: String,
    filter_name: String,
    extensions: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Option<String>, CommandError> {
    choose_path_via_dialog(&app_handle, default_name, filter_name, extensions, &state).await
}

/// Native open dialog that approves the picked path for later writes.
/// Opening a file from disk is a user choice in a native dialog — the same
/// trust level as Save-As — so the picked path is recorded in the approved
/// set, letting a subsequent Save (not Save-As) write back to the same file.
pub async fn choose_open_path_via_dialog(
    app_handle: &tauri::AppHandle,
    filter_name: String,
    extensions: Vec<String>,
    state: &AppState,
) -> Result<Option<String>, CommandError> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let ext_refs: Vec<&str> = extensions.iter().map(String::as_str).collect();
    let mut builder = tauri_plugin_dialog::FileDialogBuilder::new(app_handle.dialog().clone());
    if !filter_name.is_empty() && !extensions.is_empty() {
        builder = builder.add_filter(&filter_name, &ext_refs);
    }
    builder.pick_file(move |path| {
        let _ = tx.send(path);
    });
    let picked = rx
        .await
        .map_err(|_| CommandError::new("PathError", "open dialog channel closed"))?;
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| CommandError::new("PathError", format!("dialog returned a URL: {e}")))?;
    let canonical = canonicalize_allow_missing(&path)?;
    state
        .approved_save_paths
        .lock()
        .await
        .insert(canonical.clone());
    if let Err(e) = persist_approved_paths(state).await {
        log::warn!("could not persist approved paths (open dialog): {e}");
    }
    Ok(Some(canonical.to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn choose_open_path(
    app_handle: tauri::AppHandle,
    filter_name: String,
    extensions: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Option<String>, CommandError> {
    choose_open_path_via_dialog(&app_handle, filter_name, extensions, &state).await
}

/// Validate + write one file through the approved-path gate. Shared by the
/// `save_sql_file` command; kept separate so unit tests can exercise the gate
/// without a Tauri runtime.
pub async fn write_approved_sql_file(
    state: &AppState,
    path: &str,
    content: String,
) -> Result<(), CommandError> {
    let approved = state.approved_save_paths.lock().await.clone();
    let canonical = validate_approved(std::path::Path::new(path), &approved)?;
    std::fs::write(&canonical, content)
        .map_err(|e| CommandError::new("PathError", format!("write failed: {e}")))?;
    Ok(())
}

#[tauri::command]
pub async fn save_sql_file(
    state: State<'_, AppState>,
    path: String,
    content: String,
) -> Result<(), CommandError> {
    write_approved_sql_file(&state, &path, content).await
}

#[tauri::command]
pub async fn execute_query(
    state: State<'_, AppState>,
    sql: String,
    limit: i64,
    offset: i64,
    // The wire sort became a LIST (multi-key ORDER BY). Tauri command params
    // cannot carry `#[serde(default)]`, and Option<T> is the framework's
    // absent/null-tolerant form — so the list arrives wrapped and defaults to
    // empty, which is exactly what the old Option<SortSpec> gave clients.
    sort: Option<Vec<crate::query_paging::SortSpec>>,
    filters: Vec<crate::query_paging::FilterSpec>,
) -> Result<ExecuteResult, CommandError> {
    // Running a query is user activity: reset the idle clock the sleep-time
    // compute daemon watches so consolidation never fires mid-workload.
    state.idle_tracker.touch();
    let sort = sort.unwrap_or_default();
    let conn_id = (*state.current_connection_id.lock().await)
        .ok_or_else(|| CommandError::new("QueryError", "not connected — connect first"))?;
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected — connect first"))?;

    let capabilities = state
        .capabilities()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected — connect first"))?;
    let dialect = capabilities.sql_dialect;
    let builder = crate::sql_builder::for_driver(&capabilities);

    let final_sql = if crate::query_paging::is_wrappable_query(&sql, dialect) {
        crate::query_paging::wrap_for_page(&sql, &sort, &filters, limit, offset, builder.as_ref())
    } else {
        sql
    };

    let start = std::time::Instant::now();
    let query_id = QueryId(Uuid::new_v4());
    *state.editor_query.lock().await = Some((query_id, conn_id));
    let result = client
        .execute_with_id(
            query_id,
            conn_id,
            &final_sql,
            // Safety net for queries that cannot be LIMIT-wrapped (multi-
            // statement, EXPLAIN, unparseable-but-executable): bound what gets
            // materialized in this process and cancel the query server-side.
            // Paged queries stay far below the cap.
            Some(crate::client::HARD_ROW_CAP),
        )
        .await
        .map(|(r, _)| r);
    let duration_ms = start.elapsed().as_millis() as u64;
    let db = state
        .current_database
        .lock()
        .await
        .clone()
        .unwrap_or_default();
    // Attribute history entries to this connection so dedup never merges the
    // same SQL across different servers (dedup keys on connection_id + db + sql).
    let conn_desc = state
        .current_connection_config
        .lock()
        .await
        .clone()
        .map(|c| {
            format!(
                "{}:{}/{}",
                c.get("host").unwrap_or(""),
                c.port().unwrap_or(0),
                c.get("database").unwrap_or("")
            )
        })
        .unwrap_or_default();

    let outcome = match result {
        Ok(execute_result) => {
            let row_count = execute_result.row_count;
            // Fire-and-forget history entry
            let entry = crate::query_history::QueryHistoryEntry::new(
                conn_desc.clone(),
                conn_desc.clone(),
                db,
                final_sql.clone(),
                duration_ms,
                Some(row_count as u64),
                "success".into(),
                None,
            );
            let _ = crate::query_history::append_entry_async(entry).await;
            Ok(execute_result)
        }
        Err(e) => {
            let entry = crate::query_history::QueryHistoryEntry::new(
                conn_desc.clone(),
                conn_desc.clone(),
                db,
                final_sql.clone(),
                duration_ms,
                None,
                "error".into(),
                Some(e.clone()),
            );
            let _ = crate::query_history::append_entry_async(entry).await;
            Err(CommandError::new("QueryError", e))
        }
    };
    // Clear only if the slot still holds THIS query's id — an overlapping
    // newer query (pagination/filter refetch) must keep its registration.
    let mut slot = state.editor_query.lock().await;
    if matches!(*slot, Some((id, _)) if id == query_id) {
        *slot = None;
    }
    outcome
}

#[tauri::command]
pub async fn cancel_query(state: State<'_, AppState>) -> Result<(), String> {
    let Some((query_id, conn_id)) = *state.editor_query.lock().await else {
        return Ok(()); // nothing running — not an error
    };
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| "not connected".to_string())?;
    client.cancel(conn_id, query_id).await
}

#[tauri::command]
pub async fn get_databases(state: State<'_, AppState>) -> Result<Vec<DatabaseInfo>, CommandError> {
    let current = state
        .current_database
        .lock()
        .await
        .clone()
        .unwrap_or_default();

    if current.is_empty() {
        return Ok(Vec::new());
    }

    Ok(vec![DatabaseInfo {
        name: current,
        is_current: true,
    }])
}

#[tauri::command]
pub async fn get_schema_objects(
    state: State<'_, AppState>,
    namespace: Vec<String>,
) -> Result<SchemaObjectsResult, CommandError> {
    let conn_id = (*state.current_connection_id.lock().await)
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    // One request replaces four sequential queries. Empty `kinds` means every
    // kind the driver knows. The namespace arrives as path segments (the
    // sidebar passes SchemaInfo.path) — never as a dotted string, which would
    // be misread as a single segment by multi-segment drivers.
    let summaries = client
        .list_objects(conn_id, namespace, vec![])
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    Ok(SchemaObjectsResult {
        objects: summaries_to_schema_objects(summaries),
    })
}

#[tauri::command]
pub async fn get_editor_schema(
    state: State<'_, AppState>,
) -> Result<Vec<EditorTable>, CommandError> {
    let conn_id = (*state.current_connection_id.lock().await)
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let summaries = client
        .list_all_objects(
            conn_id,
            vec![
                lucent_protocol::ObjectKind::Table,
                lucent_protocol::ObjectKind::View,
            ],
        )
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    let refs: Vec<_> = summaries.into_iter().map(|s| s.reference).collect();
    let details = client
        .describe_objects(conn_id, refs)
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    Ok(object_details_to_editor_tables(details))
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaIndexingStatus {
    pub has_graph: bool,
    pub tier: Option<String>,
    pub table_count: usize,
    pub column_count: usize,
    pub view_count: usize,
    pub built_at_unix: i64,
}

#[tauri::command]
pub async fn get_schema_indexing_status(
    state: State<'_, AppState>,
) -> Result<SchemaIndexingStatus, CommandError> {
    let graph_opt = state.schema_graph.lock().await.clone();
    match graph_opt {
        Some(graph) => {
            let view_count = graph
                .tables
                .iter()
                .filter(|t| t.kind == "view" || t.kind == "matview")
                .count();
            let tier_str = match graph.tier {
                crate::ai::schema_graph::IndexingTier::MetadataOnly => "metadata_only",
                crate::ai::schema_graph::IndexingTier::FullyEnriched => "fully_enriched",
            };
            Ok(SchemaIndexingStatus {
                has_graph: true,
                tier: Some(tier_str.into()),
                table_count: graph.tables.len(),
                column_count: graph.columns.len(),
                view_count,
                built_at_unix: graph.built_at_unix,
            })
        }
        None => Ok(SchemaIndexingStatus {
            has_graph: false,
            tier: None,
            table_count: 0,
            column_count: 0,
            view_count: 0,
            built_at_unix: 0,
        }),
    }
}

#[tauri::command]
pub async fn sync_schema_indexing(
    force_rebuild: Option<bool>,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let conn_id = (*state.current_connection_id.lock().await)
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let capabilities = state
        .capabilities()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "capabilities not available"))?;
    let ai_cfg = state.ai_config.read().await.clone();
    let resolved = state
        .current_connection_config
        .lock()
        .await
        .clone()
        .ok_or_else(|| CommandError::new("QueryError", "connection config not found"))?;
    let connection_key = crate::ai::cache_store::connection_key_for(&resolved);
    // The memory key is captured at connect time (profile id or the inline
    // host:port/database key) — it is NOT the cache key.
    let memory_connection_key = state
        .memory_connection_key
        .lock()
        .await
        .clone()
        .unwrap_or_default();
    let sampling_conn = *state.ai_connection_id.lock().await;

    state
        .indexing
        .sync_delta(
            conn_id,
            client,
            sampling_conn,
            state.schema_graph.clone(),
            state.current_connection_id.clone(),
            state.embedder.clone(),
            Arc::new(ai_cfg),
            connection_key,
            memory_connection_key,
            capabilities,
            force_rebuild.unwrap_or(false),
            state.memory_manager.clone(),
        )
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    Ok(())
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> Result<DisconnectResult, CommandError> {
    log::info!("Disconnecting");
    let client = state.client.lock().await.take();

    if let Some(mut c) = client {
        // Best-effort graceful per-session close before shutdown() aborts the
        // reader task that routes the Disconnected reply.
        if let Some(ai_id) = *state.ai_connection_id.lock().await {
            let _ = c.disconnect_id(ai_id).await;
        }
        log::debug!("Running disconnect on connector client");
        let _ = c.shutdown().await;
    }

    // TODO(multi-connection): only shutdown supervisor when last connection disconnects
    let mut supervisor_lock = state.supervisor.lock().await;
    if let Some(mut supervisor) = supervisor_lock.take() {
        supervisor.shutdown().await.ok();
    }

    let conn_id = *state.current_connection_id.lock().await;
    if let Some(id) = conn_id {
        state.indexing.stop(id).await;
    }

    *state.current_connection_id.lock().await = None;
    *state.ai_connection_id.lock().await = None;
    *state.current_database.lock().await = None;
    *state.driver_capabilities.lock().await = None;
    *state.memory_connection_key.lock().await = None;

    // Notebook sessions hold ConnectionIds into the dying worker; clear them
    // so attach/restart fails fast instead of against dead connections.
    state.notebook_sessions.clear();

    Ok(DisconnectResult { ok: true })
}

// ─── Logs Drawer ───────────────────────────────────────────────────────

/// Pure tailing helper: returns the buffer's lines at or after `after`.
fn logs_after(logs: &std::collections::VecDeque<String>, after: usize) -> Vec<String> {
    logs.iter().skip(after).cloned().collect()
}

/// Returns worker stderr lines from the in-app ring buffer, skipping the
/// first `after` lines (tailing). The caller passes the count of lines it
/// already holds, so repeated calls fetch only new lines. The buffer is
/// capped (oldest dropped — see [`crate::supervisor::LOG_BUFFER_CAP`]), so
/// an `after` beyond the retained prefix yields just the current tail.
#[tauri::command]
pub async fn get_logs(
    state: State<'_, AppState>,
    after: Option<u64>,
) -> Result<Vec<String>, String> {
    Ok(logs_after(
        &*state.logs.lock().await,
        after.unwrap_or(0) as usize,
    ))
}

#[tauri::command]
pub async fn get_function_source(
    state: State<'_, AppState>,
    namespace: Vec<String>,
    name: String,
) -> Result<String, CommandError> {
    let conn_id = (*state.current_connection_id.lock().await)
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    client
        .object_ddl(
            conn_id,
            lucent_protocol::ObjectRef {
                namespace,
                name,
                kind: lucent_protocol::ObjectKind::Function,
            },
        )
        .await
        .map_err(|e| CommandError::new("QueryError", e))
}

#[tauri::command]
pub async fn get_view_source(
    state: State<'_, AppState>,
    namespace: Vec<String>,
    name: String,
    kind: Option<String>,
) -> Result<String, CommandError> {
    let conn_id = (*state.current_connection_id.lock().await)
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    // The driver returns the complete statement, header included — assembling
    // `CREATE OR REPLACE VIEW` in the app would hardcode Postgres syntax.
    // Materialized views need their own header, so the kind comes through
    // from the sidebar click.
    let kind = match kind.as_deref() {
        Some("matview") => lucent_protocol::ObjectKind::MaterializedView,
        _ => lucent_protocol::ObjectKind::View,
    };
    client
        .object_ddl(
            conn_id,
            lucent_protocol::ObjectRef {
                namespace,
                name,
                kind,
            },
        )
        .await
        .map_err(|e| CommandError::new("QueryError", e))
}

#[derive(Debug, Serialize)]
pub struct SequenceProperty {
    pub key: String,
    pub value: String,
}

#[tauri::command]
pub async fn get_sequence_info(
    state: State<'_, AppState>,
    namespace: Vec<String>,
    name: String,
) -> Result<Vec<SequenceProperty>, CommandError> {
    let conn_id = (*state.current_connection_id.lock().await)
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let props = client
        .object_properties(
            conn_id,
            lucent_protocol::ObjectRef {
                namespace,
                name,
                kind: lucent_protocol::ObjectKind::Sequence,
            },
        )
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    Ok(props
        .into_iter()
        .map(|p| SequenceProperty {
            key: p.key,
            value: p.value,
        })
        .collect())
}

/// `SELECT * FROM` with every namespace segment quoted separately.
///
/// The namespace arrives as PATH SEGMENTS (`["analytics", "main"]` for a
/// DuckDB file), never as a dotted display name — quoting the dotted string
/// as one identifier (`"analytics.main"`) matches nothing. Postgres passes
/// one segment (`["public"]`), so this reproduces today's SQL exactly.
pub fn table_base_sql(
    builder: &dyn crate::sql_builder::SqlBuilder,
    namespace: &[String],
    name: &str,
) -> String {
    let qualified = namespace
        .iter()
        .map(|s| builder.quote_identifier(s))
        .collect::<Vec<_>>()
        .join(".");
    format!(
        "SELECT * FROM {qualified}.{}",
        builder.quote_identifier(name)
    )
}

#[tauri::command]
pub async fn browse_table(
    state: State<'_, AppState>,
    namespace: Vec<String>,
    name: String,
    limit: i64,
    offset: i64,
    // Same list-shaped wire sort as execute_query: Option-wrapped because
    // tauri params cannot take #[serde(default)].
    sort: Option<Vec<crate::query_paging::SortSpec>>,
    filters: Vec<crate::query_paging::FilterSpec>,
) -> Result<ExecuteResult, CommandError> {
    let sort = sort.unwrap_or_default();
    let conn_id = (*state.current_connection_id.lock().await)
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let capabilities = state
        .capabilities()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let builder = crate::sql_builder::for_driver(&capabilities);

    let base_sql = table_base_sql(builder.as_ref(), &namespace, &name);
    let final_sql = crate::query_paging::wrap_for_page(
        &base_sql,
        &sort,
        &filters,
        limit,
        offset,
        builder.as_ref(),
    );

    client
        .execute_with_id(
            QueryId(Uuid::new_v4()),
            conn_id,
            &final_sql,
            // Safety net mirroring execute_query: the page size is caller-
            // supplied, so cap what gets materialized even though the SQL
            // itself is always LIMIT-wrapped.
            Some(crate::client::HARD_ROW_CAP),
        )
        .await
        .map(|(r, _)| r)
        .map_err(|e| CommandError::new("QueryError", e))
}

#[tauri::command]
pub async fn count_all_rows(
    state: State<'_, AppState>,
    sql: String,
    filters: Vec<crate::query_paging::FilterSpec>,
) -> Result<i64, CommandError> {
    let dialect = state
        .capabilities()
        .await
        .map(|c| c.sql_dialect)
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    if !crate::query_paging::is_wrappable_query(&sql, dialect) {
        return Err(CommandError::new(
            "QueryError",
            "cannot count rows for a non-SELECT statement",
        ));
    }

    let conn_id = (*state.current_connection_id.lock().await)
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let capabilities = state
        .capabilities()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let builder = crate::sql_builder::for_driver(&capabilities);

    let count_sql = crate::query_paging::wrap_for_count(&sql, &filters, builder.as_ref());
    let result = client
        .execute(conn_id, &count_sql)
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    result
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(|v| {
            // With the text protocol, COUNT(*) comes as a string "42"
            // instead of an integer 42. Try both.
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
        })
        .ok_or_else(|| CommandError::new("QueryError", "COUNT(*) returned no usable value"))
}

/// Renders the WHERE clause the given filters produce, for display in the
/// grid's SQL preview. Pure string building — takes no AppState and touches no
/// database, so it works before a connection exists. The preview renders for
/// the *current* connection, which is PostgreSQL today, so the Postgres builder
/// is the right renderer until a second driver reaches the grid.
#[tauri::command]
pub fn describe_filters(filters: Vec<crate::query_paging::FilterSpec>) -> String {
    crate::query_paging::filters_to_where_clause(&filters, &crate::sql_builder::PostgresSqlBuilder)
}

#[cfg(test)]
mod logs_after_tests {
    use std::collections::VecDeque;

    use super::logs_after;

    fn buffer(lines: &[&str]) -> VecDeque<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn returns_all_lines_from_zero() {
        let buf = buffer(&["a", "b", "c"]);
        assert_eq!(logs_after(&buf, 0), vec!["a", "b", "c"]);
    }

    #[test]
    fn returns_only_lines_after_the_index() {
        let buf = buffer(&["a", "b", "c"]);
        assert_eq!(logs_after(&buf, 1), vec!["b", "c"]);
        assert_eq!(logs_after(&buf, 3), Vec::<String>::new());
    }

    #[test]
    fn beyond_the_tail_is_empty() {
        let buf = buffer(&["a", "b"]);
        assert_eq!(logs_after(&buf, 10), Vec::<String>::new());
    }
}

#[cfg(test)]
mod describe_filters_tests {
    use crate::query_paging::FilterSpec;

    #[test]
    fn renders_the_predicate_for_the_ui() {
        let filters = vec![FilterSpec {
            column: "status".into(),
            operator: "eq".into(),
            value: Some("active".into()),
        }];
        assert_eq!(
            super::describe_filters(filters),
            r#"WHERE "status" = 'active'"#
        );
    }

    #[test]
    fn renders_an_empty_string_for_no_filters() {
        assert_eq!(super::describe_filters(vec![]), "");
    }
}

// ── AI commands ─────────────────────────────────────────────────────────

/// Load API key with fallback chain: file → env var → keychain.
/// File-first avoids macOS keychain prompts during development.
pub(crate) fn load_api_key(config: &AiConfig) -> Result<String, String> {
    // 1. Try ~/.lucent/ai-key.txt (avoids keychain prompts)
    if let Ok(home) = std::env::var("HOME") {
        let path = std::path::PathBuf::from(home)
            .join(".lucent")
            .join("ai-key.txt");
        if let Ok(key) = std::fs::read_to_string(&path) {
            let trimmed = key.trim().to_string();
            if !trimmed.is_empty() {
                log::debug!("LLM API key loaded from {:?}", path);
                return Ok(trimmed);
            }
            log::warn!("ai-key.txt is empty");
        } else {
            log::debug!("No ai-key.txt at {:?}", path);
        }
    }

    // 2. Try OPENAI_API_KEY env var
    if let Ok(key) = std::env::var("OPENAI_API_KEY") {
        if !key.is_empty() {
            log::debug!("LLM API key loaded from OPENAI_API_KEY env var");
            return Ok(key);
        }
        log::warn!("OPENAI_API_KEY env var is set but empty");
    } else {
        log::debug!("OPENAI_API_KEY env var not set");
    }

    // 3. Try keychain (last resort - causes macOS prompts)
    let account = keychain_account(&config.provider);
    match keyring::Entry::new(KEYCHAIN_SERVICE, account).and_then(|e| e.get_password()) {
        Ok(key) => {
            log::debug!("LLM API key loaded from OS keychain");
            return Ok(key);
        }
        Err(keyring_err) => {
            log::warn!(
                "Keychain lookup failed ({:?}): {keyring_err}",
                config.provider
            );
        }
    }

    let err = format!(
        "API key not found for provider {:?}. \
         Save it in AI Settings (stored in macOS Keychain), \
         or set the OPENAI_API_KEY environment variable, \
         or create ~/.lucent/ai-key.txt",
        config.provider
    );
    log::error!("{err}");
    Err(err)
}

/// Pure lookup for the in-memory key cache: hit only when the cached entry
/// belongs to the requested provider.
pub(crate) fn cached_api_key(
    cache: &Option<(crate::ai::config::AiProvider, String)>,
    provider: &crate::ai::config::AiProvider,
) -> Option<String> {
    cache
        .as_ref()
        .filter(|(p, _)| p == provider)
        .map(|(_, k)| k.clone())
}

/// Builds a conversation's system prompt: the schema tree (from the cache or,
/// on expiry, rendered from the in-memory graph) plus the context tier used
/// by preflight. Shared by `ai_chat` and `execute_dml`'s agent resume (C1).
async fn build_system_prompt(
    state: &AppState,
    connection_id: &str,
) -> (String, crate::ai::mschema::ContextTier) {
    let (prompt, tier, _) = build_system_prompt_with_query(state, connection_id, None, None).await;
    (prompt, tier)
}

/// Returns the built prompt, the context tier, and the number of learned
/// memory rules injected (F-C2). The count is `0` when memory is disabled or
/// no query was supplied; it flows to the turn's `Done` event.
async fn build_system_prompt_with_query(
    state: &AppState,
    connection_id: &str,
    query: Option<&str>,
    conversation_id: Option<&str>,
) -> (String, crate::ai::mschema::ContextTier, usize) {
    // Static prompt + tier selection happen under the schema_graph lock. The
    // guard is scoped to this block and dropped before any memory work:
    // embedding can download/run a 130 MB ONNX model, retrieval reads SQLite,
    // and reranking runs a second ONNX model. Holding the central schema lock
    // across any of that stalls autocomplete, the schema tree, background
    // indexing, and tools. The graph is cloned so Gate 2's live-schema
    // validation survives the narrowed lock.
    log::info!("Acquiring schema graph for system prompt");
    let (mut prompt, tier, graph_snapshot) = {
        let graph_guard = state.schema_graph.lock().await;
        log::info!("Schema graph locked, building tier");
        let tier = graph_guard
            .as_ref()
            .map(|g| crate::ai::mschema::select_tier(g).0)
            .unwrap_or(crate::ai::mschema::ContextTier::Pull);
        log::info!("Tier selected: {:?}", tier);
        let capabilities = state.capabilities().await;
        let current_db = state.current_database.lock().await.clone();
        let prompt = if let Some(mut tree) = state.schema_cache.get(connection_id) {
            if let Some(db) = &current_db {
                if tree.database_name.is_empty() || tree.database_name == connection_id {
                    tree.database_name = db.clone();
                }
            }
            let p = crate::ai::context::build_system_prompt(
                &tree,
                graph_guard.as_ref(),
                capabilities.as_ref(),
            );
            log::debug!("System prompt built ({} bytes, tier {:?})", p.len(), tier);
            p
        } else if let Some(g) = graph_guard.as_ref() {
            log::info!(
                "Schema tree expired for {connection_id}; rendering system prompt from in-memory graph"
            );
            let db_name = current_db
                .unwrap_or_else(|| crate::ai::context::parse_database_name(connection_id));
            let version = state
                .client_handle()
                .await
                .and_then(|c| c.server_info.map(|s| s.version))
                .unwrap_or_default();
            let tree = crate::ai::context::tree_from_graph_with_version(db_name, version, g);
            crate::ai::context::build_system_prompt(&tree, Some(g), capabilities.as_ref())
        } else {
            log::warn!(
                "Schema cache miss for connection {connection_id} and no schema graph available"
            );
            "Database context not yet loaded.".into()
        };
        (prompt, tier, graph_guard.clone())
    }; // graph_guard dropped here — no inference, SQLite, or reranker work under it

    // Hot-Tier Memory Block Injection (P0 - G-1). Runs after the schema lock is
    // released; also see B-I9 for the embedder init lock. The reranker lock is
    // taken only after the schema lock is dropped (no inversion hazard).
    let enable_memory = state
        .ai_config
        .read()
        .await
        .is_memory_enabled(connection_id);
    let mut applied_memory_count = 0usize;
    if enable_memory {
        // Profile plane: rules that apply to every turn for this connection.
        // The snapshot is built once for the conversation and then frozen —
        // a miss builds from the DB, a hit reuses the stored snapshot without
        // rebuilding, so rules learned mid-session never mutate an in-flight
        // prompt. Injected before the per-query retrieved block.
        if let Some(conv) = conversation_id {
            let cached = state.profile_snapshots.get(conv).map(|r| r.clone());
            let snapshot = match cached {
                Some(snapshot) => snapshot,
                None => {
                    match crate::ai::memory::ProfileSnapshot::build(
                        &state.memory_manager,
                        connection_id,
                        crate::ai::memory::PROFILE_CAPACITY_TOKENS,
                    )
                    .await
                    {
                        Ok(snapshot) => {
                            state
                                .profile_snapshots
                                .insert(conv.to_string(), snapshot.clone());
                            snapshot
                        }
                        Err(e) => {
                            log::warn!("profile snapshot build failed: {e}");
                            crate::ai::memory::ProfileSnapshot {
                                connection_key: connection_id.to_string(),
                                memories: Vec::new(),
                                built_at: 0,
                            }
                        }
                    }
                }
            };
            if let Some(profile_block) = crate::ai::memory::format_profile_block(&snapshot.memories)
            {
                prompt.push_str(&profile_block);
            }
        }

        if let Some(user_query) = query {
            let q_vec = if let Some(emb) = state.get_or_init_memory_embedder().await {
                emb.embed_query(user_query).await.ok()
            } else {
                let emb_guard = state.embedder.lock().await;
                if let Some(emb) = emb_guard.as_ref() {
                    emb.embed_query(user_query).await.ok()
                } else {
                    None
                }
            };

            if let Ok(active_memories) = state
                .memory_manager
                .list_memories(connection_id, false)
                .await
            {
                let reranker_guard = state.reranker.lock().await;
                let retrieved = state
                    .memory_manager
                    .retrieve_hybrid_memories(
                        user_query,
                        connection_id,
                        q_vec.as_deref(),
                        graph_snapshot.as_ref(),
                        reranker_guard.as_ref(),
                        &active_memories,
                    )
                    .await;

                let golden = if let Ok(all_golden) = state
                    .memory_manager
                    .list_golden_queries(connection_id)
                    .await
                {
                    crate::ai::memory::retrieve_golden_queries(q_vec.as_deref(), &all_golden)
                } else {
                    Vec::new()
                };

                for m in &retrieved {
                    let _ = state.memory_manager.reinforce_memory_access(&m.id).await;
                }

                // F-C2: report exactly the rules the block injected.
                applied_memory_count = retrieved.len();

                if let Some(mem_block) =
                    crate::ai::context::format_memory_block(&retrieved, &golden)
                {
                    prompt.push_str(&mem_block);
                }
            }
        }
    }

    (prompt, tier, applied_memory_count)
}

/// Lazily initializes the cross-encoder reranker (a second ONNX model
/// download). Called on first semantic search, never during connect.
pub async fn ensure_reranker(state: &AppState) {
    if state.reranker.lock().await.is_some() {
        return;
    }
    match tokio::task::spawn_blocking(crate::ai::rerank::Reranker::new).await {
        Ok(Ok(r)) => {
            *state.reranker.lock().await = Some(r);
        }
        Ok(Err(e)) => log::warn!("Reranker init failed, semantic search will skip reranking: {e}"),
        Err(e) => log::warn!("Reranker init task panicked: {e}"),
    }
}

/// Constructs the turn driver from the config: the ACP driver when `acp` is
/// set, else the rig `DatabaseAgent`. Pure so the branch is unit-testable
/// (the D1 seam test). The `provider` is only ever built on the rig path —
/// ACP agents own their auth.
pub(crate) fn pick_driver(
    acp: &Option<crate::ai::config::AcpAgentConfig>,
    provider: Option<Arc<dyn LlmProvider>>,
    tools: Vec<crate::ai::tools::LucentToolEnum>,
    tool_ctx: AiToolContext,
    acp_state: crate::ai::acp::AcpState,
) -> Box<dyn AgentDriver> {
    match acp {
        Some(cfg) => Box::new(crate::ai::acp::driver::AcpChatDriver::new(
            acp_state,
            cfg.clone(),
            tool_ctx,
        )),
        None => Box::new(DatabaseAgent::new(
            provider.expect("the rig path always builds a provider"),
            tools,
            tool_ctx,
        )),
    }
}

/// Runs one full agent turn: provider creation, state transition to
/// `Running`, preflight (skipped when `message` is empty — the resume-after-
/// DML case needs no schema injection), the agent loop, and error/timeout
/// handling. Deliberately has NO `PausedForDml` guard: `ai_chat` rejects new
/// messages while a DML is pending, but `execute_dml` resumes the agent
/// through here after the user approves.
///
/// The ACP branch happens here, after the conversation CAS (spec §3 D3):
/// with `acp` configured, the API-key load + `RigProvider` section is
/// skipped and the turn drives the `AcpChatDriver` instead. Everything
/// downstream — timeout wrapper, sink, error handling — is shared.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_agent_turn<R: tauri::Runtime>(
    state: &AppState,
    app_handle: &tauri::AppHandle<R>,
    channel: tauri::ipc::Channel<AiEvent>,
    conversation_id: String,
    message: String,
    system_prompt: String,
    applied_memory_count: usize,
    assistant_message_id: Option<String>,
) -> Result<(), String> {
    let conv = state
        .conversations
        .get(&conversation_id)
        .ok_or("Conversation not found")?
        .clone();

    let config = state.ai_config.read().await.clone();
    log::debug!(
        "ai_chat config: provider={}, model={}, max_turns={}",
        config.provider,
        config.model,
        config.max_turns
    );

    // ACP branch point: with `acp` configured the rig-only section below is
    // skipped entirely — ACP agents own their auth, and `keychain_account`
    // for `AiProvider::Acp` is a placeholder by design (spec §4.7/D8).
    let is_acp = config.acp.is_some();

    // Read the cache separately so the read guard drops before the match body
    // runs. Otherwise a write() inside the None branch would deadlock — the
    // temporary read guard from the scrutinee lives until the match ends.
    let cached_key = {
        let guard = state.api_key_cache.read().await;
        cached_api_key(&guard, &config.provider)
    };
    // RIG-ONLY: the API key load + `RigProvider` construction. `context_tier`
    // stays shared — the preflight below consumes it on both paths.
    let context_tier = {
        let guard = state.schema_graph.lock().await;
        guard
            .as_ref()
            .map(|g| crate::ai::mschema::select_tier(g).0)
            .unwrap_or(crate::ai::mschema::ContextTier::Pull)
    };
    let provider: Option<Arc<dyn LlmProvider>> = if is_acp {
        None
    } else {
        let api_key = match cached_key {
            Some(k) => k,
            None => {
                let t0 = std::time::Instant::now();
                let key = load_api_key(&config)?;
                log::info!(
                    "API key loaded from keychain/env/file in {:.0?}",
                    t0.elapsed()
                );
                *state.api_key_cache.write().await = Some((config.provider.clone(), key.clone()));
                key
            }
        };
        log::info!("Creating LLM provider");
        Some(Arc::new(RigProvider::new(
            config.provider.clone(),
            api_key,
            config.endpoint.clone(),
        )))
    };

    let cancel = tokio_util::sync::CancellationToken::new();
    {
        let mut locked = conv.lock().await;
        // E1: the CAS closes the window between ai_chat_impl's friendly
        // pre-check and this point — two concurrent calls cannot both
        // claim the conversation.
        locked.try_begin_turn(cancel.clone())?;
        // D2: keep the channel on the conversation only after the claim
        // succeeds. `execute_dml` clones it to resume the agent on the same
        // IPC stream after the user approves DML (C1). Writing it before the
        // claim left a losing concurrent ai_chat's dead channel registered,
        // and a later execute_dml resume would emit into it.
        locked.event_channel = Some(channel.clone());
        // The ACP driver keys its session-per-conversation map by this —
        // several conversations share one connection_id, so the connection
        // id alone would merge their ACP sessions.
        locked.conversation_id = Some(conversation_id.clone());
    }

    // Reranker warm-up is rig-only: the ACP path's tools run behind the
    // bridge and degrade gracefully without it (semantic search skips
    // reranking when the model is absent).
    if !is_acp {
        log::info!("Provider created, building tool context");
        ensure_reranker(state).await;
    }

    let ai_conn_id = {
        let guard = state.ai_connection_id.lock().await;
        *guard
    };
    let tool_ctx = AiToolContext {
        db: state.client.clone(),
        connection_id: ai_conn_id.or(*state.current_connection_id.lock().await),
        memory_connection_key: state.memory_connection_key.lock().await.clone(),
        capabilities: state.capabilities().await,
        config: config.clone(),
        schema_graph: state.schema_graph.clone(),
        embedder: state.embedder.clone(),
        reranker: state.reranker.clone(),
        memory_manager: state.memory_manager.clone(),
    };

    // Pre-flight: augment the message with value hints and retrieved schema
    // context before the first LLM call. Skipped for the resume-after-DML
    // turn — the outcome message is already in history.
    let augmented_message = if message.is_empty() {
        message
    } else {
        log::info!("Pre-flight starting");
        let graph_guard = tool_ctx.schema_graph.lock().await;
        let emb_guard = tool_ctx.embedder.lock().await;
        let result = crate::ai::preflight::run_preflight(
            tool_ctx.connection_id,
            Some(&tool_ctx.db),
            graph_guard.as_ref(),
            emb_guard.as_ref(),
            &context_tier,
            &message,
            tool_ctx.capabilities.as_ref(),
        )
        .await;
        log::info!("Pre-flight completed");
        match result {
            Some(block) => format!("{message}\n\n{block}"),
            None => message,
        }
    };

    log::info!("Agent loop starting");
    let tools = if is_acp {
        Vec::new() // ACP tools live behind the bridge — the agent calls them over MCP.
    } else {
        crate::ai::tools::all_tools(tool_ctx.clone())
    };
    let driver = pick_driver(&config.acp, provider, tools, tool_ctx, state.acp.clone());
    let sink: Arc<dyn AgentSink> = Arc::new(TauriSink {
        channel,
        app_handle: app_handle.clone(),
        conversation_id: conversation_id.clone(),
        assistant_message_id,
        turn_text: Arc::new(std::sync::Mutex::new(String::new())),
        turn_segments: Arc::new(std::sync::Mutex::new(Vec::new())),
        turn_start_ms: chrono::Utc::now().timestamp_millis(),
    });
    let app_err = app_handle.clone();
    let conv_err = conv.clone();
    const AGENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
    let chat_result = tokio::time::timeout(
        AGENT_TIMEOUT,
        driver.chat(
            augmented_message,
            &config,
            system_prompt,
            conv,
            sink,
            cancel.clone(),
            applied_memory_count,
        ),
    )
    .await;

    match chat_result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            log::error!("Agent error: {e}");
            let _ = app_err.emit(
                "ai:error",
                AiErrorPayload {
                    conversation_id: conversation_id.clone(),
                    message: e,
                },
            );
            let mut s = conv_err.lock().await;
            s.state = AgentState::Idle;
        }
        Err(_) => {
            log::error!("Agent timed out after 300s");
            cancel.cancel();
            let stderr_note = if is_acp {
                if let Some(acp_cfg) = config.acp.as_ref() {
                    let snip = state.acp.agent_stderr_snippet(&acp_cfg.agent_id);
                    state.acp.kill_agent(&acp_cfg.agent_id).await;
                    snip.map(|s| format!(" Last agent output:\n{}", s))
                        .unwrap_or_default()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            let _ = app_err.emit(
                "ai:error",
                AiErrorPayload {
                    conversation_id: conversation_id.clone(),
                    message: format!(
                        "Agent timed out after 300 seconds. Try simplifying the question.{}",
                        stderr_note
                    ),
                },
            );
            let mut s = conv_err.lock().await;
            s.state = AgentState::Idle;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn ai_chat(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    channel: tauri::ipc::Channel<AiEvent>,
    message: String,
    conversation_id: String,
    connection_id: String,
    profile_id: Option<String>,
    user_message_id: Option<String>,
    assistant_message_id: Option<String>,
) -> Result<(), String> {
    // A chat turn is user activity: reset the idle clock so the sleep-time
    // compute daemon does not fire while the user is actively working.
    state.idle_tracker.touch();
    // Correlate the whole turn (including bridged `log::` lines) under the
    // `ai_chat` span.
    let span = crate::trace::ai_chat_span(&conversation_id);
    ai_chat_impl(
        state,
        app_handle,
        channel,
        message,
        conversation_id,
        connection_id,
        profile_id,
        user_message_id,
        assistant_message_id,
    )
    .instrument(span)
    .await
}

#[allow(clippy::too_many_arguments)]
async fn ai_chat_impl(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    channel: tauri::ipc::Channel<AiEvent>,
    message: String,
    conversation_id: String,
    connection_id: String,
    profile_id: Option<String>,
    user_message_id: Option<String>,
    assistant_message_id: Option<String>,
) -> Result<(), String> {
    // Verify a database connection is active before starting the AI agent.
    // Without this, the agent wastes tokens and time on tools that will all
    // fail with "Database not connected".
    if state.client.lock().await.is_none() && profile_id.is_none() {
        return Err("Connect to a database before using AI features.".into());
    }

    // If profile_id is provided, resolve and ensure we're connected via that profile
    if let Some(ref pid) = profile_id {
        let profile = state
            .repo
            .get_profile(pid)
            .await
            .ok_or_else(|| "Connection profile not found".to_string())?;
        let config = profile_to_config(&state, &profile, pid)
            .await
            .map_err(|e| e.to_string())?;

        // Ensure worker is connected. Only touched when no client exists yet:
        // if a client is live, the AI reuses it (and its driver) regardless of
        // the profile, and switching the supervisor here would kill the worker
        // under the editor's connection.
        let mut client_lock = state.client.lock().await;
        if client_lock.is_none() {
            let (socket_path, token) = {
                let mut supervisor_lock = state.supervisor.lock().await;
                // Same driver-switch semantics as connect_impl: the AI can
                // attach a profile of a different driver than the editor's
                // last connection.
                let needs_replacement = supervisor_lock
                    .as_ref()
                    .is_some_and(|s| s.driver_id() != config.driver);
                if needs_replacement {
                    if let Some(mut old) = supervisor_lock.take() {
                        log::info!(
                            "Switching driver from {} to {}; stopping the old worker",
                            old.driver_id(),
                            config.driver
                        );
                        old.shutdown().await.ok();
                    }
                }
                let sup = supervisor_lock.get_or_insert_with(|| {
                    Supervisor::for_driver(&config.driver, state.logs.clone())
                });
                sup.ensure_running()
                    .await
                    .map_err(|e| format!("Worker startup failed: {e}"))?;
                let socket_path = sup.endpoint().to_string();
                let token = sup.handshake_token().to_string();
                (socket_path, token)
            };

            let (new_client, new_conn_id) = ConnectorClient::connect(&socket_path, &token, config)
                .await
                .map_err(|e| format!("Connect failed: {e}"))?;
            *state.current_connection_id.lock().await = Some(new_conn_id);
            *client_lock = Some(new_client);
        }
    }
    let conv = state
        .conversations
        .entry(conversation_id.clone())
        .or_insert_with(|| Arc::new(Mutex::new(ConversationState::new(connection_id.clone()))))
        .clone();

    {
        let locked = conv.lock().await;
        match &locked.state {
            AgentState::Idle => {}
            AgentState::PausedForDml { .. } => {
                return Err(
                    "Approve or cancel the pending DML before sending another message.".into(),
                );
            }
            AgentState::Running { .. } => {
                return Err("An AI response is already in progress for this conversation.".into());
            }
        }
    }

    // D2: the channel is registered inside run_agent_turn, after the CAS
    // claim succeeds — never here, where a losing concurrent ai_chat would
    // leave its dead channel behind for execute_dml to emit into.
    log::info!(
        "ai_chat: conversation={conversation_id}, message_len={}",
        message.len()
    );

    let now = chrono::Utc::now().timestamp();
    let conv_title = if message.len() > 40 {
        format!("{}...", &message[..40])
    } else {
        message.clone()
    };
    let _ = state
        .memory_manager
        .save_conversation(crate::ai::memory::ChatConversation {
            id: conversation_id.clone(),
            connection_id: connection_id.clone(),
            title: conv_title,
            archived: false,
            created_at: now,
            updated_at: now,
        })
        .await;

    let user_msg_id = user_message_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let _ = state
        .memory_manager
        .save_message(crate::ai::memory::ChatMessage {
            id: user_msg_id,
            conversation_id: conversation_id.clone(),
            role: "user".into(),
            content: message.clone(),
            session_json: None,
            created_at: now,
        })
        .await;

    let (system_prompt, _context_tier, applied_memory_count) = build_system_prompt_with_query(
        &state,
        &connection_id,
        Some(&message),
        Some(&conversation_id),
    )
    .await;
    log::info!("System prompt complete ({} bytes)", system_prompt.len());

    // `run_agent_turn` consumes `message`; keep a copy for the observer so an
    // explicit "remember that …" in this turn is captured at completion. The
    // memory subsystem keys on the frontend memory key, falling back to the
    // command's `connection_id` when no connect-time key was captured.
    let observer_user_text = message.clone();
    let observer_connection_id = connection_id.clone();
    let observer_conversation_id = conversation_id.clone();

    let result = run_agent_turn(
        &state,
        &app_handle,
        channel,
        conversation_id,
        message,
        system_prompt,
        applied_memory_count,
        assistant_message_id,
    )
    .await;

    // Detached: observation capture must never delay the final token, so it
    // runs on its own task after the turn has returned. Covers both the rig and
    // ACP drivers, which share `run_agent_turn`.
    let observer_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let state = observer_handle.state::<AppState>();
        let connection_key = state
            .memory_connection_key
            .lock()
            .await
            .clone()
            .unwrap_or(observer_connection_id);
        let turn = crate::ai::memory::TurnOutcome {
            connection_key,
            conversation_id: observer_conversation_id,
            turn_id: Uuid::new_v4().to_string(),
            runtime: crate::ai::memory::TurnRuntime::Rig,
            user_text: Some(observer_user_text),
            assistant_text: None,
            executed_sql: vec![],
            tool_calls: vec![],
        };
        crate::ai::memory::record_turn_observations(&state, turn).await;
    });

    result
}

#[tauri::command]
pub async fn ai_cancel(state: State<'_, AppState>, conversation_id: String) -> Result<(), String> {
    let conv = state
        .conversations
        .get(&conversation_id)
        .ok_or("Conversation not found")?;
    let mut s = conv.lock().await;
    match &s.state {
        AgentState::Running { cancel } => {
            cancel.cancel();
            // ACP: if the bridge is holding a `preview_dml` approval open,
            // nothing will ever approve it now — reject it so the agent's
            // MCP call completes with an error instead of hanging, and the
            // slot clears (a stale approve can't execute after cancel).
            if state.ai_config.read().await.acp.is_some() {
                if let Some(handle) = state
                    .acp
                    .bridges
                    .lock()
                    .await
                    .get(&conversation_id)
                    .cloned()
                {
                    let _ = reject_acp_dml(&handle).await;
                }
            }
            s.state = AgentState::Idle;
        }
        AgentState::PausedForDml { .. } => {
            let _ = s.take_staged_sql();
        }
        AgentState::Idle => {}
    }
    Ok(())
}

/// Drops a conversation's backend state (history + query_cache). Closing a
/// chat tab in the UI only forgets the conversation locally — without this,
/// `AppState.conversations` grows forever for the life of the process, since
/// nothing else ever removes an entry.
fn evict_conversation(state: &AppState, conversation_id: &str) {
    state.conversations.remove(conversation_id);
    state.llm_usage.remove(conversation_id);
    state.profile_snapshots.remove(conversation_id);
}

#[tauri::command]
pub async fn close_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    // ACP: end the conversation's session — the bridge socket closes with it
    // and any parked permission requests auto-reject (spec §4.5 teardown).
    state.acp.drop_session(&conversation_id).await;
    evict_conversation(&state, &conversation_id);
    Ok(())
}

const DML_STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(300);

/// The DML preview staleness window. `LUCENT_DML_STALE_AFTER_SECS` overrides
/// for tests (a 300s sleep is not a test).
fn dml_stale_after() -> std::time::Duration {
    std::env::var("LUCENT_DML_STALE_AFTER_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or(DML_STALE_AFTER)
}

/// Executes the staged DML on the worker (session B), returns the REAL row
/// count, and resumes the agent on the conversation's channel so it can
/// confirm the outcome (C1). The frontend signature stays
/// `executeDml(conversationId)` — tauri injects `AppHandle` itself.
#[tauri::command]
pub async fn execute_dml(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    let conv = state
        .conversations
        .get(&conversation_id)
        .ok_or("Conversation not found")?
        .clone();

    // ACP branch (spec §3 D5): with `acp` configured the staged SQL lives in
    // the bridge's single-slot pending registry, NOT `conv_state` — and the
    // agent's turn is still running while the bridge holds `preview_dml`, so
    // `run_agent_turn` is NEVER invoked here (its API-key load would fail
    // first; its `try_begin_turn` CAS would reject the running conversation).
    // Approving resolves the held MCP tool call with the execution summary.
    if state.ai_config.read().await.acp.is_some() {
        let handle = state
            .acp
            .bridges
            .lock()
            .await
            .get(&conversation_id)
            .cloned()
            .ok_or("No pending DML for this conversation (bridge not active)")?;
        // E6 parity (spec D6): an approval minutes after the preview may no
        // longer match the data — refuse and clear the hold.
        if reject_stale_acp_dml(&handle, dml_stale_after()).await {
            return Err(
                "The pending DML was staged more than 5 minutes ago. Ask the assistant \
                 to re-run the preview and approve again."
                    .into(),
            );
        }
        let conn_id = *state.ai_connection_id.lock().await;
        let conn_id = conn_id
            .or(*state.current_connection_id.lock().await)
            .ok_or("not connected")?;
        let client = state.client_handle().await.ok_or("not connected")?;
        let (rows_affected, sql) = resolve_acp_dml(&handle, |sql| {
            let client = client.clone();
            async move { execute_staged_dml(&client, conn_id, &conv, sql).await }
        })
        .await?;
        log::info!("DML executed (ACP): {rows_affected} rows affected — {sql}");
        let _ = app_handle.emit(
            "dml:executed",
            serde_json::json!({
                "conversation_id": conversation_id.clone(),
                "rows_affected": rows_affected,
            }),
        );
        return Ok(serde_json::json!({ "rows_affected": rows_affected, "sql": sql }));
    }

    let (staged_sql, staged_at) = conv
        .lock()
        .await
        .take_staged_sql()
        .ok_or("No pending DML for this conversation")?;

    // E6: a statement staged minutes ago and approved now may no longer
    // match the data it was previewed against. Refuse (after clearing the
    // state, so the conversation is not stuck) and ask for a re-run.
    if staged_at.elapsed() > dml_stale_after() {
        return Err(
            "The pending DML was staged more than 5 minutes ago. Ask the assistant \
             to re-run the preview and approve again."
                .into(),
        );
    }

    // Session B: the AI session when it exists (Task 2.2), else the editor's.
    let conn_id = *state.ai_connection_id.lock().await;
    let conn_id = conn_id
        .or(*state.current_connection_id.lock().await)
        .ok_or("not connected")?;
    let client = state.client_handle().await.ok_or("not connected")?;

    // The SQL came from take_staged_sql() — exactly what the user approved
    // in the DML card. No re-read: the staged value is the executed value.
    let rows_affected = execute_staged_dml(&client, conn_id, &conv, staged_sql.clone()).await?;

    log::info!("DML executed: {rows_affected} rows affected — {staged_sql}");
    let _ = app_handle.emit(
        "dml:executed",
        serde_json::json!({
            "conversation_id": conversation_id.clone(),
            "rows_affected": rows_affected,
        }),
    );

    // Resume the agent so it can confirm the outcome (C1).
    let channel = conv
        .lock()
        .await
        .event_channel
        .clone()
        .ok_or("no event channel")?;
    conv.lock().await.history.push(crate::ai::agent::Message::user(
        format!(
            "The user approved and executed the DML. Result: {rows_affected} rows affected. SQL: {staged_sql}. Confirm the result to the user."
        ),
    ));
    let conv_conn_id = conv.lock().await.connection_id.clone();
    let (system_prompt, _context_tier) = build_system_prompt(&state, &conv_conn_id).await;
    run_agent_turn(
        &state,
        &app_handle,
        channel,
        conversation_id.clone(),
        String::new(),
        system_prompt,
        // Resume-after-DML builds no memory block, so no rules were applied.
        0,
        None,
    )
    .await?;

    Ok(serde_json::json!({ "rows_affected": rows_affected, "sql": staged_sql }))
}

/// Core DML execution, split from the tauri command so integration tests can
/// drive it against the real worker. Executes the staged SQL on session B and
/// clears the query cache — data changed, cached query summaries are stale.
pub(crate) async fn execute_staged_dml(
    client: &ConnectorClient,
    conn_id: ConnectionId,
    conv: &Arc<Mutex<ConversationState>>,
    staged_sql: String,
) -> Result<u64, String> {
    let result = client
        .execute(conn_id, &staged_sql)
        .await
        .map_err(|e| format!("DML failed: {e}"))?;
    let rows_affected = result.rows_affected.unwrap_or(0);
    conv.lock().await.query_cache.clear();
    Ok(rows_affected)
}

/// Takes the bridge's single-slot DML approval (spec §3 D5). `take()` — not
/// a peek — so a second approve/reject can never double-send through the
/// oneshot, no matter how the frontend races its buttons.
pub(crate) async fn take_pending_dml(
    handle: &Arc<crate::ai::acp::bridge::BridgeHandle>,
) -> Result<crate::ai::acp::bridge::PendingDml, String> {
    handle
        .pending_dml
        .lock()
        .await
        .take()
        .ok_or_else(|| "No pending DML for this conversation".to_string())
}

/// ACP-mode DML execution: executes the staged SQL through `execute` and
/// resolves the held `preview_dml` tool call with the outcome — the agent
/// sees a slow tool call that returns data. Never touches `run_agent_turn`.
pub(crate) async fn resolve_acp_dml<F, Fut>(
    handle: &Arc<crate::ai::acp::bridge::BridgeHandle>,
    execute: F,
) -> Result<(u64, String), String>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<u64, String>>,
{
    let pending = take_pending_dml(handle).await?;
    let sql = pending.sql.clone();
    let rows_affected = execute(sql.clone()).await?;
    let _ = pending
        .tx
        .send(Ok(crate::ai::acp::bridge::DmlOutcome { rows_affected }));
    Ok((rows_affected, sql))
}

/// ACP-mode DML rejection: resolves the held tool call with an error, so the
/// agent streams an acknowledgement instead of hanging on a dead call.
pub(crate) async fn reject_acp_dml(
    handle: &Arc<crate::ai::acp::bridge::BridgeHandle>,
) -> Result<(), String> {
    reject_acp_dml_with(handle, "DML rejected by user").await
}

pub(crate) async fn reject_acp_dml_with(
    handle: &Arc<crate::ai::acp::bridge::BridgeHandle>,
    reason: &str,
) -> Result<(), String> {
    let pending = take_pending_dml(handle).await?;
    let _ = pending.tx.send(Err(reason.into()));
    Ok(())
}

/// Rejects the held ACP DML when its preview is older than `stale_after`
/// (spec D6 — parity with the rig path's E6 guard). Returns true when the
/// hold was rejected; the caller surfaces the staleness error.
pub(crate) async fn reject_stale_acp_dml(
    handle: &Arc<crate::ai::acp::bridge::BridgeHandle>,
    stale_after: std::time::Duration,
) -> bool {
    let stale = {
        let slot = handle.pending_dml.lock().await;
        slot.as_ref()
            .map(|p| p.staged_at.elapsed() > stale_after)
            .unwrap_or(false)
    };
    if stale {
        let _ = reject_acp_dml_with(
            handle,
            "DML rejected: the preview is stale (over 5 minutes old) — ask the assistant to re-run the preview and approve again.",
        )
        .await;
    }
    stale
}

/// The conversation's ACP session id (the permission FIFO is keyed by
/// session id — spec §4.5). Errors when the conversation has no live session.
pub(crate) async fn resolve_permission_session(
    state: &AppState,
    conversation_id: &str,
) -> Result<String, String> {
    state
        .acp
        .sessions
        .lock()
        .await
        .get(conversation_id)
        .map(|s| s.session_id.clone())
        .ok_or_else(|| "no active ACP session for this conversation".into())
}

/// ACP-mode DML rejection command: same branch shape as `execute_dml` — the
/// staged SQL lives in the bridge, so rejecting takes the slot and resolves
/// the held tool call with an error. The `dml:rejected` event closes the DML
/// card on the frontend. Rig mode uses `ai_cancel` (which drops the staged
/// SQL) instead.
#[tauri::command]
pub async fn reject_dml(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    conversation_id: String,
) -> Result<(), String> {
    let handle = state
        .acp
        .bridges
        .lock()
        .await
        .get(&conversation_id)
        .cloned()
        .ok_or("No pending DML for this conversation (bridge not active)")?;
    reject_acp_dml(&handle).await?;
    let _ = app_handle.emit(
        "dml:rejected",
        serde_json::json!({ "conversation_id": conversation_id }),
    );
    Ok(())
}

/// Answers the agent's `session/request_permission` for a conversation
/// (spec §4.5): `allow=true` selects the agent's allow-once option,
/// `allow=false` rejects. The agent's turn is blocked until this resolves.
#[tauri::command]
pub async fn respond_agent_permission(
    state: State<'_, AppState>,
    conversation_id: String,
    allow: bool,
) -> Result<(), String> {
    let session_id = resolve_permission_session(&state, &conversation_id).await?;
    state.acp.permissions.respond(&session_id, allow).await
}

#[tauri::command]
pub async fn get_ai_settings(state: State<'_, AppState>) -> Result<AiConfig, String> {
    Ok(state.ai_config.read().await.clone())
}

/// Returns the accumulated LLM token usage for a conversation. Unknown
/// conversations yield zeros, so the frontend can always render a line.
#[tauri::command]
pub async fn get_ai_usage(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    let usage = state
        .llm_usage
        .get(&conversation_id)
        .as_deref()
        .cloned()
        .unwrap_or_default();
    Ok(serde_json::json!(usage))
}

/// `Custom` has no default base URL (see `dispatch::default_base_url`) — an
/// empty endpoint there isn't "use the default," it's a config the agent
/// can never actually connect with. Every other provider is fine with no
/// endpoint override.
fn validate_custom_endpoint(config: &AiConfig) -> Result<(), String> {
    if config.provider == AiProvider::Custom {
        let has_endpoint = config
            .endpoint
            .as_deref()
            .map(|e| !e.trim().is_empty())
            .unwrap_or(false);
        if !has_endpoint {
            return Err("Custom provider requires an endpoint URL".to_string());
        }
    }
    Ok(())
}

/// Enforces the ACP-path invariant at the save boundary: the ACP path is
/// selected by the presence of `config.acp` alone (`pick_driver` branches on
/// it, never on `provider`) — a stale block with a non-ACP provider would
/// silently route chats to the agent. Normalize here rather than relying on
/// the frontend to null it out.
fn normalize_acp_config(mut config: AiConfig) -> AiConfig {
    if config.provider != AiProvider::Acp {
        config.acp = None;
    }
    config
}

#[tauri::command]
pub async fn save_ai_settings(
    state: State<'_, AppState>,
    config: AiConfig,
    api_key: Option<String>,
) -> Result<(), String> {
    validate_custom_endpoint(&config)?;
    let config = normalize_acp_config(config);
    if let Some(key) = api_key {
        keyring::Entry::new(KEYCHAIN_SERVICE, keychain_account(&config.provider))
            .map_err(|e| format!("Keychain error: {e}"))?
            .set_password(&key)
            .map_err(|e| format!("Failed to save key: {e}"))?;
    }
    crate::ai::config::save_config_to_disk(&config)
        .map_err(|e| format!("Failed to save config: {e}"))?;
    *state.ai_config.write().await = config;
    // Key or provider may have changed — force a fresh keychain read next message.
    *state.api_key_cache.write().await = None;
    Ok(())
}

#[tauri::command]
pub async fn list_ai_models(
    provider: AiProvider,
    api_key: Option<String>,
    endpoint: Option<String>,
) -> Result<Vec<crate::ai::providers::dispatch::ModelSummary>, String> {
    let key = match api_key {
        Some(k) if !k.is_empty() => k,
        _ => keyring::Entry::new(KEYCHAIN_SERVICE, keychain_account(&provider))
            .ok()
            .and_then(|entry| entry.get_password().ok())
            .unwrap_or_default(),
    };
    crate::ai::providers::dispatch::list_models_for(&provider, &key, &endpoint).await
}

/// Lists the ACP agent registry merged with installed state. Never fails: on
/// a fetch error it falls back to the cached registry, then the bundled
/// snapshot (see `registry::refresh_registry`), so the Settings panel always
/// has a list to render.
#[tauri::command]
pub async fn list_registry_agents(
    state: State<'_, AppState>,
) -> Result<Vec<crate::ai::acp::RegistryAgentSummary>, String> {
    let reg = crate::ai::acp::registry::refresh_registry(&state.acp_http).await?;
    Ok(crate::ai::acp::summarize(&reg, |id| {
        crate::ai::acp::install::read_installed(id).ok().flatten()
    }))
}

/// Installs a registry agent (npx/uvx launch spec or verified binary
/// download) into `~/.lucent/agents/<id>/` and returns its launch spec, which
/// the frontend surfaces as install confirmation.
#[tauri::command]
pub async fn install_acp_agent(
    state: State<'_, AppState>,
    agent_id: String,
) -> Result<crate::ai::acp::install::InstalledAgent, String> {
    let reg = crate::ai::acp::registry::refresh_registry(&state.acp_http).await?;
    let agent = reg
        .agents
        .iter()
        .find(|a| a.id == agent_id)
        .ok_or_else(|| format!("unknown agent: {agent_id}"))?;
    crate::ai::acp::install::install(agent, &state.acp_http).await
}

/// Removes an installed agent directory (binary downloads and installed.json).
/// Idempotent: a missing directory is not an error.
#[tauri::command]
pub async fn uninstall_acp_agent(agent_id: String) -> Result<(), String> {
    crate::ai::acp::install::uninstall(&agent_id)
}

/// Lists every installed ACP agent from disk, independent of the registry — an
/// agent installed via command override, or whose registry entry vanished,
/// still appears here (the provider picker depends on that).
#[tauri::command]
pub async fn list_installed_acp_agents() -> Result<Vec<InstalledAgent>, String> {
    Ok(crate::ai::acp::install::list_installed())
}

// ── AI Memory Subsystem IPC Commands ──────────────────────────────────────────

#[tauri::command]
pub async fn list_chat_conversations(
    state: State<'_, AppState>,
    connection_id: Option<String>,
) -> Result<Vec<crate::ai::memory::ChatConversation>, String> {
    state
        .memory_manager
        .list_conversations(connection_id.as_deref())
        .await
}

#[tauri::command]
pub async fn load_chat_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<crate::ai::memory::ChatMessage>, String> {
    state.memory_manager.list_messages(&conversation_id).await
}

#[tauri::command]
pub async fn save_chat_message(
    state: State<'_, AppState>,
    message: crate::ai::memory::ChatMessage,
) -> Result<(), String> {
    state.memory_manager.save_message(message).await
}

#[tauri::command]
pub async fn delete_chat_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<bool, String> {
    state
        .memory_manager
        .delete_conversation(&conversation_id)
        .await
}

#[tauri::command]
pub async fn list_memories(
    state: State<'_, AppState>,
    connection_key: String,
    include_archived: Option<bool>,
) -> Result<Vec<crate::ai::memory::MemoryItem>, String> {
    state
        .memory_manager
        .list_memories(&connection_key, include_archived.unwrap_or(false))
        .await
}

/// F (R23): raw captured observations for the Journal tab. The memory manager
/// already could list them; this exposes that through IPC.
#[tauri::command]
pub async fn list_observations(
    state: State<'_, AppState>,
    connection_key: String,
    status: String,
) -> Result<Vec<crate::ai::memory::Observation>, String> {
    state
        .memory_manager
        .list_observations(&connection_key, &status)
        .await
}

/// F (R23): the always-on profile plane for the drawer's Profile tab. Derived
/// from the same connection-scoped list, filtered to `injection == Always`.
#[tauri::command]
pub async fn list_always_memories(
    state: State<'_, AppState>,
    connection_key: String,
) -> Result<Vec<crate::ai::memory::MemoryItem>, String> {
    let all = state
        .memory_manager
        .list_memories(&connection_key, false)
        .await?;
    Ok(all
        .into_iter()
        .filter(|m| m.injection == crate::ai::memory::InjectionClass::Always)
        .collect())
}

#[tauri::command]
pub async fn save_memory_manual(
    state: State<'_, AppState>,
    connection_key: String,
    category: String,
    key_phrase: String,
    rule_text: String,
    sql_snippet: Option<String>,
    scope: Option<String>,
) -> Result<crate::ai::memory::MemoryItem, String> {
    save_memory_manual_impl(
        &state,
        connection_key,
        category,
        key_phrase,
        rule_text,
        sql_snippet,
        scope,
    )
    .await
}

/// Finding A: the manual save path is the only production producer of the
/// always-on profile plane. A `Preference` is a user's explicit standing
/// instruction, so it is saved with `injection = Always` and the key phrase as
/// its `preference_key`; every other category stays retrieved-only. The agent
/// tool path (`ai::tools::memory`) deliberately remains `Retrieved` — an agent
/// cannot promote itself. Split from the command so tests can exercise the
/// exact mapping without a Tauri `State`.
async fn save_memory_manual_impl(
    state: &AppState,
    connection_key: String,
    category: String,
    key_phrase: String,
    rule_text: String,
    sql_snippet: Option<String>,
    scope: Option<String>,
) -> Result<crate::ai::memory::MemoryItem, String> {
    use crate::ai::memory::security::{sanitize_rule_text, sanitize_sql_snippet, SourceTrust};
    use crate::ai::memory::{
        compute_memory_doc_hash, InjectionClass, MemoryCategory, MemoryItem, MemoryScope,
        MemoryStatus, Origin, MEMORY_FORMAT_VERSION, MEMORY_MODEL_NAME,
        USER_EXPLICIT_STABILITY_HOURS,
    };

    let sanitized_rule = sanitize_rule_text(&rule_text)?;
    let sanitized_sql = sanitize_sql_snippet(sql_snippet.as_deref())?;

    let cat = MemoryCategory::from_str(&category);
    let is_preference = cat == MemoryCategory::Preference;
    let preference_key = if is_preference {
        Some(key_phrase.clone())
    } else {
        None
    };
    let sc = scope
        .as_deref()
        .map(MemoryScope::from_str)
        .unwrap_or(MemoryScope::Connection);
    let now = chrono::Utc::now().timestamp();
    let id = uuid::Uuid::new_v4().to_string();

    let doc_text = format!(
        "Context: {} {} | Rule: {}",
        category, key_phrase, sanitized_rule
    );
    let doc_hash = compute_memory_doc_hash(&doc_text);

    let embedding = if let Some(emb) = state.get_or_init_memory_embedder().await {
        emb.embed_query(&doc_text).await.unwrap_or_default()
    } else {
        let emb_guard = state.embedder.lock().await;
        if let Some(emb) = emb_guard.as_ref() {
            emb.embed_query(&doc_text).await.unwrap_or_default()
        } else {
            Vec::new()
        }
    };

    // Clone the graph snapshot and release the lock immediately: entity
    // extraction is synchronous, and holding the central schema lock across the
    // `save_memory(...).await` below would stall autocomplete, the schema tree,
    // and background indexing for the duration of a SQLite write.
    let graph = state.schema_graph.lock().await.clone();
    let entity_links = if let Some(g) = graph.as_ref() {
        crate::ai::memory::entity_linker::extract_and_link_entities(
            &sanitized_rule,
            sanitized_sql.as_deref(),
            g,
        )
    } else {
        Vec::new()
    };

    let item = MemoryItem {
        id: id.clone(),
        connection_key: connection_key.clone(),
        scope: sc,
        scope_key: connection_key,
        category: cat,
        key_phrase,
        rule_text: sanitized_rule,
        sql_snippet: sanitized_sql,
        importance: 0.9,
        stability_hours: USER_EXPLICIT_STABILITY_HOURS,
        last_accessed_at: now,
        access_count: 1,
        source_trust: SourceTrust::UserExplicit,
        source_conv_id: None,
        source_turn_id: None,
        source_tool_id: None,
        status: MemoryStatus::Active,
        supersedes_id: None,
        valid_from: now,
        valid_until: None,
        learned_at: now,
        tombstone: false,
        tombstoned_at: None,
        doc_hash,
        embedding_model: MEMORY_MODEL_NAME.into(),
        embedding_version: MEMORY_FORMAT_VERSION,
        embedding,
        created_at: now,
        updated_at: now,
        injection: if is_preference {
            InjectionClass::Always
        } else {
            InjectionClass::Retrieved
        },
        preference_key,
        origin: Origin::Agent,
        steps_json: None,
        merge_group_id: None,
        confirmed: false,
        confirmation_conv_id: None,
    };

    state
        .memory_manager
        .save_memory(item.clone(), &entity_links)
        .await?;
    Ok(item)
}

#[tauri::command]
pub async fn delete_memory(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    state.memory_manager.delete_memory(&id).await
}

#[tauri::command]
pub async fn toggle_memory_status(
    state: State<'_, AppState>,
    id: String,
    status: String,
) -> Result<(), String> {
    let stat = crate::ai::memory::MemoryStatus::from_str(&status);
    state.memory_manager.toggle_memory_status(&id, stat).await
}

#[tauri::command]
pub async fn resolve_drift(
    state: State<'_, AppState>,
    id: String,
    resolution: String,
) -> Result<(), String> {
    if resolution == "revalidate" {
        let graph_guard = state.schema_graph.lock().await;
        state
            .memory_manager
            .revalidate_drift(&id, graph_guard.as_ref())
            .await
    } else {
        state
            .memory_manager
            .toggle_memory_status(&id, crate::ai::memory::MemoryStatus::Archived)
            .await
    }
}

#[tauri::command]
pub async fn export_memories_markdown(
    state: State<'_, AppState>,
    connection_key: String,
) -> Result<String, String> {
    let memories = state
        .memory_manager
        .list_memories(&connection_key, true)
        .await?;
    Ok(crate::ai::memory::rules_parser::export_to_markdown(
        &connection_key,
        &memories,
    ))
}

#[tauri::command]
pub async fn import_memories_markdown(
    state: State<'_, AppState>,
    connection_key: String,
    content: String,
) -> Result<usize, String> {
    let parsed = crate::ai::memory::rules_parser::parse_markdown_rules(&content);
    let now = chrono::Utc::now().timestamp();
    // Snapshot the graph under the lock, then drop it: the loop below embeds
    // each rule (ONNX inference plus a possible 130 MB model load) and writes
    // SQLite. Holding the central schema lock across that would stall every
    // schema consumer — same family as `build_system_prompt_with_query` (B-C1).
    let graph_snapshot = {
        let graph_guard = state.schema_graph.lock().await;
        graph_guard.clone()
    };

    let mut imported = 0usize;
    for rule in parsed {
        // B-I3 review finding: import was the one write path that bypassed
        // sanitization, so a hand-edited LUCENT.md could smuggle boundary
        // delimiters or blacklisted payloads straight into memory. Route it
        // through the same guard as every other write path; skip malformed
        // rules rather than aborting the whole import.
        let sanitized_rule = match crate::ai::memory::security::sanitize_rule_text(&rule.rule_text)
        {
            Ok(text) => text,
            Err(_) => continue,
        };
        let sanitized_sql =
            match crate::ai::memory::security::sanitize_sql_snippet(rule.sql_snippet.as_deref()) {
                Ok(sql) => sql,
                Err(_) => continue,
            };

        let id = uuid::Uuid::new_v4().to_string();
        // Finding A: an imported `preference` is a user-authored standing
        // instruction, same as a manual save, so it enters the always-on
        // profile plane with its key phrase as the preference key. All other
        // imported categories stay retrieved-only.
        let is_preference = rule.category == crate::ai::memory::MemoryCategory::Preference;
        let preference_key = if is_preference {
            Some(rule.key_phrase.clone())
        } else {
            None
        };
        let doc_hash = crate::ai::memory::compute_memory_doc_hash(&format!(
            "{} {}",
            rule.key_phrase, sanitized_rule
        ));
        let embedding = if let Some(emb) = state.get_or_init_memory_embedder().await {
            emb.embed_query(&format!("{}: {}", rule.key_phrase, sanitized_rule))
                .await
                .unwrap_or_default()
        } else {
            let emb_guard = state.embedder.lock().await;
            if let Some(emb) = emb_guard.as_ref() {
                emb.embed_query(&format!("{}: {}", rule.key_phrase, sanitized_rule))
                    .await
                    .unwrap_or_default()
            } else {
                Vec::new()
            }
        };
        let entity_links = if let Some(g) = graph_snapshot.as_ref() {
            crate::ai::memory::entity_linker::extract_and_link_entities(
                &sanitized_rule,
                sanitized_sql.as_deref(),
                g,
            )
        } else {
            Vec::new()
        };
        let item = crate::ai::memory::MemoryItem {
            id: id.clone(),
            connection_key: connection_key.clone(),
            scope: crate::ai::memory::MemoryScope::Connection,
            scope_key: connection_key.clone(),
            category: rule.category,
            key_phrase: rule.key_phrase,
            rule_text: sanitized_rule,
            sql_snippet: sanitized_sql,
            importance: rule.importance,
            stability_hours: crate::ai::memory::USER_EXPLICIT_STABILITY_HOURS,
            last_accessed_at: now,
            access_count: 1,
            source_trust: crate::ai::memory::security::SourceTrust::UserExplicit,
            source_conv_id: None,
            source_turn_id: None,
            source_tool_id: None,
            status: crate::ai::memory::MemoryStatus::Active,
            supersedes_id: None,
            valid_from: now,
            valid_until: None,
            learned_at: now,
            tombstone: false,
            tombstoned_at: None,
            doc_hash,
            embedding_model: crate::ai::memory::MEMORY_MODEL_NAME.into(),
            embedding_version: crate::ai::memory::MEMORY_FORMAT_VERSION,
            embedding,
            created_at: now,
            updated_at: now,
            injection: if is_preference {
                crate::ai::memory::InjectionClass::Always
            } else {
                crate::ai::memory::InjectionClass::Retrieved
            },
            preference_key,
            origin: crate::ai::memory::Origin::Agent,
            steps_json: None,
            merge_group_id: None,
            confirmed: false,
            confirmation_conv_id: None,
        };
        if state
            .memory_manager
            .save_memory(item, &entity_links)
            .await
            .is_ok()
        {
            imported += 1;
        }
    }

    Ok(imported)
}

#[tauri::command]
pub async fn list_golden_queries(
    state: State<'_, AppState>,
    connection_id: String,
) -> Result<Vec<crate::ai::memory::GoldenQuery>, String> {
    state
        .memory_manager
        .list_golden_queries(&connection_id)
        .await
}

#[tauri::command]
pub async fn save_golden_query(
    state: State<'_, AppState>,
    connection_id: String,
    schema_name: Option<String>,
    natural_prompt: String,
    sql_text: String,
    tables_used: Option<Vec<String>>,
    verified: Option<bool>,
) -> Result<crate::ai::memory::GoldenQuery, String> {
    let now = chrono::Utc::now().timestamp();
    let id = uuid::Uuid::new_v4().to_string();
    let embedding = if let Some(emb) = state.get_or_init_memory_embedder().await {
        emb.embed_query(&format!("{natural_prompt} {sql_text}"))
            .await
            .unwrap_or_default()
    } else {
        let emb_guard = state.embedder.lock().await;
        if let Some(emb) = emb_guard.as_ref() {
            emb.embed_query(&format!("{natural_prompt} {sql_text}"))
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    };

    let tables = tables_used
        .unwrap_or_else(|| crate::ai::memory::entity_linker::extract_tables_from_sql(&sql_text));

    let q = crate::ai::memory::GoldenQuery {
        id,
        connection_id,
        schema_name: schema_name.unwrap_or_else(|| "public".into()),
        natural_prompt,
        sql_text,
        tables_used: tables,
        verified: verified.unwrap_or(true),
        run_count: 1,
        last_run_at: now,
        embedding_model: crate::ai::memory::MEMORY_MODEL_NAME.into(),
        embedding_version: crate::ai::memory::MEMORY_FORMAT_VERSION,
        embedding,
        created_at: now,
    };
    state.memory_manager.save_golden_query(q.clone()).await?;
    Ok(q)
}

#[tauri::command]
pub async fn delete_golden_query(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    state.memory_manager.delete_golden_query(&id).await
}

#[tauri::command]
pub async fn run_consolidation(
    state: State<'_, AppState>,
    connection_key: Option<String>,
) -> Result<serde_json::Value, String> {
    let graph_guard = state.schema_graph.lock().await;
    let snapshot = graph_guard
        .as_ref()
        .map(crate::ai::schema_graph::snapshot_from_graph);

    let (pruned, archived, drift_alerts) = state
        .memory_manager
        .with_connection(|conn| {
            let p = crate::ai::memory::consolidation::prune_old_session_json(conn, 14)?;
            let a = crate::ai::memory::consolidation::soft_archive_decayed_memories(conn)?;
            // Per-connection scoping is mandatory (see cascade_schema_drift): a
            // catalog diff for one connection must not invalidate another
            // connection's memories. Without a key there is nothing to scope to,
            // so skip rather than over-invalidate.
            let alerts = match (connection_key.as_deref(), snapshot.as_ref()) {
                (Some(key), Some(snap)) => {
                    crate::ai::memory::drift::cascade_schema_drift(key, snap, conn)?
                }
                _ => Vec::new(),
            };
            Ok((p, a, alerts))
        })
        .await?;

    Ok(serde_json::json!({
        "pruned_session_json_count": pruned,
        "archived_memory_count": archived,
        "drift_alerts": drift_alerts,
    }))
}

#[cfg(test)]
mod memory_embedder_lock_scope_tests {
    use super::{build_system_prompt_with_query, AppState, MemoryEmbedderTestGate};
    use std::sync::Arc;
    use std::time::Duration;

    fn state_with_parking_gate() -> Arc<AppState> {
        let mut state = AppState::new();
        state.memory_embedder_test_gate = Some(Arc::new(MemoryEmbedderTestGate::new()));
        Arc::new(state)
    }

    fn gate(state: &AppState) -> Arc<MemoryEmbedderTestGate> {
        state
            .memory_embedder_test_gate
            .clone()
            .expect("test gate installed")
    }

    /// Bounded wait so the RED failure is a clear assertion, not a hang.
    async fn wait_for_entered(gate: &MemoryEmbedderTestGate, n: usize) {
        for _ in 0..400 {
            if gate.entered() >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!(
            "expected {n} prompt builder(s) to reach embedder init, saw {}",
            gate.entered()
        );
    }

    fn spawn_builder(
        state: Arc<AppState>,
        connection_id: &'static str,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            build_system_prompt_with_query(&state, connection_id, Some("recent invoices"), None)
                .await;
        })
    }

    /// α) While a prompt build is parked inside the memory block (embedder
    /// init), the central `schema_graph` mutex must be acquirable. Before
    /// B-C1 the guard was held from the top of the function through the whole
    /// pipeline, so `try_lock` failed here.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prompt_build_does_not_hold_schema_graph_across_memory_work() {
        let state = state_with_parking_gate();
        let gate = gate(&state);

        let builder = spawn_builder(state.clone(), "conn-alpha");
        wait_for_entered(&gate, 1).await;

        assert!(
            state.schema_graph.try_lock().is_ok(),
            "schema_graph must not be held while the memory embedder runs"
        );

        builder.abort();
    }

    /// ζ) B-I9: two concurrent prompt builds must both reach embedder
    /// initialization. Before the fix, builder A held `schema_graph` (and
    /// `memory_embedder`) for the entire model load, so builder B blocked on
    /// `schema_graph.lock()` and never reached init.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_prompt_builds_do_not_serialize_behind_model_load() {
        let state = state_with_parking_gate();
        let gate = gate(&state);

        let first = spawn_builder(state.clone(), "conn-zeta-1");
        wait_for_entered(&gate, 1).await;

        let second = spawn_builder(state.clone(), "conn-zeta-2");
        wait_for_entered(&gate, 2).await;

        first.abort();
        second.abort();
    }
}

#[cfg(test)]
mod applied_memory_count_tests {
    use super::{build_system_prompt_with_query, AppState, MemoryEmbedderTestGate};
    use crate::ai::config::AiConfig;
    use crate::ai::memory::{
        compute_memory_doc_hash, InjectionClass, MemoryCategory, MemoryItem, MemoryManager,
        MemoryScope, MemoryStatus, Origin, SourceTrust, MEMORY_FORMAT_VERSION, MEMORY_MODEL_NAME,
    };
    use std::sync::Arc;

    /// Isolated in-memory memory DB + an embedder that reports unavailable, so
    /// prompt building never loads an ONNX model.
    fn state() -> Arc<AppState> {
        let mut state = AppState::new();
        state.memory_manager = Arc::new(MemoryManager::open_in_memory().unwrap());
        state.memory_embedder_test_gate = Some(Arc::new(MemoryEmbedderTestGate::unavailable()));
        Arc::new(state)
    }

    async fn seed_rule(mgr: &MemoryManager, connection_key: &str) {
        let now = chrono::Utc::now().timestamp();
        let rule_text = "Invoices from the last 30 days are the recent invoices set";
        mgr.save_memory(
            MemoryItem {
                id: uuid::Uuid::new_v4().to_string(),
                connection_key: connection_key.into(),
                scope: MemoryScope::Connection,
                scope_key: connection_key.into(),
                category: MemoryCategory::Quirk,
                key_phrase: "recent_invoices".into(),
                rule_text: rule_text.into(),
                sql_snippet: Some("created_at > now() - interval '30 days'".into()),
                importance: 0.8,
                stability_hours: 720.0,
                last_accessed_at: now,
                access_count: 1,
                source_trust: SourceTrust::UserExplicit,
                source_conv_id: None,
                source_turn_id: None,
                source_tool_id: None,
                status: MemoryStatus::Active,
                supersedes_id: None,
                valid_from: now,
                valid_until: None,
                learned_at: now,
                tombstone: false,
                tombstoned_at: None,
                doc_hash: compute_memory_doc_hash(rule_text),
                embedding_model: MEMORY_MODEL_NAME.into(),
                embedding_version: MEMORY_FORMAT_VERSION,
                embedding: vec![0.0; 384],
                created_at: now,
                updated_at: now,
                injection: InjectionClass::Retrieved,
                preference_key: None,
                origin: Origin::Agent,
                steps_json: None,
                merge_group_id: None,
                confirmed: false,
                confirmation_conv_id: None,
            },
            &[],
        )
        .await
        .unwrap();
    }

    /// An `Always`-injected, active, global-scope rule, so `ProfileSnapshot`
    /// picks it up for any connection.
    async fn seed_profile_rule(mgr: &MemoryManager, rule_text: &str) {
        let now = chrono::Utc::now().timestamp();
        mgr.save_memory(
            MemoryItem {
                id: uuid::Uuid::new_v4().to_string(),
                connection_key: "global".into(),
                scope: MemoryScope::Global,
                scope_key: "global".into(),
                category: MemoryCategory::Preference,
                key_phrase: "profile_rule".into(),
                rule_text: rule_text.into(),
                sql_snippet: None,
                importance: 0.9,
                stability_hours: 720.0,
                last_accessed_at: now,
                access_count: 1,
                source_trust: SourceTrust::UserExplicit,
                source_conv_id: None,
                source_turn_id: None,
                source_tool_id: None,
                status: MemoryStatus::Active,
                supersedes_id: None,
                valid_from: now,
                valid_until: None,
                learned_at: now,
                tombstone: false,
                tombstoned_at: None,
                doc_hash: compute_memory_doc_hash(rule_text),
                embedding_model: MEMORY_MODEL_NAME.into(),
                embedding_version: MEMORY_FORMAT_VERSION,
                embedding: vec![0.0; 384],
                created_at: now,
                updated_at: now,
                injection: InjectionClass::Always,
                preference_key: None,
                origin: Origin::Owner,
                steps_json: None,
                merge_group_id: None,
                confirmed: false,
                confirmation_conv_id: None,
            },
            &[],
        )
        .await
        .unwrap();
    }

    /// R13: the profile block is session-frozen. A rule inserted *after* a
    /// conversation's first prompt build must not appear in later prompts for
    /// that same conversation, while a different conversation id (fresh key)
    /// sees the new rule. This fails if the get-or-insert ever rebuilds on a
    /// cache hit, and it exercises the profile-before-retrieved no-query path
    /// without touching the embedder.
    #[tokio::test]
    async fn profile_block_is_frozen_per_conversation_and_ignores_later_rules() {
        const FIRST: &str = "Always qualify timestamps with the reporting timezone";
        const SECOND: &str = "Always wrap deletes in an explicit transaction";

        let state = state();
        *state.ai_config.write().await = AiConfig::default();
        seed_profile_rule(&state.memory_manager, FIRST).await;

        // First turn for the conversation builds and stores the snapshot.
        let (first_prompt, _, _) =
            build_system_prompt_with_query(&state, "conn-freeze", None, Some("conv-freeze")).await;
        assert!(
            first_prompt.contains(FIRST),
            "first build must inject the profile rule"
        );

        // Rule learned mid-session — must not leak into the frozen snapshot.
        seed_profile_rule(&state.memory_manager, SECOND).await;

        let (second_prompt, _, _) =
            build_system_prompt_with_query(&state, "conn-freeze", None, Some("conv-freeze")).await;
        assert!(
            !second_prompt.contains(SECOND),
            "frozen snapshot must not pick up a rule learned after the first build"
        );
        assert_eq!(
            first_prompt, second_prompt,
            "the same conversation must reuse the byte-identical snapshot"
        );

        // A different conversation id has its own key and builds fresh, so it
        // sees the rule learned after the first conversation was frozen.
        let (other_prompt, _, _) =
            build_system_prompt_with_query(&state, "conn-freeze", None, Some("conv-other")).await;
        assert!(
            other_prompt.contains(SECOND),
            "a fresh conversation key must see the newly learned rule"
        );
    }

    /// F-C2 producer: the count comes from the retrieval that feeds the
    /// prompt — `1` for one retrieved rule, `0` for no query or disabled
    /// memory. `run_agent_turn` then threads this into the `Done` event.
    #[tokio::test]
    async fn system_prompt_reports_applied_memory_count() {
        let state = state();
        *state.ai_config.write().await = AiConfig::default();
        seed_rule(&state.memory_manager, "conn-fc2").await;

        let (_prompt, _tier, count) =
            build_system_prompt_with_query(&state, "conn-fc2", Some("recent invoices"), None).await;
        assert_eq!(count, 1, "one matching rule retrieved → one applied");

        let (_prompt, _tier, count) =
            build_system_prompt_with_query(&state, "conn-fc2", None, None).await;
        assert_eq!(count, 0, "no query → no retrieval");

        state.ai_config.write().await.enable_ai_memory = false;
        let (_prompt, _tier, count) =
            build_system_prompt_with_query(&state, "conn-fc2", Some("recent invoices"), None).await;
        assert_eq!(count, 0, "disabled memory applies no rules");
    }
}

#[cfg(test)]
mod api_key_cache_tests {
    use super::cached_api_key;
    use crate::ai::config::AiProvider;

    #[test]
    fn hit_when_provider_matches() {
        let cache = Some((AiProvider::OpenAI, "sk-abc".to_string()));
        assert_eq!(
            cached_api_key(&cache, &AiProvider::OpenAI),
            Some("sk-abc".to_string())
        );
    }

    #[test]
    fn miss_when_provider_differs() {
        let cache = Some((AiProvider::OpenAI, "sk-abc".to_string()));
        assert_eq!(
            cached_api_key(&cache, &AiProvider::Ollama),
            None,
            "switching providers must not reuse the other provider's key"
        );
    }

    #[test]
    fn miss_when_empty() {
        assert_eq!(cached_api_key(&None, &AiProvider::OpenAI), None);
    }
}

#[cfg(test)]
mod acp_config_normalization_tests {
    use super::normalize_acp_config;
    use crate::ai::config::{AcpAgentConfig, AiConfig, AiProvider};
    use std::collections::HashMap;

    fn config_with_acp(provider: AiProvider) -> AiConfig {
        AiConfig {
            provider,
            acp: Some(AcpAgentConfig {
                agent_id: "opencode".to_string(),
                command: None,
                env: HashMap::new(),
                auto_deny_permissions: false,
            }),
            ..AiConfig::default()
        }
    }

    #[test]
    fn drops_a_stale_acp_block_for_a_non_acp_provider() {
        let cfg = config_with_acp(AiProvider::OpenAI);
        let normalized = normalize_acp_config(cfg);
        assert!(
            normalized.acp.is_none(),
            "a stale acp block with a non-ACP provider must be dropped"
        );
        assert_eq!(normalized.provider, AiProvider::OpenAI);
    }

    #[test]
    fn keeps_the_acp_block_for_the_acp_provider() {
        let cfg = config_with_acp(AiProvider::Acp);
        let normalized = normalize_acp_config(cfg);
        assert!(normalized.acp.is_some(), "acp block must survive for acp");
    }

    #[test]
    fn leaves_a_config_without_acp_untouched() {
        let cfg = AiConfig {
            provider: AiProvider::Anthropic,
            ..AiConfig::default()
        };
        let normalized = normalize_acp_config(cfg);
        assert!(normalized.acp.is_none());
    }
}

#[cfg(test)]
mod custom_endpoint_validation_tests {
    use super::validate_custom_endpoint;
    use crate::ai::config::{AiConfig, AiProvider};

    fn config_with(provider: AiProvider, endpoint: Option<&str>) -> AiConfig {
        AiConfig {
            provider,
            endpoint: endpoint.map(str::to_string),
            ..AiConfig::default()
        }
    }

    #[test]
    fn rejects_custom_with_no_endpoint() {
        let cfg = config_with(AiProvider::Custom, None);
        assert!(validate_custom_endpoint(&cfg).is_err());
    }

    #[test]
    fn rejects_custom_with_blank_endpoint() {
        let cfg = config_with(AiProvider::Custom, Some("   "));
        assert!(validate_custom_endpoint(&cfg).is_err());
    }

    #[test]
    fn accepts_custom_with_endpoint() {
        let cfg = config_with(AiProvider::Custom, Some("http://localhost:8080/v1"));
        assert!(validate_custom_endpoint(&cfg).is_ok());
    }

    #[test]
    fn accepts_non_custom_with_no_endpoint() {
        let cfg = config_with(AiProvider::OpenAI, None);
        assert!(validate_custom_endpoint(&cfg).is_ok());
    }
}

#[cfg(test)]
mod usage_tests {
    use super::accumulate_usage;
    use crate::ai::events::TokenUsage;

    #[test]
    fn sums_token_fields_across_runs() {
        let existing = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 20,
            cached_prompt_tokens: 30,
        };
        let new = TokenUsage {
            prompt_tokens: 50,
            completion_tokens: 10,
            cached_prompt_tokens: 5,
        };
        let acc = accumulate_usage(&existing, &new);
        assert_eq!(acc.prompt_tokens, 150);
        assert_eq!(acc.completion_tokens, 30);
        assert_eq!(acc.cached_prompt_tokens, 35);
    }

    #[test]
    fn accumulating_into_empty_totals_is_the_new_run() {
        let new = TokenUsage {
            prompt_tokens: 42,
            completion_tokens: 7,
            cached_prompt_tokens: 3,
        };
        let acc = accumulate_usage(&TokenUsage::default(), &new);
        assert_eq!(acc.prompt_tokens, 42);
        assert_eq!(acc.completion_tokens, 7);
        assert_eq!(acc.cached_prompt_tokens, 3);
    }
}

#[cfg(test)]
mod conversation_lifecycle_tests {
    use super::{evict_conversation, AppState};
    use crate::ai::agent::ConversationState;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// Regression test for the leak where closing a chat tab never told the
    /// backend to drop its `ConversationState` — `conversations` grew for
    /// the life of the process. Exercises the same helper `close_conversation`
    /// calls, so it fails if that wiring is ever removed or short-circuited.
    #[test]
    fn closing_a_conversation_removes_it_from_app_state() {
        let state = AppState::new();
        let conversation_id = "conv-1".to_string();
        state.conversations.insert(
            conversation_id.clone(),
            Arc::new(Mutex::new(ConversationState::new("conn-1".into()))),
        );
        assert!(state.conversations.contains_key(&conversation_id));

        evict_conversation(&state, &conversation_id);

        assert!(
            !state.conversations.contains_key(&conversation_id),
            "conversation state must be evicted on close, not retained forever"
        );
    }

    #[test]
    fn closing_a_conversation_evicts_its_usage_totals() {
        let state = AppState::new();
        let conversation_id = "conv-1".to_string();
        state.llm_usage.insert(
            conversation_id.clone(),
            crate::ai::events::TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                cached_prompt_tokens: 2,
            },
        );

        evict_conversation(&state, &conversation_id);

        assert!(
            !state.llm_usage.contains_key(&conversation_id),
            "usage totals must be evicted on close, not retained forever"
        );
    }

    #[test]
    fn evicting_an_unknown_conversation_is_a_no_op() {
        let state = AppState::new();
        // Must not panic on double-close or a stale/already-evicted id.
        evict_conversation(&state, "does-not-exist");
        assert!(state.conversations.is_empty());
        assert!(state.llm_usage.is_empty());
    }
}

#[cfg(test)]
mod catalog_mapping_tests {
    use lucent_protocol::{Namespace, ObjectKind, ObjectRef, ObjectSummary};

    use super::{namespaces_to_schema_info, summaries_to_schema_objects};

    fn summary(name: &str, kind: ObjectKind, est: Option<u64>) -> ObjectSummary {
        ObjectSummary {
            reference: ObjectRef {
                namespace: vec!["app".into()],
                name: name.into(),
                kind,
            },
            est_rows: est,
            comment: None,
            partition: None,
            is_partition_child: false,
        }
    }

    #[test]
    fn a_single_segment_namespace_keeps_todays_flat_schema_name() {
        let info = namespaces_to_schema_info(vec![Namespace {
            path: vec!["public".into()],
            object_count: Some(7),
        }]);
        assert_eq!(info[0].name, "public");
        assert_eq!(info[0].object_count, 7);
    }

    #[test]
    fn an_unknown_object_count_renders_as_zero_not_as_a_crash() {
        // The frontend field is a plain i64. Until it learns about unknown,
        // map None to 0 here — deliberately, in one place.
        let info = namespaces_to_schema_info(vec![Namespace {
            path: vec!["public".into()],
            object_count: None,
        }]);
        assert_eq!(info[0].object_count, 0);
    }

    #[test]
    fn object_kinds_map_to_the_strings_the_sidebar_already_expects() {
        let objects = summaries_to_schema_objects(vec![
            summary("users", ObjectKind::Table, Some(10)),
            summary("recent", ObjectKind::View, None),
            summary("calc", ObjectKind::Function, None),
            summary("counter", ObjectKind::Sequence, None),
        ]);
        let kinds: Vec<&str> = objects.iter().map(|o| o.kind.as_str()).collect();
        assert_eq!(kinds, vec!["table", "view", "function", "sequence"]);
        assert_eq!(objects[0].row_count, Some(10));
        assert_eq!(objects[1].row_count, None);
    }

    #[test]
    fn partition_children_are_hidden_from_the_sidebar() {
        // 84 partitions of one table would bury every other object in the tree.
        let child = ObjectSummary {
            is_partition_child: true,
            ..summary("events_2026", ObjectKind::Table, Some(5))
        };
        let objects =
            summaries_to_schema_objects(vec![summary("events", ObjectKind::Table, None), child]);
        let names: Vec<&str> = objects.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(names, vec!["events"]);
    }
}

#[cfg(test)]
mod password_cache_tests {
    use super::{cached_password, AppState};

    #[tokio::test]
    async fn a_cached_secret_is_returned_without_a_keychain_read() {
        // Cache-first: once a secret is in memory, no keychain access happens
        // (which is what eliminates the repeated macOS password prompts).
        // Seeding the cache with a sentinel proves the hit path never falls
        // through to get_password — if it did, this test would read (or fail
        // on) the real keychain instead of returning the sentinel.
        let state = AppState::new();
        state
            .password_cache
            .write()
            .await
            .insert("prof-1".to_string(), "sentinel-secret".to_string());

        let secret = cached_password(&state, "prof-1")
            .await
            .expect("cache hit must succeed");
        assert_eq!(secret, "sentinel-secret");
    }

    #[tokio::test]
    async fn a_missing_profile_is_a_not_found_not_a_panic() {
        // A profile that has never been saved has no keychain item (or one we
        // cannot read) — callers treat NotFound as "no secret for this driver",
        // never as an error. This must not panic or hang on the cache path.
        let state = AppState::new();
        let result = cached_password(&state, "no-such-profile-xyz").await;
        assert!(
            result.is_err(),
            "an unknown profile must not produce a secret"
        );
        // The cache must not have been polluted by the miss.
        assert!(state.password_cache.read().await.is_empty());
    }
}

#[cfg(test)]
mod path_security_tests {
    #[tokio::test]
    async fn test_export_path_rejects_traversal() {
        let dir = std::env::temp_dir().join(format!("lucent-path-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // temp_dir may be a symlink (/var -> /private/var on macOS); canonicalize
        // so the approved set holds the same form validate_approved produces.
        let dir = std::fs::canonicalize(&dir).unwrap();
        let approved: std::collections::HashSet<std::path::PathBuf> =
            std::collections::HashSet::from([dir.join("ok.csv")]);

        // ../../ escape
        let evil = dir.join("..").join("..").join(".zshrc");
        assert!(super::validate_approved(&evil, &approved).is_err());
        // absolute escape
        let absolute = std::env::temp_dir().join("lucent-escape.csv");
        assert!(super::validate_approved(&absolute, &approved).is_err());
        // symlink escape: approved path is a symlink pointing outside
        #[cfg(unix)]
        {
            let outside =
                std::env::temp_dir().join(format!("lucent-outside-{}", std::process::id()));
            std::fs::write(&outside, b"x").unwrap();
            let link = dir.join("link.csv");
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(super::validate_approved(&link, &approved).is_err());
            let _ = std::fs::remove_file(&outside);
        }
        // approved in-bounds path accepted
        assert!(super::validate_approved(&dir.join("ok.csv"), &approved).is_ok());

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
#[path = "tests/save_sql_file_test.rs"]
mod save_sql_file_tests;

/// D1 seam tests: `pick_driver` branches on `config.acp` — the ACP path
/// constructs the `AcpChatDriver` (no provider), the rig path constructs the
/// `DatabaseAgent` with the provider it was given.
#[cfg(test)]
mod acp_driver_branch_tests {
    use super::*;
    use crate::ai::acp::AcpState;
    use crate::ai::config::AcpAgentConfig;
    use crate::ai::provider::LlmProvider;
    use async_trait::async_trait;

    /// Minimal tool context: no DB, no schema graph, no embedder — the
    /// branch test never runs a turn, only constructs drivers.
    fn tool_ctx() -> AiToolContext {
        AiToolContext {
            db: Arc::new(Mutex::new(None)),
            connection_id: None,
            memory_connection_key: None,
            capabilities: None,
            config: AiConfig::default(),
            schema_graph: Arc::new(Mutex::new(None)),
            embedder: Arc::new(Mutex::new(None)),
            reranker: Arc::new(Mutex::new(None)),
            memory_manager: crate::ai::tools::test_memory_manager(),
        }
    }

    struct FakeProvider;

    #[async_trait]
    impl LlmProvider for FakeProvider {
        async fn build_agent(
            &self,
            _model: &str,
            _preamble: String,
            _max_tokens: u32,
            _tools: Vec<crate::ai::tools::LucentToolEnum>,
        ) -> Box<dyn crate::ai::provider::LucentAgent> {
            unreachable!("the branch test never chats")
        }
    }

    #[test]
    fn driver_branch_selects_acp_when_configured() {
        let acp = Some(AcpAgentConfig {
            agent_id: "stub".into(),
            command: Some("stub-binary".into()),
            env: HashMap::new(),
            auto_deny_permissions: false,
        });
        let driver: Box<dyn AgentDriver> =
            pick_driver(&acp, None, Vec::new(), tool_ctx(), AcpState::new());
        assert!(
            driver
                .as_any()
                .is::<crate::ai::acp::driver::AcpChatDriver>(),
            "acp configured → AcpChatDriver"
        );
    }

    #[test]
    fn driver_branch_keeps_rig_when_unconfigured() {
        let driver: Box<dyn AgentDriver> = pick_driver(
            &None,
            Some(Arc::new(FakeProvider)),
            Vec::new(),
            tool_ctx(),
            AcpState::new(),
        );
        assert!(
            driver.as_any().is::<DatabaseAgent>(),
            "no acp → DatabaseAgent"
        );
    }
}

/// D4 branch tests: the ACP `execute_dml` / `reject_dml` resolution resolves
/// the bridge's held `preview_dml` tool call through its oneshot and never
/// touches `run_agent_turn`'s machinery (no conversation CAS).
#[cfg(test)]
mod acp_dml_branch_tests {
    use super::*;
    use crate::ai::acp::bridge::{BridgeHandle, PendingDml};
    use crate::ai::acp::permissions::PermissionPending;
    use crate::ai::acp::SessionEntry;
    use agent_client_protocol::schema::v1::{PermissionOptionId, RequestPermissionOutcome};

    #[tokio::test]
    async fn execute_dml_acp_branch_resolves_bridge_oneshot() {
        let handle = Arc::new(BridgeHandle::new("conv-1"));
        let (tx, rx) = tokio::sync::oneshot::channel();
        handle.pending_dml.lock().await.replace(PendingDml {
            sql: "UPDATE t SET a = 1".into(),
            tx,
            staged_at: std::time::Instant::now(),
        });
        let conv = Arc::new(Mutex::new(ConversationState::new("conn-1".into())));

        let (rows, sql) = resolve_acp_dml(&handle, |sql| async move {
            assert_eq!(sql, "UPDATE t SET a = 1");
            Ok(2u64)
        })
        .await
        .expect("resolve succeeds");
        assert_eq!(rows, 2);
        assert_eq!(sql, "UPDATE t SET a = 1");

        let outcome = rx.await.expect("oneshot fires").expect("Ok outcome");
        assert_eq!(outcome.rows_affected, 2);

        // take() already removed the slot — a second approval cannot
        // double-send.
        assert!(
            resolve_acp_dml(&handle, |_| async move { Ok(0u64) })
                .await
                .is_err(),
            "second resolution finds no pending slot"
        );
        // The ACP branch never touched the conversation CAS — the state is
        // whatever it was (Idle here), and no claim was attempted.
        assert!(
            matches!(conv.lock().await.state, AgentState::Idle),
            "no conversation CAS on the ACP path"
        );
    }

    #[tokio::test]
    async fn reject_dml_resolves_oneshot_with_error() {
        let handle = Arc::new(BridgeHandle::new("conv-1"));
        let (tx, rx) = tokio::sync::oneshot::channel();
        handle.pending_dml.lock().await.replace(PendingDml {
            sql: "UPDATE t SET a = 1".into(),
            tx,
            staged_at: std::time::Instant::now(),
        });

        reject_acp_dml(&handle).await.expect("reject succeeds");
        let err = rx
            .await
            .expect("oneshot fires")
            .expect_err("rejection carries the error to the agent");
        assert!(err.contains("rejected"), "{err}");

        assert!(
            reject_acp_dml(&handle).await.is_err(),
            "nothing left to reject after take()"
        );
    }

    #[tokio::test]
    async fn stale_acp_dml_is_rejected_before_execution() {
        let handle = Arc::new(crate::ai::acp::bridge::BridgeHandle::new("conv-1"));
        let (tx, _rx) = tokio::sync::oneshot::channel();
        // Staged ten minutes ago.
        *handle.pending_dml.lock().await = Some(crate::ai::acp::bridge::PendingDml {
            sql: "insert into t values (1)".into(),
            tx,
            staged_at: std::time::Instant::now() - std::time::Duration::from_secs(600),
        });
        let rejected = reject_stale_acp_dml(&handle, std::time::Duration::from_secs(300)).await;
        assert!(rejected, "a stale hold is rejected");
        assert!(
            handle.pending_dml.lock().await.is_none(),
            "the slot clears after the stale rejection"
        );
    }

    #[tokio::test]
    async fn fresh_acp_dml_is_not_rejected() {
        let handle = Arc::new(crate::ai::acp::bridge::BridgeHandle::new("conv-1"));
        let (tx, _rx) = tokio::sync::oneshot::channel();
        *handle.pending_dml.lock().await = Some(crate::ai::acp::bridge::PendingDml {
            sql: "insert into t values (1)".into(),
            tx,
            staged_at: std::time::Instant::now(),
        });
        let rejected = reject_stale_acp_dml(&handle, std::time::Duration::from_secs(300)).await;
        assert!(!rejected, "a fresh hold executes");
        assert!(
            handle.pending_dml.lock().await.is_some(),
            "the hold stays for the user's decision"
        );
    }

    #[tokio::test]
    async fn respond_agent_permission_resolves_via_conversation_session() {
        let state = AppState::new();
        let entry = Arc::new(SessionEntry {
            agent_id: "stub".into(),
            generation: 0,
            session_id: "s1".into(),
            bridge: Arc::new(BridgeHandle::new("conv-1")),
            tools: Arc::new(crate::ai::acp::bridge::BridgeConnection::default()),
            first_prompt: std::sync::atomic::AtomicBool::new(false),
            tools_notice: std::sync::atomic::AtomicBool::new(false),
            correlator: Arc::new(crate::ai::acp::correlator::CorrelatorState::default()),
            _endpoint_dir: None,
        });
        state
            .acp
            .sessions
            .lock()
            .await
            .insert("conv-1".into(), entry);

        let sid = resolve_permission_session(&state, "conv-1")
            .await
            .expect("conversation → session");
        assert_eq!(sid, "s1");
        let err = resolve_permission_session(&state, "conv-9")
            .await
            .expect_err("unknown conversation");
        assert!(err.contains("no active ACP session"), "{err}");

        // Full round-trip: park a pending decision, resolve it allow=true
        // through the same registry `respond_agent_permission` touches.
        let (tx, rx) = tokio::sync::oneshot::channel();
        state
            .acp
            .permissions
            .push(
                "s1",
                PermissionPending {
                    tx,
                    allow_option_id: Some(PermissionOptionId::new("allow_once")),
                },
            )
            .await;
        state
            .acp
            .permissions
            .respond("s1", true)
            .await
            .expect("allow resolves");
        let outcome = rx.await.expect("oneshot fires");
        assert!(
            matches!(outcome, RequestPermissionOutcome::Selected(_)),
            "allow selects the agent's option: {outcome:?}"
        );
    }
}

#[cfg(test)]
mod memory_connection_key_tests {
    use super::memory_connection_key_for;
    use lucent_protocol::ConnectionConfig;

    #[test]
    fn profile_id_wins_when_present() {
        let cfg = ConnectionConfig::new("postgres")
            .with("host", "db.internal")
            .with("port", "5432")
            .with("database", "analytics");
        assert_eq!(
            memory_connection_key_for(Some("profile-123"), &cfg),
            "profile-123",
            "a profile connect keys memories by the profile id"
        );
    }

    #[test]
    fn inline_key_matches_the_frontends_host_port_database_shape() {
        let cfg = ConnectionConfig::new("postgres")
            .with("host", "localhost")
            .with("port", "5432")
            .with("database", "analytics");
        assert_eq!(
            memory_connection_key_for(None, &cfg),
            "localhost:5432/analytics",
            "inline connects key memories by host:port/database (App.svelte)"
        );
    }
}

#[cfg(test)]
mod observer_seam_tests {
    use super::AppState;
    use crate::ai::memory::{record_turn_observations, Origin, TurnOutcome, TurnRuntime};

    #[tokio::test]
    async fn test_turn_completion_records_explicit_request_observation() {
        let state = AppState::new();
        let turn = TurnOutcome {
            connection_key: "conn-1".into(),
            conversation_id: "conv-1".into(),
            turn_id: "turn-1".into(),
            runtime: TurnRuntime::Rig,
            user_text: Some("Remember that customers uses uuid string ids".into()),
            assistant_text: Some("Understood.".into()),
            executed_sql: vec![],
            tool_calls: vec![],
        };

        record_turn_observations(&state, turn).await;

        let obs = state
            .memory_manager
            .list_observations("conn-1", "open")
            .await
            .unwrap();
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].signal, "explicit_request");
        assert_eq!(obs[0].origin, Origin::Owner);
    }
}

#[cfg(test)]
mod manual_memory_injection_tests {
    use super::{save_memory_manual_impl, AppState, MemoryEmbedderTestGate};
    use crate::ai::memory::{InjectionClass, ProfileSnapshot};
    use std::sync::Arc;
    use tauri::Manager;

    /// Isolated in-memory memory DB + an embedder that reports unavailable, so
    /// these tests never touch the real `memory.db` or load an ONNX model.
    fn state() -> AppState {
        let mut state = AppState::new();
        state.memory_manager =
            Arc::new(crate::ai::memory::MemoryManager::open_in_memory().unwrap());
        state.memory_embedder_test_gate = Some(Arc::new(MemoryEmbedderTestGate::unavailable()));
        state
    }

    /// Finding A: a Preference saved through the manual path is the production
    /// producer for the always-on profile plane. It must come back `Always`,
    /// carry a preference key, and be picked up by `ProfileSnapshot::build`.
    #[tokio::test]
    async fn manual_preference_becomes_always_and_reaches_profile() {
        let state = state();
        let item = save_memory_manual_impl(
            &state,
            "conn-a".into(),
            "preference".into(),
            "sql_style".into(),
            "Always use CTEs over correlated subqueries".into(),
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(item.injection, InjectionClass::Always);
        assert_eq!(item.preference_key.as_deref(), Some("sql_style"));

        let snapshot = ProfileSnapshot::build(&state.memory_manager, "conn-a", 250)
            .await
            .unwrap();
        assert_eq!(
            snapshot.memories.len(),
            1,
            "the manual preference must enter the always-on profile"
        );
        assert_eq!(snapshot.memories[0].id, item.id);
    }

    /// Finding A: every non-preference category keeps the retrieved-only
    /// default and no preference key.
    #[tokio::test]
    async fn manual_non_preference_stays_retrieved() {
        let state = state();
        let item = save_memory_manual_impl(
            &state,
            "conn-a".into(),
            "quirk".into(),
            "orders_soft_delete".into(),
            "orders uses deleted_at IS NULL".into(),
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(item.injection, InjectionClass::Retrieved);
        assert_eq!(item.preference_key, None);
    }

    /// Finding F: the registered command returns the observation that
    /// `MemoryManager` recorded. Driven through a real mock `App`/`State` so
    /// the command signature itself is exercised.
    #[tokio::test]
    async fn list_observations_command_returns_recorded_observation() {
        let state = state();
        let obs = crate::ai::memory::Observation::new(
            "conn-1".into(),
            None,
            None,
            "request".into(),
            crate::ai::memory::Origin::Owner,
            "explicit_request".into(),
            1.0,
            "{}".into(),
        );
        state.memory_manager.record_observation(obs).await.unwrap();

        let app = tauri::test::mock_app();
        app.manage(state);
        let state = app.state::<AppState>();

        let listed = super::list_observations(state, "conn-1".into(), "open".into())
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].signal, "explicit_request");
        assert_eq!(listed[0].origin, crate::ai::memory::Origin::Owner);
    }

    #[tokio::test]
    async fn save_and_load_chat_message_commands_persist_full_turn_session() {
        let state = state();
        let app = tauri::test::mock_app();
        app.manage(state);
        let app_state = app.state::<AppState>();

        // Seed conversation
        app_state
            .memory_manager
            .save_conversation(crate::ai::memory::ChatConversation {
                id: "conv-test".into(),
                connection_id: "conn-test".into(),
                title: "Test".into(),
                archived: false,
                created_at: 1000,
                updated_at: 1000,
            })
            .await
            .unwrap();

        let session_json = serde_json::json!({
            "segments": [
                {
                    "type": "thinking",
                    "content": "Analyzing tables...",
                    "streaming": false,
                    "startedAt": 1000,
                    "durationMs": 250
                },
                {
                    "type": "tool_call",
                    "call": {
                        "id": "call-1",
                        "name": "run_readonly_query",
                        "args": {"sql": "SELECT 1"},
                        "summary": "1 row",
                        "status": "completed"
                    }
                }
            ],
            "startedAt": 1000,
            "durationMs": 500,
            "active": false
        })
        .to_string();

        let msg = crate::ai::memory::ChatMessage {
            id: "msg-asst-1".into(),
            conversation_id: "conv-test".into(),
            role: "assistant".into(),
            content: "Here is the result: 1".into(),
            session_json: Some(session_json),
            created_at: 1000,
        };

        super::save_chat_message(app_state.clone(), msg)
            .await
            .unwrap();

        let loaded = super::load_chat_conversation(app_state, "conv-test".into())
            .await
            .unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "msg-asst-1");
        assert_eq!(loaded[0].role, "assistant");
        assert_eq!(loaded[0].content, "Here is the result: 1");
        assert!(loaded[0].session_json.is_some());

        let session_val: serde_json::Value =
            serde_json::from_str(loaded[0].session_json.as_ref().unwrap()).unwrap();
        assert_eq!(session_val["segments"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn tauri_sink_accumulates_events_and_persists_full_turn_session() {
        use crate::ai::agent::AgentSink;
        use crate::ai::events::{AiEvent, TokenUsage, ToolCallInfo, ToolResultStatus};

        let state = state();
        let app = tauri::test::mock_app();
        app.manage(state);
        let app_state = app.state::<AppState>();

        let channel = tauri::ipc::Channel::new(|_| Ok(()));
        let conv_id = "conv-sink-test".to_string();
        let assistant_msg_id = "asst-msg-123".to_string();

        let sink = super::TauriSink {
            channel,
            app_handle: app.handle().clone(),
            conversation_id: conv_id.clone(),
            assistant_message_id: Some(assistant_msg_id.clone()),
            turn_text: std::sync::Arc::new(std::sync::Mutex::new(String::new())),
            turn_segments: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            turn_start_ms: chrono::Utc::now().timestamp_millis(),
        };

        // 1. Thinking delta
        sink.event(AiEvent::Thinking {
            content: "Checking database schema...".into(),
        });

        // 2. Tool calls
        sink.event(AiEvent::ToolCalls {
            tools: vec![ToolCallInfo {
                id: "call-query-1".into(),
                name: "run_readonly_query".into(),
                args: serde_json::json!({ "sql": "SELECT COUNT(*) FROM users" }),
            }],
        });

        // 3. Tool result
        sink.event(AiEvent::ToolResult {
            id: "call-query-1".into(),
            tool: "run_readonly_query".into(),
            summary: "1 row".into(),
            output: Some(serde_json::json!({
                "type": "query_result",
                "columns": [{"name": "count", "type": "int"}],
                "rows": [["42"]],
                "row_count": 1
            })),
            input: Some(serde_json::json!({ "sql": "SELECT COUNT(*) FROM users" })),
            status: ToolResultStatus::Completed,
        });

        // 4. Streamed text
        sink.event(AiEvent::Text {
            content: "There are 42 users registered.".into(),
        });

        // 5. Done event
        sink.event(AiEvent::Done {
            conversation_id: "legacy-conn-key".into(), // Deliberately mismatching to verify TauriSink uses its own conversation_id
            final_message: "".into(), // empty final_message falls back to accumulated streamed_text
            usage: TokenUsage {
                prompt_tokens: 150,
                completion_tokens: 45,
                cached_prompt_tokens: 20,
            },
            applied_memory_count: 2,
            cancelled: false,
        });

        // Give the spawned task a moment to write to SQLite
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let loaded = super::load_chat_conversation(app_state.clone(), conv_id.clone())
            .await
            .unwrap();
        assert_eq!(loaded.len(), 1, "Assistant message must be loaded");
        assert_eq!(loaded[0].id, assistant_msg_id);
        assert_eq!(loaded[0].conversation_id, conv_id);
        assert_eq!(loaded[0].role, "assistant");
        assert_eq!(loaded[0].content, "There are 42 users registered.");

        assert!(loaded[0].session_json.is_some());
        let session: serde_json::Value =
            serde_json::from_str(loaded[0].session_json.as_ref().unwrap()).unwrap();
        let segments = session["segments"].as_array().unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0]["type"], "thinking");
        assert_eq!(segments[0]["content"], "Checking database schema...");
        assert_eq!(segments[0]["streaming"], false);

        assert_eq!(segments[1]["type"], "tool_call");
        assert_eq!(segments[1]["call"]["name"], "run_readonly_query");
        assert_eq!(segments[1]["call"]["status"], "completed");
        assert_eq!(segments[1]["call"]["summary"], "1 row");
        assert_eq!(segments[1]["call"]["output"]["rows"][0][0], "42");

        // Verify usage was keyed by conv_id
        let usage = super::get_ai_usage(app_state, conv_id).await.unwrap();
        assert_eq!(usage["prompt_tokens"], 150);
        assert_eq!(usage["completion_tokens"], 45);
    }
}
