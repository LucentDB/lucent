//! End-to-end driver behaviour. No Docker: DuckDB is in-process.

use std::time::Duration;

use lucent_driver_duckdb::connector::DuckDbConnector;
use lucent_protocol::{ConnectionConfig, ConnectionId, QueryId, ResultShape, Value};
use lucent_worker_host::{Connector, ExecutionEvent};
use uuid::Uuid;

async fn connected() -> (DuckDbConnector, ConnectionId) {
    let connector = DuckDbConnector::default();
    let connection_id = ConnectionId(Uuid::new_v4());
    connector
        .connect(
            connection_id,
            ConnectionConfig::new("duckdb").with("path", ":memory:"),
        )
        .await
        .expect("connect to in-memory duckdb");
    (connector, connection_id)
}

/// Run a statement and collect every row it produced.
async fn run(connector: &DuckDbConnector, cid: ConnectionId, sql: &str) -> Vec<Vec<Value>> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let query_id = QueryId(Uuid::new_v4());
    let sql = sql.to_string();
    let exec = connector.execute(cid, query_id, sql, tx);

    let collect = async {
        let mut rows = Vec::new();
        while let Some(event) = rx.recv().await {
            match event {
                ExecutionEvent::Batch(ResultShape::Tabular { rows: batch, .. }, _) => {
                    rows.extend(batch)
                }
                ExecutionEvent::Batch(..) => {}
                ExecutionEvent::Failed(e) => panic!("query failed: {e}"),
            }
        }
        rows
    };

    let (_, rows) = tokio::join!(exec, collect);
    rows
}

#[tokio::test]
async fn connects_reports_a_version_and_declares_its_capabilities() {
    let connector = DuckDbConnector::default();
    let cid = ConnectionId(Uuid::new_v4());
    let info = connector
        .connect(
            cid,
            ConnectionConfig::new("duckdb").with("path", ":memory:"),
        )
        .await
        .expect("connect");

    assert!(!info.version.is_empty(), "version must be reported");
    assert_eq!(info.capabilities.id, "duckdb");
    assert_eq!(
        info.capabilities.readonly,
        lucent_protocol::ReadOnlyMode::GuardOnly,
        "a read-write connection has no engine-enforced read-only"
    );
}

#[tokio::test]
async fn a_read_only_connection_declares_the_stronger_guarantee() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ro.duckdb");
    let path_str = path.to_string_lossy().to_string();
    {
        let (c, cid) = connected().await;
        let _ = c.disconnect(cid).await;
    }
    // Create the file first — read-only cannot create.
    {
        let connector = DuckDbConnector::default();
        let cid = ConnectionId(Uuid::new_v4());
        connector
            .connect(cid, ConnectionConfig::new("duckdb").with("path", &path_str))
            .await
            .unwrap();
        run(&connector, cid, "CREATE TABLE t (x int)").await;
        connector.disconnect(cid).await.unwrap();
    }

    let connector = DuckDbConnector::default();
    let cid = ConnectionId(Uuid::new_v4());
    let info = connector
        .connect(
            cid,
            ConnectionConfig::new("duckdb")
                .with("path", &path_str)
                .with("read_only", "true"),
        )
        .await
        .expect("connect read-only");
    assert_eq!(
        info.capabilities.readonly,
        lucent_protocol::ReadOnlyMode::SessionFlag
    );
}

#[tokio::test]
async fn a_missing_required_parameter_is_a_named_error_not_a_panic() {
    let connector = DuckDbConnector::default();
    let err = connector
        .connect(
            ConnectionId(Uuid::new_v4()),
            ConnectionConfig::new("duckdb"),
        )
        .await
        .unwrap_err();
    assert!(err.message.contains("path"), "must name the field: {err}");
}

