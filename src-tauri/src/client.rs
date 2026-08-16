use std::collections::HashMap;
use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use lucent_protocol::{
    new_codec, new_framed, read_message, write_message, CatalogRequest, CatalogResult, ColumnMeta,
    ConnectionConfig, ConnectionId, ForeignKey, Namespace, NamespacePath, ObjectDetail, ObjectKind,
    ObjectProperty, ObjectRef, ObjectSummary, QueryId, SearchHit, ServerInfo, Value, WorkerRequest,
    WorkerResponse,
};
use serde::Serialize;
use tokio::io::WriteHalf;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tracing::Instrument;
use uuid::Uuid;

use crate::ipc_stream::ClientStream;

/// Hard ceiling on the rows materialized by a single `execute_with_id` call.
///
/// The worker streams results in 500-row batches with backpressure, but this
/// client accumulates every batch in memory until the final one. Paged queries
/// (LIMIT/OFFSET-wrapped) stay far below this cap; it exists to bound queries
/// that cannot be wrapped — multi-statement scripts, EXPLAIN, unparseable-but-
/// executable statements — which would otherwise defeat the worker's streaming
/// design and grow without limit in the Tauri process.
///
/// When the cap is hit the client truncates, cancels the query server-side via
/// the DB-native cancel protocol (freeing the connection), and reports
/// `ExecuteResult.truncated` so the UI can tell the user.
pub const HARD_ROW_CAP: usize = 10_000;

#[derive(Debug, Clone, Serialize)]
pub struct ExecuteResult {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: usize,
    pub rows_affected: Option<u64>,
    /// True when the result was cut off at `HARD_ROW_CAP` (or a caller-supplied
    /// cap) and the query was cancelled server-side. The row count is the
    /// truncated count; the remaining rows were never materialized.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Clone)]
pub struct ConnectorClient {
    writer: Arc<Mutex<FramedWrite<WriteHalf<ClientStream>, LengthDelimitedCodec>>>,
    pending: Arc<std::sync::Mutex<HashMap<QueryId, mpsc::UnboundedSender<WorkerResponse>>>>,
    sync_pending: Arc<Mutex<HashMap<ConnectionId, oneshot::Sender<WorkerResponse>>>>,
    reader_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
    pub server_info: Option<ServerInfo>,
}

/// Removes the pending-map entry when the execute/catalog future is dropped
/// (cancelled or aborted), so the map cannot grow until socket EOF (C9).
/// Normal completion paths remove the entry explicitly; Drop's removal is
/// then a harmless no-op. The map is a `std::sync::Mutex` (never held across
/// an await) precisely so this synchronous `Drop` can lock it.
struct PendingGuard {
    pending: Arc<std::sync::Mutex<HashMap<QueryId, mpsc::UnboundedSender<WorkerResponse>>>>,
    query_id: QueryId,
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.query_id);
    }
}

impl ConnectorClient {
    pub async fn connect(
        endpoint: &str,
        token: &str,
        config: ConnectionConfig,
    ) -> Result<(Self, ConnectionId), String> {
        let stream =
            tokio::time::timeout(std::time::Duration::from_secs(15), connect_stream(endpoint))
                .await
                .map_err(|_| "connect to worker endpoint timed out after 15s".to_string())??;

        let mut framed = new_framed(stream);

        write_message(&mut framed, &lucent_protocol::PROTOCOL_VERSION)
            .await
            .map_err(|e| format!("version handshake failed: {e}"))?;
        write_message(&mut framed, &token.to_string())
            .await
            .map_err(|e| format!("handshake failed: {e}"))?;

        // Await the worker's handshake ack so version/token mismatches surface
        // as a typed error instead of a generic EOF on the connect read.
        let ack: WorkerResponse =
            tokio::time::timeout(std::time::Duration::from_secs(5), read_message(&mut framed))
                .await
                .map_err(|_| "handshake ack timed out after 5s".to_string())?
                .map_err(|e| format!("handshake ack failed: {e}"))?
                .ok_or("worker closed connection during handshake")?;
        match ack {
            WorkerResponse::HandshakeAccepted => {}
            WorkerResponse::Error { kind, message, .. } => {
                return Err(format!("{kind}: {message}"));
            }
            other => return Err(format!("unexpected handshake response: {other:?}")),
        }

        let connection_id = ConnectionId(Uuid::new_v4());

        write_message(
            &mut framed,
            &WorkerRequest::Connect {
                connection_id,
                config,
            },
        )
        .await
        .map_err(|e| format!("connect request failed: {e}"))?;

        let response: WorkerResponse = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            read_message(&mut framed),
        )
        .await
        .map_err(|_| "connect response timed out after 15s".to_string())?
        .map_err(|e| format!("connect response failed: {e}"))?
        .ok_or("worker closed connection during connect")?;

