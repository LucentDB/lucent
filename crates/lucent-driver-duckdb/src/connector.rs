//! `Connector` implementation for DuckDB.
//!
//! Every database call runs inside `spawn_blocking`: `duckdb::Connection`
//! blocks, and occupying a Tokio worker thread with a two-minute scan would
//! stall every other connection in this process.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use lucent_protocol::{
    CatalogRequest, CatalogResult, ColumnMeta, ConnectionConfig, ConnectionId, LucentError,
    LucentErrorKind, QueryId, ResultShape, ServerInfo, Value,
};
use lucent_worker_host::{BatchSender, Connector, ExecutionEvent};
use tokio::sync::{Mutex, RwLock};

use crate::connection::DuckHandle;

/// Rows per batch. Matches the Postgres driver so backpressure behaves
/// identically across drivers.
const BATCH_SIZE: usize = 500;

#[derive(Default)]
pub struct DuckDbConnector {
    handles: RwLock<HashMap<ConnectionId, Arc<DuckHandle>>>,
    /// The query currently running on each connection. DuckDB's interrupt is
    /// connection-scoped, so `cancel` must confirm the caller means the query
    /// that is actually in flight — otherwise a stale cancel kills a query the
    /// user never asked to stop.
    in_flight: Mutex<HashMap<ConnectionId, QueryId>>,
}

impl DuckDbConnector {
    async fn handle(&self, connection_id: ConnectionId) -> Option<Arc<DuckHandle>> {
        self.handles.read().await.get(&connection_id).cloned()
    }
}

#[async_trait]
impl Connector for DuckDbConnector {
    async fn connect(
        &self,
        connection_id: ConnectionId,
        config: ConnectionConfig,
    ) -> Result<ServerInfo, LucentError> {
        let path = config
            .require("path")
            .map_err(|m| LucentError::new(LucentErrorKind::Internal, m))?
            .to_string();
        // Absent means read-write, which is what a query editor needs.
        let read_only = config.get("read_only") == Some("true");

        // Opening a database file is filesystem work and can block.
        let handle = tokio::task::spawn_blocking(move || DuckHandle::open(&path, read_only))
            .await
            .map_err(|e| {
                LucentError::new(LucentErrorKind::Internal, format!("open task: {e}"))
            })??;

        let version = {
            let handle = Arc::new(handle);
            let probe = handle.clone();
            let version = tokio::task::spawn_blocking(move || {
                probe.with_conn(|conn| {
                    conn.query_row("SELECT version()", [], |row| row.get::<_, String>(0))
                        .map_err(|e| e.to_string())
                })
            })
            .await
            .map_err(|e| LucentError::new(LucentErrorKind::Internal, format!("version task: {e}")))?
            .unwrap_or_else(|_| "unknown".to_string());

            self.handles.write().await.insert(connection_id, handle);
            version
        };

        Ok(ServerInfo {
            version,
            capabilities: crate::capabilities::duckdb(read_only),
        })
    }

    async fn execute(
        &self,
        connection_id: ConnectionId,
        query_id: QueryId,
        command: String,
        sender: BatchSender,
    ) {
        let Some(handle) = self.handle(connection_id).await else {
            let _ = sender
                .send(ExecutionEvent::Failed(LucentError::new(
                    LucentErrorKind::Internal,
                    "unknown connection",
                )))
                .await;
            return;
        };

        self.in_flight.lock().await.insert(connection_id, query_id);

        let outcome = tokio::task::spawn_blocking(move || run_query(&handle, &command, sender))
            .await
            .unwrap_or_else(|e| {
                Err(LucentError::new(
                    LucentErrorKind::Internal,
                    format!("query task panicked: {e}"),
                ))
            });

        // Clear only if this query is still the registered one — a newer query
        // must not have its registration erased by an older one finishing.
        {
            let mut in_flight = self.in_flight.lock().await;
            if in_flight.get(&connection_id) == Some(&query_id) {
                in_flight.remove(&connection_id);
            }
        }

        if let Err(e) = outcome {
            // eprintln!, not log::debug!: the worker binary initializes no
            // logger (log:: would be silently dropped), and the worker-host
            // convention is eprintln! — the supervisor captures worker
            // stderr into the in-app Logs drawer.
            // A user-cancelled query is not an error to log: the stop button
            // fires it on purpose, and every cancel would otherwise fill the
            // Logs drawer with noise.
            if e.kind != LucentErrorKind::QueryCancelled {
                eprintln!("duckdb query {query_id:?} ended with: {e}");
            }
        }
    }

    async fn cancel(
        &self,
        connection_id: ConnectionId,
        query_id: QueryId,
    ) -> Result<(), LucentError> {
        // DuckDB interrupts the CONNECTION, not a statement. Firing it for a
        // query that already finished would kill whatever started since.
        let running = self.in_flight.lock().await.get(&connection_id).copied();
        if running != Some(query_id) {
            return Ok(());
        }
        let Some(handle) = self.handle(connection_id).await else {
            return Ok(());
        };
        // Deliberately does NOT take the connection lock: the query being
        // cancelled is holding it.
        handle.interrupt();
        Ok(())
    }

    async fn disconnect(&self, connection_id: ConnectionId) -> Result<(), LucentError> {
        self.in_flight.lock().await.remove(&connection_id);
        // Dropping the last Arc closes the database and releases the file lock.
        self.handles.write().await.remove(&connection_id);
        Ok(())
    }

