use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use lucent_protocol::{
    new_framed, read_message, write_message, ColumnMeta, ConnectionConfig, ConnectionId,
    LucentError, QueryId, ResultShape, ServerInfo, Value, WorkerRequest, WorkerResponse,
};
use lucent_worker_host::{bind, serve, BatchSender, Connector, ExecutionEvent};
use tokio::net::UnixStream;
use uuid::Uuid;

struct FakeConnector;

#[async_trait]
impl Connector for FakeConnector {
    async fn connect(
        &self,
        _connection_id: ConnectionId,
        _config: ConnectionConfig,
    ) -> Result<ServerInfo, LucentError> {
        Ok(ServerInfo {
            version: "fake-1.0".to_string(),
        })
    }

    async fn execute(
        &self,
        _connection_id: ConnectionId,
        _query_id: QueryId,
        _command: String,
        sender: BatchSender,
    ) {
        let shape = ResultShape::Tabular {
            columns: Arc::new(vec![ColumnMeta {
                name: "n".to_string(),
                type_name: "int4".to_string(),
            }]),
            rows: vec![vec![Value::Int(1)]],
        };
        let _ = sender.send(ExecutionEvent::Batch(shape, true)).await;
    }

    async fn cancel(
        &self,
        _connection_id: ConnectionId,
        _query_id: QueryId,
    ) -> Result<(), LucentError> {
        Ok(())
    }

    async fn disconnect(&self, _connection_id: ConnectionId) -> Result<(), LucentError> {
        Ok(())
    }
}

#[tokio::test]
async fn connects_and_streams_a_result_batch() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path: PathBuf = dir.path().join("worker.sock");
    let token = "test-token".to_string();

    let listener = bind(&socket_path).unwrap();
    let server_token = token.clone();
    tokio::spawn(async move {
        serve(listener, server_token, FakeConnector).await.unwrap();
    });

    let stream = UnixStream::connect(&socket_path).await.unwrap();
    let mut framed = new_framed(stream);

    write_message(&mut framed, &token).await.unwrap();

    let connection_id = ConnectionId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Connect {
            connection_id,
            config: ConnectionConfig {
                host: "localhost".to_string(),
                port: 5432,
                user: "u".to_string(),
                password: "p".to_string(),
                database: "d".to_string(),
                ssl_mode: "prefer".to_string(),
            },
        },
    )
    .await
    .unwrap();

    let response: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    match response {
        WorkerResponse::Connected { server_info, .. } => {
            assert_eq!(server_info.version, "fake-1.0")
        }
        other => panic!("expected Connected, got {other:?}"),
    }

    let query_id = QueryId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Execute {
            connection_id,
            query_id,
            command: "SELECT 1".to_string(),
        },
    )
    .await
    .unwrap();

    let response: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    match response {
        WorkerResponse::ResultBatch {
            is_final, shape, ..
        } => match shape {
            ResultShape::Tabular { rows, .. } => {
                assert!(is_final);
                assert_eq!(rows.len(), 1);
            }
            _ => panic!("expected Tabular result shape"),
        },
        other => panic!("expected ResultBatch, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_a_connection_with_the_wrong_handshake_token() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path: PathBuf = dir.path().join("worker.sock");

    let listener = bind(&socket_path).unwrap();
    tokio::spawn(async move {
        let _ = serve(listener, "expected-token".to_string(), FakeConnector).await;
    });

    let stream = UnixStream::connect(&socket_path).await.unwrap();
    let mut framed = new_framed(stream);
    write_message(&mut framed, &"wrong-token".to_string())
        .await
        .unwrap();

    let response: Option<WorkerResponse> = read_message(&mut framed).await.unwrap();
    assert!(
        response.is_none(),
        "server should close the connection on bad handshake"
    );
}