        let server_info = match response {
            WorkerResponse::Connected { server_info, .. } => server_info,
            WorkerResponse::ConnectionError { kind, message, .. } => {
                return Err(format!("{kind}: {message}"));
            }
            WorkerResponse::Error { kind, message, .. } => {
                return Err(format!("{kind}: {message}"));
            }
            other => return Err(format!("unexpected response during connect: {other:?}")),
        };

        // Split stream: reader owns read half (lock-free), writer is shared via Mutex
        let stream = framed.into_inner();
        let (read_half, write_half) = tokio::io::split(stream);
        let mut framed_read = FramedRead::new(read_half, new_codec());
        let framed_write = Arc::new(Mutex::new(FramedWrite::new(write_half, new_codec())));

        let pending: Arc<
            std::sync::Mutex<HashMap<QueryId, mpsc::UnboundedSender<WorkerResponse>>>,
        > = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let pending_clone = pending.clone();

        let sync_pending: Arc<Mutex<HashMap<ConnectionId, oneshot::Sender<WorkerResponse>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let sync_pending_clone = sync_pending.clone();
        let reader_handle = tokio::spawn(async move {
            loop {
                match framed_read.next().await {
                    Some(Ok(bytes)) => {
                        let response: WorkerResponse = match bincode::deserialize(&bytes) {
                            Ok(msg) => msg,
                            Err(e) => {
                                log::error!("Reader deserialize error: {e}");
                                continue;
                            }
                        };

                        match &response {
                            WorkerResponse::Connected { connection_id, .. }
                            | WorkerResponse::Disconnected { connection_id, .. }
                            | WorkerResponse::ConnectionError { connection_id, .. } => {
                                let mut sp = sync_pending_clone.lock().await;
                                if let Some(sender) = sp.remove(connection_id) {
                                    let _ = sender.send(response);
                                }
                            }
                            _ => {
                                let query_id = match &response {
                                    WorkerResponse::ResultBatch { query_id, .. } => Some(*query_id),
                                    WorkerResponse::Cancelled { query_id } => Some(*query_id),
                                    // Catalog replies correlate on the same map:
                                    // `request_id` is a QueryId precisely so this
                                    // works without a second routing table.
                                    WorkerResponse::CatalogResult { request_id, .. } => {
                                        Some(*request_id)
                                    }
                                    WorkerResponse::Error { query_id, .. } => *query_id,
                                    _ => {
                                        log::warn!(
                                            "reader: dropping unmatchable reply: {response:?}"
                                        );
                                        None
                                    }
                                };
                                if let Some(qid) = query_id {
                                    let pending_lock =
                                        pending_clone.lock().unwrap_or_else(|p| p.into_inner());
                                    if let Some(tx) = pending_lock.get(&qid) {
                                        // Buffered, never dropped: the caller
                                        // re-arms its receiver between batches,
                                        // and a response arriving in that window
                                        // must queue, not vanish. (The old
                                        // oneshot-per-batch correlation dropped
                                        // it — a fast multi-batch result hung
                                        // the query forever.)
                                        let _ = tx.send(response);
                                    }
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        log::error!("Reader I/O error: {e}");
                        continue;
                    }
                    None => {
                        {
                            let mut pending_lock =
                                pending_clone.lock().unwrap_or_else(|p| p.into_inner());
                            pending_lock.clear();
                        }
                        let mut sp = sync_pending_clone.lock().await;
                        sp.clear();
                        break;
                    }
                }
            }
        });

        Ok((
            Self {
                writer: framed_write,
                pending,
                sync_pending,
                reader_handle: Arc::new(Mutex::new(Some(reader_handle))),
                server_info: Some(server_info),
            },
            connection_id,
        ))
    }

    /// Serialize one request and write it to the worker socket (flush included).
    /// Shared by all request paths so write/error handling stays uniform.
    async fn write_request(&self, request: &WorkerRequest) -> Result<(), String> {
        let mut lock = self.writer.lock().await;
        let bytes = bincode::serialize(request).map_err(|e| format!("serialize failed: {e}"))?;
        lock.send(bytes.into())
            .await
            .map_err(|e| format!("request send failed: {e}"))?;
        lock.flush()
            .await
            .map_err(|e| format!("request flush failed: {e}"))?;
        Ok(())
    }

    pub async fn connect_with_id(
        &self,
        connection_id: ConnectionId,
        config: ConnectionConfig,
    ) -> Result<ServerInfo, String> {
        let (tx, rx) = oneshot::channel();

        {
            let mut sp = self.sync_pending.lock().await;
            sp.insert(connection_id, tx);
        }

        if let Err(e) = self
            .write_request(&WorkerRequest::Connect {
                connection_id,
                config,
            })
            .await
        {
            // The request never made it out — drop the orphaned oneshot so
            // `sync_pending` can't accumulate dead senders. The reader's EOF
            // path would also drain the map, but don't rely on the socket
            // dying to clean up after ourselves.
            self.sync_pending.lock().await.remove(&connection_id);
            return Err(e);
        }

        let response = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
            .await
            .map_err(|_| "connect response timed out".to_string())?
            .map_err(|_| "internal: connect response channel closed".to_string())?;

        match response {
            WorkerResponse::Connected { server_info, .. } => Ok(server_info),
            WorkerResponse::ConnectionError { kind, message, .. } => {
                Err(format!("{kind}: {message}"))
            }
            WorkerResponse::Error { kind, message, .. } => Err(format!("{kind}: {message}")),
            other => Err(format!("unexpected response: {other:?}")),
        }
    }

    pub async fn execute(
        &self,
        connection_id: ConnectionId,
        sql: &str,
    ) -> Result<ExecuteResult, String> {
        let query_id = QueryId(Uuid::new_v4());
        self.execute_with_id(query_id, connection_id, sql, None)
            .await
            .map(|(result, _)| result)
    }

    /// execute with an explicit query id — lets the caller register the id
    /// (e.g. for cancel) before the request is written.
    ///
    /// `max_rows` bounds how many rows are materialized in this process: when
    /// the cap is reached the client truncates to it, cancels the query
    /// server-side (native DB cancel, so the connection frees up and the
    /// worker stops streaming), and returns a result with `truncated = true`.
    /// Pass `None` for callers that bound rows themselves (e.g. the AI tool's
    /// SQL-level LIMIT).
    pub async fn execute_with_id(
        &self,
        query_id: QueryId,
        connection_id: ConnectionId,
        sql: &str,
        max_rows: Option<usize>,
    ) -> Result<(ExecuteResult, QueryId), String> {
        // Correlate the whole round-trip (including bridged `log::` lines)
        // under the `worker.execute` span. No await inside the construction.
        let span = crate::trace::worker_execute_span(&connection_id, &query_id);
        self.execute_with_id_impl(query_id, connection_id, sql, max_rows)
            .instrument(span)
            .await
    }

    async fn execute_with_id_impl(
        &self,
        query_id: QueryId,
        connection_id: ConnectionId,
        sql: &str,
        max_rows: Option<usize>,
    ) -> Result<(ExecuteResult, QueryId), String> {
        let (tx, mut rx) = mpsc::unbounded_channel();

        {
            let mut pending_lock = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            pending_lock.insert(query_id, tx);
        }
        // C9: if this future is dropped (caller aborts, truncation path,
        // timeout), Drop removes the entry — the map cannot grow until EOF.
        let _pending_guard = PendingGuard {
            pending: Arc::clone(&self.pending),
            query_id,
        };

        if let Err(e) = self
            .write_request(&WorkerRequest::Execute {
                connection_id,
                query_id,
                command: sql.to_string(),
            })
            .await
        {
            // The request never made it out (worker died / socket broke) — drop
            // the orphaned sender so `pending` can't accumulate dead senders.
            // The reader's EOF path would also drain the map, but don't rely on
            // the socket dying to clean up after ourselves.
            self.pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&query_id);
            return Err(e);
        }

        let mut all_columns: Option<Arc<Vec<ColumnMeta>>> = None;
        let mut all_rows: Vec<Vec<serde_json::Value>> = Vec::new();
        let mut truncated = false;

        loop {
            let response = rx
                .recv()
                .await
                .ok_or_else(|| "internal: response channel closed".to_string())?;

            match response {
                WorkerResponse::ResultBatch {
                    shape, is_final, ..
                } => match shape {
                    lucent_protocol::ResultShape::Tabular { columns, rows } => {
                        all_rows.extend(
                            rows.into_iter()
                                .map(|row| row.into_iter().map(value_to_json).collect()),
                        );
                        if all_columns.is_none() {
                            all_columns = Some(columns);
                        }

                        if let Some(cap) = max_rows {
                            if all_rows.len() > cap {
                                all_rows.truncate(cap);
                                truncated = true;
                            }
                        }

                        if truncated && !is_final {
                            // Bound the damage server-side too: cancel the query
                            // so the connection frees up and the worker stops
                            // streaming batches we will never read.
                            let _ = self.cancel(connection_id, query_id).await;
                        }

                        if is_final || truncated {
                            // Query is done — drop the sender so a late reply
                            // can never be buffered and the map cannot grow.
                            self.pending
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .remove(&query_id);
                            let row_count = all_rows.len();
                            let cols = all_columns.map(|c| (*c).clone()).unwrap_or_default();
                            return Ok((
                                ExecuteResult {
                                    columns: cols,
                                    rows: all_rows,
                                    row_count,
                                    rows_affected: None,
                                    truncated,
                                },
                                query_id,
                            ));
                        }
                    }
                    lucent_protocol::ResultShape::Affected { rows_affected } => {
                        let cols = all_columns
                            .clone()
                            .map(|c| (*c).clone())
                            .unwrap_or_default();
                        let result = ExecuteResult {
                            columns: cols,
                            rows: Vec::new(),
                            row_count: 0,
                            rows_affected: Some(rows_affected),
                            truncated: false,
                        };
                        if is_final {
                            self.pending
                                .lock()
                                .unwrap_or_else(|p| p.into_inner())
                                .remove(&query_id);
                            return Ok((result, query_id));
                        }
                    }
                    _ => return Err("unsupported result shape".into()),
                },
                WorkerResponse::Error { kind, message, .. } => {
                    self.pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&query_id);
                    return Err(format!("{kind}: {message}"));
                }
                WorkerResponse::Cancelled { .. } => {
                    self.pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&query_id);
                    return Err("query was cancelled".into());
                }
                other => {
                    self.pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .remove(&query_id);
                    return Err(format!("unexpected response: {other:?}"));
                }
            }
        }
    }

    pub async fn cancel(
        &self,
        connection_id: ConnectionId,
        query_id: QueryId,
    ) -> Result<(), String> {
        self.write_request(&WorkerRequest::Cancel {
            connection_id,
            query_id,
        })
        .await
    }

    pub async fn disconnect_id(&self, connection_id: ConnectionId) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();

        {
            let mut sp = self.sync_pending.lock().await;
            sp.insert(connection_id, tx);
        }

        if let Err(e) = self
            .write_request(&WorkerRequest::Disconnect { connection_id })
            .await
        {
            self.sync_pending.lock().await.remove(&connection_id);
            return Err(e);
        }

        let response = tokio::time::timeout(std::time::Duration::from_secs(10), rx)
            .await
            .map_err(|_| "disconnect response timed out".to_string())?
            .map_err(|_| "internal: disconnect response channel closed".to_string())?;

        match response {
            WorkerResponse::Disconnected { .. } => Ok(()),
            WorkerResponse::ConnectionError { kind, message, .. } => {
                Err(format!("{kind}: {message}"))
            }
            WorkerResponse::Error { kind, message, .. } => Err(format!("{kind}: {message}")),
            other => Err(format!("unexpected response during disconnect: {other:?}")),
        }
    }

    pub async fn shutdown(&mut self) -> Result<(), String> {
        if let Some(handle) = self.reader_handle.lock().await.take() {
            handle.abort();
        }
        {
            let mut pending_lock = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            pending_lock.clear();
        }
        {
            let mut sp = self.sync_pending.lock().await;
            sp.clear();
        }
        self.write_request(&WorkerRequest::Shutdown).await
    }

    /// Issue one catalog request and await its single reply.
    ///
    /// Correlated by a `QueryId` used as a request id, so catalog replies land
    /// in the same `pending` map as query results and `WorkerResponse::Error`
    /// (which already carries `query_id`) correlates failures for free.
    pub async fn catalog(
        &self,
        connection_id: ConnectionId,
        request: CatalogRequest,
    ) -> Result<CatalogResult, String> {
        let request_id = QueryId(Uuid::new_v4());
        let (tx, mut rx) = mpsc::unbounded_channel();

        {
            let mut pending = self.pending.lock().unwrap_or_else(|p| p.into_inner());
            pending.insert(request_id, tx);
        }
        let _pending_guard = PendingGuard {
            pending: Arc::clone(&self.pending),
            query_id: request_id,
        };

        if let Err(e) = self
            .write_request(&WorkerRequest::Catalog {
                connection_id,
                request_id,
                request,
            })
            .await
        {
            self.pending
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&request_id);
            return Err(e);
        }

        let response = match tokio::time::timeout(CATALOG_TIMEOUT, rx.recv()).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                self.pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&request_id);
                return Err("internal: catalog response channel closed".to_string());
            }
            Err(_) => {
                // Drop the orphaned sender so a late reply cannot resolve a
                // channel nobody holds, and `pending` cannot grow without bound.
                self.pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(&request_id);
                return Err(format!(
                    "catalog request timed out after {CATALOG_TIMEOUT:?}"
                ));
            }
        };

        // The reply is in hand — drop the sender so a duplicate/late reply is
        // never buffered and the map cannot grow without bound.
        self.pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&request_id);

        match response {
            WorkerResponse::CatalogResult { result, .. } => Ok(result),
            WorkerResponse::Error { kind, message, .. } => Err(format!("{kind}: {message}")),
            other => Err(format!("unexpected response to catalog request: {other:?}")),
        }
    }

    pub async fn list_namespaces(
        &self,
        connection_id: ConnectionId,
    ) -> Result<Vec<Namespace>, String> {
        expect_namespaces(
            self.catalog(connection_id, CatalogRequest::ListNamespaces)
                .await?,
        )
    }

    pub async fn list_objects(
        &self,
        connection_id: ConnectionId,
        namespace: NamespacePath,
        kinds: Vec<ObjectKind>,
    ) -> Result<Vec<ObjectSummary>, String> {
        expect_objects(
            self.catalog(
                connection_id,
                CatalogRequest::ListObjects { namespace, kinds },
            )
            .await?,
        )
    }

    pub async fn list_all_objects(
        &self,
        connection_id: ConnectionId,
        kinds: Vec<ObjectKind>,
    ) -> Result<Vec<ObjectSummary>, String> {
        expect_objects(
            self.catalog(connection_id, CatalogRequest::ListAllObjects { kinds })
                .await?,
        )
    }

    pub async fn describe_objects(
        &self,
        connection_id: ConnectionId,
        refs: Vec<ObjectRef>,
    ) -> Result<Vec<ObjectDetail>, String> {
        if refs.is_empty() {
            // Mirrors the graceful empty result the per-object loop produced.
            return Ok(Vec::new());
        }
        match self
            .catalog(connection_id, CatalogRequest::DescribeObjects { refs })
            .await?
        {
            CatalogResult::ObjectDetails(v) => Ok(v),
            other => Err(driver_bug("ObjectDetails", &other)),
        }
    }

    pub async fn list_foreign_keys(
        &self,
        connection_id: ConnectionId,
    ) -> Result<Vec<ForeignKey>, String> {
        match self
            .catalog(connection_id, CatalogRequest::ListForeignKeys)
            .await?
        {
            CatalogResult::ForeignKeys(v) => Ok(v),
            other => Err(driver_bug("ForeignKeys", &other)),
        }
    }

    pub async fn search_objects(
        &self,
        connection_id: ConnectionId,
        query: &str,
        kinds: Vec<ObjectKind>,
        namespace: Option<NamespacePath>,
        limit: u32,
    ) -> Result<Vec<SearchHit>, String> {
        match self
            .catalog(
                connection_id,
                CatalogRequest::SearchObjects {
                    query: query.to_string(),
                    kinds,
                    namespace,
                    limit,
                },
            )
            .await?
        {
            CatalogResult::SearchHits(v) => Ok(v),
            other => Err(driver_bug("SearchHits", &other)),
        }
    }

    pub async fn object_ddl(
        &self,
        connection_id: ConnectionId,
        reference: ObjectRef,
    ) -> Result<String, String> {
        match self
            .catalog(connection_id, CatalogRequest::GetObjectDdl { reference })
            .await?
        {
            CatalogResult::Ddl(s) => Ok(s),
            other => Err(driver_bug("Ddl", &other)),
        }
    }

    pub async fn object_properties(
        &self,
        connection_id: ConnectionId,
        reference: ObjectRef,
    ) -> Result<Vec<ObjectProperty>, String> {
        match self
            .catalog(
                connection_id,
                CatalogRequest::GetObjectProperties { reference },
            )
            .await?
        {
            CatalogResult::Properties(v) => Ok(v),
            other => Err(driver_bug("Properties", &other)),
        }
    }
}

