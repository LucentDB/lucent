use lucent_driver_duckdb::connector::DuckDbConnector;
use lucent_protocol::{ConnectionConfig, ConnectionId, QueryId};
use lucent_worker_host::{Connector, ExecutionEvent};
use uuid::Uuid;

#[tokio::test]
async fn duckdb_conforms() {
    let connector = DuckDbConnector::default();
    let cid = ConnectionId(Uuid::new_v4());
    connector
        .connect(cid, ConnectionConfig::new("duckdb").with("path", ":memory:"))
        .await
        .expect("connect");

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let exec = connector.execute(
        cid,
        QueryId(Uuid::new_v4()),
        lucent_driver_conformance::SEED_SQL.into(),
        tx,
    );
    let drain = async {
        while let Some(e) = rx.recv().await {
            if let ExecutionEvent::Failed(err) = e {
                panic!("seed failed: {err}");
            }
        }
    };
    tokio::join!(exec, drain);

    let failures = lucent_driver_conformance::run_all(&connector, cid).await;
    assert!(failures.is_empty(), "DuckDB conformance failures: {failures:#?}");
}
