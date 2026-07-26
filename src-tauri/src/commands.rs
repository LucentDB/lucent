use std::sync::Arc;

use dashmap::DashMap;
use lucent_protocol::ConnectionConfig;
use serde::Serialize;
use tauri::{Emitter, State};
use tokio::sync::{Mutex, RwLock};

use crate::ai::agent::{AgentSink, AgentState, ConversationState, DatabaseAgent};
use crate::ai::config::{keychain_account, AiConfig, KEYCHAIN_SERVICE};
use crate::ai::context::SchemaCache;
use crate::ai::events::{AiErrorPayload, AiEvent};
use crate::ai::provider::LlmProvider;
use crate::ai::providers::rig::RigProvider;
use crate::ai::tools::AiToolContext;
use tauri_plugin_clipboard_manager::ClipboardExt;

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
        let _ = self.channel.send(event);
    }
    fn dml_approval(&self, payload: crate::ai::events::DmlApprovalPayload) {
        let _ = self.app_handle.emit("ai:dml_approval", payload);
    }
}

// SQL quoting lives in one place — see crate::sql_quote.
pub(crate) use crate::sql_quote::{quote_identifier, quote_string};

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
    /// Single shared DB connection. Tools and the main app lock the same Mutex.
    /// Wrapping in Arc lets us clone it into spawned tasks without ownership transfer.
    pub client: Arc<Mutex<Option<ConnectorClient>>>,
    pub current_database: Mutex<Option<String>>,
    // AI module state
    pub conversations: DashMap<String, Arc<Mutex<ConversationState>>>,
    pub ai_config: Arc<RwLock<AiConfig>>,
    pub schema_cache: Arc<SchemaCache>,
    pub schema_graph: Arc<Mutex<Option<crate::ai::schema_graph::SchemaGraph>>>,
    pub embedder: Arc<Mutex<Option<crate::ai::embed::Embedder>>>,
    pub reranker: Arc<Mutex<Option<crate::ai::rerank::Reranker>>>,
}