/// cfg-split endpoint connect: socket file path on Unix, pipe name on Windows.
async fn connect_stream(endpoint: &str) -> Result<ClientStream, String> {
    #[cfg(unix)]
    {
        use tokio::net::UnixStream;
        UnixStream::connect(endpoint)
            .await
            .map(ClientStream::Unix)
            .map_err(|e| format!("connect to worker socket failed: {e}"))
    }
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ClientOptions;
        ClientOptions::new()
            .open(endpoint)
            .map(ClientStream::Pipe)
            .map_err(|e| format!("connect to worker pipe failed: {e}"))
    }
}

/// Largest integer JavaScript can represent exactly (2^53 - 1). Beyond this the
/// webview's `JSON.parse` silently rounds, so we send a string instead: an exact
/// left-aligned value beats a right-aligned wrong one.
const JS_SAFE_INT: i64 = 9_007_199_254_740_991;

/// Catalog requests hit a live database. 30s is generous for `DescribeObjects`
/// over a wide schema and still short enough that a wedged worker surfaces as
/// an error rather than a hung schema browser.
const CATALOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A driver answered with the wrong `CatalogResult` variant. Name both sides —
/// degrading to an empty Vec would render as "this database is empty", which is
/// indistinguishable from the truth and impossible to debug.
fn driver_bug(expected: &str, got: &CatalogResult) -> String {
    format!("driver bug: expected CatalogResult::{expected}, got {got:?}")
}

