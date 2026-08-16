use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lucent_protocol::{
    new_framed, read_message, write_message, CatalogRequest, CatalogResult, ColumnMeta,
    ConnectionConfig, ConnectionId, LucentError, LucentErrorKind, QueryId, ResultShape, ServerInfo,
    Value, WorkerRequest, WorkerResponse,
};
use lucent_worker_host::{bind, serve, BatchSender, Connector, ExecutionEvent};
use tokio::net::UnixStream;
use uuid::Uuid;

struct FakeConnector;

/// A fake `ServerInfo` for the fake connectors. The fake declares itself
/// TransactionScoped like Postgres so the serve loop sees a realistic
/// capabilities payload; no test asserts on it.
fn fake_server_info() -> ServerInfo {
    ServerInfo {
        version: "fake-1.0".to_string(),
        capabilities: lucent_protocol::DriverCapabilities {
            id: "fake".into(),
            display_name: "Fake".into(),
            sql_dialect: lucent_protocol::SqlDialect::PostgreSql,
            namespace_model: lucent_protocol::NamespaceModel::DbSchemaObject,
            readonly: lucent_protocol::ReadOnlyMode::TransactionScoped,
            statement_timeout: lucent_protocol::TimeoutSupport::Statement,
            cancel: lucent_protocol::CancelMode::Native,
            paging: lucent_protocol::PagingStyle::LimitOffset,
            identifier_quote: '"',
            string_literal: lucent_protocol::StringLiteralStyle::StandardConforming,
            auth: lucent_protocol::AuthModel::UserPassword,
        },
    }
}

