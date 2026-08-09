use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use lucent_protocol::{ConnectionConfig, ConnectionId, QueryId};
use serde::Serialize;
use tauri::{Emitter, Manager, State};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::ai::agent::{AgentSink, AgentState, ConversationState, DatabaseAgent};
use crate::ai::config::{keychain_account, AiConfig, KEYCHAIN_SERVICE};
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

/// Tauri-side sink bridging the agent loop to IPC events.
pub(crate) struct TauriSink {
    channel: tauri::ipc::Channel<crate::ai::events::AiEvent>,
    app_handle: tauri::AppHandle,
}

impl AgentSink for TauriSink {
    fn event(&self, event: crate::ai::events::AiEvent) {
        // Accumulate before forwarding so a frontend fetch racing the `done`
        // delivery (fire-and-forget `get_ai_usage`) never sees stale totals.
        if let crate::ai::events::AiEvent::Done {
            conversation_id,
            usage,
            ..
        } = &event
        {
            let state = self.app_handle.state::<AppState>();
            let mut entry = state.llm_usage.entry(conversation_id.clone()).or_default();
            let accumulated = accumulate_usage(&entry, usage);
            *entry = accumulated;
        }
        let _ = self.channel.send(event);
    }
    fn dml_approval(&self, payload: crate::ai::events::DmlApprovalPayload) {
        let _ = self.app_handle.emit("ai:dml_approval", payload);
    }
}

