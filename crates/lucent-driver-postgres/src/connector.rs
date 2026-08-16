use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use lucent_protocol::{
    ColumnMeta, ConnectionConfig, ConnectionId, LucentError, LucentErrorKind, QueryId, ResultShape,
    ServerInfo, Value,
};
use lucent_worker_host::{BatchSender, Connector, ExecutionEvent};
use tokio::sync::{Mutex, RwLock};
use tokio_postgres::Client;

fn pg_error_message(e: &tokio_postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        db.to_string()
    } else {
        e.to_string()
    }
}

/// Map the config's ssl_mode parameter onto tokio-postgres. An absent or
/// unknown value falls back to Prefer — never to a stronger mode, which would
/// turn a working connection into a confusing failure.
fn ssl_mode(config: &ConnectionConfig) -> tokio_postgres::config::SslMode {
    match config.get("ssl_mode") {
        Some("disable") => tokio_postgres::config::SslMode::Disable,
        Some("require") => tokio_postgres::config::SslMode::Require,
        _ => tokio_postgres::config::SslMode::Prefer,
    }
}

fn cfg_err(message: String) -> LucentError {
    LucentError::new(LucentErrorKind::Internal, message)
}

/// rustls connector over the system root store. `require` against a server
/// without TLS fails loudly here — there is no silent plaintext downgrade.
fn make_tls() -> tokio_postgres_rustls::MakeRustlsConnect {
    // The workspace graph enables both rustls crypto providers (bollard +
    // this crate), so no default is auto-selected — install one explicitly.
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(cert);
    }
    tokio_postgres_rustls::MakeRustlsConnect::new(
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    )
}

const BATCH_SIZE: usize = 500;

/// Cap on rows buffered per result set in the multi-statement path, set one
/// row ABOVE the app's client-side `HARD_ROW_CAP` (10,000 — `src-tauri/
/// client.rs`). The +1 sentinel makes the client's truncation trigger
/// (`all_rows.len() > cap`) fire: a script whose last statement returns more
/// than 10,000 rows arrives as a 10,001-row final batch, the client
/// truncates to 10,000, and reports `ExecuteResult.truncated` so the UI can
/// tell the user. WITHOUT the sentinel the client sees exactly `cap` rows,
/// the `>` comparison never fires, and the cut is silent. Must stay
/// `HARD_ROW_CAP + 1` — the two are coupled by contract (C3).
const MULTI_STATEMENT_BUFFER_CAP: usize = 10_001;

/// PostgreSQL's prepare-time rejection of multi-command bodies. The message
/// is stable across versions (it comes from the parser's grammar action).
/// Detected by message rather than by sqlparser: the driver crate has no
/// parser dependency.
fn is_multiple_commands_error(e: &tokio_postgres::Error) -> bool {
    e.as_db_error()
        .map(|d| {
            d.message()
                .contains("cannot insert multiple commands into a prepared statement")
        })
        .unwrap_or(false)
}

/// One result set buffered in the multi-statement path: column metadata
/// (names only — the simple protocol carries no type OIDs) plus its rows.
type BufferedResultSet = (Arc<Vec<ColumnMeta>>, Vec<Vec<Value>>);

/// Cells larger than this are truncated (with a visible marker) before they
/// enter a batch. Bounds one row's contribution to the IPC frame so the
/// worker's batch splitting always terminates; 1 MiB covers every realistic
/// cell while staying far below the 256 MiB frame ceiling (C2).
const MAX_CELL_BYTES: usize = 1024 * 1024;
const CELL_TRUNCATION_MARKER: &str = "… [truncated at 1 MiB]";

