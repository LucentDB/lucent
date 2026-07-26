use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::LucentErrorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnectionId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QueryId(pub Uuid);

#[derive(Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    /// SSL mode: "disable", "prefer", or "require".
    /// Worker implementations may ignore this (e.g. NoTls) but it is
    /// stored for profile round-trip and future TLS support.
    #[serde(default = "default_ssl_mode")]
    pub ssl_mode: String,
}

pub fn default_ssl_mode() -> String {
    "prefer".into()
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 0,
            user: String::new(),
            password: String::new(),
            database: String::new(),
            ssl_mode: default_ssl_mode(),
        }
    }
}

impl std::fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &"***")
            .field("database", &self.database)
            .field("ssl_mode", &self.ssl_mode)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnMeta {
    pub name: String,
    pub type_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ResultShape {
    Tabular {
        columns: Arc<Vec<ColumnMeta>>,
        rows: Vec<Vec<Value>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerRequest {
    Connect {
        connection_id: ConnectionId,
        config: ConnectionConfig,
    },
    Execute {
        connection_id: ConnectionId,
        query_id: QueryId,
        command: String,
    },
    Cancel {
        connection_id: ConnectionId,
        query_id: QueryId,
    },
    Disconnect {
        connection_id: ConnectionId,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerResponse {
    Connected {
        connection_id: ConnectionId,
        server_info: ServerInfo,
    },
    ResultBatch {
        query_id: QueryId,
        shape: ResultShape,
        sequence: u32,
        is_final: bool,
    },
    Error {
        kind: LucentErrorKind,
        message: String,
    },
    Cancelled {
        query_id: QueryId,
    },
    Disconnected {
        connection_id: ConnectionId,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_connect_request_through_bincode() {
        let request = WorkerRequest::Connect {
            connection_id: ConnectionId(uuid::Uuid::new_v4()),
            config: ConnectionConfig {
                host: "localhost".to_string(),
                port: 5432,
                user: "postgres".to_string(),
                password: "postgres".to_string(),
                database: "postgres".to_string(),
                ssl_mode: default_ssl_mode(),
            },
        };

        let bytes = bincode::serialize(&request).unwrap();
        let decoded: WorkerRequest = bincode::deserialize(&bytes).unwrap();

        match decoded {
            WorkerRequest::Connect { config, .. } => {
                assert_eq!(config.host, "localhost");
                assert_eq!(config.port, 5432);
            }
            _ => panic!("expected Connect variant"),
        }
    }

    #[test]
    fn round_trips_a_tabular_result_batch() {
        let response = WorkerResponse::ResultBatch {
            query_id: QueryId(uuid::Uuid::new_v4()),
            shape: ResultShape::Tabular {
                columns: Arc::new(vec![ColumnMeta {
                    name: "id".to_string(),
                    type_name: "int4".to_string(),
                }]),
                rows: vec![vec![Value::Int(1)], vec![Value::Null]],
            },
            sequence: 0,
            is_final: true,
        };

        let bytes = bincode::serialize(&response).unwrap();
        let decoded: WorkerResponse = bincode::deserialize(&bytes).unwrap();

        match decoded {
            WorkerResponse::ResultBatch {
                shape, is_final, ..
            } => match shape {
                ResultShape::Tabular { rows, .. } => {
                    assert_eq!(rows.len(), 2);
                    assert!(is_final);
                }
            },
            _ => panic!("expected ResultBatch variant"),
        }
    }
}
