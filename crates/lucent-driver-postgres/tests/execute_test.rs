use std::time::Duration;

use lucent_driver_postgres::PostgresConnector;
use lucent_protocol::{ConnectionConfig, ConnectionId, QueryId, ResultShape, Value};
use lucent_worker_host::ExecutionEvent;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::sync::mpsc;
use uuid::Uuid;

async fn connect_with_retry(
    connector: &PostgresConnector,
    connection_id: ConnectionId,
    config: ConnectionConfig,
) {
    for i in 0..10 {
        match connector.connect(connection_id, config.clone()).await {
            Ok(_) => return,
            Err(_) if i < 9 => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(e) => panic!("connect failed after 10 retries: {e}"),
        }
    }
}

async fn connected(port: u16) -> (PostgresConnector, ConnectionId) {
    let connector = PostgresConnector::default();
    let connection_id = ConnectionId(Uuid::new_v4());
    connect_with_retry(
        &connector,
        connection_id,
        ConnectionConfig::new("postgres")
            .with("host", "127.0.0.1")
            .with("port", port.to_string())
            .with("user", "postgres")
            .with("database", "postgres")
            .with("ssl_mode", "prefer")
            .with_secret("postgres"),
    )
    .await;
    (connector, connection_id)
}

#[tokio::test]
async fn executes_a_query_and_streams_typed_rows() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (connector, connection_id) = connected(port).await;

    let (tx, mut rx) = mpsc::channel(4);
    let query_id = QueryId(Uuid::new_v4());
    connector
        .execute(
            connection_id,
            query_id,
            "SELECT 1::int4 AS n, true AS flag, 'hello' AS greeting".to_string(),
            tx,
        )
        .await;

    let event = rx.recv().await.unwrap();
    match event {
        ExecutionEvent::Batch(ResultShape::Tabular { columns, rows }, is_final) => {
            assert!(is_final);
            assert_eq!(columns.len(), 3);
            assert_eq!(rows.len(), 1);
            assert!(matches!(rows[0][0], Value::Int64(1)));
            assert!(matches!(rows[0][1], Value::Bool(true)));
            assert!(matches!(rows[0][2], Value::Text(ref s) if s == "hello"));
        }
        ExecutionEvent::Batch(_, _) => panic!("expected Tabular result shape"),
        ExecutionEvent::Failed(e) => panic!("expected a batch, got error: {e}"),
    }

    assert!(
        rx.recv().await.is_none(),
        "channel should close after the final batch"
    );
}

#[tokio::test]
async fn splits_large_results_into_multiple_batches() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (connector, connection_id) = connected(port).await;

    let (tx, mut rx) = mpsc::channel(4);
    let query_id = QueryId(Uuid::new_v4());
    connector
        .execute(
            connection_id,
            query_id,
            "SELECT generate_series(1, 1200)::int4 AS n".to_string(),
            tx,
        )
        .await;

    let mut total_rows = 0;
    let mut batch_count = 0;
    let mut saw_final = false;

    while let Some(event) = rx.recv().await {
        match event {
            ExecutionEvent::Batch(ResultShape::Tabular { rows, .. }, is_final) => {
                total_rows += rows.len();
                batch_count += 1;
                saw_final = is_final;
            }
            ExecutionEvent::Batch(_, _) => {}
            ExecutionEvent::Failed(e) => panic!("unexpected error: {e}"),
        }
    }

    assert_eq!(total_rows, 1200);
    assert!(
        batch_count >= 3,
        "1200 rows at 500/batch should take at least 3 batches, got {batch_count}"
    );
    assert!(saw_final);
}

#[tokio::test]
async fn reports_bad_sql_as_a_failed_event() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (connector, connection_id) = connected(port).await;

    let (tx, mut rx) = mpsc::channel(4);
    let query_id = QueryId(Uuid::new_v4());
    connector
        .execute(
            connection_id,
            query_id,
            "SELECT this is not sql".to_string(),
            tx,
        )
        .await;

    let event = rx.recv().await.unwrap();
    assert!(matches!(event, ExecutionEvent::Failed(_)));
}

#[tokio::test]
async fn rejects_multi_statement_sql_explicitly() {
    // prepare() rejects multi-command bodies — this is intended (metadata
    // requires a single statement). The failure must be a Failed event,
    // never a hang or a partial execution.
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (connector, connection_id) = connected(port).await;

    let (tx, mut rx) = mpsc::channel(4);
    let qid = QueryId(Uuid::new_v4());
    connector
        .execute(connection_id, qid, "SELECT 1; SELECT 2".to_string(), tx)
        .await;
    let event = rx.recv().await.unwrap();
    assert!(matches!(event, ExecutionEvent::Failed(_)));
}

#[tokio::test]
async fn reports_rows_affected_for_dml() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (connector, connection_id) = connected(port).await;

    let (create_tx, mut create_rx) = mpsc::channel(4);
    let create_qid = QueryId(Uuid::new_v4());
    connector
        .execute(
            connection_id,
            create_qid,
            "CREATE TEMP TABLE t(x int)".to_string(),
            create_tx,
        )
        .await;
    while let Some(_) = create_rx.recv().await {}

    let (tx, mut rx) = mpsc::channel(4);
    let qid = QueryId(Uuid::new_v4());
    connector
        .execute(
            connection_id,
            qid,
            "INSERT INTO t VALUES (1),(2),(3)".to_string(),
            tx,
        )
        .await;
    let mut saw_affected = false;
    while let Some(event) = rx.recv().await {
        match event {
            ExecutionEvent::Batch(ResultShape::Affected { rows_affected }, true) => {
                saw_affected = true;
                assert_eq!(rows_affected, 3);
            }
            ExecutionEvent::Failed(e) => panic!("expected Affected, got error: {e}"),
            _ => {}
        }
    }
    assert!(saw_affected, "DML must emit an Affected shape");
}

#[tokio::test]
async fn empty_select_stays_tabular_with_columns() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (connector, connection_id) = connected(port).await;

    let (tx, mut rx) = mpsc::channel(4);
    let query_id = QueryId(Uuid::new_v4());
    connector
        .execute(
            connection_id,
            query_id,
            "SELECT 1::int4 AS n WHERE false".to_string(),
            tx,
        )
        .await;

    let event = rx.recv().await.unwrap();
    match event {
        ExecutionEvent::Batch(ResultShape::Tabular { columns, rows }, is_final) => {
            assert!(is_final);
            assert_eq!(columns.len(), 1);
            assert_eq!(rows.len(), 0);
        }
        ExecutionEvent::Batch(_, _) => panic!("expected Tabular result shape"),
        ExecutionEvent::Failed(e) => panic!("expected a batch, got error: {e}"),
    }

    assert!(
        rx.recv().await.is_none(),
        "channel should close after the final batch"
    );
}
