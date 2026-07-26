use std::sync::Arc;

use lucent_protocol::{
    new_framed, read_message, write_message, ColumnMeta, ConnectionConfig, ConnectionId, QueryId,
    ServerInfo, Value, WorkerRequest, WorkerResponse,
};
use serde::Serialize;
use tokio::net::UnixStream;
use tokio_util::codec::Framed;
use tokio_util::codec::LengthDelimitedCodec;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
pub struct ExecuteResult {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub row_count: usize,
}

pub struct ConnectorClient {
    framed: Framed<UnixStream, LengthDelimitedCodec>,
    connection_id: ConnectionId,
    pub server_info: Option<ServerInfo>,
}

impl ConnectorClient {
    pub async fn connect(
        socket_path: &std::path::Path,
        token: &str,
        config: ConnectionConfig,
    ) -> Result<Self, String> {
        let stream = UnixStream::connect(socket_path)
            .await
            .map_err(|e| format!("failed to connect to worker socket: {e}"))?;

        let mut framed = new_framed(stream);

        write_message(&mut framed, &token.to_string())
            .await
            .map_err(|e| format!("handshake failed: {e}"))?;

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

        let response: WorkerResponse = read_message(&mut framed)
            .await
            .map_err(|e| format!("connect response failed: {e}"))?
            .ok_or("worker closed connection during connect")?;

        match response {
            WorkerResponse::Connected { server_info, .. } => Ok(Self {
                framed,
                connection_id,
                server_info: Some(server_info),
            }),
            WorkerResponse::Error { kind, message } => Err(format!("{kind}: {message}")),
            other => Err(format!("unexpected response during connect: {other:?}")),
        }
    }

    pub async fn execute(&mut self, sql: &str) -> Result<ExecuteResult, String> {
        let query_id = QueryId(Uuid::new_v4());

        write_message(
            &mut self.framed,
            &WorkerRequest::Execute {
                connection_id: self.connection_id,
                query_id,
                command: sql.to_string(),
            },
        )
        .await
        .map_err(|e| format!("execute request failed: {e}"))?;

        let mut all_columns: Option<Arc<Vec<ColumnMeta>>> = None;
        let mut all_rows: Vec<Vec<serde_json::Value>> = Vec::new();

        loop {
            let response: WorkerResponse = read_message(&mut self.framed)
                .await
                .map_err(|e| format!("execute response failed: {e}"))?
                .ok_or("worker closed connection during execute")?;

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

                        if is_final {
                            let row_count = all_rows.len();
                            let cols = all_columns.map(|c| (*c).clone()).unwrap_or_default();
                            return Ok(ExecuteResult {
                                columns: cols,
                                rows: all_rows,
                                row_count,
                            });
                        }
                    }
                    _ => return Err("unsupported result shape".into()),
                },
                WorkerResponse::Error { kind, message } => {
                    return Err(format!("{kind}: {message}"));
                }
                WorkerResponse::Cancelled { .. } => {
                    return Err("query was cancelled".into());
                }
                other => {
                    return Err(format!("unexpected response: {other:?}"));
                }
            }
        }
    }

    pub async fn disconnect(&mut self) -> Result<(), String> {
        write_message(
            &mut self.framed,
            &WorkerRequest::Disconnect {
                connection_id: self.connection_id,
            },
        )
        .await
        .map_err(|e| format!("disconnect request failed: {e}"))?;

        let response: WorkerResponse = read_message(&mut self.framed)
            .await
            .map_err(|e| format!("disconnect response failed: {e}"))?
            .ok_or("worker closed connection during disconnect")?;

        match response {
            WorkerResponse::Disconnected { .. } => Ok(()),
            WorkerResponse::Error { kind, message } => Err(format!("{kind}: {message}")),
            other => Err(format!("unexpected response during disconnect: {other:?}")),
        }
    }
}

fn value_to_json(value: Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(b),
        Value::Int(i) => serde_json::json!(i),
        Value::Float(f) => serde_json::json!(f),
        Value::Text(s) => serde_json::Value::String(s),
    }
}