/// Truncate a `&str` at a char boundary no later than `max_bytes`.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Truncate an oversized cell payload so a pathological row cannot blow the
/// IPC frame ceiling. Truncation is visible — the marker guarantees the user
/// can tell the value was cut, never a silently wrong datum.
fn cap_cell(value: Value) -> Value {
    fn cap_text(s: String) -> String {
        if s.len() <= MAX_CELL_BYTES {
            s
        } else {
            let mut cut = truncate_utf8(&s, MAX_CELL_BYTES).to_string();
            cut.push_str(CELL_TRUNCATION_MARKER);
            cut
        }
    }
    match value {
        Value::Text(s) => Value::Text(cap_text(s)),
        Value::Json(s) => Value::Json(cap_text(s)),
        Value::Interval(s) => Value::Interval(cap_text(s)),
        Value::Decimal(s) => Value::Decimal(cap_text(s)),
        Value::Other { type_name, text } => Value::Other {
            type_name,
            text: cap_text(text),
        },
        Value::Binary(bytes) => {
            if bytes.len() > MAX_CELL_BYTES {
                let mut cut = bytes[..MAX_CELL_BYTES].to_vec();
                cut.extend_from_slice(CELL_TRUNCATION_MARKER.as_bytes());
                Value::Binary(cut)
            } else {
                Value::Binary(bytes)
            }
        }
        other => other,
    }
}

#[derive(Default)]
pub struct PostgresConnector {
    clients: RwLock<HashMap<ConnectionId, Arc<Client>>>,
    cancel_tokens: Mutex<HashMap<QueryId, tokio_postgres::CancelToken>>,
}

impl PostgresConnector {
    pub async fn connect(
        &self,
        connection_id: ConnectionId,
        config: ConnectionConfig,
    ) -> Result<ServerInfo, LucentError> {
        let mut pg_config = tokio_postgres::Config::new();
        pg_config.host(config.require("host").map_err(cfg_err)?);
        pg_config.port(config.port().unwrap_or(5432));
        pg_config.user(config.require("user").map_err(cfg_err)?);
        pg_config.password(config.secret.as_deref().unwrap_or(""));
        pg_config.dbname(config.require("database").map_err(cfg_err)?);
        pg_config.ssl_mode(ssl_mode(&config));

        let (client, connection) = pg_config.connect(make_tls()).await.map_err(|e| {
            let kind = match e.code() {
                Some(code) if code == &tokio_postgres::error::SqlState::INVALID_PASSWORD => {
                    LucentErrorKind::AuthenticationFailed
                }
                Some(code) if code == &tokio_postgres::error::SqlState::CANNOT_CONNECT_NOW => {
                    LucentErrorKind::ConnectionRefused
                }
                _ if e.to_string().contains("timeout") => LucentErrorKind::Timeout,
                _ => LucentErrorKind::ConnectionRefused,
            };
            LucentError::new(kind, pg_error_message(&e))
        })?;

        tokio::spawn(async move {
            let _ = connection.await;
        });

        let version_row = client
            .query_one("SHOW server_version", &[])
            .await
            .map_err(|e| LucentError::new(LucentErrorKind::Internal, pg_error_message(&e)))?;
        let version: String = version_row.get(0);

        self.clients
            .write()
            .await
            .insert(connection_id, Arc::new(client));

        Ok(ServerInfo {
            version,
            capabilities: crate::capabilities::postgres(),
        })
    }

    pub async fn disconnect(&self, connection_id: ConnectionId) -> Result<(), LucentError> {
        self.clients.write().await.remove(&connection_id);
        Ok(())
    }