#[async_trait]
impl Connector for FakeConnector {
    async fn connect(
        &self,
        _connection_id: ConnectionId,
        _config: ConnectionConfig,
    ) -> Result<ServerInfo, LucentError> {
        Ok(fake_server_info())
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
            rows: vec![vec![Value::Text("1".into())]],
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

    async fn catalog(
        &self,
        _connection_id: ConnectionId,
        _request: CatalogRequest,
    ) -> Result<CatalogResult, LucentError> {
        Err(LucentError::new(
            LucentErrorKind::Internal,
            "FakeConnector does not answer catalog requests",
        ))
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

    write_message(&mut framed, &lucent_protocol::PROTOCOL_VERSION)
        .await
        .unwrap();
    write_message(&mut framed, &token).await.unwrap();

    // NEW: ack must arrive before Connect
    let ack: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(matches!(ack, WorkerResponse::HandshakeAccepted));

    let connection_id = ConnectionId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Connect {
            connection_id,
            config: ConnectionConfig::new("postgres")
                .with("host", "localhost")
                .with("port", "5432")
                .with("user", "u")
                .with("database", "d")
                .with("ssl_mode", "prefer"),
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
    write_message(&mut framed, &lucent_protocol::PROTOCOL_VERSION)
        .await
        .unwrap();
    write_message(&mut framed, &"wrong-token".to_string())
        .await
        .unwrap();

    let resp: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    match resp {
        WorkerResponse::Error { kind, message, .. } => {
            assert_eq!(kind, lucent_protocol::LucentErrorKind::Protocol);
            assert!(message.contains("token"));
        }
        other => panic!("expected Protocol error, got {other:?}"),
    }
}

#[tokio::test]
async fn rejects_a_wrong_protocol_version_with_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path: PathBuf = dir.path().join("worker.sock");

    let listener = bind(&socket_path).unwrap();
    tokio::spawn(async move {
        let _ = serve(listener, "expected-token".to_string(), FakeConnector).await;
    });

    let stream = UnixStream::connect(&socket_path).await.unwrap();
    let mut framed = new_framed(stream);
    // Send a version the worker does not expect as the first message.
    write_message(&mut framed, &(lucent_protocol::PROTOCOL_VERSION + 1))
        .await
        .unwrap();

    let resp: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    match resp {
        WorkerResponse::Error { kind, message, .. } => {
            assert_eq!(kind, lucent_protocol::LucentErrorKind::Protocol);
            assert!(
                message.contains("version"),
                "mismatch error must mention the version: {message}"
            );
        }
        other => panic!("expected Protocol error, got {other:?}"),
    }
}

struct PanicOnceConnector {
    panicked: AtomicBool,
}

#[async_trait]
impl Connector for PanicOnceConnector {
    async fn connect(
        &self,
        _connection_id: ConnectionId,
        _config: ConnectionConfig,
    ) -> Result<ServerInfo, LucentError> {
        Ok(fake_server_info())
    }

    async fn execute(
        &self,
        _connection_id: ConnectionId,
        _query_id: QueryId,
        _command: String,
        sender: BatchSender,
    ) {
        if !self.panicked.swap(true, Ordering::SeqCst) {
            panic!("simulated execute panic");
        }
        let shape = ResultShape::Tabular {
            columns: Arc::new(vec![ColumnMeta {
                name: "n".to_string(),
                type_name: "int4".to_string(),
            }]),
            rows: vec![vec![Value::Text("1".into())]],
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

    async fn catalog(
        &self,
        _connection_id: ConnectionId,
        _request: CatalogRequest,
    ) -> Result<CatalogResult, LucentError> {
        Err(LucentError::new(
            LucentErrorKind::Internal,
            "PanicOnceConnector does not answer catalog requests",
        ))
    }
}

#[tokio::test]
async fn dropped_batch_sender_does_not_spin_or_leak() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path: PathBuf = dir.path().join("worker.sock");
    let token = "test-token".to_string();

    let listener = bind(&socket_path).unwrap();
    let server_token = token.clone();
    let connector = PanicOnceConnector {
        panicked: AtomicBool::new(false),
    };
    tokio::spawn(async move {
        serve(listener, server_token, connector).await.unwrap();
    });

    let stream = UnixStream::connect(&socket_path).await.unwrap();
    let mut framed = new_framed(stream);

    write_message(&mut framed, &lucent_protocol::PROTOCOL_VERSION)
        .await
        .unwrap();
    write_message(&mut framed, &token).await.unwrap();

    // ack must arrive before Connect
    let ack: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(matches!(ack, WorkerResponse::HandshakeAccepted));

    let connection_id = ConnectionId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Connect {
            connection_id,
            config: ConnectionConfig::new("postgres")
                .with("host", "localhost")
                .with("port", "5432")
                .with("user", "u")
                .with("database", "d")
                .with("ssl_mode", "prefer"),
        },
    )
    .await
    .unwrap();

    let response: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    match response {
        WorkerResponse::Connected { .. } => {}
        other => panic!("expected Connected, got {other:?}"),
    }

    let qid1 = QueryId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Execute {
            connection_id,
            query_id: qid1,
            command: "PANIC".to_string(),
        },
    )
    .await
    .unwrap();

    let response: WorkerResponse = tokio::time::timeout(
        Duration::from_secs(5),
        read_message::<WorkerResponse, _>(&mut framed),
    )
    .await
    .expect("serve loop should emit an error, not spin")
    .unwrap()
    .unwrap();
    match response {
        WorkerResponse::Error {
            kind: LucentErrorKind::Internal,
            message,
            query_id,
        } => {
            assert_eq!(message, "query task exited unexpectedly");
            assert_eq!(query_id, Some(qid1));
        }
        other => panic!("expected Internal Error, got {other:?}"),
    }

    let qid2 = QueryId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Execute {
            connection_id,
            query_id: qid2,
            command: "SELECT 1".to_string(),
        },
    )
    .await
    .unwrap();

    let response: WorkerResponse = tokio::time::timeout(
        Duration::from_secs(5),
        read_message::<WorkerResponse, _>(&mut framed),
    )
    .await
    .expect("serve loop should still serve a second Execute")
    .unwrap()
    .unwrap();
    match response {
        WorkerResponse::ResultBatch { is_final: true, .. } => {}
        other => panic!("expected ResultBatch, got {other:?}"),
    }

    write_message(&mut framed, &WorkerRequest::Disconnect { connection_id })
        .await
        .unwrap();

    let response: WorkerResponse = tokio::time::timeout(
        Duration::from_secs(5),
        read_message::<WorkerResponse, _>(&mut framed),
    )
    .await
    .expect("serve loop should still serve Disconnect")
    .unwrap()
    .unwrap();
    match response {
        WorkerResponse::Disconnected { .. } => {}
        other => panic!("expected Disconnected, got {other:?}"),
    }
}

fn test_config() -> ConnectionConfig {
    ConnectionConfig::new("postgres")
        .with("host", "localhost")
        .with("port", "5432")
        .with("user", "u")
        .with("database", "d")
        .with("ssl_mode", "prefer")
}

struct SlowFirstConnectConnector {
    first_connect_hung: AtomicBool,
}

