//! Main-process side of the `lucent-db-tools-mcp` bridge socket.
//!
//! Holds the `ToolExecutor` seam (injectable in tests; the real adapter runs
//! `LucentToolEnum::call` with the conversation's `AiToolContext`), the
//! `serve` loop (token auth, request dispatch, structured UI emission, the
//! DML approval hold), and the `BridgeClient` used by the MCP binary's tests.

use crate::ai::acp::wire;
use crate::ai::events::{AiEvent, DmlApprovalPayload};
use crate::ai::tools::{AiToolContext, LucentToolEnum, ToolError, ToolOutput};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, Notify};

#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn call(&self, tool: &str, args: serde_json::Value) -> Result<ToolOutput, ToolError>;
}

/// Executes through the real tool stack — the same path the rig loop uses
/// today. Holds the conversation's `AiToolContext` and the tool list built
/// once in `new` (exactly like the rig path builds `tools` once).
pub struct ContextToolExecutor {
    ctx: AiToolContext,
    tools: Vec<LucentToolEnum>,
}

impl ContextToolExecutor {
    pub fn new(ctx: AiToolContext) -> Self {
        let tools = crate::ai::tools::all_tools(ctx.clone());
        Self { ctx, tools }
    }
}

#[async_trait::async_trait]
impl ToolExecutor for ContextToolExecutor {
    async fn call(&self, tool: &str, args: serde_json::Value) -> Result<ToolOutput, ToolError> {
        match self.tools.iter().find(|t| t.name() == tool) {
            Some(t) => t.call(args, &self.ctx).await,
            None => Err(ToolError::Execution(format!("unknown tool: {tool}"))),
        }
    }
}

/// Live connectivity state of a conversation's DB-tools bridge: flips to
/// connected the moment the agent's MCP client completes the hello handshake
/// on the bridge socket. `lucent-db-tools-mcp` connects to the socket
/// immediately on spawn, so this is the ground-truth signal for whether the
/// ACP agent honored `session/new`'s `mcpServers` — the prompt must not
/// claim DB tools that never reached the agent.
#[derive(Default)]
pub struct BridgeConnection {
    connected: AtomicBool,
    notify: Notify,
}

impl BridgeConnection {
    /// Marks the bridge as connected (hello with the correct token arrived).
    /// Idempotent; wakes every `wait_connected` waiter.
    pub fn mark_connected(&self) {
        self.connected.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    /// Waits up to `timeout` for the bridge to connect, returning the state
    /// at the end. Returns `true` as soon as the hello lands (a late connect
    /// is still a connect); `false` only when the deadline passes without
    /// one. The loop re-checks after every wake so a notification that raced
    /// ahead of `notified()` registration is never lost.
    pub async fn wait_connected(&self, timeout: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.is_connected() {
                return true;
            }
            let remain = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remain.is_zero() {
                return false;
            }
            if tokio::time::timeout(remain, self.notify.notified())
                .await
                .is_err()
            {
                return false;
            }
        }
    }
}

/// The single in-flight DML approval for a conversation. One per conversation
/// — a second `preview_dml` while one is pending is rejected.
pub struct BridgeHandle {
    pub conversation_id: String,
    pub pending_dml: Arc<Mutex<Option<PendingDml>>>,
    connection: Arc<BridgeConnection>,
}

impl BridgeHandle {
    pub fn new(conversation_id: impl Into<String>) -> Self {
        Self {
            conversation_id: conversation_id.into(),
            pending_dml: Arc::new(Mutex::new(None)),
            connection: Arc::new(BridgeConnection::default()),
        }
    }

    /// The conversation's bridge-connect state. `session_for` extracts it
    /// into the `SessionEntry` so the driver can gate the preamble on it.
    pub fn connection(&self) -> Arc<BridgeConnection> {
        self.connection.clone()
    }
}

/// The held `preview_dml` tool call: the staged SQL plus the oneshot that
/// Phase D's `execute_dml` / `reject_dml` resolve with the user's decision.
/// The frontend only knows the conversation id, never the MCP call id, so
/// this registry is keyed per conversation, not per call.
pub struct PendingDml {
    pub sql: String,
    pub tx: tokio::sync::oneshot::Sender<Result<DmlOutcome, String>>,
}

/// What a user-approved DML execution produced.
#[derive(Debug)]
pub struct DmlOutcome {
    pub rows_affected: u64,
}