fn expect_namespaces(result: CatalogResult) -> Result<Vec<Namespace>, String> {
    match result {
        CatalogResult::Namespaces(v) => Ok(v),
        other => Err(driver_bug("Namespaces", &other)),
    }
}

fn expect_objects(result: CatalogResult) -> Result<Vec<ObjectSummary>, String> {
    match result {
        CatalogResult::Objects(v) => Ok(v),
        other => Err(driver_bug("Objects", &other)),
    }
}

fn value_to_json(value: Value) -> serde_json::Value {
    use serde_json::Value as J;
    match value {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(b),
        Value::Int64(i) if (-JS_SAFE_INT..=JS_SAFE_INT).contains(&i) => J::Number(i.into()),
        Value::Int64(i) => J::String(i.to_string()),
        // JSON cannot represent NaN or Infinity; `from_f64` returns None there.
        Value::Float64(f) => serde_json::Number::from_f64(f)
            .map(J::Number)
            .unwrap_or_else(|| J::String(format_non_finite(f))),
        Value::Decimal(s) | Value::Text(s) | Value::Interval(s) | Value::Json(s) => J::String(s),
        Value::Uuid(u) => J::String(u.to_string()),
        Value::Binary(b) => J::String(format_bytea_hex(&b)),
        Value::Timestamp { micros, tz } => J::String(format_timestamp(micros, tz)),
        Value::Date(days) => J::String(format_date(days)),
        Value::Time(micros) => J::String(format_time(micros)),
        Value::Other { text, .. } => J::String(text),
        // `Value` is #[non_exhaustive]; a driver on a newer protocol could send
        // a variant this build does not know. Degrade rather than fail.
        other => J::String(format!("{other:?}")),
    }
}