#[tokio::test]
async fn returns_typed_values_end_to_end() {
    let (connector, cid) = connected().await;
    let rows = run(
        &connector,
        cid,
        "SELECT 42::BIGINT AS i, 1.5::DOUBLE AS f, 'hi' AS s, \
                true AS b, DATE '2024-01-15' AS d, NULL::INTEGER AS n, \
                170141183460469231731687303715884105727::HUGEINT AS h, \
                now()::TIMESTAMPTZ AS ts",
    )
    .await;

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert!(matches!(row[0], Value::Int64(42)), "{:?}", row[0]);
    assert!(
        matches!(row[1], Value::Float64(f) if f == 1.5),
        "{:?}",
        row[1]
    );
    assert!(
        matches!(row[2], Value::Text(ref s) if s == "hi"),
        "{:?}",
        row[2]
    );
    assert!(matches!(row[3], Value::Bool(true)), "{:?}", row[3]);
    // 2024-01-15 is 19737 days after the Unix epoch.
    assert!(matches!(row[4], Value::Date(19737)), "{:?}", row[4]);
    assert!(matches!(row[5], Value::Null), "{:?}", row[5]);
    assert!(
        matches!(row[6], Value::Decimal(ref s) if s == "170141183460469231731687303715884105727"),
        "a HUGEINT must survive exactly: {:?}",
        row[6]
    );
    assert!(
        matches!(
            row[7],
            Value::Timestamp {
                micros: _,
                tz: true
            }
        ),
        "a TIMESTAMPTZ column must decode as an instant: {:?}",
        row[7]
    );
}

#[tokio::test]
async fn streams_large_results_in_batches_rather_than_one_giant_message() {
    let (connector, cid) = connected().await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let exec = connector.execute(
        cid,
        QueryId(Uuid::new_v4()),
        "SELECT i FROM range(0, 1200) t(i)".into(),
        tx,
    );

    let collect = async {
        let mut batches = 0;
        let mut rows = 0;
        let mut saw_final = false;
        while let Some(event) = rx.recv().await {
            if let ExecutionEvent::Batch(ResultShape::Tabular { rows: batch, .. }, is_final) = event
            {
                batches += 1;
                rows += batch.len();
                saw_final |= is_final;
            }
        }
        (batches, rows, saw_final)
    };

    let (_, (batches, rows, saw_final)) = tokio::join!(exec, collect);
    assert_eq!(rows, 1200);
    assert!(
        batches >= 3,
        "1200 rows at 500/batch must be at least 3 batches, got {batches}"
    );
    assert!(saw_final, "the last batch must be flagged final");
}

#[tokio::test]
async fn a_syntax_error_reports_failure_without_killing_the_connection() {
    let (connector, cid) = connected().await;

    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let exec = connector.execute(cid, QueryId(Uuid::new_v4()), "SELEKT 1".into(), tx);
    let collect = async {
        let mut failed = false;
        while let Some(e) = rx.recv().await {
            failed |= matches!(e, ExecutionEvent::Failed(_));
        }
        failed
    };
    let (_, failed) = tokio::join!(exec, collect);
    assert!(failed, "a syntax error must surface as Failed");

    // The connection must still work.
    let rows = run_ok(&connector, cid).await;
    assert_eq!(rows.len(), 1);
}

async fn run_ok(connector: &DuckDbConnector, cid: ConnectionId) -> Vec<Vec<Value>> {
    run(connector, cid, "SELECT 1").await
}

/// Execute a statement and return the `Failed` error it produced.
async fn failed_error(
    connector: &DuckDbConnector,
    cid: ConnectionId,
    sql: &str,
) -> lucent_protocol::LucentError {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let exec = connector.execute(cid, QueryId(Uuid::new_v4()), sql.to_string(), tx);
    let collect = async {
        let mut error = None;
        while let Some(event) = rx.recv().await {
            if let ExecutionEvent::Failed(e) = event {
                error = Some(e);
            }
        }
        error.expect("the query must fail")
    };
    let (_, error) = tokio::join!(exec, collect);
    error
}