    async fn catalog(
        &self,
        connection_id: ConnectionId,
        request: CatalogRequest,
    ) -> Result<CatalogResult, LucentError> {
        let handle = self
            .handle(connection_id)
            .await
            .ok_or_else(|| LucentError::new(LucentErrorKind::Internal, "unknown connection"))?;
        tokio::task::spawn_blocking(move || crate::catalog::handle(&handle, request))
            .await
            .map_err(|e| {
                LucentError::new(LucentErrorKind::Internal, format!("catalog task: {e}"))
            })?
    }
}

/// Run one statement, streaming batches. Blocking — call inside `spawn_blocking`.
fn run_query(handle: &DuckHandle, command: &str, sender: BatchSender) -> Result<(), LucentError> {
    let result = handle.with_conn(|conn| {
        let mut stmt = conn.prepare(command).map_err(|e| e.to_string())?;

        // Execute FIRST: in duckdb 1.10505.0 the statement-metadata accessors
        // panic until the statement has been stepped. `Rows` borrows `stmt`,
        // but re-exposes it via `as_ref()` precisely so metadata can be read
        // while the rows are alive.
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        let stmt_ref = rows
            .as_ref()
            .expect("rows always borrow the statement they came from");
        let column_count = stmt_ref.column_count();
        let columns: Vec<ColumnMeta> = (0..column_count)
            .map(|i| ColumnMeta {
                name: stmt_ref
                    .column_name(i)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|_| format!("column{i}")),
                type_name: decl_type(stmt_ref, i),
            })
            .collect();
        let decl_types: Vec<String> = (0..column_count).map(|i| decl_type(stmt_ref, i)).collect();
        let columns = Arc::new(columns);
        let mut batch: Vec<Vec<Value>> = Vec::with_capacity(BATCH_SIZE);

        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let values = (0..column_count)
                .map(|i| match row.get_ref(i) {
                    Ok(v) => crate::decode::duck_value_to_value(v, &decl_types[i]),
                    // A cell we cannot read must not lose the whole row.
                    Err(_) => Value::Null,
                })
                .collect();
            batch.push(values);

            if batch.len() >= BATCH_SIZE {
                // `blocking_send` applies the same backpressure the Postgres
                // driver gets from `send().await`: when the consumer is slow,
                // this blocks and we stop pulling rows.
                let full = std::mem::replace(&mut batch, Vec::with_capacity(BATCH_SIZE));
                sender
                    .blocking_send(ExecutionEvent::Batch(
                        ResultShape::Tabular {
                            columns: columns.clone(),
                            rows: full,
                        },
                        false,
                    ))
                    .map_err(|_| "result consumer went away".to_string())?;
            }
        }

        // The final batch always sends, even when empty, so the app's
        // `execute_with_id` loop always terminates.
        sender
            .blocking_send(ExecutionEvent::Batch(
                ResultShape::Tabular {
                    columns,
                    rows: batch,
                },
                true,
            ))
            .map_err(|_| "result consumer went away".to_string())?;

        Ok(())
    });

    // An interrupted query is not a syntax error. `with_conn` maps every
    // closure error to QuerySyntaxError, but the stop-button path must
    // surface as cancelled — otherwise the app blames the user's SQL for a
    // cancel it requested. DuckDB reports interruption as a runtime error
    // whose text mentions "interrupt"/"cancelled"; detect that and remap.
    // Everything that is not parser/binder/interrupt is a runtime failure
    // (constraint violations, file errors, engine errors) — those must not
    // surface as syntax errors either, or the UI and the AI guard blame the
    // user's SQL for problems that have nothing to do with it.
    let result = result.map_err(|e| {
        let lower = e.message.to_ascii_lowercase();
        if lower.contains("interrupt") || lower.contains("cancelled") {
            LucentError::new(LucentErrorKind::QueryCancelled, "query interrupted")
        } else if lower.contains("parser error")
            || lower.contains("binder error")
            // A missing table/column surfaces as "Catalog Error" in duckdb
            // 1.10505.0. The Postgres driver reports the same class of
            // prepare-time failure ("relation does not exist") as
            // QuerySyntaxError; map it the same way so the two drivers
            // agree about whose fault it is.
            || lower.contains("catalog error")
        {
            // The user's SQL is at fault; QuerySyntaxError is the right kind.
            e
        } else {
            LucentError::new(LucentErrorKind::Internal, e.message)
        }
    });

    if let Err(ref e) = result {
        let _ = sender.blocking_send(ExecutionEvent::Failed(e.clone()));
    }
    result
}

/// The column's declared type, which is what tells the decoder whether a
/// timestamp is an instant or a wall-clock reading.
///
/// duckdb 1.10505.0's C API distinguishes `TIMESTAMP_TZ` from `TIMESTAMP`
/// (`LogicalTypeId::TimestampTZ`); the Arrow `DataType` conversion and
/// `Type`'s `Display` both drop the flag, which would make every timestamp
/// look like a wall-clock reading. Everything else falls back to the Arrow
/// `DataType`'s `Display`.
fn decl_type(stmt: &duckdb::Statement<'_>, index: usize) -> String {
    let logical = stmt.column_logical_type(index);
    match logical.id() {
        duckdb::core::LogicalTypeId::TimestampTZ => "TIMESTAMPTZ".to_string(),
        // A JSON column is VARCHAR with the alias "JSON" — the only signal
        // that distinguishes it from plain text at this crate version (no
        // LogicalTypeId::Json variant exists in duckdb 1.10505.0).
        _ if logical.get_alias().as_deref() == Some("JSON") => "JSON".to_string(),
        _ => stmt.column_type(index).to_string(),
    }
}
