//! Integration tests for ConnectorClient concurrent access.
//!
//! These tests require Docker (testcontainers Postgres) and the
//! compiled lucent-driver-postgres binary.

#![cfg(feature = "integration-tests")]

use lucent_protocol::{
    new_framed, read_message, write_message, ConnectionConfig, ConnectionId, QueryId,
    WorkerRequest, WorkerResponse,
};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::net::UnixStream;

use lucent_lib::ai::mschema::ContextTier;
use lucent_lib::ai::preflight::run_preflight;
use lucent_lib::ai::schema_graph::{ColumnEntry, SchemaGraph, TableEntry};
use lucent_lib::client::ConnectorClient;
use lucent_lib::supervisor::Supervisor;
use lucent_lib::AppState;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use testcontainers::ContainerAsync;

/// Start a postgres container and return the host port plus the container handle.
/// The caller MUST hold the container reference until the test ends — dropping
/// it stops and removes the container automatically.
async fn start_postgres() -> (u16, ContainerAsync<Postgres>) {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    (port, container)
}

/// Wait until Postgres is accepting connections.
async fn wait_for_postgres(port: u16) {
    let conn_string =
        format!("host=127.0.0.1 port={port} user=postgres password=postgres dbname=postgres");
    for i in 0..20 {
        match tokio_postgres::connect(&conn_string, tokio_postgres::NoTls).await {
            Ok((_client, connection)) => {
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                return;
            }
            Err(_) if i < 19 => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(e) => panic!("postgres not ready after 20 retries: {e}"),
        }
    }
}

fn pg_config(port: u16) -> ConnectionConfig {
    ConnectionConfig::new("postgres")
        .with("host", "127.0.0.1")
        .with("port", port.to_string())
        .with("user", "postgres")
        .with("database", "postgres")
        .with("ssl_mode", "prefer")
        .with_secret("postgres")
}

/// Verify that a slow query can be cancelled at the protocol level,
/// and that the cancelled query returns an Error response.
///
/// This validates the server.rs HashMap<QueryId, Receiver> refactor:
/// the cancel must find the correct query's receiver even when
/// it's structurally concurrent-safe.
#[tokio::test]
async fn test_concurrent_execute_and_cancel() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path_buf = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let stream = UnixStream::connect(&socket_path_buf).await.unwrap();
    let mut framed = new_framed(stream);

    // Handshake
    write_message(&mut framed, &lucent_protocol::PROTOCOL_VERSION)
        .await
        .unwrap();
    write_message(&mut framed, &token).await.unwrap();

    let connection_id = ConnectionId(uuid::Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Connect {
            connection_id,
            config: pg_config(port),
        },
    )
    .await
    .unwrap();

    let connect_response: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(
        matches!(connect_response, WorkerResponse::Connected { .. }),
        "expected Connected, got {connect_response:?}"
    );

    // Start a slow query (pg_sleep)
    let query_id = QueryId(uuid::Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Execute {
            connection_id,
            query_id,
            command: "SELECT pg_sleep(30)".to_string(),
        },
    )
    .await
    .unwrap();

    // Give the query a moment to start executing
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Send cancel for the same query_id
    write_message(
        &mut framed,
        &WorkerRequest::Cancel {
            connection_id,
            query_id,
        },
    )
    .await
    .unwrap();

    let cancel_response: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(
        matches!(cancel_response, WorkerResponse::Cancelled { .. }),
        "expected Cancelled, got {cancel_response:?}"
    );

    // The cancelled query's execute task should produce an Error
    let execute_outcome: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(
        matches!(execute_outcome, WorkerResponse::Error { .. }),
        "expected Error from cancelled query, got {execute_outcome:?}"
    );

    let _ = supervisor.shutdown().await;
}