fn format_non_finite(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_sign_positive() {
        "inf".into()
    } else {
        "-inf".into()
    }
}

fn format_bytea_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str(r"\x");
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn epoch_date() -> chrono::NaiveDate {
    // 1970-01-01 is always a valid date; the unwrap cannot fire.
    chrono::NaiveDate::from_ymd_opt(1970, 1, 1).expect("epoch date is valid")
}

fn format_date(days: i32) -> String {
    match epoch_date().checked_add_signed(chrono::Duration::days(days as i64)) {
        Some(d) => d.format("%Y-%m-%d").to_string(),
        None => format!("date+{days}"),
    }
}

fn format_time(micros: i64) -> String {
    let secs = micros.div_euclid(1_000_000);
    let rem = micros.rem_euclid(1_000_000);
    match chrono::NaiveTime::from_num_seconds_from_midnight_opt(secs as u32, (rem * 1_000) as u32) {
        Some(t) => t.format("%H:%M:%S%.6f").to_string(),
        None => format!("time+{micros}"),
    }
}

/// `tz: true` renders RFC 3339 in UTC — unambiguous and session-independent.
/// `tz: false` is a wall-clock reading and deliberately carries no offset.
/// Fractional seconds are preserved via `%.f`; the dot is omitted when the
/// fraction is zero (so whole-second values render without trailing zeros).
fn format_timestamp(micros: i64, tz: bool) -> String {
    let secs = micros.div_euclid(1_000_000);
    let nanos = (micros.rem_euclid(1_000_000) * 1_000) as u32;
    let Some(dt) = chrono::DateTime::from_timestamp(secs, nanos) else {
        return format!("timestamp+{micros}");
    };
    if tz {
        dt.format("%Y-%m-%dT%H:%M:%S%.f%:z").to_string()
    } else {
        dt.naive_utc().format("%Y-%m-%d %H:%M:%S%.f").to_string()
    }
}

