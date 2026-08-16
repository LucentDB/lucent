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
async fn executes_multi_statement_returning_the_last_result_set() {
    // C3: multi-statement scripts are documented as executing (HARD_ROW_CAP
    // doc, row-cap tests, AGENTS.md) but prepare() rejected them with
    // "cannot insert multiple commands into a prepared statement". The
    // simple-query fallback executes them; the grid shows the LAST result
    // set (the single-grid contract), as text cells.
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (connector, connection_id) = connected(port).await;

    let (tx, mut rx) = mpsc::channel(4);
    let qid = QueryId(Uuid::new_v4());
    connector
        .execute(
            connection_id,
            qid,
            "SELECT 1 AS a; SELECT 2 AS b, 3 AS c".to_string(),
            tx,
        )
        .await;

    let event = rx.recv().await.unwrap();
    match event {
        ExecutionEvent::Batch(shape, true) => match shape {
            ResultShape::Tabular { columns, rows } => {
                let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
                assert_eq!(names, ["b", "c"], "the LAST result set defines the grid");
                assert_eq!(rows.len(), 1);
                match (&rows[0][0], &rows[0][1]) {
                    (Value::Text(b), Value::Text(c)) => {
                        assert_eq!(b, "2");
                        assert_eq!(c, "3");
                    }
                    other => panic!("expected text cells, got {other:?}"),
                }
            }
            other => panic!("expected Tabular, got {other:?}"),
        },
        _other => panic!("expected final Batch, got non-batch event"),
    }
}

#[tokio::test]
async fn multi_statement_with_a_non_query_last_statement_reports_rows_affected() {
    // C3: when the LAST statement returns no rows, the script reports the
    // last command's row count — a script of only non-query statements
    // renders as "affected", exactly like a single DML statement.
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (connector, connection_id) = connected(port).await;

    let (tx, mut rx) = mpsc::channel(4);
    let qid = QueryId(Uuid::new_v4());
    connector
        .execute(
            connection_id,
            qid,
            "CREATE TEMP TABLE multi_test (id int); INSERT INTO multi_test VALUES (1), (2)"
                .to_string(),
            tx,
        )
        .await;

    let event = rx.recv().await.unwrap();
    match event {
        ExecutionEvent::Batch(shape, true) => match shape {
            ResultShape::Affected { rows_affected } => {
                assert_eq!(rows_affected, 2, "the LAST command's count wins");
            }
            other => panic!("expected Affected, got {other:?}"),
        },
        _other => panic!("expected final Batch, got non-batch event"),
    }
}

#[tokio::test]
async fn oversized_multi_statement_result_sets_are_bounded_at_the_sentinel() {
    // C3/E1: the last result set is buffered at HARD_ROW_CAP + 1 = 10,001
    // rows — the sentinel that makes the client's truncation trigger
    // (`all_rows.len() > cap`) fire. The worker must never buffer more, and
    // the excess rows are counted, never silently merged.
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (connector, connection_id) = connected(port).await;

    let (tx, mut rx) = mpsc::channel(4);
    let qid = QueryId(Uuid::new_v4());
    connector
        .execute(
            connection_id,
            qid,
            "SELECT 1 AS a; SELECT generate_series(1, 20000) AS n".to_string(),
            tx,
        )
        .await;

    let event = rx.recv().await.unwrap();
    match event {
        ExecutionEvent::Batch(shape, true) => match shape {
            ResultShape::Tabular { columns, rows } => {
                let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
                assert_eq!(names, ["n"], "the LAST result set defines the grid");
                assert_eq!(
                    rows.len(),
                    10_001,
                    "the sentinel (HARD_ROW_CAP + 1) bounds the buffered set"
                );
                match &rows[0][0] {
                    Value::Text(v) => assert_eq!(v, "1", "first value of generate_series"),
                    other => panic!("expected text cells, got {other:?}"),
                }
            }
            other => panic!("expected Tabular, got {other:?}"),
        },
        ExecutionEvent::Batch(_, false) => {
            panic!("expected final Batch, got a non-final batch")
        }
        ExecutionEvent::Failed(e) => panic!("expected final Batch, got Failed: {e}"),
    }
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

#[tokio::test]
async fn wide_rows_survive_the_frame_ceiling() {
    // C2 regression: 600 rows × 20 KB = 12 MiB — over the old 8 MiB IPC
    // frame ceiling, which silently dropped the batch and reported "query
    // task exited unexpectedly". The query must stream intact now.
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (connector, connection_id) = connected(port).await;

    let (tx, mut rx) = mpsc::channel(4);
    let qid = QueryId(Uuid::new_v4());
    connector
        .execute(
            connection_id,
            qid,
            "SELECT repeat('x', 20000) AS wide FROM generate_series(1, 600)".to_string(),
            tx,
        )
        .await;

    let mut total_rows = 0usize;
    loop {
        let event = rx.recv().await.unwrap();
        match event {
            ExecutionEvent::Batch(shape, is_final) => match shape {
                ResultShape::Tabular { rows, .. } => {
                    total_rows += rows.len();
                    for row in &rows {
                        match &row[0] {
                            Value::Text(s) => {
                                assert_eq!(s.len(), 20000, "20 KB cells must arrive intact");
                                assert!(!s.contains("truncated"), "no truncation under the cap");
                            }
                            other => panic!("expected Text, got {other:?}"),
                        }
                    }
                    if is_final {
                        break;
                    }
                }
                other => panic!("expected Tabular, got {other:?}"),
            },
            ExecutionEvent::Failed(e) => panic!("expected Batch, got Failed: {e}"),
        }
    }
    assert_eq!(total_rows, 600);
}

#[tokio::test]
async fn oversized_cells_are_truncated_with_a_visible_marker() {
    // C2: a single > 1 MiB cell must not fail the query — it is truncated
    // with a visible marker. (Before the cap, such a cell could blow the
    // frame ceiling and kill the whole batch with a lying error.)
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let (connector, connection_id) = connected(port).await;

    let (tx, mut rx) = mpsc::channel(4);
    let qid = QueryId(Uuid::new_v4());
    connector
        .execute(
            connection_id,
            qid,
            "SELECT repeat('x', 2 * 1024 * 1024) AS big".to_string(),
            tx,
        )
        .await;

    let event = rx.recv().await.unwrap();
    match event {
        ExecutionEvent::Batch(shape, true) => match shape {
            ResultShape::Tabular { rows, .. } => {
                assert_eq!(rows.len(), 1);
                match &rows[0][0] {
                    Value::Text(s) => {
                        assert!(
                            s.ends_with("[truncated at 1 MiB]"),
                            "the truncation marker must be visible: {s:?}"
                        );
                        assert!(s.len() <= 1024 * 1024 + 64, "len = {}", s.len());
                    }
                    other => panic!("expected Text, got {other:?}"),
                }
            }
            other => panic!("expected Tabular, got {other:?}"),
        },
        ExecutionEvent::Batch(_, false) => {
            panic!("expected final Batch, got a non-final batch")
        }
        ExecutionEvent::Failed(e) => panic!("expected final Batch, got Failed: {e}"),
    }
}