#[async_trait]
impl Connector for SlowFirstConnectConnector {
    async fn connect(
        &self,
        _connection_id: ConnectionId,
        _config: ConnectionConfig,
    ) -> Result<ServerInfo, LucentError> {
        if !self.first_connect_hung.swap(true, Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        Ok(fake_server_info())
    }

    async fn execute(
        &self,
        _connection_id: ConnectionId,
        _query_id: QueryId,
        _command: String,
        _sender: BatchSender,
    ) {
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

    async fn catalog(
        &self,
        _connection_id: ConnectionId,
        _request: CatalogRequest,
    ) -> Result<CatalogResult, LucentError> {
        Err(LucentError::new(
            LucentErrorKind::Internal,
            "SlowFirstConnectConnector does not answer catalog requests",
        ))
    }
}

#[tokio::test]
async fn connect_timeout_does_not_block_the_serve_loop() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path: PathBuf = dir.path().join("worker.sock");
    let token = "test-token".to_string();

    let listener = bind(&socket_path).unwrap();
    let server_token = token.clone();
    let connector = SlowFirstConnectConnector {
        first_connect_hung: AtomicBool::new(false),
    };
    tokio::spawn(async move {
        serve(listener, server_token, connector).await.unwrap();
    });

    let stream = UnixStream::connect(&socket_path).await.unwrap();
    let mut framed = new_framed(stream);

    write_message(&mut framed, &lucent_protocol::PROTOCOL_VERSION)
        .await
        .unwrap();
    write_message(&mut framed, &token).await.unwrap();

    // ack must arrive before Connect
    let ack: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(matches!(ack, WorkerResponse::HandshakeAccepted));

    let hang_id = ConnectionId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Connect {
            connection_id: hang_id,
            config: test_config(),
        },
    )
    .await
    .unwrap();

    let response: WorkerResponse = tokio::time::timeout(
        Duration::from_secs(20),
        read_message::<WorkerResponse, _>(&mut framed),
    )
    .await
    .expect("serve loop should answer a hanging connect within the timeout")
    .unwrap()
    .unwrap();
    match response {
        WorkerResponse::ConnectionError {
            kind: LucentErrorKind::Timeout,
            message,
            connection_id,
        } => {
            assert_eq!(connection_id, hang_id);
            assert!(
                message.contains("connect timed out after"),
                "unexpected message: {message}"
            );
        }
        other => panic!("expected ConnectionError, got {other:?}"),
    }

    // The same connector succeeds on its next connect, proving the serve loop
    // kept serving after the first connect timed out.
    let ok_id = ConnectionId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Connect {
            connection_id: ok_id,
            config: test_config(),
        },
    )
    .await
    .unwrap();

    let response: WorkerResponse = tokio::time::timeout(
        Duration::from_secs(5),
        read_message::<WorkerResponse, _>(&mut framed),
    )
    .await
    .expect("serve loop should still serve a second Connect")
    .unwrap()
    .unwrap();
    match response {
        WorkerResponse::Connected { server_info, .. } => {
            assert_eq!(server_info.version, "fake-1.0")
        }
        other => panic!("expected Connected, got {other:?}"),
    }
}

#[derive(Clone, Default)]
struct PanicOnDemandConnector {
    panic_queries: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

#[async_trait]
impl Connector for PanicOnDemandConnector {
    async fn connect(
        &self,
        _connection_id: ConnectionId,
        _config: ConnectionConfig,
    ) -> Result<ServerInfo, LucentError> {
        Ok(fake_server_info())
    }
    async fn execute(
        &self,
        _connection_id: ConnectionId,
        _query_id: QueryId,
        command: String,
        sender: BatchSender,
    ) {
        let should_panic = self.panic_queries.lock().unwrap().contains(&command);
        tokio::spawn(async move {
            let _ = sender
                .send(ExecutionEvent::Batch(
                    ResultShape::Tabular {
                        columns: Arc::new(vec![ColumnMeta {
                            name: "n".to_string(),
                            type_name: "int4".to_string(),
                        }]),
                        rows: vec![vec![Value::Text("1".into())]],
                    },
                    false,
                ))
                .await;
            if should_panic {
                panic!("query task panicked on demand");
            }
            let _ = sender
                .send(ExecutionEvent::Batch(
                    ResultShape::Tabular {
                        columns: Arc::new(vec![]),
                        rows: vec![],
                    },
                    true,
                ))
                .await;
        });
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
    async fn catalog(
        &self,
        _connection_id: ConnectionId,
        _request: CatalogRequest,
    ) -> Result<CatalogResult, LucentError> {
        Err(LucentError::new(
            LucentErrorKind::Internal,
            "PanicOnDemandConnector does not answer catalog requests",
        ))
    }
}

#[tokio::test]
async fn a_panicking_query_does_not_evict_sibling_queries() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path: PathBuf = dir.path().join("evict.sock");
    let token = "test-token".to_string();

