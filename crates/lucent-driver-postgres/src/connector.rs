use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use lucent_protocol::{
    ColumnMeta, ConnectionConfig, ConnectionId, LucentError, LucentErrorKind, QueryId, ResultShape,
    ServerInfo, Value,
};
use lucent_worker_host::{BatchSender, Connector, ExecutionEvent};
use tokio::sync::{Mutex, RwLock};
use tokio_postgres::{Client, NoTls};

fn pg_error_message(e: &tokio_postgres::Error) -> String {
    if let Some(db) = e.as_db_error() {
        db.to_string()
    } else {
        e.to_string()
    }
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
        pg_config.host(&config.host);
        pg_config.port(config.port);
        pg_config.user(&config.user);
        pg_config.password(&config.password);
        pg_config.dbname(&config.database);

        let (client, connection) = pg_config.connect(NoTls).await.map_err(|e| {
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

        Ok(ServerInfo { version })
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

        use futures::StreamExt;
        let mut msg_stream = std::pin::pin!(msg_stream);
        while let Some(msg) = msg_stream.next().await {
            match msg {
                Ok(tokio_postgres::SimpleQueryMessage::Row(row)) => {
                    let values = (0..columns.len())
                        .map(|i| match row.try_get(i) {
                            Ok(Some(v)) => Value::Text(v.to_string()),
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
                Ok(tokio_postgres::SimpleQueryMessage::CommandComplete(_)) => {}
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

        let shape = ResultShape::Tabular {
            columns,
            rows: batch,
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
                .cancel_query(NoTls)
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
}
