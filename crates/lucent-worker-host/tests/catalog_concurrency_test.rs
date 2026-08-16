//! A slow catalog request must not stall query results on the same socket.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lucent_protocol::{
    new_framed, read_message, write_message, CatalogRequest, CatalogResult, ColumnMeta,
    ConnectionConfig, ConnectionId, LucentError, QueryId, ResultShape, ServerInfo, WorkerRequest,
    WorkerResponse,
};
use lucent_worker_host::{BatchSender, Connector, ExecutionEvent};
use tokio::net::UnixStream;
use uuid::Uuid;

/// The fake catalog connector's ServerInfo. No test asserts on capabilities;
/// a TransactionScoped literal keeps the payload realistic.
fn fake_server_info() -> ServerInfo {
    ServerInfo {
        version: "test".into(),
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

/// `catalog` sleeps; `execute` answers immediately. If the serve loop awaits
/// catalog inline, the execute reply arrives *after* the catalog reply.
struct SlowCatalogConnector;

#[async_trait]
impl Connector for SlowCatalogConnector {
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
        let _ = sender
            .send(ExecutionEvent::Batch(
                ResultShape::Tabular {
                    columns: Arc::new(vec![ColumnMeta {
                        name: "n".into(),
                        type_name: "int4".into(),
                    }]),
                    rows: vec![],
                },
                true,
            ))
            .await;
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
        tokio::time::sleep(Duration::from_millis(400)).await;
        Ok(CatalogResult::Namespaces(vec![]))
    }
}

#[tokio::test]
async fn a_slow_catalog_request_does_not_delay_query_results() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("worker.sock");
    let listener = lucent_worker_host::bind(&socket).unwrap();

    tokio::spawn(async move {
        let _ = lucent_worker_host::serve(listener, "tok".into(), SlowCatalogConnector).await;
    });

    let stream = UnixStream::connect(&socket).await.unwrap();
    let mut framed = new_framed(stream);
    write_message(&mut framed, &lucent_protocol::PROTOCOL_VERSION)
        .await
        .unwrap();
    write_message(&mut framed, &"tok".to_string())
        .await
        .unwrap();

    // ack must arrive before Connect
    let ack: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(matches!(ack, WorkerResponse::HandshakeAccepted));

    let connection_id = ConnectionId(Uuid::new_v4());
    let request_id = QueryId(Uuid::new_v4());
    let query_id = QueryId(Uuid::new_v4());

    // Catalog FIRST, then the query. If catalog is awaited inline, the query
    // request is not even read until the sleep finishes.
    write_message(
        &mut framed,
        &WorkerRequest::Catalog {
            connection_id,
            request_id,
            request: CatalogRequest::ListNamespaces,
        },
    )
    .await
    .unwrap();
    write_message(
        &mut framed,
        &WorkerRequest::Execute {
            connection_id,
            query_id,
            command: "SELECT 1".into(),
        },
    )
    .await
    .unwrap();

    // A regression that stops catalog replies from ever arriving (oob arm
    // removed, spawned task dropped) must fail the test, not hang CI to the
    // job-level timeout — bound both reads.
    let first: WorkerResponse =
        tokio::time::timeout(Duration::from_secs(5), read_message(&mut framed))
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for the query ResultBatch"))
            .unwrap()
            .unwrap();
    assert!(
        matches!(first, WorkerResponse::ResultBatch { .. }),
        "the query result must arrive first — a slow catalog request must not \
         block the serve loop; got {first:?}"
    );

    let second: WorkerResponse =
        tokio::time::timeout(Duration::from_secs(5), read_message(&mut framed))
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for the CatalogResult reply"))
            .unwrap()
            .unwrap();
    match second {
        WorkerResponse::CatalogResult {
            request_id: got, ..
        } => assert_eq!(got, request_id),
        other => panic!("expected CatalogResult second, got {other:?}"),
    }
}