    pub async fn execute(
        &self,
        connection_id: ConnectionId,
        query_id: QueryId,
        command: String,
        sender: BatchSender,
    ) {
        let client = {
            let clients = self.clients.read().await;
            clients.get(&connection_id).cloned()
        };
        let client = match client {
            Some(c) => c,
            None => {
                let _ = sender
                    .send(ExecutionEvent::Failed(LucentError::new(
                        LucentErrorKind::Internal,
                        "unknown connection",
                    )))
                    .await;
                return;
            }
        };

        self.cancel_tokens
            .lock()
            .await
            .insert(query_id, client.cancel_token());

        // Step 1: Prepare the statement to get column metadata (names + type names).
        // prepare() only parses the query on the server — it does NOT execute it.
        let statement = match client.prepare(&command).await {
            Ok(s) => s,
            Err(e) if is_multiple_commands_error(&e) => {
                // Multi-statement scripts cannot be prepared, but they ARE
                // meant to execute (HARD_ROW_CAP doc, row-cap tests,
                // AGENTS.md). Fall back to the simple-query path, which
                // streams every result set and keeps the LAST one (C3).
                return self
                    .execute_multi_statement(query_id, command, sender, client)
                    .await;
            }
            Err(e) => {
                let _ = sender
                    .send(ExecutionEvent::Failed(LucentError::new(
                        LucentErrorKind::QuerySyntaxError,
                        pg_error_message(&e),
                    )))
                    .await;
                self.cancel_tokens.lock().await.remove(&query_id);
                return;
            }
        };

        let pg_types: Arc<Vec<tokio_postgres::types::Type>> = Arc::new(
            statement
                .columns()
                .iter()
                .map(|c| c.type_().clone())
                .collect(),
        );

        let columns: Arc<Vec<ColumnMeta>> = Arc::new(
            statement
                .columns()
                .iter()
                .map(|c| ColumnMeta {
                    name: c.name().to_string(),
                    type_name: c.type_().name().to_string(),
                })
                .collect(),
        );

        // Step 2: Execute via simple_query_raw (text protocol) so PostgreSQL
        // handles ALL type-to-text conversions server-side. This supports every
        // type (interval, tstzrange, arrays, etc.) without binary decoders.
        let msg_stream = match client.simple_query_raw(&command).await {
            Ok(s) => s,
            Err(e) => {
                let _ = sender
                    .send(ExecutionEvent::Failed(LucentError::new(
                        LucentErrorKind::Internal,
                        pg_error_message(&e),
                    )))
                    .await;
                self.cancel_tokens.lock().await.remove(&query_id);
                return;
            }
        };

        let mut batch: Vec<Vec<Value>> = Vec::with_capacity(BATCH_SIZE);
        let mut emitted_rows = false;
        let mut rows_affected: Option<u64> = None;

        use futures::StreamExt;
        let mut msg_stream = std::pin::pin!(msg_stream);
        while let Some(msg) = msg_stream.next().await {
            match msg {
                Ok(tokio_postgres::SimpleQueryMessage::Row(row)) => {
                    emitted_rows = true;
                    let values = (0..columns.len())
                        .map(|i| match row.try_get(i) {
                            Ok(Some(v)) => match pg_types.get(i) {
                                Some(ty) => cap_cell(crate::decode::pg_text_to_value(v, ty)),
                                // No metadata for this column (shape mismatch
                                // between prepare() and the result). Text is
                                // always correct-if-untyped; never drop the value.
                                None => cap_cell(Value::Text(v.to_string())),
                            },
                            _ => Value::Null,
                        })
                        .collect();
                    batch.push(values);

                    if batch.len() >= BATCH_SIZE {
                        let shape = ResultShape::Tabular {
                            columns: Arc::clone(&columns),
                            rows: std::mem::take(&mut batch),
                        };
                        if sender
                            .send(ExecutionEvent::Batch(shape, false))
                            .await
                            .is_err()
                        {
                            self.cancel_tokens.lock().await.remove(&query_id);
                            return;
                        }
                    }
                }
                Ok(tokio_postgres::SimpleQueryMessage::CommandComplete(n)) => {
                    if !emitted_rows && columns.is_empty() {
                        rows_affected = Some(n);
                    }
                }
                Ok(tokio_postgres::SimpleQueryMessage::RowDescription(_)) => {
                    // Column metadata already obtained from prepare() above
                }
                Err(e) => {
                    let _ = sender
                        .send(ExecutionEvent::Failed(LucentError::new(
                            LucentErrorKind::Internal,
                            pg_error_message(&e),
                        )))
                        .await;
                    self.cancel_tokens.lock().await.remove(&query_id);
                    return;
                }
                _ => {}
            }
        }

        if let Some(n) = rows_affected {
            let _ = sender
                .send(ExecutionEvent::Batch(
                    ResultShape::Affected { rows_affected: n },
                    true,
                ))
                .await;
        } else {
            let shape = ResultShape::Tabular {
                columns,
                rows: batch,
            };
            let _ = sender.send(ExecutionEvent::Batch(shape, true)).await;
        }
        self.cancel_tokens.lock().await.remove(&query_id);
    }