#[cfg(test)]
mod value_json_tests {
    use super::value_to_json;
    use lucent_protocol::Value;
    use serde_json::json;

    #[test]
    fn null_and_bool_map_to_json_primitives() {
        assert_eq!(value_to_json(Value::Null), json!(null));
        assert_eq!(value_to_json(Value::Bool(true)), json!(true));
    }

    #[test]
    fn small_integers_become_json_numbers() {
        assert_eq!(value_to_json(Value::Int64(42)), json!(42));
        assert_eq!(value_to_json(Value::Int64(-42)), json!(-42));
    }

    #[test]
    fn integers_beyond_the_js_safe_range_become_strings() {
        // JSON.parse in the webview silently rounds past 2^53-1. Sending a
        // string keeps the value exact at the cost of numeric alignment.
        const SAFE: i64 = 9_007_199_254_740_991;
        assert_eq!(value_to_json(Value::Int64(SAFE)), json!(SAFE));
        assert_eq!(
            value_to_json(Value::Int64(SAFE + 1)),
            json!("9007199254740992")
        );
        assert_eq!(
            value_to_json(Value::Int64(i64::MAX)),
            json!("9223372036854775807")
        );
        // i64::MIN must not panic (a naive `.abs()` would).
        assert_eq!(
            value_to_json(Value::Int64(i64::MIN)),
            json!("-9223372036854775808")
        );
    }