    let listener = bind(&socket_path).unwrap();
    let connector = PanicOnDemandConnector {
        panic_queries: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::from(
            ["SELECT PANIC".to_string()],
        ))),
    };
    let server_token = token.clone();
    let server = tokio::spawn(async move {
        serve(listener, server_token, connector).await.unwrap();
    });

    let stream = UnixStream::connect(&socket_path).await.unwrap();
    let mut framed = new_framed(stream);

    write_message(&mut framed, &lucent_protocol::PROTOCOL_VERSION)
        .await
        .unwrap();
    write_message(&mut framed, &token).await.unwrap();
    let ack: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(matches!(ack, WorkerResponse::HandshakeAccepted));

    let conn_id = ConnectionId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Connect {
            connection_id: conn_id,
            config: ConnectionConfig::new("fake"),
        },
    )
    .await
    .unwrap();
    let resp: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(matches!(resp, WorkerResponse::Connected { .. }));

    let ids = [
        QueryId(Uuid::new_v4()),
        QueryId(Uuid::new_v4()),
        QueryId(Uuid::new_v4()),
    ];
    for (cmd, id) in ["SELECT A", "SELECT PANIC", "SELECT C"].iter().zip(ids) {
        write_message(
            &mut framed,
            &WorkerRequest::Execute {
                connection_id: conn_id,
                query_id: id,
                command: cmd.to_string(),
            },
        )
        .await
        .unwrap();
    }

    // Both healthy queries must reach is_final; the panicking one gets an Error.
    let mut healthy_finals = 0;
    let mut panic_error = false;
    for _ in 0..20 {
        let resp: WorkerResponse =
            tokio::time::timeout(Duration::from_secs(2), read_message(&mut framed))
                .await
                .expect("responses keep coming")
                .unwrap()
                .unwrap();
        match &resp {
            WorkerResponse::ResultBatch { is_final: true, .. } => healthy_finals += 1,
            WorkerResponse::Error { message, .. } if message.contains("exited unexpectedly") => {
                panic_error = true;
            }
            _ => {}
        }
        if healthy_finals >= 2 && panic_error {
            break;
        }
    }
    assert!(healthy_finals >= 2, "siblings must finish");
    assert!(panic_error, "panicking query must produce an Error");
    server.abort();
}

struct FailingConnectConnector;

#[async_trait]
impl Connector for FailingConnectConnector {
    async fn connect(
        &self,
        _connection_id: ConnectionId,
        _config: ConnectionConfig,
    ) -> Result<ServerInfo, LucentError> {
        Err(LucentError::new(
            LucentErrorKind::AuthenticationFailed,
            "password authentication failed for user \"postgres\"",
        ))
    }

    async fn execute(
        &self,
        _connection_id: ConnectionId,
        _query_id: QueryId,
        _command: String,
        _sender: BatchSender,
    ) {
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

    async fn catalog(
        &self,
        _connection_id: ConnectionId,
        _request: CatalogRequest,
    ) -> Result<CatalogResult, LucentError> {
        Err(LucentError::new(
            LucentErrorKind::Internal,
            "FailingConnectConnector does not answer catalog requests",
        ))
    }
}

/// Like SlowFirstConnectConnector, but execute actually streams a batch so
/// tests can prove the serve loop keeps forwarding while a Connect hangs.
struct StreamingSlowConnectConnector {
    first_connect_hung: AtomicBool,
}

#[async_trait]
impl Connector for StreamingSlowConnectConnector {
    async fn connect(
        &self,
        _connection_id: ConnectionId,
        _config: ConnectionConfig,
    ) -> Result<ServerInfo, LucentError> {
        if !self.first_connect_hung.swap(true, Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        Ok(fake_server_info())
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
                name: "c".into(),
                type_name: "int4".into(),
            }]),
            rows: vec![vec![Value::Int64(1)]],
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

    async fn catalog(
        &self,
        _connection_id: ConnectionId,
        _request: CatalogRequest,
    ) -> Result<CatalogResult, LucentError> {
        Err(LucentError::new(
            LucentErrorKind::Internal,
            "StreamingSlowConnectConnector does not answer catalog requests",
        ))
    }
}

