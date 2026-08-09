//! SSL mode behavior against a real (plaintext) Postgres: `require` must
//! fail loudly, `prefer` and `disable` must connect.

use std::time::Duration;

use lucent_driver_postgres::PostgresConnector;
use lucent_protocol::{ConnectionConfig, ConnectionId};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

fn config(port: u16, ssl_mode: &str) -> ConnectionConfig {
    ConnectionConfig::new("postgres")
        .with("host", "127.0.0.1")
        .with("port", port.to_string())
        .with("user", "postgres")
        .with("database", "postgres")
        .with("ssl_mode", ssl_mode)
        .with_secret("postgres")
}

/// Wait until the container's Postgres accepts plaintext connections.
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

#[tokio::test]
async fn require_fails_loudly_against_plaintext_server() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    wait_for_postgres(port).await;

    let connector = PostgresConnector::default();
    let result = connector
        .connect(ConnectionId(Uuid::new_v4()), config(port, "require"))
        .await;

    assert!(
        result.is_err(),
        "ssl_mode=require must fail loudly against a plaintext server — \
         a silent downgrade would leak credentials"
    );
}

#[tokio::test]
async fn prefer_connects_plaintext_when_server_lacks_tls() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    wait_for_postgres(port).await;

    let connector = PostgresConnector::default();
    let result = connector
        .connect(ConnectionId(Uuid::new_v4()), config(port, "prefer"))
        .await;

    assert!(
        result.is_ok(),
        "prefer must fall back to plaintext: {result:?}"
    );
}

#[tokio::test]
async fn disable_connects_plaintext() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    wait_for_postgres(port).await;

    let connector = PostgresConnector::default();
    let result = connector
        .connect(ConnectionId(Uuid::new_v4()), config(port, "disable"))
        .await;

    assert!(result.is_ok(), "disable must connect plaintext: {result:?}");
}