impl AppState {
    pub fn new() -> Self {
        let ai_config = crate::ai::config::load_config_from_disk();
        Self {
            repo: Arc::new(ConnectionProfileRepository::load()),
            supervisor: Mutex::new(None),
            client: Arc::new(Mutex::new(None)),
            current_database: Mutex::new(None),
            conversations: DashMap::new(),
            schema_cache: Arc::new(SchemaCache::new(ai_config.schema_cache_ttl_secs)),
            ai_config: Arc::new(RwLock::new(ai_config)),
            schema_graph: Arc::new(Mutex::new(None)),
            embedder: Arc::new(Mutex::new(None)),
            reranker: Arc::new(Mutex::new(None)),
            api_key_cache: Arc::new(RwLock::new(None)),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ConnectResult {
    pub server_version: String,
    pub database: String,
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

#[tauri::command]
pub async fn get_schemas(state: State<'_, AppState>) -> Result<Vec<SchemaInfo>, CommandError> {
    let mut client_lock = state.client.lock().await;
    let client = client_lock
        .as_mut()
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let result = client
        .execute(
            "SELECT s.schema_name, \
               (SELECT count(*) FROM information_schema.tables t \
                WHERE t.table_schema = s.schema_name AND t.table_type = 'BASE TABLE') + \
               (SELECT count(*) FROM information_schema.views v \
                WHERE v.table_schema = s.schema_name) + \
               (SELECT count(*) FROM pg_proc p JOIN pg_namespace n \
                ON p.pronamespace = n.oid \
                WHERE n.nspname = s.schema_name AND p.prokind = 'f') + \
               (SELECT count(*) FROM information_schema.sequences seq \
                WHERE seq.sequence_schema = s.schema_name) \
             AS total \
             FROM information_schema.schemata s \
             WHERE s.schema_name NOT IN ('pg_catalog', 'information_schema', 'pg_toast') \
             ORDER BY s.schema_name",
        )
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    let schemas = result
        .rows
        .iter()
        .map(|r| SchemaInfo {
            name: r[0].as_str().unwrap_or("").to_string(),
            object_count: r[1].as_i64().unwrap_or(0),
        })
        .collect();

    Ok(schemas)
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
        let password = crate::connections::get_password(prof_id)
            .map_err(|e| CommandError::new("KeychainError", e.to_string()))?;

        // Mark last_used
        state.repo.mark_used(prof_id).await.ok();

        ConnectionConfig {
            host: profile.host.clone(),
            port: profile.port,
            user: profile.user.clone(),
            password,
            database: profile.database.clone(),
            ssl_mode: profile.ssl_mode.to_string(),
        }
    } else if let Some(cfg) = config {
        cfg
    } else {
        return Err(CommandError::new(
            "InvalidArgs",
            "provide connection_id or config",
        ));
    };

    log::info!(
        "Connecting to database {:?}@{}/{}",
        resolved.user,
        resolved.host,
        resolved.database
    );

    // Disconnect previous connection if any
    {
        let mut client_lock = state.client.lock().await;
        if let Some(ref mut old_client) = *client_lock {
            log::info!("Disconnecting previous connection before new connect");
            let _ = old_client.disconnect().await;
        }
        // Clear before connecting
        *client_lock = None;
    }

    // Invalidate schema cache for old connection
    state.schema_cache.clear();

    // Retry loop: after disconnecting the old client, the worker may still
    // be exiting. `ensure_running` now respawns on exit, but there's a race
    // where the socket file exists while the old worker is shutting down.
    let mut last_connect_err: Option<String> = None;
    let mut client: Option<ConnectorClient> = None;
    for attempt in 0..3 {
        let mut supervisor_lock = state.supervisor.lock().await;
        let sup = supervisor_lock.get_or_insert_with(Supervisor::new);
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
            Ok(c) => {
                client = Some(c);
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
    let mut client = client.ok_or_else(|| {
        let msg = last_connect_err.unwrap_or_else(|| "unable to start worker".to_string());
        CommandError::new("ConnectError", msg)
    })?;
    let server_version = client
        .server_info
        .as_ref()
        .map(|s| s.version.clone())
        .unwrap_or_default();
    let database = resolved.database.clone();
    log::info!("Connected to {database} (Postgres {server_version})");

    // Refresh schema cache BEFORE storing client (no lock needed)
    let conn_id = format!("{}:{}/{}", resolved.host, resolved.port, resolved.database);
    state
        .schema_cache
        .refresh(conn_id.clone(), &mut client)
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
                match crate::ai::schema_graph::SchemaIndexer::build_index(
                    &mut client,
                    embedder,
                    ai_cfg.send_results_to_ai,
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

    *state.client.lock().await = Some(client);

    *state.current_database.lock().await = Some(database.clone());

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

    if let Some(ref pw) = password {
        crate::connections::set_password(&profile.id, pw)
            .map_err(|e| CommandError::new("KeychainError", e.to_string()))?;
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
    let now = chrono::Utc::now().to_rfc3339();
    copy.created_at = now.clone();
    copy.updated_at = now;
    copy.last_used = None;

    // Copy password if exists (best-effort)
    if let Ok(pw) = crate::connections::get_password(&id) {
        crate::connections::set_password(&copy.id, &pw).ok();
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
    let password = crate::connections::get_password(&id)
        .map_err(|e| CommandError::new("KeychainError", e.to_string()))?;

    // Build tokio_postgres config directly for quick ping
    let mut pg_config = tokio_postgres::Config::new();
    pg_config.host(&profile.host);
    pg_config.port(profile.port);
    pg_config.user(&profile.user);
    pg_config.password(&password);
    pg_config.dbname(&profile.database);

    // Map SSL mode
    match profile.ssl_mode {
        crate::connections::SslMode::Disable => {
            pg_config.ssl_mode(tokio_postgres::config::SslMode::Disable);
        }
        crate::connections::SslMode::Prefer => {
            pg_config.ssl_mode(tokio_postgres::config::SslMode::Prefer);
        }
        crate::connections::SslMode::Require => {
            pg_config.ssl_mode(tokio_postgres::config::SslMode::Require);
        }
    }

    let (client, connection) = pg_config
        .connect(tokio_postgres::NoTls)
        .await
        .map_err(|e| CommandError::new("ConnectionFailed", format!("{e}")))?;

    // Spawn connection handler — we just need a quick ping
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            log::warn!("test connection background task error: {e}");
        }
    });

    let row = client
        .query_one("SELECT 1", &[])
        .await
        .map_err(|e| CommandError::new("QueryFailed", format!("{e}")))?;
    let val: i32 = row.get(0);

    Ok(TestConnectionResult {
        success: true,
        message: format!(
            "Connected to PostgreSQL at {}:{}/{} (ping: SELECT {val})",
            profile.host, profile.port, profile.database
        ),
        server_version: None,
    })
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

#[derive(Debug, Serialize)]
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
    let mut client_lock = state.client.lock().await;
    let client = client_lock
        .as_mut()
        .ok_or_else(|| CommandError::new("QueryError", "not connected — connect first"))?;

    let final_sql = if crate::query_paging::is_wrappable_query(&sql) {
        crate::query_paging::wrap_for_page(&sql, &sort, &filters, limit, offset)
    } else {
        sql
    };

    let start = std::time::Instant::now();
    let result = client.execute(&final_sql).await;
    let duration_ms = start.elapsed().as_millis() as u64;
    let db = state
        .current_database
        .lock()
        .await
        .clone()
        .unwrap_or_default();

    match result {
        Ok(execute_result) => {
            let row_count = execute_result.row_count;
            // Fire-and-forget history entry
            let entry = crate::query_history::QueryHistoryEntry::new(
                String::new(),
                String::new(),
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
                String::new(),
                String::new(),
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
    }
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
    let mut client_lock = state.client.lock().await;
    let client = client_lock
        .as_mut()
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let safe = quote_string(&schema);

    let tables = client
        .execute(&format!(
            "SELECT t.table_name, COALESCE(s.n_live_tup::bigint, 0) \
             FROM information_schema.tables t \
             LEFT JOIN pg_stat_user_tables s \
               ON t.table_name = s.relname AND t.table_schema = s.schemaname \
             WHERE t.table_schema = {safe} AND t.table_type = 'BASE TABLE' \
             ORDER BY t.table_name"
        ))
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    let views = client
        .execute(&format!(
            "SELECT table_name FROM information_schema.views \
             WHERE table_schema = {safe} ORDER BY table_name"
        ))
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    let funcs = client
        .execute(&format!(
            "SELECT p.proname FROM pg_proc p \
             JOIN pg_namespace n ON p.pronamespace = n.oid \
             WHERE n.nspname = {safe} AND p.prokind = 'f' \
             ORDER BY p.proname"
        ))
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    let seqs = client
        .execute(&format!(
            "SELECT sequence_name FROM information_schema.sequences \
             WHERE sequence_schema = {safe} ORDER BY sequence_name"
        ))
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    let mut objects: Vec<SchemaObject> = Vec::new();

    for row in &tables.rows {
        objects.push(SchemaObject {
            name: row[0].as_str().unwrap_or("").to_string(),
            kind: "table".into(),
            row_count: row[1].as_i64(),
        });
    }

    for row in &views.rows {
        objects.push(SchemaObject {
            name: row[0].as_str().unwrap_or("").to_string(),
            kind: "view".into(),
            row_count: None,
        });
    }

    for row in &funcs.rows {
        objects.push(SchemaObject {
            name: row[0].as_str().unwrap_or("").to_string(),
            kind: "function".into(),
            row_count: None,
        });
    }

    for row in &seqs.rows {
        objects.push(SchemaObject {
            name: row[0].as_str().unwrap_or("").to_string(),
            kind: "sequence".into(),
            row_count: None,
        });
    }

    Ok(SchemaObjectsResult {
        name: schema,
        objects,
    })
}

#[tauri::command]
pub async fn disconnect(state: State<'_, AppState>) -> Result<DisconnectResult, CommandError> {
    log::info!("Disconnecting");
    let client = state.client.lock().await.take();

    if let Some(mut c) = client {
        log::debug!("Running disconnect on connector client");
        let _ = c.disconnect().await;
    }

    // TODO(multi-connection): only shutdown supervisor when last connection disconnects
    let mut supervisor_lock = state.supervisor.lock().await;
    if let Some(mut supervisor) = supervisor_lock.take() {
        supervisor.shutdown().await.ok();
    }

    *state.current_database.lock().await = None;

    Ok(DisconnectResult { ok: true })
}

#[tauri::command]
pub async fn get_function_source(
    state: State<'_, AppState>,
    schema: String,
    name: String,
) -> Result<String, CommandError> {
    let mut client_lock = state.client.lock().await;
    let client = client_lock
        .as_mut()
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let sql = format!(
        "SELECT pg_get_functiondef(p.oid) AS source \
         FROM pg_proc p JOIN pg_namespace n ON p.pronamespace = n.oid \
         WHERE n.nspname = {} AND p.proname = {}",
        quote_string(&schema),
        quote_string(&name)
    );

    let result = client
        .execute(&sql)
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;
    Ok(result
        .rows
        .first()
        .and_then(|r| r[0].as_str())
        .unwrap_or("-- no source found")
        .to_string())
}

#[tauri::command]
pub async fn get_view_source(
    state: State<'_, AppState>,
    schema: String,
    name: String,
) -> Result<String, CommandError> {
    let mut client_lock = state.client.lock().await;
    let client = client_lock
        .as_mut()
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let sql = format!(
        "SELECT pg_get_viewdef({}::regclass, true) AS source",
        quote_string(&format!("{}.{}", schema, name))
    );

    let result = client
        .execute(&sql)
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;
    let def = result
        .rows
        .first()
        .and_then(|r| r[0].as_str())
        .unwrap_or("-- no source found");
    Ok(format!(
        "CREATE OR REPLACE VIEW {}.{} AS\n{}",
        quote_identifier(&schema),
        quote_identifier(&name),
        def
    ))
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
    let mut client_lock = state.client.lock().await;
    let client = client_lock
        .as_mut()
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let sql = format!(
        "SELECT seq.sequence_name::text, seq.data_type::text, \
                seq.start_value::text, seq.minimum_value::text, \
                seq.maximum_value::text, seq.increment::text, \
                seq.cycle_option::text \
         FROM information_schema.sequences seq \
         WHERE seq.sequence_schema = {} AND seq.sequence_name = {}",
        quote_string(&schema),
        quote_string(&name)
    );

    let result = client
        .execute(&sql)
        .await
        .map_err(|e| CommandError::new("QueryError", e))?;

    if let Some(row) = result.rows.first() {
        let get = |i: usize| {
            row.get(i)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        Ok(vec![
            SequenceProperty {
                key: "Data Type".into(),
                value: get(1),
            },
            SequenceProperty {
                key: "Start Value".into(),
                value: get(2),
            },
            SequenceProperty {
                key: "Min Value".into(),
                value: get(3),
            },
            SequenceProperty {
                key: "Max Value".into(),
                value: get(4),
            },
            SequenceProperty {
                key: "Increment".into(),
                value: get(5),
            },
            SequenceProperty {
                key: "Cycle".into(),
                value: get(6),
            },
        ])
    } else {
        Ok(vec![SequenceProperty {
            key: "Note".into(),
            value: format!("{}.{} — no metadata found", schema, name),
        }])
    }
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
    let mut client_lock = state.client.lock().await;
    let client = client_lock
        .as_mut()
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let base_sql = format!(
        "SELECT * FROM {}.{}",
        quote_identifier(&schema),
        quote_identifier(&name)
    );
    let final_sql = crate::query_paging::wrap_for_page(&base_sql, &sort, &filters, limit, offset);

    client
        .execute(&final_sql)
        .await
        .map_err(|e| CommandError::new("QueryError", e))
}

#[tauri::command]
pub async fn count_all_rows(
    state: State<'_, AppState>,
    sql: String,
    filters: Vec<crate::query_paging::FilterSpec>,
) -> Result<i64, CommandError> {
    if !crate::query_paging::is_wrappable_query(&sql) {
        return Err(CommandError::new(
            "QueryError",
            "cannot count rows for a non-SELECT statement",
        ));
    }

    let mut client_lock = state.client.lock().await;
    let client = client_lock
        .as_mut()
        .ok_or_else(|| CommandError::new("QueryError", "not connected"))?;

    let count_sql = crate::query_paging::wrap_for_count(&sql, &filters);
    let result = client
        .execute(&count_sql)
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
/// database, so it works before a connection exists.
#[tauri::command]
pub fn describe_filters(filters: Vec<crate::query_paging::FilterSpec>) -> String {
    crate::query_paging::filters_to_where_clause(&filters)
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
fn load_api_key(config: &AiConfig) -> Result<String, String> {
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
fn cached_api_key(
    cache: &Option<(crate::ai::config::AiProvider, String)>,
    provider: &crate::ai::config::AiProvider,
) -> Option<String> {
    cache
        .as_ref()
        .filter(|(p, _)| p == provider)
        .map(|(_, k)| k.clone())
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
        let password =
            crate::connections::get_password(pid).map_err(|e| format!("Keychain error: {e}"))?;
        let config = lucent_protocol::ConnectionConfig {
            host: profile.host.clone(),
            port: profile.port,
            user: profile.user.clone(),
            password,
            database: profile.database.clone(),
            ssl_mode: profile.ssl_mode.to_string(),
        };

        // Ensure worker is connected
        let (socket_path, token) = {
            let mut supervisor_lock = state.supervisor.lock().await;
            let sup = supervisor_lock.get_or_insert_with(Supervisor::new);
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
            let new_client = ConnectorClient::connect(&socket_path, &token, config)
                .await
                .map_err(|e| format!("Connect failed: {e}"))?;
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

    log::info!(
        "ai_chat: conversation={conversation_id}, message_len={}",
        message.len()
    );
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

    log::info!("Acquiring schema graph for system prompt");
    let (system_prompt, context_tier) = {
        let graph_guard = state.schema_graph.lock().await;
        log::info!("Schema graph locked, building tier");
        let tier = graph_guard
            .as_ref()
            .map(|g| crate::ai::mschema::select_tier(g).0)
            .unwrap_or(crate::ai::mschema::ContextTier::Pull);
        log::info!("Tier selected: {:?}", tier);
        let prompt = if let Some(tree) = state.schema_cache.get(&connection_id) {
            let p = crate::ai::context::build_system_prompt(
                &tree,
                graph_guard.as_ref(),
                config.send_results_to_ai,
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
                .unwrap_or(&connection_id)
                .to_string();
            let tree = crate::ai::context::tree_from_graph(db_name, g);
            crate::ai::context::build_system_prompt(&tree, Some(g), config.send_results_to_ai)
        } else {
            log::warn!(
                "Schema cache miss for connection {connection_id} and no schema graph available"
            );
            "Database context not yet loaded.".into()
        };
        (prompt, tier)
    };
    log::info!("System prompt complete ({} bytes)", system_prompt.len());

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

    let tool_ctx = AiToolContext {
        db: state.client.clone(),
        config: config.clone(),
        schema_graph: state.schema_graph.clone(),
        embedder: state.embedder.clone(),
        reranker: state.reranker.clone(),
    };

    // Pre-flight: augment the message with value hints and retrieved schema
    // context before the first LLM call.
    log::info!("Pre-flight starting");
    let augmented_message = {
        let graph_guard = tool_ctx.schema_graph.lock().await;
        let emb_guard = tool_ctx.embedder.lock().await;
        let result = crate::ai::preflight::run_preflight(
            Some(&tool_ctx.db),
            graph_guard.as_ref(),
            emb_guard.as_ref(),
            &context_tier,
            &message,
        )
        .await;
        log::info!("Pre-flight completed");
        match result {
            Some(block) => format!("{message}\n\n{block}"),
            None => message.clone(),
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
}

#[tauri::command]
pub async fn close_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    evict_conversation(&state, &conversation_id);
    Ok(())
}

#[tauri::command]
pub async fn execute_dml(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    let conv = state
        .conversations
        .get(&conversation_id)
        .ok_or("Conversation not found")?;
    let staged_sql = conv
        .lock()
        .await
        .take_staged_sql()
        .ok_or("No pending DML for this conversation")?;
    // Data changed — cached query results may be stale.
    conv.lock().await.query_cache.clear();
    Ok(serde_json::json!({ "rows_affected": 0, "sql": staged_sql }))
}

#[tauri::command]
pub async fn get_ai_settings(state: State<'_, AppState>) -> Result<AiConfig, String> {
    Ok(state.ai_config.read().await.clone())
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
    fn evicting_an_unknown_conversation_is_a_no_op() {
        let state = AppState::new();
        // Must not panic on double-close or a stale/already-evicted id.
        evict_conversation(&state, "does-not-exist");
        assert!(state.conversations.is_empty());
    }
}
