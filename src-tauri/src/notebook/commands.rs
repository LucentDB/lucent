use std::path::PathBuf;
use std::sync::Arc;

use lucent_protocol::{ConnectionId, QueryId};
use tauri::State;
use uuid::Uuid;

use crate::ai::agent::{AgentSink, AgentState, ConversationState, DatabaseAgent};
use crate::ai::events::{AiEvent, DmlApprovalPayload};
use crate::ai::provider::LlmProvider;
use crate::ai::providers::rig::RigProvider;
use crate::ai::tools::AiToolContext;
use crate::commands::{
    cached_api_key, cached_password, load_api_key, validate_approved, AppState, CommandError,
};
use crate::notebook::events::NotebookEvent;
use crate::notebook::file::{self, NotebookFileV2};
use crate::notebook::paging::{build_page_sql, is_pageable, PageRequest, DEFAULT_CELL_PAGE_SIZE};
use crate::notebook::rewrite;
use crate::notebook::session::NotebookSession;
use crate::notebook::types::*;

enum CancelTarget {
    None,
    Query(QueryId),
    AiCell {
        cell_id: String,
        token: tokio_util::sync::CancellationToken,
    },
}

/// What the notebook Stop button should cancel right now: the running SQL
/// query if one is registered, otherwise the running AI cell's agent loop.
/// A SQL query wins when both are somehow set (a cell run registers exactly
/// one of the two) (E2).
fn resolve_cancel_target(
    active_query_id: Option<QueryId>,
    active_ai_cell: &Option<(String, tokio_util::sync::CancellationToken)>,
) -> CancelTarget {
    if let Some(qid) = active_query_id {
        return CancelTarget::Query(qid);
    }
    match active_ai_cell {
        Some((cell_id, token)) => CancelTarget::AiCell {
            cell_id: cell_id.clone(),
            token: token.clone(),
        },
        None => CancelTarget::None,
    }
}

