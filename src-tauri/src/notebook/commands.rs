use std::path::PathBuf;
use std::sync::Arc;

use lucent_protocol::{ConnectionId, QueryId};
use tauri::State;
use uuid::Uuid;

use crate::ai::agent::{AgentSink, AgentState, ConversationState};
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
    /// ACP only: the state and session key needed to answer a
    /// `session/request_permission`. A notebook cell has no approval UI, so
    /// the request must still be *resolved* — leaving it parked would block
    /// the agent's turn until the 300-second timeout.
    acp: Option<(crate::ai::acp::AcpState, String)>,
    /// Permission requests this cell refused, surfaced in the cell's error so
    /// a user who wonders why the agent gave up can see what it asked for.
    denied_permissions: Arc<std::sync::Mutex<Vec<String>>>,
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
            AiEvent::ToolResult {
                id,
                tool,
                summary,
                input,
                output,
                ..
            } => {
                // Everything below repairs what the agent's own report of a
                // call cannot tell us. On the ACP CLI path the announced call
                // is an opaque shell command with `args: null`, so the tool
                // name, its arguments and its result are only knowable here,
                // where the bridge reports what it actually executed.
                if tool == "run_readonly_query" {
                    if let Some(sql) = input
                        .as_ref()
                        .and_then(|i| i.get("sql"))
                        .and_then(|v| v.as_str())
                    {
                        let mut fs = self.final_sql.lock().unwrap();
                        *fs = Some(sql.to_string());
                    }
                }
                {
                    let mut tc = self.tool_calls.lock().unwrap();
                    if let Some(t) = tc
                        .iter_mut()
                        .rev()
                        .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
                    {
                        // `classify_ai_output` keys the AI Table tab off the
                        // tool NAME and its `sql` argument; both are missing on
                        // the CLI path, which is why those cells produced no
                        // table at all. Only fill what the call itself lacked.
                        if !tool.is_empty() && !is_tool_id(&announced_name(t)) {
                            t["name"] = serde_json::json!(tool);
                        }
                        if let Some(inp) = &input {
                            if t.get("args").map(args_missing).unwrap_or(true) {
                                t["args"] = inp.clone();
                            }
                        }
                        if !summary.is_empty() {
                            t["summary"] = serde_json::json!(summary);
                        }
                        if let Some(out) = &output {
                            // Both shapes, deliberately: `type` is what the
                            // tool card renders (the chat pane's card, reused
                            // here), `table` is what `classify_ai_output`
                            // converts into the cell's Table tab.
                            let mut stored = out.clone();
                            if out.get("type").and_then(|v| v.as_str()) == Some("query_result") {
                                stored["table"] = query_result_as_table(out);
                            }
                            t["output"] = stored;
                        }
                    }
                }
                // Stream it: the card fills in during the run instead of
                // staying a bare "Done" row until `cell_done` lands.
                let _ = self.channel.send(NotebookEvent::ToolResult {
                    cell_id: self.cell_id.clone(),
                    id,
                    tool,
                    summary,
                    input,
                    output,
                });
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

    /// An ACP agent asked to run one of its *own* tools (a shell command, a
    /// file write). A notebook cell has no approval card, and the default
    /// no-op would leave the request parked — the agent's turn would then
    /// block until the 300-second timeout. So refuse it, promptly and
    /// visibly: the agent gets a well-formed `Cancelled` outcome and can
    /// choose another route (Lucent's database tools need no permission), and
    /// the refusal is recorded so the cell can say what was asked for.
    ///
    /// Lucent's own tools are unaffected: they run behind the bridge with the
    /// guardrails in-process, and never go through this path.
    fn permission_request(&self, payload: crate::ai::events::AgentPermissionPayload) {
        log::warn!(
            "AI cell '{}': agent asked permission for '{}' — notebook cells have no approval \
             dialog, so the request is refused.",
            self.cell_id,
            payload.title
        );
        self.denied_permissions
            .lock()
            .unwrap()
            .push(payload.title.clone());
        // Resolving is async; the sink contract is sync. The registry is
        // keyed by ACP session, which `resolve_permission_session` looks up
        // from the conversation key this cell's session was created under.
        if let Some((acp, conv_key)) = self.acp.clone() {
            let cell_id = self.cell_id.clone();
            tokio::spawn(async move {
                let session_id = {
                    let sessions = acp.sessions.lock().await;
                    sessions.get(&conv_key).map(|s| s.session_id.clone())
                };
                match session_id {
                    Some(sid) => {
                        if let Err(e) = acp.permissions.respond(&sid, false).await {
                            log::warn!("AI cell '{cell_id}': refusing permission failed: {e}");
                        }
                    }
                    None => log::warn!(
                        "AI cell '{cell_id}': no ACP session for '{conv_key}' — permission \
                         request left unresolved"
                    ),
                }
            });
        }
    }
}