/// Serves bridge connections for the conversation: token-authenticated hello,
/// then a request/response loop until client EOF. Accepts connections in a
/// loop so that both persistent MCP stdio processes and multiple sequential
/// or parallel CLI tool calls (`lucent-tool`) can execute during the session.
#[cfg(unix)]
pub async fn serve(
    listener: tokio::net::UnixListener,
    token: String,
    executor: Arc<dyn ToolExecutor>,
    sink: Arc<dyn crate::ai::agent::AgentSink>,
    handle: Arc<BridgeHandle>,
) -> Result<(), String> {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let (reader, writer) = stream.into_split();
                let token = token.clone();
                let executor = executor.clone();
                let sink = sink.clone();
                let handle = handle.clone();
                tokio::spawn(async move {
                    let _ = serve_io(reader, writer, token, executor, sink, handle).await;
                });
            }
            Err(e) => {
                log::debug!("bridge listener stopped accepting: {e}");
                break;
            }
        }
    }
    Ok(())
}

/// Windows variant: the pipe server is created pre-bound; `connect` waits for
/// the client.
#[cfg(windows)]
pub async fn serve(
    listener: tokio::net::windows::named_pipe::NamedPipeServer,
    token: String,
    executor: Arc<dyn ToolExecutor>,
    sink: Arc<dyn crate::ai::agent::AgentSink>,
    handle: Arc<BridgeHandle>,
) -> Result<(), String> {
    let mut server = listener;
    if server.connect().await.is_ok() {
        let (reader, writer) = tokio::io::split(server);
        let _ = serve_io(reader, writer, token, executor, sink, handle).await;
    }
    Ok(())
}

async fn serve_io<R, W>(
    reader: R,
    writer: W,
    token: String,
    executor: Arc<dyn ToolExecutor>,
    sink: Arc<dyn crate::ai::agent::AgentSink>,
    handle: Arc<BridgeHandle>,
) -> Result<(), String>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = tokio::io::BufWriter::new(writer);
    let Some(hello) = wire::read_hello(&mut reader).await? else {
        return Ok(()); // EOF before hello — nothing to serve
    };
    let wire::Hello::Hello { token: hello_token } = hello;
    if hello_token != token {
        return Ok(()); // silent close — never confirm a wrong token
    }
    // The agent's MCP client is alive on the bridge: Lucent's DB tools are
    // genuinely in the agent's toolset. Signal it so the prompt can claim
    // them (and the Settings/UX can trust the connection).
    handle.connection.mark_connected();
    loop {
        let Some(req) = wire::read_message(&mut reader).await? else {
            return Ok(()); // client closed
        };
        let wire::BridgeRequest::Call { id, tool, args } = req;
        let response = dispatch(&executor, &sink, &handle, id, &tool, args).await;
        wire::write_message(&mut writer, &response).await?;
        tokio::io::AsyncWriteExt::flush(&mut writer)
            .await
            .map_err(|e| format!("flush bridge response: {e}"))?;
    }
}

