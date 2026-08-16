//! The capability declaration is a promise to the read-only ladder. Verify it
//! against a running server rather than trusting the documentation.

// The seeded-container fixture created in Plan B, Task 5.
mod common;
use common::seeded;

use lucent_protocol::{QueryId, ReadOnlyMode, TimeoutSupport};
use lucent_worker_host::ExecutionEvent;

#[tokio::test]
async fn postgres_really_supports_a_read_only_transaction() {
    let (_c, connector, cid) = seeded().await;

    // Run one statement; report whether it failed.
    let run = |sql: &'static str| {
        let connector = &connector;
        async move {
            let (tx, mut rx) = tokio::sync::mpsc::channel(8);
            connector
                .execute(cid, QueryId(uuid::Uuid::new_v4()), sql.into(), tx)
                .await;
            let mut failed = false;
            while let Some(ev) = rx.recv().await {
                if matches!(ev, ExecutionEvent::Failed(_)) {
                    failed = true;
                }
            }
            failed
        }
    };

    assert!(!run("BEGIN").await, "BEGIN must succeed");
    assert!(
        !run("SET TRANSACTION READ ONLY").await,
        "declared TransactionScoped, so this statement must be accepted"
    );
    assert!(
        run("CREATE TABLE app.should_not_exist (x int)").await,
        "the engine must refuse a write inside a READ ONLY transaction — \
         this is the second layer the README advertises"
    );
    let _ = run("ROLLBACK").await;

    assert_eq!(
        lucent_driver_postgres::capabilities::postgres().readonly,
        ReadOnlyMode::TransactionScoped
    );
}

#[tokio::test]
async fn postgres_really_supports_a_statement_timeout() {
    let (_c, connector, cid) = seeded().await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    connector
        .execute(
            cid,
            QueryId(uuid::Uuid::new_v4()),
            "SET statement_timeout = 100".into(),
            tx,
        )
        .await;
    let mut failed = false;
    while let Some(ev) = rx.recv().await {
        if matches!(ev, ExecutionEvent::Failed(_)) {
            failed = true;
        }
    }
    assert!(!failed, "declared TimeoutSupport::Statement");
    assert_eq!(
        lucent_driver_postgres::capabilities::postgres().statement_timeout,
        TimeoutSupport::Statement
    );
}