    #[test]
    fn non_finite_floats_become_strings_because_json_has_no_nan() {
        assert_eq!(value_to_json(Value::Float64(1.5)), json!(1.5));
        assert_eq!(value_to_json(Value::Float64(f64::NAN)), json!("NaN"));
        assert_eq!(value_to_json(Value::Float64(f64::INFINITY)), json!("inf"));
    }

    #[test]
    fn decimal_stays_a_string_to_preserve_precision() {
        let big = "12345678901234567890.123456789012345678";
        assert_eq!(value_to_json(Value::Decimal(big.into())), json!(big));
    }

    #[test]
    fn binary_renders_in_postgres_hex_form() {
        assert_eq!(
            value_to_json(Value::Binary(vec![0x48, 0x69, 0x0a])),
            json!(r"\x48690a")
        );
    }

    #[test]
    fn temporal_values_render_as_iso_8601() {
        assert_eq!(value_to_json(Value::Date(0)), json!("1970-01-01"));
        assert_eq!(value_to_json(Value::Date(10957)), json!("2000-01-01"));
        assert_eq!(
            value_to_json(Value::Time(45_296_789_000)),
            json!("12:34:56.789000")
        );
        // tz = true renders UTC with an explicit offset — session-independent.
        assert_eq!(
            value_to_json(Value::Timestamp {
                micros: 0,
                tz: true
            }),
            json!("1970-01-01T00:00:00+00:00")
        );
        // tz = false is a wall-clock reading and carries no offset.
        assert_eq!(
            value_to_json(Value::Timestamp {
                micros: 1_000_000,
                tz: false
            }),
            json!("1970-01-01 00:00:01")
        );
        // Fractional seconds survive — regression guard: %S alone drops them.
        assert_eq!(
            value_to_json(Value::Timestamp {
                micros: 1_789_012,
                tz: true
            }),
            json!("1970-01-01T00:00:01.789012+00:00")
        );
        assert_eq!(
            value_to_json(Value::Timestamp {
                micros: 1_789_012,
                tz: false
            }),
            json!("1970-01-01 00:00:01.789012")
        );
    }

    #[test]
    fn text_json_interval_uuid_and_other_all_render_as_strings() {
        assert_eq!(value_to_json(Value::Text("hi".into())), json!("hi"));
        assert_eq!(
            value_to_json(Value::Json(r#"{"a":1}"#.into())),
            json!(r#"{"a":1}"#)
        );
        assert_eq!(
            value_to_json(Value::Interval("1 day".into())),
            json!("1 day")
        );
        assert_eq!(
            value_to_json(Value::Uuid(uuid::Uuid::nil())),
            json!("00000000-0000-0000-0000-000000000000")
        );
        assert_eq!(
            value_to_json(Value::Other {
                type_name: "int4range".into(),
                text: "[1,5)".into()
            }),
            json!("[1,5)")
        );
    }
}

#[cfg(test)]
mod catalog_client_tests {
    use lucent_protocol::{CatalogResult, Namespace};

    use super::expect_namespaces;

    #[test]
    fn the_right_variant_unwraps() {
        let result = CatalogResult::Namespaces(vec![Namespace {
            path: vec!["public".into()],
            object_count: Some(2),
        }]);
        let got = expect_namespaces(result).expect("should unwrap");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].display(), "public");
    }

    #[test]
    fn the_wrong_variant_is_a_named_driver_bug_not_a_silent_empty() {
        // A driver answering ListNamespaces with Ddl is broken. Returning an
        // empty Vec would render as "this database has no schemas", which is
        // indistinguishable from the truth and impossible to debug.
        let err = expect_namespaces(CatalogResult::Ddl("nope".into())).unwrap_err();
        assert!(
            err.contains("Namespaces"),
            "error must name the expected variant: {err}"
        );
    }
}

#[cfg(test)]
mod sync_routing_tests {
    use super::*;
    use lucent_protocol::{
        new_framed, read_message, write_message, DriverCapabilities, ReadOnlyMode, ServerInfo,
        TimeoutSupport, WorkerRequest,
    };
    use tokio::net::UnixListener;

