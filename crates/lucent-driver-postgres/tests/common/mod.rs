//! Shared fixture: a seeded Postgres container plus a connected connector.
//!
//! Declared with `mod common;` from each test binary that needs it.

#![allow(dead_code)] // Not every test binary uses every helper.

use lucent_driver_postgres::PostgresConnector;
use lucent_protocol::{ConnectionConfig, ConnectionId};
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

/// Seed schema. Deliberately includes a matview (invisible to
/// information_schema.columns), a composite FK, a single-column FK, a
/// never-analyzed table, and a partitioned parent.
pub const SEED: &str = "
    CREATE SCHEMA app;
    CREATE TABLE app.users (
        id bigint PRIMARY KEY,
        email varchar(100) NOT NULL,
        note text
    );
    COMMENT ON TABLE app.users IS 'people';
    COMMENT ON COLUMN app.users.email IS 'login';
    CREATE TABLE app.orders (
        org_id bigint NOT NULL,
        user_id bigint NOT NULL REFERENCES app.users(id),
        total numeric(12,2),
        PRIMARY KEY (org_id, user_id)
    );
    -- Composite FK: pairs (org_id, user_id) by position with orders' PK.
    CREATE TABLE app.deliveries (
        org_id bigint NOT NULL,
        user_id bigint NOT NULL,
        PRIMARY KEY (org_id, user_id),
        FOREIGN KEY (org_id, user_id) REFERENCES app.orders(org_id, user_id)
    );
    CREATE VIEW app.recent AS SELECT id FROM app.users;
    CREATE MATERIALIZED VIEW app.mv_users AS SELECT id, email FROM app.users;
    CREATE SEQUENCE app.counter START 5 INCREMENT 2 MINVALUE 5 MAXVALUE 100;
    CREATE TABLE app.events (id bigint, created_at timestamptz)
        PARTITION BY RANGE (created_at);
    CREATE TABLE app.events_2026 PARTITION OF app.events
        FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');
    ANALYZE app.users;
";

/// Start a container, connect, and run [`SEED`]. The container is returned so
/// the caller keeps it alive — dropping it stops the database.
pub async fn seeded() -> (
    testcontainers::ContainerAsync<Postgres>,
    PostgresConnector,
    ConnectionId,
) {
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let connector = PostgresConnector::default();
    let connection_id = ConnectionId(Uuid::new_v4());

    // The container needs a moment after start() before it accepts connections.
    let config = ConnectionConfig::new("postgres")
        .with("host", "127.0.0.1")
        .with("port", port.to_string())
        .with("user", "postgres")
        .with("database", "postgres")
        .with("ssl_mode", "disable")
        .with_secret("postgres");
    let mut last = None;
    for _ in 0..10 {
        match connector.connect(connection_id, config.clone()).await {
            Ok(_) => {
                last = None;
                break;
            }
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
        }
    }
    assert!(last.is_none(), "connect never succeeded: {last:?}");

    // Seed through a raw client, not the connector's `execute` path: the
    // SEED is multi-statement DDL, and the driver deliberately rejects
    // multi-statement input at prepare() (a hard QuerySyntaxError before
    // execution). batch_execute runs the whole script in one round trip.
    let mut raw = tokio_postgres::config::Config::new();
    raw.host("127.0.0.1")
        .port(port)
        .user("postgres")
        .password("postgres")
        .dbname("postgres");
    let (client, connection) = raw
        .connect(tokio_postgres::NoTls)
        .await
        .expect("raw seed connection");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client.batch_execute(SEED).await.expect("seed must execute");

    (container, connector, connection_id)
}