/// D3: clear the active AI cell registration only when it still names
/// `cell_id`. Two overlapping runs mean an older run's completion (or a
/// stale cancel) must not wipe a NEWER run's registration — that would
/// silently make the newer run uncancellable.
fn clear_active_ai_cell_if_same(
    active: &mut Option<(String, tokio_util::sync::CancellationToken)>,
    cell_id: &str,
) {
    if active
        .as_ref()
        .map(|(id, _)| id == cell_id)
        .unwrap_or(false)
    {
        *active = None;
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ResolvedQuery {
    pub cte_chain: Vec<String>,
    pub final_sql: String,
    pub errors: Vec<CellError>,
}

/// Read a notebook file ONLY if the frontend-supplied path was chosen in a
/// native dialog (approved-path set). Mirrors the `notebook_save` gate: the
/// renderer is untrusted, and an ungated read is a local-file disclosure
/// primitive (S3).
fn read_notebook_file(
    path: &std::path::Path,
    approved: &std::collections::HashSet<std::path::PathBuf>,
) -> Result<String, CommandError> {
    let canonical = validate_approved(path, approved)?;
    std::fs::read_to_string(&canonical).map_err(|e| {
        CommandError::new(
            "FileError",
            format!("cannot read {}: {e}", canonical.display()),
        )
    })
}

#[tauri::command]
pub async fn notebook_open(
    path: String,
    state: State<'_, AppState>,
) -> Result<NotebookFileV2, CommandError> {
    let approved = state.approved_save_paths.lock().await.clone();
    let content = read_notebook_file(std::path::Path::new(&path), &approved)?;
    file::parse_file(&path, &content).map_err(|e| CommandError::new("ParseError", e))
}

#[tauri::command]
pub async fn notebook_save(
    session_key: String,
    path: String,
    metadata: NotebookMetadata,
    cells: Vec<CellModel>,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    // The frontend is an untrusted boundary: a write path must have been
    // chosen by the user in a native save dialog (approved-path set). Raw IPC
    // paths are never written directly — same gate as save_sql_file/export.
    let approved = state.approved_save_paths.lock().await.clone();
    let canonical = validate_approved(std::path::Path::new(&path), &approved)?;
    let canonical_str = canonical.to_string_lossy().to_string();

    let json =
        file::to_json(&metadata, &cells).map_err(|e| CommandError::new("SerializeError", e))?;
    std::fs::write(&canonical, &json).map_err(|e| {
        CommandError::new("FileError", format!("cannot write {canonical_str}: {e}"))
    })?;

    // Re-key the session so an untitled notebook's temp-UUID key becomes its path.
    if session_key != canonical_str {
        if let Some((_, mut session)) = state.notebook_sessions.remove(&session_key) {
            session.session_key = canonical_str.clone();
            session.file_path = Some(canonical);
            state
                .notebook_sessions
                .insert(canonical_str.clone(), session);
        }
    } else if let Some(mut session) = state.notebook_sessions.get_mut(&session_key) {
        session.file_path = Some(canonical);
    }

    Ok(canonical_str)
}

#[tauri::command]
pub async fn notebook_attach(
    file_path: Option<String>,
    profile_id: String,
    database: String,
    state: State<'_, AppState>,
) -> Result<String, CommandError> {
    log::info!("notebook_attach: profile={profile_id} db={database} file={file_path:?}");
    let connection_id = ConnectionId(Uuid::new_v4());

    let session_key = file_path
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("not_connected", "no active connection"))?;

    let config = if let Some(profile) = state.repo.get_profile(&profile_id).await {
        let mut config = lucent_protocol::ConnectionConfig::new(profile.driver.clone());
        for (key, value) in &profile.params {
            config = config.with(key.clone(), value.clone());
        }
        // The notebook session always pins its own database, which may differ
        // from the profile's default.
        config = config.with("database", database.clone());
        // The secret lives in the keychain, never in connections.json.
        match cached_password(&state, &profile_id).await {
            Ok(secret) => config = config.with_secret(secret),
            // A driver with AuthModel::FilePath or None has no secret to fetch.
            Err(crate::connections::KeychainError::NotFound) => {}
            Err(e) => return Err(CommandError::new("KeychainError", e.to_string())),
        }
        config
    } else if let Some(ref cfg) = *state.current_connection_config.lock().await {
        // Fallback: use the current connection's config
        let mut c = cfg.clone();
        c.params.insert("database".to_string(), database.clone());
        c
    } else {
        return Err(CommandError::new(
            "NotFound",
            "no connection profile or active connection config available",
        ));
    };

    client
        .connect_with_id(connection_id, config)
        .await
        .map_err(|e| {
            log::error!("notebook_attach: connect_with_id failed: {e}");
            CommandError::new("connect_failed", e)
        })?;

    let mut session = NotebookSession::new(session_key.clone(), connection_id, database);
    session.profile_id = Some(profile_id);
    if let Some(path) = file_path {
        session.file_path = Some(PathBuf::from(&path));
    }

    log::info!(
        "notebook_attach: session created session_key={session_key} conn_id={connection_id:?}"
    );
    state.notebook_sessions.insert(session_key.clone(), session);
    Ok(session_key)
}

#[tauri::command]
pub async fn notebook_detach(
    session_key: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    log::info!("notebook_detach: session_key={session_key}");
    let session = state
        .notebook_sessions
        .remove(&session_key)
        .ok_or_else(|| CommandError::new("not_found", "session not found"))?;

    let (_, session) = session;

    let client = state.client.lock().await.clone();
    if let Some(client) = client {
        client
            .disconnect_id(session.connection_id)
            .await
            .map_err(|e| CommandError::new("disconnect_failed", e))?;
    }

    Ok(())
}

#[tauri::command]
pub async fn notebook_restart_session(
    session_key: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let session = state
        .notebook_sessions
        .get(&session_key)
        .ok_or_else(|| CommandError::new("not_found", "session not found"))?;

    let conn_id = session.connection_id;
    let db = session.database.clone();
    let profile_id = session.profile_id.clone();
    drop(session);

    let client = state.client.lock().await.clone();
    if let Some(client) = client {
        client
            .disconnect_id(conn_id)
            .await
            .map_err(|e| CommandError::new("disconnect_failed", e))?;
    }

    let pid = profile_id
        .as_ref()
        .ok_or_else(|| CommandError::new("no_profile", "session has no profile to reconnect"))?;
    let config = {
        let profile = state.repo.get_profile(pid).await.ok_or_else(|| {
            CommandError::new("NotFound", "connection profile not found for reconnect")
        })?;
        let mut config = lucent_protocol::ConnectionConfig::new(profile.driver.clone());
        for (key, value) in &profile.params {
            config = config.with(key.clone(), value.clone());
        }
        // The notebook session always pins its own database, which may differ
        // from the profile's default.
        config = config.with("database", db.clone());
        // The secret lives in the keychain, never in connections.json.
        match cached_password(&state, pid).await {
            Ok(secret) => config = config.with_secret(secret),
            // A driver with AuthModel::FilePath or None has no secret to fetch.
            Err(crate::connections::KeychainError::NotFound) => {}
            Err(e) => return Err(CommandError::new("KeychainError", e.to_string())),
        }
        config
    };

    let client = state.client.lock().await.clone();
    if let Some(client) = client {
        client
            .connect_with_id(conn_id, config)
            .await
            .map_err(|e| CommandError::new("reconnect_failed", e))?;
    }

    if let Some(mut session) = state.notebook_sessions.get_mut(&session_key) {
        session.reset_execution_counter();
        session.active_query_id = None;
    }

    Ok(())
}

#[tauri::command]
pub async fn notebook_run_cell(
    session_key: String,
    cell_id: String,
    cells: Vec<CellModel>,
    channel: tauri::ipc::Channel<NotebookEvent>,
    state: State<'_, AppState>,
) -> Result<CellOutput, CommandError> {
    log::info!(
        "notebook_run_cell: session={session_key:.8} cell={cell_id} num_cells={}",
        cells.len()
    );
    let session = state
        .notebook_sessions
        .get(&session_key)
        .ok_or_else(|| CommandError::new("not_found", "notebook session not found"))?;

    let cell = cells
        .iter()
        .find(|c| c.id == cell_id)
        .ok_or_else(|| CommandError::new("cell_not_found", format!("cell {cell_id} not found")))?;

    let conn_id = session.connection_id;
    drop(session);

    match &cell.kind {
        CellKind::Sql => {
            log::info!("notebook_run_cell: running SQL cell {cell_id}");
            run_sql_cell(&cell_id, &cells, conn_id, &session_key, channel, &state).await
        }
        CellKind::Ai => {
            log::info!("notebook_run_cell: running AI cell {cell_id}");
            run_ai_cell(cell, &cells, conn_id, &session_key, channel, &state).await
        }
        CellKind::Markdown => Ok(CellOutput::Text(TextOutput {
            content: cell.source.clone(),
        })),
    }
}

async fn run_sql_cell(
    cell_id: &str,
    cells: &[CellModel],
    conn_id: ConnectionId,
    session_key: &str,
    channel: tauri::ipc::Channel<NotebookEvent>,
    state: &State<'_, AppState>,
) -> Result<CellOutput, CommandError> {
    let start = std::time::Instant::now();

    let capabilities = state
        .capabilities()
        .await
        .ok_or_else(|| CommandError::new("not_connected", "no active connection"))?;
    let dialect = capabilities.sql_dialect;
    let builder = crate::sql_builder::for_driver(&capabilities);

    let rewritten = rewrite::rewrite_sql(cell_id, cells, dialect).map_err(|e| {
        log::warn!("notebook SQL rewrite failed for {cell_id}: {e:?}");
        CommandError::new(
            "rewrite_failed",
            serde_json::to_string(&e).unwrap_or_default(),
        )
    })?;

    let pageable = is_pageable(&rewritten, dialect);
    let req = PageRequest {
        limit: DEFAULT_CELL_PAGE_SIZE,
        offset: 0,
        sort: None,
        filters: vec![],
    };
    let page_sql = build_page_sql(&rewritten, &req, dialect, builder.as_ref());

    log::debug!("notebook SQL for {cell_id}: {page_sql}");

    let query_id = QueryId(Uuid::new_v4());
    if let Some(mut s) = state.notebook_sessions.get_mut(session_key) {
        s.active_query_id = Some(query_id);
    }

    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("not_connected", "no active connection"))?;
    // Execute under the registered query_id (so notebook cancel reaches the
    // real query) with the hard row cap: non-wrappable cell bodies (DML, DDL,
    // multi-statement) run unpaginated, so the cap is what bounds them.
    let result = client
        .execute_with_id(
            query_id,
            conn_id,
            &page_sql,
            Some(crate::client::HARD_ROW_CAP),
        )
        .await;

    if let Some(mut s) = state.notebook_sessions.get_mut(session_key) {
        s.active_query_id = None;
    }

    let result = result.map_err(|e| CommandError::new("query_failed", e))?;
    let (result, _query_id) = result;
    let duration_ms = start.elapsed().as_millis() as u64;

    let output = CellOutput::Table(TableOutput {
        columns: result.columns,
        rows: result.rows,
        total_count: None,
        is_truncated: result.truncated,
        page_size: DEFAULT_CELL_PAGE_SIZE,
        is_wrappable: pageable,
        rows_affected: result.rows_affected,
    });

    let exec_order = state
        .notebook_sessions
        .get_mut(session_key)
        .map(|mut s| s.next_execution_order())
        .ok_or_else(|| CommandError::new("not_found", "notebook session not found"))?;

    let _ = channel.send(NotebookEvent::CellDone {
        cell_id: cell_id.to_string(),
        output: output.clone(),
        ai_state: None,
        execution_order: exec_order,
        duration_ms,
    });

    log::info!(
        "notebook SQL cell {cell_id} done: {} rows, {duration_ms}ms",
        result.row_count
    );
    Ok(output)
}

