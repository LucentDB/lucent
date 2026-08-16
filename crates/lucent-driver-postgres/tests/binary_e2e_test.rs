use std::time::Duration;

use lucent_protocol::{
    new_framed, read_message, write_message, ConnectionConfig, ConnectionId, QueryId,
    WorkerRequest, WorkerResponse,
};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use tokio::net::UnixStream;
use tokio::process::Command;
use uuid::Uuid;

async fn wait_for_postgres(port: u16) {
    let conn_string =
        format!("host=127.0.0.1 port={port} user=postgres password=postgres dbname=postgres");
    for i in 0..10 {
        match tokio_postgres::connect(&conn_string, tokio_postgres::NoTls).await {
            Ok((_client, connection)) => {
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                return;
            }
            Err(_) if i < 9 => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(e) => panic!("postgres not ready after 10 retries: {e}"),
        }
    }
}

#[tokio::test]
async fn cancels_a_long_running_query_via_the_real_binary() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    wait_for_postgres(port).await;

    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("worker.sock");
    let token = "e2e-token";

    let binary = env!("CARGO_BIN_EXE_lucent-driver-postgres");
    let mut child = Command::new(binary)
        .arg(&socket_path)
        .arg(token)
        .spawn()
        .expect("failed to spawn lucent-driver-postgres binary");

    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let stream = UnixStream::connect(&socket_path).await.unwrap();
    let mut framed = new_framed(stream);
    write_message(&mut framed, &lucent_protocol::PROTOCOL_VERSION)
        .await
        .unwrap();
    write_message(&mut framed, &token.to_string())
        .await
        .unwrap();

    // ack must arrive before Connect
    let ack: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(matches!(ack, WorkerResponse::HandshakeAccepted));

    let connection_id = ConnectionId(Uuid::new_v4());
    write_message(
        &mut framed,
        &WorkerRequest::Connect {
            connection_id,
            config: ConnectionConfig::new("postgres")
                .with("host", "127.0.0.1")
                .with("port", port.to_string())
                .with("user", "postgres")
                .with("database", "postgres")
                .with("ssl_mode", "prefer")
                .with_secret("postgres"),
        },
    )
    .await
    .unwrap();
    let connect_response: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(matches!(connect_response, WorkerResponse::Connected { .. }));

    let query_id = QueryId(Uuid::new_v4());
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

    tokio::time::sleep(Duration::from_millis(300)).await;

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

    let execute_outcome: WorkerResponse = read_message(&mut framed).await.unwrap().unwrap();
    assert!(
        matches!(execute_outcome, WorkerResponse::Error { .. }),
        "the cancelled query's execute task should report an Error, got {execute_outcome:?}"
    );

    let _ = child.kill().await;
}

#[tokio::test]
async fn bad_password_surfaces_a_typed_error_not_a_timeout() {
    // C1 e2e: connecting with the wrong password must return
    // "authentication failed" quickly — before the fix it burned the full
    // 10s sync timeout and reported "connect response timed out".
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    wait_for_postgres(port).await;

    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("worker.sock");
    let token = "e2e-token";

    let binary = env!("CARGO_BIN_EXE_lucent-driver-postgres");
    let mut child = Command::new(binary)
        .arg(&socket_path)
        .arg(token)
        .spawn()
        .expect("failed to spawn lucent-driver-postgres binary");

    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let (client, _conn_id) = lucent_lib::client::ConnectorClient::connect(
        socket_path.to_str().unwrap(),
        token,
        // A VALID config — the initial connect inside ConnectorClient::connect
        // must succeed so the test reaches connect_with_id with the bad one.
        lucent_protocol::ConnectionConfig::new("postgres")
            .with("host", "127.0.0.1")
            .with("port", port.to_string())
            .with("user", "postgres")
            .with("database", "postgres")
            .with_secret("postgres"),
    )
    .await
    .expect("worker handshake with a valid config");

    let bad = lucent_protocol::ConnectionConfig::new("postgres")
        .with("host", "127.0.0.1")
        .with("port", port.to_string())
        .with("user", "postgres")
        .with("database", "postgres")
        .with_secret("definitely-wrong-password");

    let err = tokio::time::timeout(
        Duration::from_secs(5),
        client.connect_with_id(lucent_protocol::ConnectionId(Uuid::new_v4()), bad),
    )
    .await
    .expect("connect failure must surface well within the old 10s timeout")
    .expect_err("a wrong password must fail");

    assert!(
        err.contains("authentication failed") || err.contains("password"),
        "the real auth error must surface, got: {err}"
    );
    assert!(
        !err.contains("timed out"),
        "no lying timeout message, got: {err}"
    );

    let _ = child.kill().await;
}

#[tokio::test]
async fn wide_rows_stream_through_the_real_client() {
    // C2 e2e (GAP-1 pin): 600 rows × 20 KB = 12 MiB per 500-row batch — over
    // the old 8 MiB ceiling on BOTH ends. Through the real compiled binary
    // and the real ConnectorClient, the full stack (worker encode + client
    // decode) must stream the rows intact. Before A1's split-site codec
    // swap, the client's read half rejected the first batch and this test
    // hung on "Reader I/O error".
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    wait_for_postgres(port).await;

    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("worker.sock");
    let token = "e2e-token";

    let binary = env!("CARGO_BIN_EXE_lucent-driver-postgres");
    let mut child = Command::new(binary)
        .arg(&socket_path)
        .arg(token)
        .spawn()
        .expect("failed to spawn lucent-driver-postgres binary");

    for _ in 0..50 {
        if socket_path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let good = lucent_protocol::ConnectionConfig::new("postgres")
        .with("host", "127.0.0.1")
        .with("port", port.to_string())
        .with("user", "postgres")
        .with("database", "postgres")
        .with_secret("postgres");
    let (client, conn_id) =
        lucent_lib::client::ConnectorClient::connect(socket_path.to_str().unwrap(), token, good)
            .await
            .expect("worker handshake");

    let qid = lucent_protocol::QueryId(Uuid::new_v4());
    let (result, _) = tokio::time::timeout(
        Duration::from_secs(30),
        client.execute_with_id(
            qid,
            conn_id,
            "SELECT repeat('x', 20000) AS wide FROM generate_series(1, 600)",
            None,
        ),
    )
    .await
    .expect("wide rows must stream — do not hang on the frame ceiling")
    .expect("wide rows must execute");

    assert_eq!(result.row_count, 600, "all rows must arrive");
    assert!(!result.truncated, "no client-side truncation at this size");
    let cell = result.rows[0][0].as_str().expect("cell must be a string");
    assert_eq!(cell.len(), 20000, "20 KB cells must arrive intact");
    assert!(
        !cell.contains("truncated"),
        "no truncation marker under the cap"
    );

    let _ = child.kill().await;
}