    /// Execute a multi-statement script via the simple query protocol.
    ///
    /// The simple protocol streams per-statement message groups:
    /// `RowDescription` → rows → `CommandComplete`. The grid contract is one
    /// shape per query, so the LAST result set wins: earlier sets' rows are
    /// counted and discarded (never silently — see the warning below), and
    /// every value renders as `Value::Text` because the simple protocol
    /// carries column NAMES only, no type OIDs. Single statements keep the
    /// typed `prepare()` path above.
    ///
    /// The cancel token inserted before `prepare()` stays live for this
    /// whole call, so cancels work mid-script.
    async fn execute_multi_statement(
        &self,
        query_id: QueryId,
        command: String,
        sender: BatchSender,
        client: Arc<Client>,
    ) {
        let msg_stream = match client.simple_query_raw(&command).await {
            Ok(s) => s,
            Err(e) => {
                let _ = sender
                    .send(ExecutionEvent::Failed(LucentError::new(
                        LucentErrorKind::Internal,
                        pg_error_message(&e),
                    )))
                    .await;
                self.cancel_tokens.lock().await.remove(&query_id);
                return;
            }
        };

        let mut current: Option<BufferedResultSet> = None;
        let mut superseded_rows: usize = 0;
        let mut cap_excess_rows: usize = 0;
        let mut last_command_complete: Option<u64> = None;

        use futures::StreamExt;
        let mut msg_stream = std::pin::pin!(msg_stream);
        while let Some(msg) = msg_stream.next().await {
            match msg {
                Ok(tokio_postgres::SimpleQueryMessage::RowDescription(desc)) => {
                    // A new result set began. Drop the previous set's
                    // buffered rows — last result set wins — but count them
                    // so the discard is never silent.
                    if let Some((_, rows)) = current.take() {
                        superseded_rows += rows.len();
                    }
                    let columns = Arc::new(
                        desc.iter()
                            .map(|c| ColumnMeta {
                                name: c.name().to_string(),
                                // The simple protocol carries names only;
                                // every value renders as text.
                                type_name: "text".to_string(),
                            })
                            .collect::<Vec<_>>(),
                    );
                    current = Some((columns, Vec::new()));
                }
                Ok(tokio_postgres::SimpleQueryMessage::Row(row)) => {
                    let values = (0..row.columns().len())
                        .map(|i| match row.get(i) {
                            Some(v) => cap_cell(Value::Text(v.to_string())),
                            None => Value::Null,
                        })
                        .collect();
                    match current.as_mut() {
                        Some((_, rows)) if rows.len() < MULTI_STATEMENT_BUFFER_CAP => {
                            rows.push(values);
                        }
                        Some((_, rows)) => {
                            // Sentinel reached: keep draining the stream (the
                            // query must run to completion — this set may not
                            // be the last, and cancelling mid-script would
                            // abort later statements' side effects). Rows
                            // beyond the sentinel are dropped here, in the
                            // worker, so worker memory stays bounded; the
                            // client's own truncation fires on the 10,001st
                            // row and reports truncated=true.
                            let _ = rows;
                            cap_excess_rows += 1;
                        }
                        None => {
                            // Unreachable by the simple protocol (Row always
                            // follows RowDescription); counted as dropped either
                            // way so the grid never silently loses a row.
                            cap_excess_rows += 1;
                        }
                    }
                }
                Ok(tokio_postgres::SimpleQueryMessage::CommandComplete(n)) => {
                    last_command_complete = Some(n);
                }
                Err(e) => {
                    let _ = sender
                        .send(ExecutionEvent::Failed(LucentError::new(
                            LucentErrorKind::Internal,
                            pg_error_message(&e),
                        )))
                        .await;
                    self.cancel_tokens.lock().await.remove(&query_id);
                    return;
                }
                // SimpleQueryMessage is #[non_exhaustive]; unknown variants
                // (e.g. PortalSuspended in extended protocol) are inert here.
                _ => {}
            }
        }

        if superseded_rows > 0 || cap_excess_rows > 0 {
            // eprintln!, not log::warn!: the worker binary initializes no
            // logger (log:: would be silently dropped), and the worker-host
            // convention is eprintln! — the supervisor captures worker
            // stderr into the in-app Logs drawer, so this warning is
            // actually visible.
            eprintln!(
                "worker: multi-statement query {query_id:?}: {superseded_rows} rows from \
                 earlier result sets discarded (last result set wins); {cap_excess_rows} rows \
                 dropped past the per-set cap of {MULTI_STATEMENT_BUFFER_CAP}"
            );
        }

        let shape = match current {
            Some((columns, rows)) => ResultShape::Tabular { columns, rows },
            None => ResultShape::Affected {
                rows_affected: last_command_complete.unwrap_or(0),
            },
        };
        let _ = sender.send(ExecutionEvent::Batch(shape, true)).await;
        self.cancel_tokens.lock().await.remove(&query_id);
    }

