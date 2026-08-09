use std::time::Duration;

use lucent_driver_postgres::PostgresConnector;
use lucent_protocol::{ConnectionConfig, ConnectionId, ServerInfo};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

async fn connect_with_retry(
    connector: &PostgresConnector,
    connection_id: ConnectionId,
    config: ConnectionConfig,
) -> Result<ServerInfo, Box<dyn std::error::Error>> {
    for i in 0..10 {
        match connector.connect(connection_id, config.clone()).await {
            Ok(info) => return Ok(info),
            Err(e) => {
                if i < 9 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                } else {
                    return Err(e.into());
                }
            }
        }
    }
    unreachable!()
}

#[tokio::test]
async fn connects_to_a_real_postgres_instance_and_reads_its_version() {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();

    let connector = PostgresConnector::default();
    let connection_id = ConnectionId(Uuid::new_v4());

    let server_info = connect_with_retry(
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
    .await
    .unwrap();

    assert!(!server_info.version.is_empty());

    connector.disconnect(connection_id).await.unwrap();
}

#[tokio::test]
async fn connect_fails_with_connection_refused_for_a_closed_port() {
    let connector = PostgresConnector::default();
    let connection_id = ConnectionId(Uuid::new_v4());

    let result = connector
        .connect(
            connection_id,
            ConnectionConfig::new("postgres")
                .with("host", "127.0.0.1")
                .with("port", "1")
                .with("user", "postgres")
                .with("database", "postgres")
                .with("ssl_mode", "prefer")
                .with_secret("postgres"),
        )
        .await;

    assert!(result.is_err());
}