#[tokio::test]
async fn error_kinds_distinguish_syntax_runtime_and_binder_failures() {
    use lucent_protocol::LucentErrorKind;

    let (connector, cid) = connected().await;
    run(&connector, cid, "CREATE TABLE t (x INTEGER PRIMARY KEY)").await;

    // A parser error is the user's SQL — syntax class.
    let err = failed_error(&connector, cid, "SELEKT 1").await;
    assert_eq!(err.kind, LucentErrorKind::QuerySyntaxError, "{err}");

    // A binder error is also the user's SQL — a missing table is not a
    // runtime engine failure.
    let err = failed_error(&connector, cid, "SELECT * FROM no_such_table").await;
    assert_eq!(err.kind, LucentErrorKind::QuerySyntaxError, "{err}");

    // A constraint violation is a runtime failure. Mislabeling it as a
    // syntax error makes the UI and the AI guard blame the SQL for a data
    // problem.
    run(&connector, cid, "INSERT INTO t VALUES (1)").await;
    let err = failed_error(&connector, cid, "INSERT INTO t VALUES (1)").await;
    assert_eq!(err.kind, LucentErrorKind::Internal, "{err}");
    assert!(
        err.message.contains("constraint") || err.message.contains("Constraint"),
        "the engine message must survive: {err}"
    );
}

#[tokio::test]
async fn cancel_interrupts_the_in_flight_query() {
    let (connector, cid) = connected().await;
    let query_id = QueryId(Uuid::new_v4());
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    let exec = connector.execute(
        cid,
        query_id,
        "SELECT count(*) FROM range(1, 200000000) t1, range(1, 100) t2".into(),
        tx,
    );

    let canceller = async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        connector.cancel(cid, query_id).await
    };

    let collect = async {
        let mut failed = false;
        while let Some(e) = rx.recv().await {
            failed |= matches!(e, ExecutionEvent::Failed(_));
        }
        failed
    };

    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let (_, cancel_result, failed) = tokio::join!(exec, canceller, collect);
        (cancel_result, failed)
    })
    .await
    .expect("cancel must land — a timeout here means cancel is deadlocked behind the query");

    assert!(result.0.is_ok(), "cancel itself must not error");
    assert!(result.1, "the cancelled query must report failure");
}

#[tokio::test]
async fn cancelling_a_query_that_is_not_running_is_a_no_op() {
    // DuckDB's interrupt is connection-scoped, so firing it for a stale query
    // id would kill whatever is running now instead.
    let (connector, cid) = connected().await;
    let stale = QueryId(Uuid::new_v4());
    assert!(connector.cancel(cid, stale).await.is_ok());
}

#[tokio::test]
async fn disconnect_releases_the_connection() {
    let (connector, cid) = connected().await;
    connector.disconnect(cid).await.expect("disconnect");

    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let exec = connector.execute(cid, QueryId(Uuid::new_v4()), "SELECT 1".into(), tx);
    let collect = async {
        let mut failed = false;
        while let Some(e) = rx.recv().await {
            failed |= matches!(e, ExecutionEvent::Failed(_));
        }
        failed
    };
    let (_, failed) = tokio::join!(exec, collect);
    assert!(
        failed,
        "querying a disconnected connection must fail cleanly"
    );
}

#[tokio::test]
async fn json_columns_decode_as_json_not_plain_text() {
    // A JSON column is VARCHAR with the alias "JSON" in DuckDB's C API. The
    // decoder's JSON branch only fires when decl_type reports "JSON", so this
    // pins that the alias survives the metadata round-trip end to end.
    let (connector, cid) = connected().await;
    let _ = run(&connector, cid, "CREATE TABLE docs (j JSON)").await;
    let _ = run(&connector, cid, r#"INSERT INTO docs VALUES ('{"a": 1}')"#).await;
    let rows = run(&connector, cid, "SELECT j FROM docs").await;
    assert_eq!(rows.len(), 1, "one row expected: {rows:?}");
    match &rows[0][0] {
        Value::Json(s) => assert!(s.contains("a"), "JSON content must survive: {s}"),
        other => panic!("a JSON column must decode as Value::Json, got {other:?}"),
    }
}