// ── AI cell execution ─────────────────────────────────────────────────────

/// Whether a name is one of Lucent's tool ids rather than an agent's free-text
/// title. ACP carries no tool name — a bash-first agent's "name" is the whole
/// shell command — so this is how we tell a real name from a placeholder.
fn is_tool_id(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Whether a call's announced arguments tell us nothing, so a result that
/// carries some may fill them in. Null, an empty object, an empty array and a
/// blank string all mean the same thing here.
///
/// The distinction matters because an ACP agent announces a call with
/// `raw_input: {}` before its arguments are settled, not with nothing at all.
/// A guard that only asked `is_null()` read `{}` as arguments the agent had
/// really reported and refused the backfill — which is why notebook tool cards
/// rendered `INPUT {}` beside an output full of rows. Falsy scalars (`0`,
/// `false`) are values the agent chose to send, not the absence of one.
fn args_missing(args: &serde_json::Value) -> bool {
    match args {
        serde_json::Value::Null => true,
        serde_json::Value::String(s) => s.trim().is_empty(),
        serde_json::Value::Array(a) => a.is_empty(),
        serde_json::Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// The `name` a tool call was announced under, or "" when it carried none.
fn announced_name(call: &serde_json::Value) -> String {
    call.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// The tool card's `query_result` payload as the notebook's `TableOutput`
/// shape. The two disagree on one field name (`type` vs `type_name`), and the
/// cell's Table tab is deserialized straight from this.
fn query_result_as_table(out: &serde_json::Value) -> serde_json::Value {
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
    serde_json::json!({
        "columns": cols,
        "rows": out.get("rows").cloned().unwrap_or_else(|| serde_json::json!([])),
        "total_count": out.get("row_count").cloned(),
        "is_truncated": out.get("truncated").cloned().unwrap_or(serde_json::json!(false)),
    })
}

/// The ACP session key for one AI-cell run. Notebook cells are stateless per
/// run — the rig path builds a fresh `ConversationState` and starts from an
/// empty history every time — so the key is scoped to the notebook AND the
/// cell, and the session is dropped when the run ends. Keying on the cell
/// alone (without the drop) would let a re-run continue the previous
/// conversation, quietly making the same cell answer differently the second
/// time; keying on the notebook alone would merge every cell's context.
fn acp_cell_session_key(session_key: &str, cell_id: &str) -> String {
    format!("notebook:{session_key}:{cell_id}")
}

/// The note appended to a cell's response when Lucent refused permission
/// requests on its behalf. `denied` is never empty at the call site.
fn permission_refusal_note(denied: &[String]) -> String {
    format!(
        "_Lucent refused {} permission request{} from the agent ({}). \
         Notebook cells have no approval dialog — ask in the chat panel if \
         the agent needs to run its own tools._",
        denied.len(),
        if denied.len() == 1 { "" } else { "s" },
        denied.join(", ")
    )
}

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
        let capabilities = state.capabilities().await;
        let current_db = state.current_database.lock().await.clone();
        let schema_prompt = if let Some(mut tree) = state.schema_cache.get(&connection_id_str) {
            if let Some(db) = &current_db {
                if tree.database_name.is_empty() || tree.database_name == connection_id_str {
                    tree.database_name = db.clone();
                }
            }
            crate::ai::context::build_system_prompt(
                &tree,
                graph_guard.as_ref(),
                capabilities.as_ref(),
            )
        } else if let Some(g) = graph_guard.as_ref() {
            let db_name = current_db
                .unwrap_or_else(|| crate::ai::context::parse_database_name(&connection_id_str));
            let version = state
                .client_handle()
                .await
                .and_then(|c| c.server_info.map(|s| s.version))
                .unwrap_or_default();
            let tree = crate::ai::context::tree_from_graph_with_version(db_name, version, g);
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

    // ── ACP branch point ──────────────────────────────────────────────────
    // With `acp` configured the whole rig section below is skipped: ACP
    // agents own their auth, and `AiProvider::Acp` has no rig client to
    // build (constructing one yielded `StubAgent`, which is what surfaced as
    // "Provider not configured: RigAgent construction failed" on the first
    // turn). The driver seam is the same one `ai_chat` uses.
    let is_acp = config.acp.is_some();

    let provider: Option<Arc<dyn LlmProvider>> = if is_acp {
        None
    } else {
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
        log::info!("AI cell '{cell_id}': creating LLM provider");
        Some(Arc::new(RigProvider::new(
            config.provider.clone(),
            api_key,
            config.endpoint.clone(),
        )))
    };

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

    // The ACP driver keys its session-per-conversation map by this. Notebook
    // cells are stateless per run — the rig path builds a fresh
    // `ConversationState` every time and starts from an empty history — so
    // each run gets its own ACP session, dropped on the way out. A key that
    // included only the cell id would instead let a re-run continue the
    // previous conversation, quietly making the same cell answer differently
    // the second time.
    let acp_session_key = acp_cell_session_key(session_key, &cell_id);
    if is_acp {
        conv.lock().await.conversation_id = Some(acp_session_key.clone());
    }

    // ── Create notebook event sink ────────────────────────────────────────
    let sink_channel = channel.clone();
    let sink = Arc::new(NotebookAgentSink {
        cell_id: cell_id.clone(),
        channel: sink_channel,
        thinking_content: Arc::new(std::sync::Mutex::new(String::new())),
        tool_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
        final_sql: Arc::new(std::sync::Mutex::new(None)),
        final_message: Arc::new(std::sync::Mutex::new(None)),
        acp: is_acp.then(|| (state.acp.clone(), acp_session_key.clone())),
        denied_permissions: Arc::new(std::sync::Mutex::new(Vec::new())),
    });

    // Send thinking started
    let _ = channel.send(NotebookEvent::ThinkingStarted {
        cell_id: cell_id.clone(),
    });

    // ── Create tools and agent ────────────────────────────────────────────
    log::info!("AI cell '{cell_id}': building agent (acp={is_acp})");
    let tools = if is_acp {
        Vec::new() // ACP tools live behind the bridge — the agent calls them over MCP or the CLI helper.
    } else {
        crate::ai::tools::all_tools(tool_ctx.clone())
    };
    let agent =
        crate::commands::pick_driver(&config.acp, provider, tools, tool_ctx, state.acp.clone());

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

    // Same reasoning as the per-run session key: the run is over, so the ACP
    // session goes with it. Dropping here (before every `return` below) also
    // keeps a notebook from accumulating one live session per AI cell.
    if is_acp {
        state.acp.drop_session(&acp_session_key).await;
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
    let mut final_message = sink.final_message.lock().unwrap().clone();

    // A refused permission request usually explains a thin answer ("I wasn't
    // able to run that"), so say what was refused rather than leaving the
    // user to guess. Appended to the response, not raised as an error: the
    // agent may well have answered the question by another route.
    let denied = sink.denied_permissions.lock().unwrap().clone();
    if !denied.is_empty() {
        let note = permission_refusal_note(&denied);
        final_message = Some(match final_message {
            Some(m) if !m.is_empty() => format!("{m}\n\n{note}"),
            _ => note,
        });
    }

    let output = classify_ai_output(&tool_calls, final_message.as_deref());

    let mut messages: Vec<serde_json::Value> = Vec::new();
    if !thinking_content.is_empty() {
        // One message holding the whole run's reasoning, so `durationMs` is the
        // run's duration. The live stream builds finer per-segment messages
        // (each timed on its own) and the frontend keeps those; this snapshot
        // is what a reopened notebook restores from, and without a duration its
        // card would render as still-thinking forever.
        messages.push(serde_json::json!({
            "thinking": thinking_content,
            "durationMs": duration_ms,
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
                    // `table` is the canonical shape. A card-shaped
                    // `query_result` is accepted too, so a result that only
                    // ever passed through the tool card still yields a table.
                    let table = output.get("table").cloned().or_else(|| {
                        (output.get("type").and_then(|v| v.as_str()) == Some("query_result"))
                            .then(|| query_result_as_table(output))
                    });
                    if let Some(table) = table {
                        return serde_json::from_value(table).unwrap_or(CellOutput::Text(
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

#[cfg(test)]
mod acp_cell_tests {
    use super::*;
    use crate::ai::events::{AgentPermissionPayload, ToolResultStatus};

    fn sink(acp: Option<(crate::ai::acp::AcpState, String)>) -> NotebookAgentSink {
        NotebookAgentSink {
            cell_id: "cell-1".into(),
            channel: tauri::ipc::Channel::new(|_| Ok(())),
            thinking_content: Arc::new(std::sync::Mutex::new(String::new())),
            tool_calls: Arc::new(std::sync::Mutex::new(Vec::new())),
            final_sql: Arc::new(std::sync::Mutex::new(None)),
            final_message: Arc::new(std::sync::Mutex::new(None)),
            acp,
            denied_permissions: Arc::new(std::sync::Mutex::new(Vec::new())),
        }
    }

    #[test]
    fn session_keys_are_scoped_to_the_notebook_and_the_cell() {
        let a = acp_cell_session_key("/nb/one.ln", "cell-1");
        assert_ne!(
            a,
            acp_cell_session_key("/nb/one.ln", "cell-2"),
            "two cells in one notebook must not share an ACP session"
        );
        assert_ne!(
            a,
            acp_cell_session_key("/nb/two.ln", "cell-1"),
            "the same cell id in two notebooks must not collide"
        );
        assert!(a.contains("cell-1") && a.contains("/nb/one.ln"), "{a}");
    }

    #[tokio::test]
    async fn a_permission_request_is_recorded_and_refused_not_left_parked() {
        // The sink's default `permission_request` is a no-op, which would park
        // the request forever: the agent's turn then blocks until the
        // 300-second timeout. A notebook cell has no approval card, so the
        // only honest answer is a prompt refusal.
        let s = sink(None);
        s.permission_request(AgentPermissionPayload {
            conversation_id: "notebook:/nb.ln:cell-1".into(),
            title: "Run shell command".into(),
            description: "ls -la".into(),
            options: vec![],
        });
        let denied = s.denied_permissions.lock().unwrap().clone();
        assert_eq!(denied, vec!["Run shell command".to_string()]);
    }

    #[tokio::test]
    async fn refusing_without_an_acp_session_does_not_panic() {
        // The rig path builds the sink with `acp: None`; a stray request there
        // must be recorded and dropped, never unwrap a missing session.
        let s = sink(Some((crate::ai::acp::AcpState::new(), "missing".into())));
        s.permission_request(AgentPermissionPayload {
            conversation_id: "missing".into(),
            title: "Write file".into(),
            description: String::new(),
            options: vec![],
        });
        assert_eq!(s.denied_permissions.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_refusal_note_names_what_was_asked_for() {
        let one = permission_refusal_note(&["Run shell command".into()]);
        assert!(one.contains("1 permission request "), "singular: {one}");
        assert!(one.contains("Run shell command"), "{one}");
        let two = permission_refusal_note(&["Run shell command".into(), "Write file".into()]);
        assert!(two.contains("2 permission requests"), "plural: {two}");
        assert!(two.contains("Write file"), "{two}");
    }

    #[test]
    fn final_sql_is_recovered_from_the_bridges_backfilled_arguments() {
        // The CLI path announces a shell command with `args: null`, so the
        // `ToolCalls` arm learns no SQL — the AI Table tab would stay empty.
        // The bridge's backfilled input on the result is where it shows up.
        let s = sink(None);
        s.event(AiEvent::ToolCalls {
            tools: vec![crate::ai::events::ToolCallInfo {
                id: "tc1".into(),
                name: "./lucent-tool run_readonly_query …".into(),
                args: serde_json::Value::Null,
            }],
        });
        assert!(
            s.final_sql.lock().unwrap().is_none(),
            "a shell command carries no SQL"
        );
        s.event(AiEvent::ToolResult {
            id: "tc1".into(),
            tool: "run_readonly_query".into(),
            summary: "3 rows".into(),
            output: None,
            input: Some(serde_json::json!({"sql": "SELECT 1"})),
            status: ToolResultStatus::Completed,
        });
        assert_eq!(
            s.final_sql.lock().unwrap().clone(),
            Some("SELECT 1".to_string()),
            "the executed SQL must reach the cell's ai_state"
        );
    }

    #[test]
    fn a_cli_path_call_is_backfilled_with_the_real_name_and_arguments() {
        // The cell's Table tab is classified from the tool's NAME and its
        // `sql` argument. An ACP agent on the CLI path announces neither —
        // its call is a shell command with `args: null` — so without this
        // backfill those cells produced no table at all.
        let s = sink(None);
        s.event(AiEvent::ToolCalls {
            tools: vec![crate::ai::events::ToolCallInfo {
                id: "tc1".into(),
                name: "./lucent-tool run_readonly_query 'SELECT 1'".into(),
                args: serde_json::Value::Null,
            }],
        });
        s.event(AiEvent::ToolResult {
            id: "tc1".into(),
            tool: "run_readonly_query".into(),
            summary: "1 rows".into(),
            output: Some(serde_json::json!({
                "type": "query_result",
                "columns": [{"name": "n", "type": "int4"}],
                "rows": [[1]],
                "row_count": 1,
                "sql": "SELECT 1",
                "execution_time_ms": 2,
                "truncated": false,
            })),
            input: Some(serde_json::json!({"sql": "SELECT 1"})),
            status: ToolResultStatus::Completed,
        });

        let calls = s.tool_calls.lock().unwrap().clone();
        assert_eq!(calls[0]["name"], "run_readonly_query", "{:?}", calls[0]);
        assert_eq!(calls[0]["args"]["sql"], "SELECT 1");
        assert_eq!(calls[0]["summary"], "1 rows");
        // Both shapes: `type` is what the tool card renders, `table` is what
        // `classify_ai_output` turns into the cell's Table tab.
        assert_eq!(calls[0]["output"]["type"], "query_result");
        assert_eq!(
            calls[0]["output"]["table"]["columns"][0]["type_name"],
            "int4"
        );

        match classify_ai_output(&calls, Some("here you go")) {
            CellOutput::Table(t) => assert_eq!(t.rows.len(), 1),
            other => panic!("a backfilled query call must classify as a table: {other:?}"),
        }
    }

    #[test]
    fn an_empty_object_of_arguments_is_backfilled_like_a_null() {
        // What actually reached users: an ACP agent announces the call with
        // `raw_input: {}` — not null — before its arguments are settled. A
        // guard that only asked `is_null()` read that as "the agent reported
        // its arguments, and there were none" and refused the backfill, so the
        // cell's tool card rendered `INPUT {}` next to an output full of rows.
        let s = sink(None);
        s.event(AiEvent::ToolCalls {
            tools: vec![crate::ai::events::ToolCallInfo {
                id: "tc1".into(),
                name: "Bash".into(),
                args: serde_json::json!({}),
            }],
        });
        s.event(AiEvent::ToolResult {
            id: "tc1".into(),
            tool: "run_readonly_query".into(),
            summary: "10 rows".into(),
            output: None,
            input: Some(serde_json::json!({"sql": "SELECT 1"})),
            status: ToolResultStatus::Completed,
        });
        let calls = s.tool_calls.lock().unwrap().clone();
        assert_eq!(
            calls[0]["args"]["sql"], "SELECT 1",
            "an empty object carries no arguments, so the bridge's must fill in: {:?}",
            calls[0]
        );
    }

    #[test]
    fn blank_arguments_are_recognised_whatever_shape_they_arrive_in() {
        for blank in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!([]),
            serde_json::json!(""),
        ] {
            assert!(args_missing(&blank), "{blank:?} carries no arguments");
        }
        for present in [
            serde_json::json!({"sql": "SELECT 1"}),
            serde_json::json!(["a"]),
            serde_json::json!("SELECT 1"),
            // A value the agent chose to send, not the absence of one.
            serde_json::json!(0),
            serde_json::json!(false),
        ] {
            assert!(!args_missing(&present), "{present:?} carries arguments");
        }
    }

    #[test]
    fn a_genuine_tool_name_and_arguments_are_never_overwritten() {
        // The MCP path reports both; the bridge must not clobber them.
        let s = sink(None);
        s.event(AiEvent::ToolCalls {
            tools: vec![crate::ai::events::ToolCallInfo {
                id: "tc1".into(),
                name: "run_readonly_query".into(),
                args: serde_json::json!({"sql": "SELECT 1"}),
            }],
        });
        s.event(AiEvent::ToolResult {
            id: "tc1".into(),
            tool: "something_else".into(),
            summary: "1 rows".into(),
            output: None,
            input: Some(serde_json::json!({"sql": "SOMETHING ELSE"})),
            status: ToolResultStatus::Completed,
        });
        let calls = s.tool_calls.lock().unwrap().clone();
        assert_eq!(calls[0]["name"], "run_readonly_query");
        assert_eq!(calls[0]["args"]["sql"], "SELECT 1");
    }

    #[test]
    fn classify_accepts_a_card_shaped_query_result() {
        let calls = vec![serde_json::json!({
            "name": "run_readonly_query",
            "args": {"sql": "SELECT 1 AS n"},
            "output": {
                "type": "query_result",
                "columns": [{"name": "n", "type": "int4"}],
                "rows": [[1]],
                "row_count": 1,
                "truncated": false,
            },
        })];
        match classify_ai_output(&calls, None) {
            CellOutput::Table(t) => {
                assert_eq!(t.columns[0].name, "n");
                assert_eq!(t.total_count, Some(1));
            }
            other => panic!("expected a table: {other:?}"),
        }
    }

    #[test]
    fn tool_id_recognition_separates_names_from_shell_commands() {
        assert!(is_tool_id("run_readonly_query"));
        assert!(is_tool_id("search_schema"));
        assert!(!is_tool_id("./lucent-tool run_readonly_query 'SELECT 1'"));
        assert!(!is_tool_id("Run Readonly Query"));
        assert!(!is_tool_id(""));
    }

    #[test]
    fn a_non_query_tool_never_sets_final_sql() {
        let s = sink(None);
        s.event(AiEvent::ToolResult {
            id: "tc1".into(),
            tool: "search_schema".into(),
            summary: "done".into(),
            output: None,
            input: Some(serde_json::json!({"query": "invoices"})),
            status: ToolResultStatus::Completed,
        });
        assert!(s.final_sql.lock().unwrap().is_none());
    }
}
