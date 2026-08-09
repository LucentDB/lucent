//! Integration tests for the client-side row cap.
//!
//! The worker streams results in 500-row batches with backpressure, but the
//! ConnectorClient materializes every batch in the Tauri process. Queries that
//! cannot be LIMIT-wrapped (multi-statement, EXPLAIN, unparseable-but-executable)
//! would therefore accumulate unbounded rows in memory. These tests pin the
//! safety net: `execute_with_id(..., Some(cap))` truncates at the cap, cancels
//! the query server-side (native DB cancel), and reports `truncated`.
//!
//! Requires Docker (testcontainers Postgres) and the compiled
//! lucent-driver-postgres binary.

#![cfg(feature = "integration-tests")]

use std::time::{Duration, Instant};

use lucent_lib::client::ConnectorClient;
use lucent_lib::supervisor::Supervisor;
use lucent_protocol::{ConnectionConfig, ConnectionId, QueryId};
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

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

async fn setup() -> (
    Supervisor,
    ConnectorClient,
    ConnectionId,
    ContainerAsync<Postgres>,
) {
    let (port, container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path_buf = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let (client, conn_id) = ConnectorClient::connect(&socket_path_buf, &token, pg_config(port))
        .await
        .expect("connect ConnectorClient");

    (supervisor, client, conn_id, container)
}

async fn capped_execute(
    client: &ConnectorClient,
    conn_id: ConnectionId,
    sql: &str,
    max_rows: Option<usize>,
) -> Result<(lucent_lib::client::ExecuteResult, QueryId), String> {
    client
        .execute_with_id(QueryId(Uuid::new_v4()), conn_id, sql, max_rows)
        .await
}

/// A query that streams far more than the cap must come back with exactly
/// `cap` rows, flagged truncated — never the full result set.
#[tokio::test]
async fn capped_execute_truncates_oversized_result() {
    let (mut supervisor, client, conn_id, _container) = setup().await;

    let (result, _qid) = capped_execute(
        &client,
        conn_id,
        "SELECT generate_series(1, 100000)",
        Some(1000),
    )
    .await
    .expect("capped execute should succeed");

    assert_eq!(result.rows.len(), 1000, "rows must be truncated at the cap");
    assert_eq!(result.row_count, 1000);
    assert!(
        result.truncated,
        "oversized result must be flagged truncated"
    );

    let _ = supervisor.shutdown().await;
}

/// A query under the cap passes through untouched and unflagged.
#[tokio::test]
async fn capped_execute_within_cap_is_not_truncated() {
    let (mut supervisor, client, conn_id, _container) = setup().await;

    let (result, _qid) = capped_execute(
        &client,
        conn_id,
        "SELECT generate_series(1, 50)",
        Some(1000),
    )
    .await
    .expect("capped execute should succeed");

    assert_eq!(result.rows.len(), 50);
    assert!(
        !result.truncated,
        "small result must not be flagged truncated"
    );

    let _ = supervisor.shutdown().await;
}

/// Without a cap, multi-batch results still accumulate in full (the existing
/// behavior — the cap is opt-in at the command layer).
#[tokio::test]
async fn uncapped_execute_returns_every_batch() {
    let (mut supervisor, client, conn_id, _container) = setup().await;

    // 2500 rows spans 5 worker batches of 500.
    let (result, _qid) = capped_execute(&client, conn_id, "SELECT generate_series(1, 2500)", None)
        .await
        .expect("uncapped execute should succeed");

    assert_eq!(result.rows.len(), 2500);
    assert!(!result.truncated);

    let _ = supervisor.shutdown().await;
}

/// The capped execute must cancel the query server-side: the connection is
/// held by the still-streaming query, so a follow-up query only succeeds once
/// the native cancel has freed it. The test query takes ~100s to complete
/// naturally, so success within a few seconds proves the cancel landed.
#[tokio::test]
async fn capped_execute_cancels_query_server_side() {
    let (mut supervisor, client, conn_id, _container) = setup().await;

    let start = Instant::now();
    let (result, _qid) = capped_execute(
        &client,
        conn_id,
        "SELECT g, pg_sleep(0.001) FROM generate_series(1, 100000) AS g",
        Some(1000),
    )
    .await
    .expect("capped execute should succeed");
    let capped_elapsed = start.elapsed();

    assert_eq!(result.rows.len(), 1000);
    assert!(result.truncated);
    assert!(
        capped_elapsed < Duration::from_secs(15),
        "capped execute took {capped_elapsed:?} — cancel did not fire"
    );

    // If the server-side cancel failed, the connection stays busy for the
    // query's natural ~100s lifetime and every retry fails with "another
    // command is already in progress". Success within 15s is conclusive.
    let follow_up_start = Instant::now();
    let deadline = Duration::from_secs(15);
    loop {
        match client.execute(conn_id, "SELECT 1").await {
            Ok(r) => {
                assert_eq!(r.rows.len(), 1, "follow-up query returned the expected row");
                break;
            }
            Err(e) => {
                assert!(
                    follow_up_start.elapsed() < deadline,
                    "connection never freed after capped query: {e}"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }

    let _ = supervisor.shutdown().await;
}