/// One `tools/call` dispatch. Runs the tool through the executor (all
/// guardrails live in the tool stack), then maps the output:
///
/// - `preview_dml` holds the call open on a oneshot until Phase D resolves it
///   (approval → execution summary, rejection → tool error);
/// - `QueryResult` emits the structured `ToolResult` + `QueryResult` pair to
///   the sink (mirroring the rig loop, agent.rs) and returns the text summary;
/// - `Text` passes through;
/// - errors become `BridgeResponse::Err` (the MCP layer reports `isError`).
pub(crate) async fn dispatch(
    executor: &Arc<dyn ToolExecutor>,
    sink: &Arc<dyn crate::ai::agent::AgentSink>,
    handle: &Arc<BridgeHandle>,
    id: u64,
    tool: &str,
    args: serde_json::Value,
) -> wire::BridgeResponse {
    match executor.call(tool, args).await {
        Ok(ToolOutput::DmlPreview {
            sql,
            description,
            tables_affected,
            estimated_rows_affected,
            ..
        }) => {
            let (tx, rx) = tokio::sync::oneshot::channel();
            let mut slot = handle.pending_dml.lock().await;
            if slot.is_some() {
                return wire::BridgeResponse::Err {
                    id,
                    error: "another DML approval is already pending".into(),
                };
            }
            *slot = Some(PendingDml {
                sql: sql.clone(),
                tx,
            });
            drop(slot);
            sink.dml_approval(DmlApprovalPayload {
                conversation_id: handle.conversation_id.clone(),
                sql: sql.clone(),
                tables_affected,
                description,
                estimated_rows_affected,
            });
            match rx.await {
                Ok(Ok(outcome)) => wire::BridgeResponse::Ok {
                    id,
                    output: serde_json::json!({ "text": format!("DML executed: {} rows affected. SQL: {}", outcome.rows_affected, sql) }),
                },
                Ok(Err(e)) => wire::BridgeResponse::Err {
                    id,
                    error: format!("DML rejected by user: {e}"),
                },
                Err(_) => wire::BridgeResponse::Err {
                    id,
                    error: "DML rejected by user: the approval was dropped".into(),
                },
            }
        }
        Ok(ToolOutput::QueryResult {
            text_summary,
            columns,
            rows,
            row_count,
            sql,
            execution_time_ms,
            truncated,
        }) => {
            // Structured UI events — the frontend renders the grid from these.
            // Mirrors agent.rs: columns map data_type → "type" (the frontend's
            // ToolOutputPayload shape) and ToolResult carries a 10-row preview;
            // the full set rides in QueryResult.
            let cols: Vec<serde_json::Value> = columns
                .iter()
                .map(|c| serde_json::json!({ "name": c.name, "type": c.data_type }))
                .collect();
            sink.event(AiEvent::ToolResult {
                id: format!("acp-{id}"),
                tool: tool.to_string(),
                summary: text_summary.clone(),
                output: Some(serde_json::json!({
                    "type": "query_result",
                    "columns": cols,
                    "rows": rows.iter().take(10).collect::<Vec<_>>(),
                    "row_count": row_count,
                    "sql": sql,
                    "execution_time_ms": execution_time_ms,
                    "truncated": truncated,
                })),
            });
            sink.event(AiEvent::QueryResult {
                columns,
                rows,
                row_count,
                sql,
                execution_time_ms,
            });
            wire::BridgeResponse::Ok {
                id,
                output: serde_json::json!({ "text": text_summary }),
            }
        }
        Ok(ToolOutput::Text { content }) => wire::BridgeResponse::Ok {
            id,
            output: serde_json::json!({ "text": content }),
        },
        Err(e) => wire::BridgeResponse::Err {
            id,
            error: e.to_string(),
        },
    }
}

/// Client half of the bridge socket: hello handshake + `call`. Used by tests
/// and by Phase F's conformance capstone to drive the real bridge (the MCP
/// binary uses the wire helpers directly). Unix-only for now — the named-pipe
/// client path is verified in Phase F's Windows conformance run.
#[cfg(unix)]
pub struct BridgeClient {
    reader: tokio::io::BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

#[cfg(unix)]
impl BridgeClient {
    pub async fn connect(path: &std::path::Path, token: &str) -> Result<Self, String> {
        let stream = tokio::net::UnixStream::connect(path)
            .await
            .map_err(|e| format!("connect bridge: {e}"))?;
        let (reader, mut writer) = stream.into_split();
        wire::write_hello(&mut writer, token).await?;
        Ok(Self {
            reader: tokio::io::BufReader::new(reader),
            writer,
        })
    }