/// Pure accumulation of one run's usage into a conversation's totals. Cost is
/// per-run (one model response), so the Option values sum when both present.
pub(crate) fn accumulate_usage(existing: &TokenUsage, new: &TokenUsage) -> TokenUsage {
    TokenUsage {
        prompt_tokens: existing.prompt_tokens.saturating_add(new.prompt_tokens),
        completion_tokens: existing
            .completion_tokens
            .saturating_add(new.completion_tokens),
        cached_prompt_tokens: existing
            .cached_prompt_tokens
            .saturating_add(new.cached_prompt_tokens),
        estimated_cost_usd: match (existing.estimated_cost_usd, new.estimated_cost_usd) {
            (Some(a), Some(b)) => Some(a + b),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        },
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
    pub current_connection_id: Mutex<Option<ConnectionId>>,
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
    // AI module state
    pub conversations: DashMap<String, Arc<Mutex<ConversationState>>>,
    pub ai_config: Arc<RwLock<AiConfig>>,
    pub schema_cache: Arc<SchemaCache>,
    pub schema_graph: Arc<Mutex<Option<crate::ai::schema_graph::SchemaGraph>>>,
    pub embedder: Arc<Mutex<Option<crate::ai::embed::Embedder>>>,
    pub reranker: Arc<Mutex<Option<crate::ai::rerank::Reranker>>>,
    pub notebook_sessions: DashMap<String, crate::notebook::session::NotebookSession>,
    /// Ring buffer of worker stderr lines for the in-app Logs drawer: the
    /// supervisor's drain task appends; `get_logs` tails from the frontend.
    pub logs: crate::supervisor::LogBuffer,
    /// Per-conversation accumulated LLM token usage, keyed by conversation id.
    /// Fed by `TauriSink` on every `AiEvent::Done`; read by `get_ai_usage`.
    pub llm_usage: DashMap<String, TokenUsage>,
    /// Capabilities of the connected driver. `None` when disconnected.
    /// Phase 2 moves this onto `LiveConnection`; `capabilities()` is the seam
    /// that keeps that a small change.
    pub driver_capabilities: Mutex<Option<lucent_protocol::DriverCapabilities>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let ai_config = crate::ai::config::load_config_from_disk();
        Self {
            repo: Arc::new(ConnectionProfileRepository::load()),
            supervisor: Mutex::new(None),
            client: Arc::new(Mutex::new(None)),
            current_connection_id: Mutex::new(None),
            ai_connection_id: Mutex::new(None),
            editor_query: Mutex::new(None),
            current_database: Mutex::new(None),
            current_connection_config: Mutex::new(None),
            driver_capabilities: Mutex::new(None),
            conversations: DashMap::new(),
            schema_cache: Arc::new(SchemaCache::new(ai_config.schema_cache_ttl_secs)),
            ai_config: Arc::new(RwLock::new(ai_config)),
            schema_graph: Arc::new(Mutex::new(None)),
            embedder: Arc::new(Mutex::new(None)),
            reranker: Arc::new(Mutex::new(None)),
            api_key_cache: Arc::new(RwLock::new(None)),
            password_cache: Arc::new(RwLock::new(HashMap::new())),
            notebook_sessions: DashMap::new(),
            logs: crate::supervisor::new_log_buffer(),
            llm_usage: DashMap::new(),
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
}

#[derive(Debug, Serialize)]
pub struct ConnectResult {
    pub server_version: String,
    pub database: String,
}

#[cfg(test)]
mod capability_state_tests {
    use lucent_protocol::ReadOnlyMode;

    use super::CapabilityView;

    #[test]
    fn the_view_the_frontend_gets_names_the_enforcement_level() {
        let strong = CapabilityView::from(&lucent_driver_postgres_caps());
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
        let mut caps = lucent_driver_postgres_caps();
        caps.readonly = ReadOnlyMode::GuardOnly;
        let view = CapabilityView::from(&caps);
        assert!(!view.engine_enforced_readonly);
        let note = view.readonly_disclosure.expect("must disclose");
        assert!(note.to_lowercase().contains("not enforced"), "{note}");
    }

    fn lucent_driver_postgres_caps() -> lucent_protocol::DriverCapabilities {
        lucent_protocol::DriverCapabilities {
            id: "postgres".into(),
            display_name: "PostgreSQL".into(),
            sql_dialect: lucent_protocol::SqlDialect::PostgreSql,
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
}

impl From<&lucent_protocol::DriverCapabilities> for CapabilityView {
    fn from(c: &lucent_protocol::DriverCapabilities) -> Self {
        Self {
            driver: c.id.clone(),
            display_name: c.display_name.clone(),
            engine_enforced_readonly: c.readonly.is_engine_enforced(),
            readonly_disclosure: c.readonly.disclosure().map(str::to_string),
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

#[derive(Debug, Serialize, Clone)]
pub struct SchemaInfo {
    pub name: String,
    pub object_count: i64,
}

/// Normalized namespaces → the flat `SchemaInfo` the sidebar consumes.
///
/// Postgres emits one path segment, so this reproduces today's schema names
/// exactly. A driver with deeper namespaces renders dotted.
fn namespaces_to_schema_info(namespaces: Vec<lucent_protocol::Namespace>) -> Vec<SchemaInfo> {
    namespaces
        .into_iter()
        .map(|n| SchemaInfo {
            name: n.display(),
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
    pub name: String,
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
    connect_impl(state, resolved).instrument(span).await
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
) -> Result<ConnectResult, CommandError> {
    log::info!(
        "Connecting to database {:?}@{}/{}",
        resolved.get("user").unwrap_or(""),
        resolved.get("host").unwrap_or(""),
        resolved.get("database").unwrap_or("")
    );

    // Disconnect previous connection if any
    {
        let mut client_lock = state.client.lock().await;
        if let Some(ref mut old_client) = *client_lock {
            log::info!("Disconnecting previous connection before new connect");
            let _ = old_client.shutdown().await;
        }
        // Clear before connecting
        *client_lock = None;
        // The AI session dies with the old client.
        *state.ai_connection_id.lock().await = None;
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
        let sup = supervisor_lock.get_or_insert_with(|| Supervisor::with_logs(state.logs.clone()));
        let sp = match sup.ensure_running().await {
            Ok(p) => p.to_path_buf(),
            Err(e) => {
                log::error!("Worker startup failed (attempt {}): {e}", attempt + 1);
                last_connect_err = Some(e);
                continue;
            }
        };
        let tk = sup.handshake_token().to_string();
        log::debug!("Worker socket at {sp:?}, token={tk}");
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
    let database = resolved.get("database").unwrap_or("").to_string();
    log::info!("Connected to {database} (Postgres {server_version})");

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
    let conn_id = format!(
        "{}:{}/{}",
        resolved.get("host").unwrap_or(""),
        resolved.port().unwrap_or(0),
        resolved.get("database").unwrap_or("")
    );
    state
        .schema_cache
        .refresh(conn_id.clone(), &client, worker_conn_id)
        .await
        .ok();

    // Build semantic schema index — non-blocking failure, never fails connect()
    *state.schema_graph.lock().await = None;
    {
        let ai_cfg = state.ai_config.read().await;
        if ai_cfg.enable_semantic_index {
            // embedder is created once and reused across reconnects
            let embedder_ready = state.embedder.lock().await.is_some();
            if !embedder_ready {
                match tokio::task::spawn_blocking(crate::ai::embed::Embedder::new).await {
                    Ok(Ok(embedder)) => {
                        *state.embedder.lock().await = Some(embedder);
                    }
                    Ok(Err(e)) => {
                        log::warn!("Embedder init failed, continuing without semantic index: {e}");
                    }
                    Err(e) => {
                        log::warn!("Embedder init task panicked: {e}");
                    }
                }
            }
            // reranker is created once and reused across reconnects
            let reranker_ready = state.reranker.lock().await.is_some();
            if !reranker_ready {
                match tokio::task::spawn_blocking(crate::ai::rerank::Reranker::new).await {
                    Ok(Ok(reranker)) => {
                        *state.reranker.lock().await = Some(reranker);
                    }
                    Ok(Err(e)) => {
                        log::warn!(
                            "Reranker init failed, semantic search will skip reranking: {e}"
                        );
                    }
                    Err(e) => {
                        log::warn!("Reranker init task panicked: {e}");
                    }
                }
            }

            let embedder_guard = state.embedder.lock().await;
            if let Some(embedder) = embedder_guard.as_ref() {
                let capabilities = state.capabilities().await;
                if let Some(capabilities) = capabilities.as_ref() {
                    match crate::ai::schema_graph::SchemaIndexer::build_index(
                        worker_conn_id,
                        &client,
                        embedder,
                        ai_cfg.send_results_to_ai,
                        capabilities,
                    )
                    .await
                    {
                        Ok(graph) => {
                            *state.schema_graph.lock().await = Some(graph);
                        }
                        Err(e) => {
                            log::warn!("Schema index build failed, continuing without it: {e}");
                        }
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

/// Probe a connection config through a dedicated, short-lived worker process.
///
/// A throwaway worker is required, not an optimization: the app's real worker
/// serves exactly one socket — the live connection's — so a second probe socket
/// would sit unaccepted in the backlog and time out after 15s, reporting a
/// healthy database as unreachable. A fresh worker per probe leaves the live
/// connection untouched and exercises the real seam.
pub async fn probe_connection(
    config: ConnectionConfig,
    display_fallback: String,
) -> Result<TestConnectionResult, CommandError> {
    let mut supervisor = Supervisor::new();

    let socket_and_token = match supervisor.ensure_running().await {
        Ok(path) => (path.to_path_buf(), supervisor.handshake_token().to_string()),
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
    std::fs::write(&path, formatted.as_bytes())
        .map_err(|e| CommandError::new("FileError", e.to_string()))?;
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

#[tauri::command]
pub async fn execute_query(
    state: State<'_, AppState>,
    sql: String,
    limit: i64,
    offset: i64,
    sort: Option<crate::query_paging::SortSpec>,
    filters: Vec<crate::query_paging::FilterSpec>,
) -> Result<ExecuteResult, CommandError> {
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
            let _ = crate::query_history::append_entry(entry);
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
            let _ = crate::query_history::append_entry(entry);
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
    schema: String,
) -> Result<SchemaObjectsResult, CommandError> {
    let conn_id = (*state.current_connection_id.lock().await)
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;
    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    // One request replaces four sequential queries. Empty `kinds` means every
    // kind the driver knows.
    let summaries = client
        .list_objects(conn_id, vec![schema.clone()], vec![])
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    Ok(SchemaObjectsResult {
        name: schema,
        objects: summaries_to_schema_objects(summaries),
    })
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

    *state.current_connection_id.lock().await = None;
    *state.ai_connection_id.lock().await = None;
    *state.current_database.lock().await = None;
    *state.driver_capabilities.lock().await = None;

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
    schema: String,
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
                namespace: vec![schema],
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
    schema: String,
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
                namespace: vec![schema],
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
    schema: String,
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
                namespace: vec![schema],
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

#[tauri::command]
pub async fn browse_table(
    state: State<'_, AppState>,
    schema: String,
    name: String,
    limit: i64,
    offset: i64,
    sort: Option<crate::query_paging::SortSpec>,
    filters: Vec<crate::query_paging::FilterSpec>,
) -> Result<ExecuteResult, CommandError> {
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

    let base_sql = format!(
        "SELECT * FROM {}.{}",
        builder.quote_identifier(&schema),
        builder.quote_identifier(&name)
    );
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
    log::info!("Acquiring schema graph for system prompt");
    let graph_guard = state.schema_graph.lock().await;
    log::info!("Schema graph locked, building tier");
    let tier = graph_guard
        .as_ref()
        .map(|g| crate::ai::mschema::select_tier(g).0)
        .unwrap_or(crate::ai::mschema::ContextTier::Pull);
    log::info!("Tier selected: {:?}", tier);
    let prompt = if let Some(tree) = state.schema_cache.get(connection_id) {
        let capabilities = state.capabilities().await;
        let p = crate::ai::context::build_system_prompt(
            &tree,
            graph_guard.as_ref(),
            state.ai_config.read().await.send_results_to_ai,
            capabilities.as_ref(),
        );
        log::debug!("System prompt built ({} bytes, tier {:?})", p.len(), tier);
        p
    } else if let Some(g) = graph_guard.as_ref() {
        log::info!(
            "Schema tree expired for {connection_id}; rendering system prompt from in-memory graph"
        );
        let db_name = connection_id
            .rsplit('/')
            .next()
            .unwrap_or(connection_id)
            .to_string();
        let tree = crate::ai::context::tree_from_graph(db_name, g);
        let capabilities = state.capabilities().await;
        crate::ai::context::build_system_prompt(
            &tree,
            Some(g),
            state.ai_config.read().await.send_results_to_ai,
            capabilities.as_ref(),
        )
    } else {
        log::warn!(
            "Schema cache miss for connection {connection_id} and no schema graph available"
        );
        "Database context not yet loaded.".into()
    };
    (prompt, tier)
}

/// Runs one full agent turn: provider creation, state transition to
/// `Running`, preflight (skipped when `message` is empty — the resume-after-
/// DML case needs no schema injection), the agent loop, and error/timeout
/// handling. Deliberately has NO `PausedForDml` guard: `ai_chat` rejects new
/// messages while a DML is pending, but `execute_dml` resumes the agent
/// through here after the user approves.
#[allow(clippy::too_many_arguments)]
async fn run_agent_turn(
    state: &AppState,
    app_handle: &tauri::AppHandle,
    channel: tauri::ipc::Channel<AiEvent>,
    conversation_id: String,
    message: String,
    system_prompt: String,
) -> Result<(), String> {
    let conv = state
        .conversations
        .get(&conversation_id)
        .ok_or("Conversation not found")?
        .clone();

    let config = state.ai_config.read().await.clone();
    log::debug!(
        "ai_chat config: provider={}, model={}, max_turns={}, send_results_to_ai={}",
        config.provider,
        config.model,
        config.max_turns,
        config.send_results_to_ai
    );

    // Read the cache separately so the read guard drops before the match body
    // runs. Otherwise a write() inside the None branch would deadlock — the
    // temporary read guard from the scrutinee lives until the match ends.
    let cached_key = {
        let guard = state.api_key_cache.read().await;
        cached_api_key(&guard, &config.provider)
    };
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

    let context_tier = {
        let guard = state.schema_graph.lock().await;
        guard
            .as_ref()
            .map(|g| crate::ai::mschema::select_tier(g).0)
            .unwrap_or(crate::ai::mschema::ContextTier::Pull)
    };

    log::info!("Creating LLM provider");
    let provider: Arc<dyn LlmProvider> = Arc::new(RigProvider::new(
        config.provider.clone(),
        api_key,
        config.endpoint.clone(),
    ));

    let cancel = tokio_util::sync::CancellationToken::new();
    {
        let mut locked = conv.lock().await;
        locked.state = AgentState::Running {
            cancel: cancel.clone(),
        };
    }

    log::info!("Provider created, building tool context");

    let ai_conn_id = {
        let guard = state.ai_connection_id.lock().await;
        *guard
    };
    let tool_ctx = AiToolContext {
        db: state.client.clone(),
        connection_id: ai_conn_id.or(*state.current_connection_id.lock().await),
        capabilities: state.capabilities().await,
        config: config.clone(),
        schema_graph: state.schema_graph.clone(),
        embedder: state.embedder.clone(),
        reranker: state.reranker.clone(),
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
    let tools = crate::ai::tools::all_tools(tool_ctx.clone());
    let agent = DatabaseAgent::new(provider, tools, tool_ctx);
    let sink: Arc<dyn AgentSink> = Arc::new(TauriSink {
        channel,
        app_handle: app_handle.clone(),
    });
    let app_err = app_handle.clone();
    let conv_err = conv.clone();
    const AGENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
    let chat_result = tokio::time::timeout(
        AGENT_TIMEOUT,
        agent.chat(
            augmented_message,
            &config,
            system_prompt,
            conv,
            sink,
            cancel,
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
            let _ = app_err.emit(
                "ai:error",
                AiErrorPayload {
                    conversation_id: conversation_id.clone(),
                    message: "Agent timed out after 300 seconds. Try simplifying the question."
                        .into(),
                },
            );
            let mut s = conv_err.lock().await;
            s.state = AgentState::Idle;
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn ai_chat(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    channel: tauri::ipc::Channel<AiEvent>,
    message: String,
    conversation_id: String,
    connection_id: String,
    profile_id: Option<String>,
) -> Result<(), String> {
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
    )
    .instrument(span)
    .await
}

async fn ai_chat_impl(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
    channel: tauri::ipc::Channel<AiEvent>,
    message: String,
    conversation_id: String,
    connection_id: String,
    profile_id: Option<String>,
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

        // Ensure worker is connected
        let (socket_path, token) = {
            let mut supervisor_lock = state.supervisor.lock().await;
            let sup =
                supervisor_lock.get_or_insert_with(|| Supervisor::with_logs(state.logs.clone()));
            let socket_path = sup
                .ensure_running()
                .await
                .map_err(|e| format!("Worker startup failed: {e}"))?
                .to_path_buf();
            let token = sup.handshake_token().to_string();
            (socket_path, token)
        };

        let mut client_lock = state.client.lock().await;
        if client_lock.is_none() {
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
        if let AgentState::PausedForDml { .. } = &locked.state {
            return Err("Approve or cancel the pending DML before sending another message.".into());
        }
    }

    // Keep the channel on the conversation: `execute_dml` clones it to resume
    // the agent on the same IPC stream after the user approves DML (C1).
    conv.lock().await.event_channel = Some(channel.clone());

    log::info!(
        "ai_chat: conversation={conversation_id}, message_len={}",
        message.len()
    );
    let (system_prompt, _context_tier) = build_system_prompt(&state, &connection_id).await;
    log::info!("System prompt complete ({} bytes)", system_prompt.len());

    run_agent_turn(
        &state,
        &app_handle,
        channel,
        conversation_id,
        message,
        system_prompt,
    )
    .await
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
            s.state = AgentState::Idle;
        }
        AgentState::PausedForDml { .. } => {
            s.take_staged_sql();
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
}

#[tauri::command]
pub async fn close_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    evict_conversation(&state, &conversation_id);
    Ok(())
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
    let staged_sql = conv
        .lock()
        .await
        .take_staged_sql()
        .ok_or("No pending DML for this conversation")?;

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

#[tauri::command]
pub async fn save_ai_settings(
    state: State<'_, AppState>,
    config: AiConfig,
    api_key: Option<String>,
) -> Result<(), String> {
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
mod usage_tests {
    use super::accumulate_usage;
    use crate::ai::events::TokenUsage;

    #[test]
    fn sums_token_fields_and_cost_across_runs() {
        let existing = TokenUsage {
            prompt_tokens: 100,
            completion_tokens: 20,
            cached_prompt_tokens: 30,
            estimated_cost_usd: Some(0.5),
        };
        let new = TokenUsage {
            prompt_tokens: 50,
            completion_tokens: 10,
            cached_prompt_tokens: 5,
            estimated_cost_usd: Some(0.25),
        };
        let acc = accumulate_usage(&existing, &new);
        assert_eq!(acc.prompt_tokens, 150);
        assert_eq!(acc.completion_tokens, 30);
        assert_eq!(acc.cached_prompt_tokens, 35);
        assert_eq!(acc.estimated_cost_usd, Some(0.75));
    }

    #[test]
    fn cost_survives_runs_that_report_none() {
        let with_cost = TokenUsage {
            estimated_cost_usd: Some(1.0),
            ..TokenUsage::default()
        };
        let no_cost = TokenUsage::default();
        assert_eq!(
            accumulate_usage(&with_cost, &no_cost).estimated_cost_usd,
            Some(1.0)
        );
        assert_eq!(
            accumulate_usage(&no_cost, &no_cost).estimated_cost_usd,
            None
        );
    }

    #[test]
    fn accumulating_into_empty_totals_is_the_new_run() {
        let new = TokenUsage {
            prompt_tokens: 42,
            completion_tokens: 7,
            cached_prompt_tokens: 3,
            estimated_cost_usd: None,
        };
        let acc = accumulate_usage(&TokenUsage::default(), &new);
        assert_eq!(acc.prompt_tokens, 42);
        assert_eq!(acc.completion_tokens, 7);
        assert_eq!(acc.cached_prompt_tokens, 3);
        assert_eq!(acc.estimated_cost_usd, None);
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
                estimated_cost_usd: Some(0.01),
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