// ── NotebookAgentSink ─────────────────────────────────────────────────────

struct NotebookAgentSink {
    cell_id: String,
    channel: tauri::ipc::Channel<NotebookEvent>,
    /// Accumulated thinking content for the final message.
    thinking_content: Arc<std::sync::Mutex<String>>,
    /// Accumulated tool calls.
    tool_calls: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
    /// Last run_readonly_query SQL (for final_sql).
    final_sql: Arc<std::sync::Mutex<Option<String>>>,
    /// Final text response from the AI.
    final_message: Arc<std::sync::Mutex<Option<String>>>,
}

impl AgentSink for NotebookAgentSink {
    fn event(&self, event: AiEvent) {
        match event {
            AiEvent::Thinking { content } => {
                // Accumulate thinking for final ai_state
                {
                    let mut buf = self.thinking_content.lock().unwrap();
                    buf.push_str(&content);
                }
                // Stream thinking chunk to frontend
                let _ = self.channel.send(NotebookEvent::ThinkingChunk {
                    cell_id: self.cell_id.clone(),
                    chunk: content,
                });
            }
            AiEvent::Text { .. } => {
                // Intermediate text responses — not streamed for notebook cells.
                // The final text is captured in the agent loop's Done event.
            }
            AiEvent::ToolCalls { tools } => {
                for tool in &tools {
                    let tool_json = serde_json::json!({
                        "id": tool.id,
                        "name": tool.name,
                        "args": tool.args,
                    });
                    // Track the last run_readonly_query for final_sql classification
                    if tool.name == "run_readonly_query" {
                        if let Some(sql) = tool.args.get("sql").and_then(|v| v.as_str()) {
                            let mut fs = self.final_sql.lock().unwrap();
                            *fs = Some(sql.to_string());
                        }
                    }
                    // Accumulate for final ai_state
                    {
                        let mut tc = self.tool_calls.lock().unwrap();
                        tc.push(tool_json.clone());
                    }
                    // Stream tool call to frontend
                    let _ = self.channel.send(NotebookEvent::ToolCall {
                        cell_id: self.cell_id.clone(),
                        tool: tool_json,
                    });
                }
            }
            AiEvent::ToolResult { id, output, .. } => {
                // Attach the structured result to the matching tool call so
                // classify_ai_output can surface tables (the AI Table tab). The
                // agent emits the query_result shape; classify expects the
                // notebook `table` shape, so convert here.
                if let Some(out) = output {
                    let mut tc = self.tool_calls.lock().unwrap();
                    if let Some(t) = tc
                        .iter_mut()
                        .rev()
                        .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
                    {
                        if out.get("type").and_then(|v| v.as_str()) == Some("query_result") {
                            let cols: Vec<serde_json::Value> = out
                                .get("columns")
                                .and_then(|v| v.as_array())
                                .map(|arr| {
                                    arr.iter()
                                        .map(|c| {
                                            serde_json::json!({
                                                "name": c.get("name"),
                                                "type_name": c.get("type"),
                                            })
                                        })
                                        .collect()
                                })
                                .unwrap_or_default();
                            t["output"] = serde_json::json!({
                                "table": {
                                    "columns": cols,
                                    "rows": out.get("rows").cloned().unwrap_or_else(|| serde_json::json!([])),
                                    "total_count": out.get("row_count").cloned(),
                                    "is_truncated": out.get("truncated").cloned().unwrap_or(serde_json::json!(false)),
                                }
                            });
                        }
                    }
                }
            }
            AiEvent::QueryResult { .. } => {
                // Internal — the agent loop routes results back to the model.
                // The frontend receives the final aggregated state via CellDone.
            }
            AiEvent::Notice { .. } => {
                // System notice (e.g. DB tools unavailable): notebook cells
                // don't render notes — the cell simply continues.
            }
            AiEvent::Done { final_message, .. } => {
                // Capture the final text response
                let mut msg = self.final_message.lock().unwrap();
                *msg = Some(final_message);
            }
        }
    }