    pub async fn cancel(
        &self,
        _connection_id: ConnectionId,
        query_id: QueryId,
    ) -> Result<(), LucentError> {
        let token = self.cancel_tokens.lock().await.get(&query_id).cloned();
        match token {
            Some(token) => token
                // cancel opens its own connection — it must speak TLS too
                .cancel_query(make_tls())
                .await
                .map_err(|e| LucentError::new(LucentErrorKind::Internal, pg_error_message(&e))),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl Connector for PostgresConnector {
    async fn connect(
        &self,
        connection_id: ConnectionId,
        config: ConnectionConfig,
    ) -> Result<ServerInfo, LucentError> {
        PostgresConnector::connect(self, connection_id, config).await
    }

    async fn execute(
        &self,
        connection_id: ConnectionId,
        query_id: QueryId,
        command: String,
        sender: BatchSender,
    ) {
        PostgresConnector::execute(self, connection_id, query_id, command, sender).await
    }

    async fn cancel(
        &self,
        connection_id: ConnectionId,
        query_id: QueryId,
    ) -> Result<(), LucentError> {
        PostgresConnector::cancel(self, connection_id, query_id).await
    }

    async fn disconnect(&self, connection_id: ConnectionId) -> Result<(), LucentError> {
        PostgresConnector::disconnect(self, connection_id).await
    }

    async fn catalog(
        &self,
        connection_id: ConnectionId,
        request: lucent_protocol::CatalogRequest,
    ) -> Result<lucent_protocol::CatalogResult, LucentError> {
        let client = {
            let clients = self.clients.read().await;
            clients.get(&connection_id).cloned()
        }
        .ok_or_else(|| LucentError::new(LucentErrorKind::Internal, "unknown connection"))?;
        crate::catalog::handle(&client, request).await
    }
}

#[cfg(test)]
mod ssl_mode_tests {
    use super::*;

    fn cfg(mode: &str) -> ConnectionConfig {
        ConnectionConfig::new("postgres").with("ssl_mode", mode)
    }

    #[test]
    fn maps_disable_prefer_require() {
        assert!(matches!(
            ssl_mode(&cfg("disable")),
            tokio_postgres::config::SslMode::Disable
        ));
        assert!(matches!(
            ssl_mode(&cfg("prefer")),
            tokio_postgres::config::SslMode::Prefer
        ));
        assert!(matches!(
            ssl_mode(&cfg("require")),
            tokio_postgres::config::SslMode::Require
        ));
    }

    #[test]
    fn unknown_values_fall_back_to_prefer() {
        assert!(matches!(
            ssl_mode(&cfg("bogus")),
            tokio_postgres::config::SslMode::Prefer
        ));
    }
}
