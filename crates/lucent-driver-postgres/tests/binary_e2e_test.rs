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