    fn dml_approval(&self, payload: DmlApprovalPayload) {
        log::warn!(
            "DML requested in notebook AI cell '{}' ({}) — not yet supported in notebook cells. Use a SQL cell instead.",
            self.cell_id, payload.description
        );
    }
}

// ── AI cell execution ─────────────────────────────────────────────────────

/// The error a notebook AI cell produces when its agent tries to run DML.
/// The notebook sink cannot approve DML (its `dml_approval` is a log-only
/// refusal), so the cell must fail loudly instead of silently "succeeding"
/// with a dead preview card (E3).
fn ai_cell_dml_refusal_error() -> String {
    "The assistant wanted to run a DML statement, which isn't supported in \
     AI cells. Use a SQL cell instead."
        .into()
}

async fn run_ai_cell(
    cell: &CellModel,
    cells: &[CellModel],
    conn_id: ConnectionId,
    session_key: &str,
    channel: tauri::ipc::Channel<NotebookEvent>,
    state: &State<'_, AppState>,
) -> Result<CellOutput, CommandError> {
    let start = std::time::Instant::now();
    let cell_id = cell.id.clone();
    log::info!("AI cell '{cell_id}': starting agent loop");

    // ── Build notebook-context system prompt ──────────────────────────────
    let context = assemble_ai_context(cells, &cell_id, 5, 4000);
    let notebook_context_prompt = if context.is_empty() {
        String::new()
    } else {
        format!("The following cells have already run:\n{}\n\n", context)
    };

    let config = state.ai_config.read().await.clone();
    let connection_id_str = conn_id.0.to_string();

    let (system_prompt, tier) = {
        let graph_guard = state.schema_graph.lock().await;
        let tier = graph_guard
            .as_ref()
            .map(|g| crate::ai::mschema::select_tier(g).0)
            .unwrap_or(crate::ai::mschema::ContextTier::Pull);
        let schema_prompt = if let Some(tree) = state.schema_cache.get(&connection_id_str) {
            let capabilities = state.capabilities().await;
            crate::ai::context::build_system_prompt(
                &tree,
                graph_guard.as_ref(),
                capabilities.as_ref(),
            )
        } else if let Some(g) = graph_guard.as_ref() {
            let db_name = connection_id_str
                .rsplit('/')
                .next()
                .unwrap_or(&connection_id_str)
                .to_string();
            let tree = crate::ai::context::tree_from_graph(db_name, g);
            let capabilities = state.capabilities().await;
            crate::ai::context::build_system_prompt(&tree, Some(g), capabilities.as_ref())
        } else {
            "Database context not yet loaded.".to_string()
        };
        (schema_prompt, tier)
    };

    let full_prompt = format!(
        "You are working in a SQL notebook.\n\n\
         {notebook_context_prompt}\
\
         When asked for data, write and execute a query.\n\
         When asked for explanation, respond with text.\n\n\
         {system_prompt}"
    );

    log::info!(
        "AI cell '{cell_id}': system prompt built ({} bytes)",
        full_prompt.len()
    );

    // ── Load API key ──────────────────────────────────────────────────────
    log::info!("AI cell '{cell_id}': loading API key");
    let cached = {
        let guard = state.api_key_cache.read().await;
        cached_api_key(&guard, &config.provider)
    };
    let api_key = match cached {
        Some(k) => k,
        None => {
            let key = load_api_key(&config).map_err(|e| CommandError::new("ai_config", e))?;
            *state.api_key_cache.write().await = Some((config.provider.clone(), key.clone()));
            key
        }
    };

    // ── Create LLM provider ───────────────────────────────────────────────
    log::info!("AI cell '{cell_id}': creating LLM provider");
    let provider: Arc<dyn LlmProvider> = Arc::new(RigProvider::new(
        config.provider.clone(),
        api_key,
        config.endpoint.clone(),
    ));

    // ── Create AI tool context with notebook connection ───────────────────
    let tool_ctx = AiToolContext {
        db: state.client.clone(),
        connection_id: Some(conn_id),
        capabilities: state.capabilities().await,
        config: config.clone(),
        schema_graph: state.schema_graph.clone(),
        embedder: state.embedder.clone(),
        reranker: state.reranker.clone(),
    };

    // ── Pre-flight: augment message with schema context ───────────────────
    log::info!("AI cell '{cell_id}': running pre-flight");
    let augmented_message = {
        let graph_guard = tool_ctx.schema_graph.lock().await;
        let emb_guard = tool_ctx.embedder.lock().await;
        let result = crate::ai::preflight::run_preflight(
            tool_ctx.connection_id,
            Some(&tool_ctx.db),
            graph_guard.as_ref(),
            emb_guard.as_ref(),
            &tier,
            &cell.source,
            tool_ctx.capabilities.as_ref(),
        )
        .await;
        match result {
            Some(block) => format!("{}\n\n{}", cell.source, block),
            None => cell.source.clone(),
        }
    };
    log::info!("AI cell '{cell_id}': pre-flight complete");

    // ── Set up cancellation and conversation state ────────────────────────
    let cancel = tokio_util::sync::CancellationToken::new();
    // E2: register the cell run so notebook_cancel_cell (the Stop button)
    // can stop the agent loop. Cleared on every exit path below.
    if let Some(mut s) = state.notebook_sessions.get_mut(session_key) {
        s.active_ai_cell = Some((cell_id.clone(), cancel.clone()));
    }
    let conv = Arc::new(tokio::sync::Mutex::new(ConversationState::new(
        cell_id.clone(),
    )));

    // ── Create notebook event sink ────────────────────────────────────────
    let sink_channel = channel.clone();
    let sink = Arc::new(NotebookAgentSink {
        cell_id: cell_id.clone(),
        channel: sink_channel,
        thinking_content: Arc::new(std::sync::Mutex::new(String::new())),
        tool_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
        final_sql: Arc::new(std::sync::Mutex::new(None)),
        final_message: Arc::new(std::sync::Mutex::new(None)),
    });

    // Send thinking started
    let _ = channel.send(NotebookEvent::ThinkingStarted {
        cell_id: cell_id.clone(),
    });

    // ── Create tools and agent ────────────────────────────────────────────
    log::info!("AI cell '{cell_id}': building agent");
    let tools = crate::ai::tools::all_tools(tool_ctx.clone());
    let agent = DatabaseAgent::new(provider, tools, tool_ctx);

    // ── Run the agent loop (with 5-minute timeout) ────────────────────────
    log::info!("AI cell '{cell_id}': entering agent loop");
    const AGENT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
    let chat_result = tokio::time::timeout(
        AGENT_TIMEOUT,
        agent.chat(
            augmented_message,
            &config,
            full_prompt,
            conv.clone(),
            sink.clone(),
            cancel,
        ),
    )
    .await;

    // E2: the run is over (success, error, or timeout) — unregister the cell
    // so the Stop button stops targeting it. Must happen before the match
    // below because every arm of it is a terminal exit path. Guarded by cell
    // id: a newer overlapping run may have replaced this registration, and
    // wiping it would silently make that run uncancellable (D3).
    if let Some(mut s) = state.notebook_sessions.get_mut(session_key) {
        clear_active_ai_cell_if_same(&mut s.active_ai_cell, &cell_id);
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    log::info!("AI cell '{cell_id}': agent loop completed in {duration_ms}ms");

    let _ = channel.send(NotebookEvent::ThinkingDone {
        cell_id: cell_id.clone(),
        duration_ms,
    });

    // Handle agent errors / timeouts
    match chat_result {
        Ok(Ok(())) => {
            // E3: if the agent paused for DML approval, refuse loudly. The
            // conversation is per-cell and discarded anyway — reset the
            // leaked PausedForDml state so nothing downstream misreads it.
            let paused_for_dml = matches!(conv.lock().await.state, AgentState::PausedForDml { .. });
            if paused_for_dml {
                conv.lock().await.state = AgentState::Idle;
                let error = CellError::QueryError {
                    message: ai_cell_dml_refusal_error(),
                    sql_error: String::new(),
                };
                let _ = channel.send(NotebookEvent::CellError {
                    cell_id: cell_id.clone(),
                    error,
                });
                return Err(CommandError::new(
                    "dml_not_supported",
                    ai_cell_dml_refusal_error(),
                ));
            }
        }
        Ok(Err(e)) => {
            log::error!("AI cell '{cell_id}' agent error: {e}");
            let error = CellError::QueryError {
                message: format!("agent error: {e}"),
                sql_error: String::new(),
            };
            let _ = channel.send(NotebookEvent::CellError {
                cell_id: cell_id.clone(),
                error,
            });
            return Err(CommandError::new("agent_error", e.to_string()));
        }
        Err(_) => {
            log::error!("AI cell '{cell_id}' agent timed out after 300s");
            let error = CellError::QueryError {
                message: "Agent timed out after 300 seconds. Try simplifying the question.".into(),
                sql_error: String::new(),
            };
            let _ = channel.send(NotebookEvent::CellError {
                cell_id: cell_id.clone(),
                error,
            });
            return Err(CommandError::new("agent_timeout", "timed out after 300s"));
        }
    }

    // ── Build final output and ai_state ───────────────────────────────────
    let tool_calls = sink.tool_calls.lock().unwrap().clone();
    let thinking_content = sink.thinking_content.lock().unwrap().clone();
    let final_sql = sink.final_sql.lock().unwrap().clone();
    let final_message = sink.final_message.lock().unwrap().clone();

    let output = classify_ai_output(&tool_calls, final_message.as_deref());

    let mut messages: Vec<serde_json::Value> = Vec::new();
    if !thinking_content.is_empty() {
        messages.push(serde_json::json!({
            "thinking": thinking_content,
        }));
    }

    let ai_state = AiCellState {
        conversation_id: cell_id.clone(),
        final_sql,
        response: final_message,
        messages,
        tool_calls,
    };

    let exec_order = state
        .notebook_sessions
        .get_mut(session_key)
        .map(|mut s| s.next_execution_order())
        .ok_or_else(|| CommandError::new("not_found", "notebook session not found"))?;

    log::info!(
        "AI cell '{cell_id}': done, output type={:?}, exec_order={exec_order}",
        std::mem::discriminant(&output)
    );

    let _ = channel.send(NotebookEvent::CellDone {
        cell_id: cell_id.clone(),
        output: output.clone(),
        ai_state: Some(ai_state),
        execution_order: exec_order,
        duration_ms,
    });

    Ok(output)
}

#[tauri::command]
pub async fn notebook_cancel_cell(
    session_key: String,
    _cell_id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let session = state
        .notebook_sessions
        .get(&session_key)
        .ok_or_else(|| CommandError::new("not_found", "notebook session not found"))?;
    let conn_id = session.connection_id;
    let target = resolve_cancel_target(session.active_query_id, &session.active_ai_cell);
    // Drop the session read guard before any branch runs: the shard's RwLock
    // is not reentrant, and the branches below take a write lock via
    // get_mut on the same key (parking_lot would otherwise deadlock here).
    drop(session);

    match target {
        CancelTarget::None => Err(CommandError::new(
            "no_active_query",
            "no query or AI cell is running for this notebook",
        )),
        CancelTarget::Query(query_id) => {
            let client = state
                .client_handle()
                .await
                .ok_or_else(|| CommandError::new("not_connected", "no active connection"))?;
            client
                .cancel(conn_id, query_id)
                .await
                .map_err(|e| CommandError::new("cancel_failed", e))?;
            if let Some(mut s) = state.notebook_sessions.get_mut(&session_key) {
                s.active_query_id = None;
            }
            Ok(())
        }
        CancelTarget::AiCell { cell_id, token } => {
            token.cancel();
            log::info!("notebook_cancel_cell: cancelled AI cell '{cell_id}'");
            if let Some(mut s) = state.notebook_sessions.get_mut(&session_key) {
                // D3: clear only the registration that was actually cancelled —
                // a newer overlapping run may have replaced it since the cancel
                // target was resolved, and wiping it would make that run
                // uncancellable.
                clear_active_ai_cell_if_same(&mut s.active_ai_cell, &cell_id);
            }
            Ok(())
        }
    }
}

#[tauri::command]
pub async fn notebook_clear_outputs(
    session_key: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let mut session = state
        .notebook_sessions
        .get_mut(&session_key)
        .ok_or_else(|| CommandError::new("not_found", "notebook session not found"))?;
    session.reset_execution_counter();
    Ok(())
}

#[tauri::command]
pub async fn notebook_resolve_refs(
    session_key: String,
    cell_id: String,
    cells: Vec<CellModel>,
    state: State<'_, AppState>,
) -> Result<ResolvedQuery, CommandError> {
    let _session = state
        .notebook_sessions
        .get(&session_key)
        .ok_or_else(|| CommandError::new("not_found", "notebook session not found"))?;

    // Resolving refs is advisory SQL composition — it must keep working for
    // offline notebook editing. When no connection is live there are no
    // capabilities, so fall back to the Postgres dialect (the only dialect in
    // this build); the execution paths error with "not connected" first, so a
    // missing dialect never reaches a parser on a runnable query.
    let dialect = state
        .capabilities()
        .await
        .map(|c| c.sql_dialect)
        .unwrap_or(lucent_protocol::SqlDialect::PostgreSql);

    let dag = rewrite::build_dag(&cell_id, &cells, dialect).map_err(|e| {
        CommandError::new("dag_failed", serde_json::to_string(&e).unwrap_or_default())
    })?;
    let order = rewrite::topological_sort(&dag);
    let cte_chain: Vec<String> = order.iter().map(|id| format!("_cell_{}", id)).collect();
    let final_sql = rewrite::rewrite_sql(&cell_id, &cells, dialect).map_err(|e| {
        CommandError::new(
            "rewrite_failed",
            serde_json::to_string(&e).unwrap_or_default(),
        )
    })?;

    Ok(ResolvedQuery {
        cte_chain,
        final_sql,
        errors: vec![],
    })
}

pub fn assemble_ai_context(
    cells: &[CellModel],
    current_cell_id: &str,
    max_cells: usize,
    token_budget: usize,
) -> String {
    use crate::ai::mschema::estimate_tokens;
    let mut prior: Vec<&CellModel> = cells
        .iter()
        .filter(|c| c.id != current_cell_id && c.status == CellStatus::Ok && c.outputs.is_some())
        .collect();
    prior.reverse();
    prior.truncate(max_cells);
    prior.reverse();

    let mut lines = Vec::new();
    let mut token_count = 0;
    for cell in &prior {
        let summary = match &cell.outputs {
            Some(CellOutput::Table(t)) => {
                let cols: Vec<String> = t.columns.iter().map(|c| c.name.clone()).collect();
                // `total_count` is only `Some` once the user has explicitly asked for a
                // true count (e.g. via notebook_count_rows). A fresh run always yields
                // `None` with `rows` capped at the page size, so an unknown count must
                // never be presented as though it were the whole result set.
                let row_desc = match t.total_count {
                    Some(n) => format!("{n} rows"),
                    None => format!("showing {} rows (total unknown)", t.rows.len()),
                };
                format!(
                    "[Cell {}] (SQL) — {row_desc}, columns: {}",
                    cell.id,
                    cols.join(", ")
                )
            }
            Some(CellOutput::Text(t)) => {
                let preview: String = t.content.chars().take(200).collect();
                format!("[Cell {}] (text) — {}", cell.id, preview)
            }
            None => continue,
        };
        token_count += estimate_tokens(&summary);
        if token_count > token_budget {
            break;
        }
        lines.push(summary);
    }
    lines.join("\n")
}

pub fn classify_ai_output(
    tool_calls: &[serde_json::Value],
    final_message: Option<&str>,
) -> CellOutput {
    // Check for SQL query results first
    for tc in tool_calls.iter().rev() {
        let name = tc.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if name == "execute_sql" || name == "run_readonly_query" {
            let args = tc
                .get("args")
                .and_then(|v| v.get("sql"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let trimmed = args.trim().to_uppercase();
            let is_tabular = trimmed.starts_with("SELECT")
                || (trimmed.contains("RETURNING")
                    && (trimmed.starts_with("INSERT")
                        || trimmed.starts_with("UPDATE")
                        || trimmed.starts_with("DELETE")))
                || trimmed.starts_with("EXPLAIN");
            if is_tabular {
                if let Some(output) = tc.get("output") {
                    if let Some(table) = output.get("table") {
                        return serde_json::from_value(table.clone()).unwrap_or(CellOutput::Text(
                            TextOutput {
                                content: String::new(),
                            },
                        ));
                    }
                }
            }
        }
    }
    // Fall back to text response
    let content = if let Some(msg) = final_message {
        msg.to_string()
    } else {
        let summaries: Vec<&str> = tool_calls
            .iter()
            .filter_map(|tc| tc.get("summary").and_then(|v| v.as_str()))
            .collect();
        summaries.join("\n")
    };
    CellOutput::Text(TextOutput { content })
}

#[cfg(test)]
mod ai_context_tests {
    use super::*;
    use lucent_protocol::ColumnMeta;

    fn table_cell(id: &str, total_count: Option<u64>, row_count: usize) -> CellModel {
        CellModel {
            id: id.to_string(),
            kind: CellKind::Sql,
            source: "SELECT 1".to_string(),
            alias: None,
            collapsed: false,
            outputs: Some(CellOutput::Table(TableOutput {
                columns: vec![ColumnMeta {
                    name: "x".into(),
                    type_name: "int4".into(),
                }],
                rows: vec![vec![serde_json::json!(1)]; row_count],
                total_count,
                is_truncated: false,
                page_size: 10,
                is_wrappable: true,
                rows_affected: None,
            })),
            status: CellStatus::Ok,
            execution_order: Some(1),
            duration_ms: Some(5),
            error: None,
            stale_since: None,
            ai_state: None,
        }
    }

    /// A fresh run always yields `total_count: None` with `rows` capped at the page
    /// size (e.g. 10). The AI context must never present that page size as though
    /// it were the true row count of a cell that may have queried millions.
    #[test]
    fn unknown_total_count_is_not_presented_as_a_true_count() {
        let cell = table_cell("c1", None, 10);
        let ctx = assemble_ai_context(&[cell], "current", 5, 4000);
        assert!(
            ctx.contains("showing 10 rows"),
            "expected an explicit 'showing' qualifier, got {ctx}"
        );
        assert!(
            !ctx.contains("— 10 rows,"),
            "must not present the unknown count as a bare, confident number: got {ctx}"
        );
    }

    #[test]
    fn known_total_count_is_reported_verbatim() {
        let cell = table_cell("c1", Some(42), 10);
        let ctx = assemble_ai_context(&[cell], "current", 5, 4000);
        assert!(ctx.contains("— 42 rows,"), "got {ctx}");
    }

    /// A genuinely empty result with a known count of 0 must still say 0 — the
    /// branching keys off `None` vs `Some`, not off `rows.is_empty()`.
    #[test]
    fn known_zero_total_count_still_says_zero() {
        let cell = table_cell("c1", Some(0), 0);
        let ctx = assemble_ai_context(&[cell], "current", 5, 4000);
        assert!(ctx.contains("— 0 rows,"), "got {ctx}");
        assert!(!ctx.contains("showing"), "got {ctx}");
    }
}

#[cfg(test)]
mod notebook_open_tests {
    use super::read_notebook_file;

    #[test]
    fn unapproved_paths_are_rejected_before_any_read() {
        // S3: an ungated read is a local-file disclosure primitive from a
        // compromised renderer. Any path not chosen in a native dialog must
        // be refused with the same error the save path uses.
        let approved = std::collections::HashSet::new();
        let err = read_notebook_file(std::path::Path::new("/etc/hostname"), &approved)
            .expect_err("unapproved paths must be rejected");
        assert!(
            err.message.contains("native"),
            "error must name the dialog gate: {}",
            err.message
        );
    }

    #[test]
    fn approved_paths_are_read() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("nb.ln");
        std::fs::write(&path, "[]").unwrap();
        let mut approved = std::collections::HashSet::new();
        approved.insert(path.canonicalize().unwrap());
        let content = read_notebook_file(&path, &approved).expect("approved read must succeed");
        assert_eq!(content, "[]");
    }

    #[test]
    fn cancel_prefers_the_active_db_query_then_the_ai_cell() {
        use super::{resolve_cancel_target, CancelTarget};
        use lucent_protocol::QueryId;
        use uuid::Uuid;

        let cancel = tokio_util::sync::CancellationToken::new();
        // A SQL query is running: it wins.
        let qid = QueryId(Uuid::new_v4());
        assert!(matches!(
            resolve_cancel_target(Some(qid), &Some(("c1".into(), cancel.clone()))),
            CancelTarget::Query(got) if got == qid
        ));
        // Only an AI cell is running: the token comes back.
        assert!(matches!(
            resolve_cancel_target(None, &Some(("c1".into(), cancel.clone()))),
            CancelTarget::AiCell { ref cell_id, .. } if cell_id == "c1"
        ));
        // Nothing running.
        assert!(matches!(
            resolve_cancel_target(None, &None),
            CancelTarget::None
        ));
    }

    #[test]
    fn clear_active_ai_cell_only_when_it_names_this_cell() {
        // D3: an older run finishing (or a stale cancel) must never wipe a
        // NEWER overlapping run's registration — that would silently make the
        // newer run uncancellable.
        use super::clear_active_ai_cell_if_same;

        let token = tokio_util::sync::CancellationToken::new();
        let mut active = Some(("cell_a".to_string(), token.clone()));
        // A different cell's completion: the newer registration survives.
        clear_active_ai_cell_if_same(&mut active, "cell_b");
        assert!(
            active.is_some(),
            "a different cell's run must not wipe this registration"
        );
        // This cell's own completion: cleared.
        clear_active_ai_cell_if_same(&mut active, "cell_a");
        assert!(
            active.is_none(),
            "this cell's own run clears its registration"
        );
        // No registration: no-op, no panic.
        clear_active_ai_cell_if_same(&mut active, "cell_a");
        assert!(active.is_none());
    }

    #[test]
    fn dml_refusal_error_names_the_fix() {
        use super::ai_cell_dml_refusal_error;
        // E3: the notebook sink cannot approve DML, so a cell whose agent
        // paused for approval must fail with a message that tells the user
        // what to do — never a silent "success" with a dead preview card.
        let err = ai_cell_dml_refusal_error();
        assert!(err.contains("SQL cell"), "{err}");
        assert!(err.contains("DML"), "{err}");
    }
}