    /// Sends one `Call` and awaits the matching response. Returns the Ok
    /// output JSON, or Err on `BridgeResponse::Err` / EOF (a wrong token gets
    /// a silent close, so the first call fails with an EOF-style error).
    pub async fn call(
        &mut self,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        wire::write_request(
            &mut self.writer,
            &wire::BridgeRequest::Call {
                id: 1, // one in flight at a time
                tool: tool.to_string(),
                args,
            },
        )
        .await?;
        match wire::read_response(&mut self.reader).await? {
            Some(wire::BridgeResponse::Ok { output, .. }) => Ok(output),
            Some(wire::BridgeResponse::Err { error, .. }) => Err(error),
            None => Err("bridge closed the connection".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::acp::wire;
    use crate::ai::config::AiConfig;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// A minimal `AiToolContext` with no database connection — every tool
    /// errors with `NotConnected` first, which is exactly what the adapter
    /// tests need (no Docker, no network).
    pub(crate) fn test_ctx() -> AiToolContext {
        AiToolContext {
            db: Arc::new(Mutex::new(None)),
            connection_id: None,
            capabilities: None,
            config: AiConfig::default(),
            schema_graph: Arc::new(Mutex::new(None)),
            embedder: Arc::new(Mutex::new(None)),
            reranker: Arc::new(Mutex::new(None)),
        }
    }

    #[tokio::test]
    async fn unknown_tool_is_an_error() {
        let ex = ContextToolExecutor::new(test_ctx());
        let err = ex
            .call("definitely_not_a_tool", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("definitely_not_a_tool"));
    }

    #[tokio::test]
    async fn real_tool_executes_without_db_connection() {
        // search_schema with no connection must return the NotConnected error,
        // proving the adapter dispatches into the real tool stack.
        let ex = ContextToolExecutor::new(test_ctx());
        let err = ex
            .call("search_schema", serde_json::json!({"query": "users"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::NotConnected));
    }

    #[tokio::test]
    async fn executor_accepts_trait_object_bounds() {
        // The executor is used through `Arc<dyn ToolExecutor>` in serve — the
        // trait must be object-safe and Send + Sync.
        let ex: Arc<dyn ToolExecutor> = Arc::new(ContextToolExecutor::new(test_ctx()));
        let err = ex.call("nope", serde_json::json!({})).await.unwrap_err();
        assert!(err.to_string().contains("nope"));
    }

    // ── serve-loop test infrastructure ────────────────────────────────────────
    // A scripted executor: each `call` pops the next (tool, args, result) entry
    // from the queue; exhausting the script is a test bug and panics loudly.
    type ScriptEntry = (String, serde_json::Value, Result<ToolOutput, ToolError>);

    struct ScriptedExecutor {
        script: std::sync::Mutex<std::collections::VecDeque<ScriptEntry>>,
    }

    impl ScriptedExecutor {
        fn new(script: Vec<ScriptEntry>) -> Self {
            Self {
                script: std::sync::Mutex::new(script.into()),
            }
        }

        fn text(text: &str) -> Self {
            Self::new(vec![(
                "echo".into(),
                serde_json::json!({}),
                Ok(ToolOutput::Text {
                    content: text.into(),
                }),
            )])
        }
    }

    #[async_trait::async_trait]
    impl ToolExecutor for ScriptedExecutor {
        async fn call(
            &self,
            tool: &str,
            _args: serde_json::Value,
        ) -> Result<ToolOutput, ToolError> {
            let entry = self.script.lock().unwrap().pop_front().unwrap_or_else(|| {
                panic!("scripted executor script exhausted (unexpected call to {tool})")
            });
            assert_eq!(
                entry.0, tool,
                "scripted executor received an unexpected tool"
            );
            entry.2
        }
    }

    /// Records every `AiEvent` and `DmlApprovalPayload` the sink receives.
    struct RecordingSink {
        events: Arc<std::sync::Mutex<Vec<crate::ai::events::AiEvent>>>,
        approvals: Arc<std::sync::Mutex<Vec<crate::ai::events::DmlApprovalPayload>>>,
    }

    impl RecordingSink {
        fn new() -> Self {
            Self {
                events: Arc::new(std::sync::Mutex::new(Vec::new())),
                approvals: Arc::new(std::sync::Mutex::new(Vec::new())),
            }
        }

        #[allow(dead_code)]
        fn event_types(&self) -> Vec<&'static str> {
            use crate::ai::events::AiEvent::*;
            self.events
                .lock()
                .unwrap()
                .iter()
                .map(|e| match e {
                    Thinking { .. } => "thinking",
                    Text { .. } => "text",
                    ToolCalls { .. } => "tool_calls",
                    ToolResult { .. } => "tool_result",
                    Notice { .. } => "notice",
                    QueryResult { .. } => "query_result",
                    Done { .. } => "done",
                })
                .collect()
        }
    }

    impl crate::ai::agent::AgentSink for RecordingSink {
        fn event(&self, event: crate::ai::events::AiEvent) {
            self.events.lock().unwrap().push(event);
        }

        fn dml_approval(&self, payload: crate::ai::events::DmlApprovalPayload) {
            self.approvals.lock().unwrap().push(payload);
        }
    }

    fn bind_listener(dir: &tempfile::TempDir) -> (tokio::net::UnixListener, std::path::PathBuf) {
        let path = dir.path().join("t.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        (listener, path)
    }

    #[tokio::test]
    async fn rejects_wrong_token() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, path) = bind_listener(&dir);
        let executor: Arc<dyn ToolExecutor> = Arc::new(ScriptedExecutor::text("unused"));
        let handle = Arc::new(BridgeHandle::new("conv-1"));
        let task = tokio::spawn(serve(
            listener,
            "right".into(),
            executor,
            Arc::new(RecordingSink::new()),
            handle.clone(),
        ));
        let mut sock = tokio::net::UnixStream::connect(&path).await.unwrap();
        wire::write_hello(&mut sock, "wrong").await.unwrap();
        drop(sock);
        task.abort(); // serve is a long-lived accept loop; it never returns on its own
        assert!(
            !handle.connection().is_connected(),
            "a wrong token must never mark the bridge connected"
        );
    }

    #[tokio::test]
    async fn query_result_emits_structured_events() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, path) = bind_listener(&dir);
        let token = "tok123".to_string();
        let script = vec![(
            "run_readonly_query".into(),
            serde_json::json!({"sql": "select 1"}),
            Ok(ToolOutput::QueryResult {
                text_summary: "1 row".into(),
                columns: vec![crate::ai::events::ColumnMeta {
                    name: "x".into(),
                    data_type: "INTEGER".into(),
                }],
                rows: vec![vec![serde_json::json!(1)]],
                row_count: 1,
                sql: "select 1".into(),
                execution_time_ms: 5,
                truncated: false,
            }),
        )];
        let executor: Arc<dyn ToolExecutor> = Arc::new(ScriptedExecutor::new(script));
        let sink = Arc::new(RecordingSink::new());
        let task = tokio::spawn(serve(
            listener,
            token.clone(),
            executor,
            sink.clone(),
            Arc::new(BridgeHandle::new("conv-1")),
        ));
        let mut sock = tokio::net::UnixStream::connect(&path).await.unwrap();
        wire::write_hello(&mut sock, &token).await.unwrap();
        wire::write_request(
            &mut sock,
            &wire::BridgeRequest::Call {
                id: 1,
                tool: "run_readonly_query".into(),
                args: serde_json::json!({"sql": "select 1"}),
            },
        )
        .await
        .unwrap();
        let mut reader = tokio::io::BufReader::new(sock);
        let resp = wire::read_response(&mut reader).await.unwrap().unwrap();
        match resp {
            wire::BridgeResponse::Ok { id, output } => {
                assert_eq!(id, 1);
                assert_eq!(output["text"], "1 row");
            }
            wire::BridgeResponse::Err { error, .. } => panic!("expected Ok, got Err: {error}"),
        }
        drop(reader);
        task.abort(); // serve is a long-lived accept loop; it never returns on its own

        // The sink saw the structured ToolResult (frontend grid shape: columns
        // carry `type`, not `data_type`) plus the full QueryResult.
        let events = sink.events.lock().unwrap();
        assert!(events
            .iter()
            .any(|e| matches!(e, crate::ai::events::AiEvent::ToolResult { .. })));
        assert!(events
            .iter()
            .any(|e| matches!(e, crate::ai::events::AiEvent::QueryResult { .. })));
        let tool_result = events
            .iter()
            .find(|e| matches!(e, crate::ai::events::AiEvent::ToolResult { .. }))
            .unwrap();
        if let crate::ai::events::AiEvent::ToolResult {
            id, tool, output, ..
        } = tool_result
        {
            assert_eq!(id, "acp-1");
            assert_eq!(tool, "run_readonly_query");
            let o = output.as_ref().expect("structured output present");
            assert_eq!(o["type"], "query_result");
            assert_eq!(
                o["columns"][0]["type"], "INTEGER",
                "columns must use the frontend's {{name,type}} shape"
            );
            assert_eq!(o["row_count"], 1);
            assert_eq!(o["truncated"], false);
            assert_eq!(o["sql"], "select 1");
        }
    }