/// Verify execute_with_id + cancel through the high-level ConnectorClient API:
/// the caller-registered QueryId (the C3 mechanism behind `cancel_query`) lets
/// `cancel()` abort a query while it runs, and the execute call returns an
/// error instead of blocking for the full pg_sleep.
#[tokio::test]
async fn test_execute_with_id_cancel_aborts_query() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path_buf = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let (client, conn_id) = ConnectorClient::connect(&socket_path_buf, &token, pg_config(port))
        .await
        .expect("connect ConnectorClient");

    let query_id = QueryId(uuid::Uuid::new_v4());
    let execute_handle = tokio::spawn({
        let client = client.clone();
        async move {
            client
                .execute_with_id(query_id, conn_id, "SELECT pg_sleep(30)", None)
                .await
        }
    });

    // Give the query a moment to start, then cancel through the registered id.
    tokio::time::sleep(Duration::from_millis(500)).await;
    client
        .cancel(conn_id, query_id)
        .await
        .expect("cancel should be accepted");

    // The cancelled execute must finish well under pg_sleep(30) and report the
    // cancellation as an error.
    let result = tokio::time::timeout(Duration::from_secs(5), execute_handle)
        .await
        .expect("cancelled query should finish within 5s")
        .expect("execute task should not panic");
    let err = result.expect_err("cancelled query should return Err");
    assert!(
        err.to_lowercase().contains("cancel"),
        "error should mention cancellation, got: {err}"
    );

    let _ = supervisor.shutdown().await;
}

/// Verify that multiple ConnectionIds on one ConnectorClient
/// operate on truly isolated database sessions.
#[tokio::test]
async fn test_multiple_connection_ids_on_one_client() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path_buf = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let (client, conn_a) = ConnectorClient::connect(&socket_path_buf, &token, pg_config(port))
        .await
        .expect("connect ConnectorClient");

    // Query on connection A
    let result_a = client
        .execute(conn_a, "SELECT 1 AS a")
        .await
        .expect("execute on conn A");
    assert_eq!(result_a.columns[0].name, "a");
    assert_eq!(result_a.rows[0][0], serde_json::json!(1));

    // Create a second connection on the same client
    let conn_b = ConnectionId(uuid::Uuid::new_v4());
    let _server_info = client
        .connect_with_id(conn_b, pg_config(port))
        .await
        .expect("connect_with_id");

    // Query on connection B
    let result_b = client
        .execute(conn_b, "SELECT 2 AS b")
        .await
        .expect("execute on conn B");
    assert_eq!(result_b.columns[0].name, "b");
    assert_eq!(result_b.rows[0][0], serde_json::json!(2));

    // SET a session variable on connection A
    client
        .execute(conn_a, "SET myapp.test_var = 'hello_from_a'")
        .await
        .expect("SET on conn A");

    // Verify the variable is visible on A
    let var_a = client
        .execute(conn_a, "SHOW myapp.test_var")
        .await
        .expect("SHOW on conn A");
    let val_a = var_a.rows[0][0].as_str().unwrap();
    assert_eq!(val_a, "hello_from_a");

    // Verify the variable is NOT visible on B (session isolation). A custom GUC
    // only exists after SET on that session, so SHOW on B must FAIL — that error
    // is the isolation proof.
    let var_b = client.execute(conn_b, "SHOW myapp.test_var").await;
    assert!(
        var_b.is_err(),
        "connection B should NOT see connection A's session variable"
    );

    // Clean up: disconnect conn_b, then shutdown
    client
        .disconnect_id(conn_b)
        .await
        .expect("disconnect conn B");

    let mut client = client;
    client.shutdown().await.expect("shutdown");
    let _ = supervisor.shutdown().await;
}

