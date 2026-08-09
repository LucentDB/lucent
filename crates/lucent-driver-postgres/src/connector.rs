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
                                Some(ty) => crate::decode::pg_text_to_value(v, ty),
                                // No metadata for this column (shape mismatch
                                // between prepare() and the result). Text is
                                // always correct-if-untyped; never drop the value.
                                None => Value::Text(v.to_string()),
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