    #[tokio::test]
    async fn dml_hold_waits_for_approval_then_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, path) = bind_listener(&dir);
        let token = "tok123".to_string();
        let script = vec![(
            "preview_dml".into(),
            serde_json::json!({"sql": "insert into t values (1)"}),
            Ok(ToolOutput::DmlPreview {
                sql: "insert into t values (1)".into(),
                statement_type: "INSERT".into(),
                tables_affected: vec!["t".into()],
                description: "Insert 1 row into t".into(),
                estimated_rows_affected: Some(1),
            }),
        )];
        let executor: Arc<dyn ToolExecutor> = Arc::new(ScriptedExecutor::new(script));
        let sink = Arc::new(RecordingSink::new());
        let handle = Arc::new(BridgeHandle::new("conv-1"));
        let task = tokio::spawn(serve(
            listener,
            token.clone(),
            executor,
            sink.clone(),
            handle.clone(),
        ));
        let mut sock = tokio::net::UnixStream::connect(&path).await.unwrap();
        wire::write_hello(&mut sock, &token).await.unwrap();
        wire::write_request(
            &mut sock,
            &wire::BridgeRequest::Call {
                id: 9,
                tool: "preview_dml".into(),
                args: serde_json::json!({"sql": "insert into t values (1)"}),
            },
        )
        .await
        .unwrap();
        let mut reader = tokio::io::BufReader::new(sock);

