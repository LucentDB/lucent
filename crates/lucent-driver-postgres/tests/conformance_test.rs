//! Cross-driver conformance suite against a real Postgres container.

mod common;

use lucent_worker_host::{Connector, ExecutionEvent};

#[tokio::test]
async fn postgres_conforms() {
    let (container, connector, cid) = common::seeded().await;
    let port = container.get_host_port_ipv4(5432).await.unwrap();

    // The driver rejects multi-statement input at prepare(), and SEED_SQL is a
    // script — seed through a raw client, exactly as common::seeded() does.
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
    client
        .batch_execute(lucent_driver_conformance::SEED_SQL)
        .await
        .expect("conformance seed must execute");

    let failures = lucent_driver_conformance::run_all(&connector, cid).await;
    assert!(
        failures.is_empty(),
        "Postgres conformance failures: {failures:#?}"
    );
}