    fn fake_capabilities() -> DriverCapabilities {
        DriverCapabilities {
            id: "fake".into(),
            display_name: "Fake".into(),
            sql_dialect: lucent_protocol::SqlDialect::PostgreSql,
            namespace_model: lucent_protocol::NamespaceModel::DbSchemaObject,
            readonly: ReadOnlyMode::TransactionScoped,
            statement_timeout: TimeoutSupport::Statement,
            cancel: lucent_protocol::CancelMode::Native,
            paging: lucent_protocol::PagingStyle::LimitOffset,
            identifier_quote: '"',
            string_literal: lucent_protocol::StringLiteralStyle::StandardConforming,
            auth: lucent_protocol::AuthModel::UserPassword,
        }
    }

    fn fake_server_info() -> ServerInfo {
        ServerInfo {
            version: "fake".into(),
            capabilities: fake_capabilities(),
        }
    }

    /// C1 regression: a failed Connect used to arrive as
    /// `Error { query_id: None }`, which the reader dropped — the caller
    /// burned its full 10s timeout and reported "connect response timed out".
    /// The worker now replies `ConnectionError`, which must route to the sync
    /// oneshot and surface as the real error.
    #[tokio::test]
    async fn connect_failure_surfaces_as_the_real_error_not_a_timeout() {
        let dir = tempfile::TempDir::new().unwrap();
        let socket_path = dir.path().join("worker.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut framed = new_framed(stream);
            let _version: u32 = read_message(&mut framed).await.unwrap().unwrap();
            let _token: String = read_message(&mut framed).await.unwrap().unwrap();
            write_message(&mut framed, &WorkerResponse::HandshakeAccepted)
                .await
                .unwrap();
            // Connect #1 (the initial connect in ConnectorClient::connect):
            // succeed.
            let request: WorkerRequest = read_message(&mut framed).await.unwrap().unwrap();
            let connection_id = match request {
                WorkerRequest::Connect { connection_id, .. } => connection_id,
                other => panic!("expected Connect, got {other:?}"),
            };
            write_message(
                &mut framed,
                &WorkerResponse::Connected {
                    connection_id,
                    server_info: fake_server_info(),
                },
            )
            .await
            .unwrap();
            // Connect #2 (connect_with_id): fail with the typed error.
            let request: WorkerRequest = read_message(&mut framed).await.unwrap().unwrap();
            let connection_id = match request {
                WorkerRequest::Connect { connection_id, .. } => connection_id,
                other => panic!("expected Connect, got {other:?}"),
            };
            write_message(
                &mut framed,
                &WorkerResponse::ConnectionError {
                    connection_id,
                    kind: lucent_protocol::LucentErrorKind::AuthenticationFailed,
                    message: "password authentication failed for user \"postgres\"".into(),
                },
            )
            .await
            .unwrap();
        });

        let (client, _conn_id) = ConnectorClient::connect(
            socket_path.to_str().unwrap(),
            "test-token",
            ConnectionConfig::default(),
        )
        .await
        .expect("handshake + initial connect must succeed");

        let err = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            client.connect_with_id(ConnectionId(Uuid::new_v4()), ConnectionConfig::default()),
        )
        .await
        .expect("connect_with_id must NOT burn the full 10s timeout")
        .expect_err("the failed connect must return an error");
        assert!(
            err.contains("password authentication failed"),
            "the real error must surface, got: {err}"
        );
        assert!(
            !err.contains("timed out"),
            "no lying timeout message, got: {err}"
        );
    }

    #[test]
    fn pending_guard_removes_the_entry_on_drop() {
        // C9 regression: a dropped/cancelled execute future used to leave its
        // pending-map entry in place until socket EOF. The guard must remove it.
        let pending: Arc<
            std::sync::Mutex<HashMap<QueryId, mpsc::UnboundedSender<WorkerResponse>>>,
        > = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let qid = QueryId(Uuid::new_v4());
        let (tx, _rx) = mpsc::unbounded_channel();
        pending.lock().unwrap().insert(qid, tx);
        {
            let _guard = PendingGuard {
                pending: Arc::clone(&pending),
                query_id: qid,
            };
        }
        assert!(
            pending.lock().unwrap().is_empty(),
            "dropping the guard must remove the entry"
        );
    }
}