#[tokio::test]
async fn connect_failure_replies_connection_error() {
    // C1: a failed Connect must reply ConnectionError (correlatable by
    // connection_id), NOT Error { query_id: None }, which the app reader
    // drops and turns into a 10s timeout.
    let dir = tempfile::tempdir().unwrap();
    let socket_path: PathBuf = dir.path().join("worker.sock");
    let token = "test-token".to_string();
    let listener = bind(&socket_path).unwrap();
    tokio::spawn(async move {
        serve(listener, token, FailingConnectConnector)
            .await
            .unwrap();
    });

    let stream = UnixStream::connect(&socket_path).await.unwrap();
    let mut framed = new_framed(stream);
    write_message(&mut framed, &lucent_protocol::PROTOCOL_VERSION)
        .await
        .unwrap();
    write_message(&mut framed, &"test-token").await.unwrap();
    let ack: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(matches!(ack, WorkerResponse::HandshakeAccepted));

    let connection_id = ConnectionId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Connect {
            connection_id,
            config: test_config(),
        },
    )
    .await
    .unwrap();

    let response: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    match response {
        WorkerResponse::ConnectionError {
            connection_id: got,
            kind,
            message,
        } => {
            assert_eq!(got, connection_id);
            assert!(matches!(kind, LucentErrorKind::AuthenticationFailed));
            assert!(
                message.contains("password authentication failed"),
                "{message}"
            );
        }
        other => panic!("expected ConnectionError, got {other:?}"),
    }
}

#[tokio::test]
async fn a_hanging_connect_does_not_stall_streaming_for_other_queries() {
    // C8 regression: Connect used to run inline in the select loop, so a
    // hanging connect (15s timeout) froze every in-flight query on the
    // socket. Spawned, the loop must keep forwarding batches.
    let dir = tempfile::tempdir().unwrap();
    let socket_path: PathBuf = dir.path().join("worker.sock");
    let token = "test-token".to_string();
    let listener = bind(&socket_path).unwrap();
    let connector = StreamingSlowConnectConnector {
        first_connect_hung: AtomicBool::new(false),
    };
    tokio::spawn(async move {
        serve(listener, token, connector).await.unwrap();
    });

    let stream = UnixStream::connect(&socket_path).await.unwrap();
    let mut framed = new_framed(stream);
    write_message(&mut framed, &lucent_protocol::PROTOCOL_VERSION)
        .await
        .unwrap();
    write_message(&mut framed, &"test-token").await.unwrap();
    let ack: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(matches!(ack, WorkerResponse::HandshakeAccepted));

    // An in-flight query that streams a batch.
    let conn = ConnectionId(Uuid::new_v4());
    let qid1 = QueryId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Execute {
            connection_id: conn,
            query_id: qid1,
            command: "SELECT 1".into(),
        },
    )
    .await
    .unwrap();
    let response: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(matches!(response, WorkerResponse::ResultBatch { .. }));

    // NOW hang the worker on a Connect. The next query's batch must arrive
    // long before the 15s connect timeout.
    let hang_id = ConnectionId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Connect {
            connection_id: hang_id,
            config: test_config(),
        },
    )
    .await
    .unwrap();

    let qid2 = QueryId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Execute {
            connection_id: conn,
            query_id: qid2,
            command: "SELECT 2".into(),
        },
    )
    .await
    .unwrap();

    let response: WorkerResponse = tokio::time::timeout(
        Duration::from_secs(2),
        read_message::<WorkerResponse, _>(&mut framed),
    )
    .await
    .expect("serve loop must keep streaming while a Connect hangs")
    .unwrap()
    .unwrap();
    assert!(matches!(response, WorkerResponse::ResultBatch { .. }));

    // And the hanging connect still resolves, with a typed timeout error.
    let response: WorkerResponse = tokio::time::timeout(
        Duration::from_secs(20),
        read_message::<WorkerResponse, _>(&mut framed),
    )
    .await
    .expect("connect timeout reply must arrive")
    .unwrap()
    .unwrap();
    match response {
        WorkerResponse::ConnectionError {
            connection_id: got,
            kind,
            message,
        } => {
            assert_eq!(got, hang_id);
            assert!(matches!(kind, LucentErrorKind::Timeout), "{message}");
            assert!(message.contains("connect timed out after"), "{message}");
        }
        other => panic!("expected ConnectionError, got {other:?}"),
    }
}