/// The pre-flight probe path on session B must never leave a session-level
/// statement_timeout behind. The probe target is a view that sleeps ~0.3s per
/// scan, so eight literals ≈ 2.4s of probes exceed the 2s tokio timeout,
/// which drops the probe future mid-transaction — the exact leak window.
/// Whatever happened, SHOW statement_timeout on B must settle back to 0 AND
/// a fresh wrapped query must succeed (a stale probe transaction would fail
/// it).
#[tokio::test]
async fn test_preflight_probe_cannot_leak_statement_timeout() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path_buf = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let (client, conn_a) = ConnectorClient::connect(&socket_path_buf, &token, pg_config(port))
        .await
        .expect("connect ConnectorClient");
    let conn_b = ConnectionId(uuid::Uuid::new_v4());
    client
        .connect_with_id(conn_b, pg_config(port))
        .await
        .expect("connect_with_id");

    client
        .execute(
            conn_a,
            "CREATE VIEW big_probe_target AS SELECT 'stored'::text AS v \
             WHERE pg_sleep(0.3) IS NULL LIMIT 1",
        )
        .await
        .expect("create view");

    let db: Arc<tokio::sync::Mutex<Option<ConnectorClient>>> =
        Arc::new(tokio::sync::Mutex::new(Some(client.clone())));
    let graph = SchemaGraph {
        tables: vec![TableEntry {
            id: 0,
            schema: "public".into(),
            name: "big_probe_target".into(),
            row_count_estimate: 1,
            partition_info: None,
        }],
        columns: vec![ColumnEntry {
            id: 0,
            table_id: 0,
            schema: "public".into(),
            table: "big_probe_target".into(),
            name: "v".into(),
            data_type: "text".into(),
            is_primary_key: true,
            sample_values: vec![],
            fk_ref: None,
            embedding: vec![],
            doc_text: String::new(),
        }],
        columns_by_table: HashMap::from([(0, vec![0])]),
        fk_edges: vec![],
        table_adjacency: HashMap::new(),
        built_at: std::time::Instant::now(),
    };
    let caps = lucent_protocol::DriverCapabilities {
        id: "postgres".into(),
        display_name: "PostgreSQL".into(),
        sql_dialect: lucent_protocol::SqlDialect::PostgreSql,
        namespace_model: lucent_protocol::NamespaceModel::DbSchemaObject,
        readonly: lucent_protocol::ReadOnlyMode::TransactionScoped,
        statement_timeout: lucent_protocol::TimeoutSupport::Statement,
        cancel: lucent_protocol::CancelMode::Native,
        paging: lucent_protocol::PagingStyle::LimitOffset,
        identifier_quote: '"',
        string_literal: lucent_protocol::StringLiteralStyle::StandardConforming,
        auth: lucent_protocol::AuthModel::UserPassword,
    };
    let _ = run_preflight(
        Some(conn_b),
        Some(&db),
        Some(&graph),
        None,
        &ContextTier::Push,
        "find A001 B002 C003 D004 E005 F006 G007 H008",
        Some(&caps),
    )
    .await;

    let mut final_value = String::new();
    for _ in 0..25 {
        let r = client
            .execute(conn_b, "SHOW statement_timeout")
            .await
            .expect("SHOW on B");
        final_value = r.rows[0][0].as_str().unwrap_or("?").to_string();
        if final_value == "0" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(
        final_value, "0",
        "statement_timeout must be reset after pre-flight probes"
    );

    let mut txn_state = String::new();
    for _ in 0..25 {
        let pid = client
            .execute(conn_b, "SELECT pg_backend_pid()")
            .await
            .expect("backend pid on B");
        let pid_val: i64 = pid.rows[0][0].as_i64().unwrap();
        let r = client
            .execute(
                conn_a,
                &format!("SELECT state FROM pg_stat_activity WHERE pid = {pid_val}"),
            )
            .await
            .expect("state of B from A");
        txn_state = r.rows[0][0].as_str().unwrap_or("?").to_string();
        if txn_state == "idle" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(
        txn_state, "idle",
        "session B must not be left inside a stale probe transaction"
    );

    client
        .execute(conn_b, "BEGIN")
        .await
        .expect("wrapped BEGIN on B: a stale probe transaction would reject it");
    client
        .execute(conn_b, "SET TRANSACTION READ ONLY")
        .await
        .expect(
            "wrapped SET TRANSACTION READ ONLY on B: a stale probe transaction would reject it",
        );
    let wrapped = client
        .execute(conn_b, "SELECT 1")
        .await
        .expect("wrapped query on B must succeed: a stale probe transaction would fail it");
    assert_eq!(wrapped.rows[0][0], serde_json::json!(1));
    client
        .execute(conn_b, "ROLLBACK")
        .await
        .expect("wrapped ROLLBACK on B");

    let mut client = client;
    client.shutdown().await.expect("shutdown");
    let _ = supervisor.shutdown().await;
}

/// The AI readonly path (BEGIN/SET TRANSACTION READ ONLY/ROLLBACK) on
/// connection B must never touch an open user transaction on connection A.
#[tokio::test]
async fn test_ai_rollback_cannot_touch_user_transaction() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path_buf = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let (client, conn_a) = ConnectorClient::connect(&socket_path_buf, &token, pg_config(port))
        .await
        .expect("connect ConnectorClient");
    let conn_b = ConnectionId(uuid::Uuid::new_v4());
    client
        .connect_with_id(conn_b, pg_config(port))
        .await
        .expect("connect_with_id");

    // User opens a writable transaction on A and makes uncommitted changes.
    // Plain (committed) table, not TEMP: temp tables are session-local and
    // connection B could not see them.
    client
        .execute(conn_a, "CREATE TABLE t(x int)")
        .await
        .expect("create");
    client.execute(conn_a, "BEGIN").await.expect("begin");
    client
        .execute(conn_a, "INSERT INTO t VALUES (1)")
        .await
        .expect("insert");

    // The exact AI readonly path on B: BEGIN → SET TRANSACTION READ ONLY →
    // query → ROLLBACK (the pre-fix code ran this on the shared session,
    // destroying A's transaction).
    client.execute(conn_b, "BEGIN").await.expect("ai begin");
    client
        .execute(conn_b, "SET TRANSACTION READ ONLY")
        .await
        .expect("ai set readonly");
    let r = client.execute(conn_b, "SELECT count(*) FROM t").await;
    assert!(r.is_ok(), "AI query must succeed on B");
    client
        .execute(conn_b, "ROLLBACK")
        .await
        .expect("ai rollback");

    // A's transaction must still be intact: COMMIT persists the row.
    client.execute(conn_a, "COMMIT").await.expect("user commit");
    let rows = client
        .execute(conn_a, "SELECT count(*) FROM t")
        .await
        .expect("count");
    assert_eq!(rows.rows[0][0], serde_json::json!(1));

    let mut client = client;
    client.shutdown().await.expect("shutdown");
    let _ = supervisor.shutdown().await;
}

