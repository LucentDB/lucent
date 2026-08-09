//! Integration tests for the `test_connection` worker probe.
//!
//! The probe must go through a THROWAWAY worker process: the app's real
//! worker serves exactly one socket (the live connection's), so a probe that
//! reuses the live supervisor's worker hangs for 15s and reports a healthy
//! database as unreachable. These tests pin that behaviour.
//!
//! Requires Docker (testcontainers Postgres) and the compiled
//! lucent-driver-postgres binary.

#![cfg(feature = "integration-tests")]

use lucent_protocol::ConnectionConfig;
use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use testcontainers::ContainerAsync;
use testcontainers_modules::postgres::Postgres;

use lucent_lib::client::ConnectorClient;
use lucent_lib::supervisor::Supervisor;

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

/// The regression test for the reviewer finding: probing while a live
/// connection is active must succeed quickly through a throwaway worker,
/// never hang behind the live connection's one-socket worker.
#[tokio::test]
async fn probe_succeeds_while_a_live_connection_is_active() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    // Live connection: one worker, one socket — the exact state where the old
    // probe (reusing the state supervisor) sat unaccepted in the backlog,
    // timed out at 15s, and reported ConnectionFailed for a healthy database.
    let mut live_supervisor = Supervisor::new();
    let live_socket = live_supervisor
        .ensure_running()
        .await
        .unwrap()
        .to_path_buf();
    let live_token = live_supervisor.handshake_token().to_owned();
    let (live_client, live_cid) =
        ConnectorClient::connect(&live_socket, &live_token, pg_config(port))
            .await
            .expect("live connection must connect");
    let r = live_client
        .execute(live_cid, "SELECT 1")
        .await
        .expect("live connection must serve queries");
    assert_eq!(r.rows[0][0], serde_json::json!(1));

    // Probe through a FRESH worker while the live one is serving.
    let t0 = std::time::Instant::now();
    let result = lucent_lib::probe_connection(pg_config(port), "PostgreSQL".into())
        .await
        .expect("probe must succeed while a live connection is active");
    let elapsed = t0.elapsed();

    assert!(result.success, "probe must report success");
    assert!(
        result.server_version.is_some(),
        "probe must report the server version"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "probe must not hang behind the live connection's worker (took {elapsed:?})"
    );

    // The live connection must be untouched by the probe.
    let r2 = live_client
        .execute(live_cid, "SELECT 2")
        .await
        .expect("live connection must keep working after the probe");
    assert_eq!(r2.rows[0][0], serde_json::json!(2));

    let mut live_client = live_client;
    live_client.shutdown().await.expect("live shutdown");
    let _ = live_supervisor.shutdown().await;
}

/// A bad config must fail fast through the probe's error path, and the
/// throwaway worker must not leak behind it. No database needed — the probe
/// fails at the throwaway worker's own connect.
#[tokio::test]
async fn probe_reports_failure_for_a_bad_config() {
    // Nothing listens on port 1: the throwaway worker's connect fails and the
    // probe must return an error, not hang.
    let bad = ConnectionConfig::new("postgres")
        .with("host", "127.0.0.1")
        .with("port", "1")
        .with("user", "postgres")
        .with("database", "postgres")
        .with("ssl_mode", "prefer")
        .with_secret("postgres");

    let t0 = std::time::Instant::now();
    let err = lucent_lib::probe_connection(bad, "PostgreSQL".into()).await;
    let elapsed = t0.elapsed();
    assert!(err.is_err(), "a bad config must produce a probe error");
    assert!(
        elapsed < Duration::from_secs(5),
        "a failed probe must not hang (took {elapsed:?})"
    );
}