        // The call stays open: no response yet, and the approval payload
        // reached the sink with the conversation id.
        let held = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            wire::read_response(&mut reader),
        )
        .await;
        assert!(
            held.is_err(),
            "preview_dml must stay open until the approval resolves"
        );
        {
            let approvals = sink.approvals.lock().unwrap();
            assert_eq!(approvals.len(), 1);
            assert_eq!(approvals[0].conversation_id, "conv-1");
            assert_eq!(approvals[0].sql, "insert into t values (1)");
            assert_eq!(approvals[0].tables_affected, vec!["t"]);
        }

        // User approves: take the slot and resolve the oneshot.
        let pending = handle
            .pending_dml
            .lock()
            .await
            .take()
            .expect("pending DML slot registered");
        pending
            .tx
            .send(Ok(DmlOutcome { rows_affected: 3 }))
            .unwrap();

        let resp = wire::read_response(&mut reader).await.unwrap().unwrap();
        match resp {
            wire::BridgeResponse::Ok { id, output } => {
                assert_eq!(id, 9);
                let text = output["text"].as_str().expect("text field");
                assert!(text.contains("3 rows affected"), "text: {text}");
            }
            wire::BridgeResponse::Err { error, .. } => panic!("expected Ok, got Err: {error}"),
        }
        drop(reader);
        task.abort(); // serve is a long-lived accept loop; it never returns on its own
    }

    #[tokio::test]
    async fn dml_rejection_resolves_with_error() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, path) = bind_listener(&dir);
        let token = "tok123".to_string();
        let script = vec![(
            "preview_dml".into(),
            serde_json::json!({"sql": "insert into t values (1)"}),
            Ok(ToolOutput::DmlPreview {
                sql: "insert into t values (1)".into(),
                statement_type: "INSERT".into(),
                tables_affected: vec!["t".into()],
                description: "Insert 1 row into t".into(),
                estimated_rows_affected: Some(1),
            }),
        )];
        let executor: Arc<dyn ToolExecutor> = Arc::new(ScriptedExecutor::new(script));
        let sink = Arc::new(RecordingSink::new());
        let handle = Arc::new(BridgeHandle::new("conv-1"));
        let task = tokio::spawn(serve(
            listener,
            token.clone(),
            executor,
            sink.clone(),
            handle.clone(),
        ));
        let mut sock = tokio::net::UnixStream::connect(&path).await.unwrap();
        wire::write_hello(&mut sock, &token).await.unwrap();
        wire::write_request(
            &mut sock,
            &wire::BridgeRequest::Call {
                id: 10,
                tool: "preview_dml".into(),
                args: serde_json::json!({"sql": "insert into t values (1)"}),
            },
        )
        .await
        .unwrap();
        let mut reader = tokio::io::BufReader::new(sock);

        let held = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            wire::read_response(&mut reader),
        )
        .await;
        assert!(
            held.is_err(),
            "preview_dml must stay open until the approval resolves"
        );

        // User rejects: resolve the oneshot with an Err.
        let pending = handle.pending_dml.lock().await.take().unwrap();
        pending.tx.send(Err("user said no".into())).unwrap();
        let resp = wire::read_response(&mut reader).await.unwrap().unwrap();
        match resp {
            wire::BridgeResponse::Err { id, error } => {
                assert_eq!(id, 10);
                assert!(error.contains("rejected"), "error: {error}");
            }
            wire::BridgeResponse::Ok { .. } => panic!("expected Err on rejection"),
        }
        drop(reader);
        task.abort(); // serve is a long-lived accept loop; it never returns on its own
    }

    #[tokio::test]
    async fn second_dml_while_pending_is_rejected() {
        // The single-slot registry refuses a second preview_dml while one is
        // pending. (The serve loop is sequential, so through the socket a
        // second call can only arrive after the first resolves — this unit
        // test drives `dispatch` directly to pin the guard.)
        let executor: Arc<dyn ToolExecutor> = Arc::new(ScriptedExecutor::new(vec![(
            "preview_dml".into(),
            serde_json::json!({"sql": "insert into t values (2)"}),
            Ok(ToolOutput::DmlPreview {
                sql: "insert into t values (2)".into(),
                statement_type: "INSERT".into(),
                tables_affected: vec!["t".into()],
                description: "Insert 1 row into t".into(),
                estimated_rows_affected: Some(1),
            }),
        )]));
        let sink = Arc::new(RecordingSink::new());
        let sink_dyn: Arc<dyn crate::ai::agent::AgentSink> = sink.clone();
        let handle = Arc::new(BridgeHandle::new("conv-1"));
        // Occupy the slot with a first (never resolved) approval.
        let (tx, _rx) = tokio::sync::oneshot::channel();
        *handle.pending_dml.lock().await = Some(PendingDml {
            sql: "first".into(),
            tx,
        });
        let resp = dispatch(
            &executor,
            &sink_dyn,
            &handle,
            2,
            "preview_dml",
            serde_json::json!({"sql": "insert into t values (2)"}),
        )
        .await;
        match resp {
            wire::BridgeResponse::Err { id, error } => {
                assert_eq!(id, 2);
                assert!(error.contains("already pending"), "error: {error}");
            }
            wire::BridgeResponse::Ok { .. } => panic!("second preview_dml must be rejected"),
        }
    }

    #[tokio::test]
    async fn text_output_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, path) = bind_listener(&dir);
        let token = "tok123".to_string();
        let executor: Arc<dyn ToolExecutor> = Arc::new(ScriptedExecutor::text("plain answer"));
        let task = tokio::spawn(serve(
            listener,
            token.clone(),
            executor,
            Arc::new(RecordingSink::new()),
            Arc::new(BridgeHandle::new("conv-1")),
        ));
        let mut sock = tokio::net::UnixStream::connect(&path).await.unwrap();
        wire::write_hello(&mut sock, &token).await.unwrap();
        wire::write_request(
            &mut sock,
            &wire::BridgeRequest::Call {
                id: 4,
                tool: "echo".into(),
                args: serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        let mut reader = tokio::io::BufReader::new(sock);
        let resp = wire::read_response(&mut reader).await.unwrap().unwrap();
        match resp {
            wire::BridgeResponse::Ok { id, output } => {
                assert_eq!(id, 4);
                assert_eq!(output["text"], "plain answer");
            }
            wire::BridgeResponse::Err { error, .. } => panic!("expected Ok, got Err: {error}"),
        }
        drop(reader);
        task.abort(); // serve is a long-lived accept loop; it never returns on its own
    }

    #[tokio::test]
    async fn tool_error_maps_to_err_response() {
        let dir = tempfile::tempdir().unwrap();
        let (listener, path) = bind_listener(&dir);
        let token = "tok123".to_string();
        let executor: Arc<dyn ToolExecutor> = Arc::new(ScriptedExecutor::new(vec![(
            "run_readonly_query".into(),
            serde_json::json!({"sql": "select 1"}),
            Err(ToolError::SqlValidation("read-only guard refused".into())),
        )]));
        let task = tokio::spawn(serve(
            listener,
            token.clone(),
            executor,
            Arc::new(RecordingSink::new()),
            Arc::new(BridgeHandle::new("conv-1")),
        ));
        let mut sock = tokio::net::UnixStream::connect(&path).await.unwrap();
        wire::write_hello(&mut sock, &token).await.unwrap();
        wire::write_request(
            &mut sock,
            &wire::BridgeRequest::Call {
                id: 5,
                tool: "run_readonly_query".into(),
                args: serde_json::json!({"sql": "select 1"}),
            },
        )
        .await
        .unwrap();
        let mut reader = tokio::io::BufReader::new(sock);
        let resp = wire::read_response(&mut reader).await.unwrap().unwrap();
        match resp {
            wire::BridgeResponse::Err { id, error } => {
                assert_eq!(id, 5);
                assert!(error.contains("read-only"), "error: {error}");
            }
            wire::BridgeResponse::Ok { .. } => panic!("expected Err on tool failure"),
        }
        drop(reader);
        task.abort(); // serve is a long-lived accept loop; it never returns on its own
    }

    #[tokio::test]
    async fn full_loop_with_bridge_client() {
        // Real listener + real serve + real BridgeClient — the whole wire
        // path, with only the executor and sink scripted.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let token = "tok123".to_string();
        let executor: Arc<dyn ToolExecutor> = Arc::new(ScriptedExecutor::text("hello from bridge"));
        let sink = Arc::new(RecordingSink::new());
        let handle = Arc::new(BridgeHandle::new("conv-1"));
        let serve_task = tokio::spawn(serve(
            listener,
            token.clone(),
            executor,
            sink,
            handle.clone(),
        ));

        let mut client = BridgeClient::connect(&path, &token).await.unwrap();
        assert!(
            handle
                .connection()
                .wait_connected(Duration::from_secs(2))
                .await,
            "a valid hello marks the bridge connected"
        );
        let out = client.call("echo", serde_json::json!({})).await.unwrap();
        assert_eq!(out["text"], "hello from bridge");
        drop(client);
        serve_task.abort(); // serve is a long-lived accept loop; it never returns on its own
    }

    #[tokio::test]
    async fn wait_connected_returns_when_hello_lands_and_times_out_otherwise() {
        // Bounded wait semantics: a connect inside the window resolves the
        // wait (even if it races the waiter), and no connect means false at
        // the deadline.
        let conn = Arc::new(BridgeConnection::default());
        let waiter = {
            let conn = conn.clone();
            tokio::spawn(async move { conn.wait_connected(Duration::from_secs(5)).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        conn.mark_connected();
        assert!(
            waiter.await.unwrap(),
            "connect inside the window resolves the wait"
        );

        let conn = Arc::new(BridgeConnection::default());
        let conn2 = conn.clone();
        let start = tokio::time::Instant::now();
        let ok = tokio::time::timeout(
            Duration::from_secs(2),
            conn2.wait_connected(Duration::from_millis(100)),
        )
        .await
        .expect("wait_connected itself is bounded by its timeout");
        assert!(!ok, "no connect -> false");
        assert!(
            start.elapsed() >= Duration::from_millis(100),
            "the wait actually waited"
        );
        let _ = conn;
    }

    #[tokio::test]
    async fn bridge_client_wrong_token_fails_on_first_call() {
        // A wrong token gets a silent close — the client sees EOF, never an
        // error message confirming the guess.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let executor: Arc<dyn ToolExecutor> = Arc::new(ScriptedExecutor::text("unused"));
        let handle = Arc::new(BridgeHandle::new("conv-1"));
        let serve_task = tokio::spawn(serve(
            listener,
            "right".into(),
            executor,
            Arc::new(RecordingSink::new()),
            handle.clone(),
        ));

        let mut client = BridgeClient::connect(&path, "wrong").await.unwrap();
        let err = client
            .call("echo", serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(!err.is_empty());
        assert!(
            !handle.connection().is_connected(),
            "wrong-token hello must not mark the bridge connected"
        );
        drop(client);
        serve_task.abort(); // serve is a long-lived accept loop; it never returns on its own
    }

    #[tokio::test]
    async fn bridge_client_reports_tool_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let token = "tok123".to_string();
        let executor: Arc<dyn ToolExecutor> = Arc::new(ScriptedExecutor::new(vec![(
            "run_readonly_query".into(),
            serde_json::json!({"sql": "select 1"}),
            Err(ToolError::SqlValidation("read-only guard refused".into())),
        )]));
        let serve_task = tokio::spawn(serve(
            listener,
            token.clone(),
            executor,
            Arc::new(RecordingSink::new()),
            Arc::new(BridgeHandle::new("conv-1")),
        ));

        let mut client = BridgeClient::connect(&path, &token).await.unwrap();
        let err = client
            .call("run_readonly_query", serde_json::json!({"sql": "select 1"}))
            .await
            .unwrap_err();
        assert!(err.contains("read-only"), "err: {err}");
        drop(client);
        serve_task.abort(); // serve is a long-lived accept loop; it never returns on its own
    }
}