/// A slow query through the app-level client handle must NOT block a fast
/// query on another connection. This is the regression test for the
/// app-side serialization fix: `AppState.client` must be held only to clone
/// the handle (via `AppState::client_handle`), never across `execute().await`.
/// It goes through the production lock pattern, not a test-local clone.
#[tokio::test]
async fn test_app_client_handle_does_not_serialize_queries() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path_buf = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let (client, conn_a) = ConnectorClient::connect(&socket_path_buf, &token, pg_config(port))
        .await
        .expect("connect ConnectorClient");
    let conn_b = ConnectionId(uuid::Uuid::new_v4());
    client
        .connect_with_id(conn_b, pg_config(port))
        .await
        .expect("connect_with_id");

    // Wire the client into a real AppState exactly as the `connect` command
    // does, so both queries run through the production `client_handle` seam.
    let state = std::sync::Arc::new(AppState::new());
    *state.client.lock().await = Some(client);
    *state.current_connection_id.lock().await = Some(conn_a);
    *state.ai_connection_id.lock().await = Some(conn_b);

    let state_for_a = state.clone();
    let a_handle = tokio::spawn(async move {
        let client = state_for_a.client_handle().await.expect("client A");
        client.execute(conn_a, "SELECT pg_sleep(2)").await
    });
    tokio::time::sleep(Duration::from_millis(300)).await; // let A start

    let t0 = std::time::Instant::now();
    let client_b = state.client_handle().await.expect("client B");
    let result_b = client_b
        .execute(conn_b, "SELECT 1")
        .await
        .expect("fast query on B");
    assert_eq!(result_b.rows[0][0], serde_json::json!(1));
    assert!(
        t0.elapsed() < Duration::from_millis(1500),
        "B must not wait behind A — client_handle must drop the mutex before execute"
    );

    a_handle
        .await
        .expect("A finishes")
        .expect("pg_sleep succeeds");

    let state = std::sync::Arc::try_unwrap(state)
        .ok()
        .expect("single owner");
    let mut client = state.client.lock().await.take().expect("client present");
    client.shutdown().await.expect("shutdown");
    let _ = supervisor.shutdown().await;
}

/// get_objects_info must batch its columns lookup into ONE query for any
/// number of objects, returning per-object columns identical to the old
/// per-object query loop.
#[tokio::test]
async fn test_get_objects_info_batches_columns_queries() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path_buf = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let (client, conn_a) = ConnectorClient::connect(&socket_path_buf, &token, pg_config(port))
        .await
        .expect("connect ConnectorClient");

    for i in 0..5 {
        client
            .execute(conn_a, &format!("CREATE TABLE t{i} (id int, name text)"))
            .await
            .expect("create table");
    }

    let ctx = lucent_lib::ai::tools::AiToolContext {
        db: std::sync::Arc::new(tokio::sync::Mutex::new(Some(client.clone()))),
        connection_id: Some(conn_a),
        capabilities: None,
        config: lucent_lib::ai::config::AiConfig::default(),
        schema_graph: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        embedder: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        reranker: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
    };
    let tool = lucent_lib::ai::tools::objects::GetObjectsInfo::new(ctx.clone());
    let args = serde_json::json!({
        "objects": (0..5)
            .map(|i| serde_json::json!({ "schema": "public", "name": format!("t{i}") }))
            .collect::<Vec<_>>()
    });
    let out = tool.call(args, &ctx).await.expect("tool call");
    let content = match out {
        lucent_lib::ai::tools::ToolOutput::Text { content } => content,
        other => panic!("expected text output, got {other:?}"),
    };
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    let objects = parsed["objects"].as_array().expect("objects array");
    assert_eq!(objects.len(), 5, "all 5 objects present");
    for o in objects {
        assert_eq!(o["name"].as_str().unwrap().chars().next(), Some('t'));
        assert_eq!(o["columns"].as_array().unwrap().len(), 2, "id + name");
    }

    let mut client = client;
    client.shutdown().await.expect("shutdown");
    let _ = supervisor.shutdown().await;
}

/// Regression: a FAST query returning more than the driver's 500-row batch
/// size must not hang. The worker streams the second batch back-to-back with
/// the first (they are both already in the socket buffer), and the old
/// oneshot-per-batch correlation could drop that second batch when it arrived
/// while the caller was re-arming its oneshot — the query then waited forever.
/// The schema-index sampling query hit this deterministically (~550 rows in a
/// single fast statement). `generate_series` reproduces the burst shape
/// without any table.
#[tokio::test]
async fn test_fast_multibatch_result_does_not_hang() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path_buf = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let (client, conn_id) = ConnectorClient::connect(&socket_path_buf, &token, pg_config(port))
        .await
        .expect("connect ConnectorClient");

    let result = tokio::time::timeout(
        Duration::from_secs(15),
        client.execute(conn_id, "SELECT generate_series(1, 1500)"),
    )
    .await
    .expect("fast multi-batch query must complete within 15s")
    .expect("query must succeed");

    assert_eq!(result.row_count, 1500, "all three batches must arrive");

    let _ = supervisor.shutdown().await;
}
