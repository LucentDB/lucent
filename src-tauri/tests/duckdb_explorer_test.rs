//! The explorer flow against a real DuckDB file, driven exactly the way the
//! sidebar drives it: list namespaces → map to `SchemaInfo` (the wire shape
//! `get_schemas` returns) → list objects by the namespace **path segments**.
//!
//! Regression test for the empty schema browser: the sidebar used to
//! round-trip the dotted display name (`analytics.main`) back as a single
//! namespace segment, which a multi-segment driver (DuckDB's catalog.schema)
//! reads as a bare schema named `analytics.main` — matching nothing.
//!
//! Requires `cargo build --workspace` first (spawns the real worker binary,
//! same contract as `duckdb_e2e_test`).

use lucent_lib::client::ConnectorClient;
use lucent_lib::namespaces_to_schema_info;
use lucent_lib::supervisor::{new_log_buffer, Supervisor};
use lucent_protocol::{ConnectionConfig, ConnectionId, ObjectKind};

async fn connected_file() -> (Supervisor, ConnectorClient, ConnectionId, tempfile::TempDir) {
    // A file-backed database named `analytics.duckdb` — its catalog is
    // `analytics`, so the namespace the sidebar will see is `analytics.main`.
    // The TempDir must outlive the connection (dropping it deletes the
    // database file mid-test).
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("analytics.duckdb");
    let path_str = path.to_string_lossy().to_string();

    let mut supervisor = Supervisor::for_driver("duckdb", new_log_buffer());
    supervisor
        .ensure_running()
        .await
        .expect("duckdb worker binary must be built — run `cargo build --workspace`");
    let socket = supervisor.endpoint().to_string();
    let token = supervisor.handshake_token().to_string();

    let (client, cid) = ConnectorClient::connect(
        &socket,
        &token,
        ConnectionConfig::new("duckdb").with("path", &path_str),
    )
    .await
    .expect("connect through the worker");

    (supervisor, client, cid, dir)
}

#[tokio::test]
async fn the_sidebar_flow_lists_tables_under_a_file_backed_duckdb_schema() {
    let (mut supervisor, client, cid, _dir) = connected_file().await;

    client
        .execute(
            cid,
            "CREATE TABLE users (id BIGINT PRIMARY KEY, name VARCHAR)",
        )
        .await
        .expect("create users");
    client
        .execute(cid, "INSERT INTO users VALUES (1, 'ada'), (2, 'grace')")
        .await
        .expect("seed users");

    // Step 1 — `get_schemas`: namespaces through the exact wire mapper.
    let namespaces = client.list_namespaces(cid).await.expect("list namespaces");
    let schemas = namespaces_to_schema_info(namespaces);
    let main = schemas
        .iter()
        .find(|s| s.name == "analytics.main")
        .unwrap_or_else(|| panic!("expected analytics.main in {schemas:#?}"));
    assert_eq!(main.path, vec!["analytics", "main"]);
    assert!(main.object_count >= 1, "users must count: {main:?}");

    // Step 2 — `get_schema_objects` with the path segments: users is listed.
    let objects = client
        .list_objects(cid, main.path.clone(), vec![])
        .await
        .expect("list objects by path");
    assert!(
        objects
            .iter()
            .any(|o| o.reference.name == "users" && o.reference.kind == ObjectKind::Table),
        "users must be listed under the path segments: {objects:#?}"
    );

    // The old bug, pinned: round-tripping the dotted display name as ONE
    // segment matches nothing (a bare schema named `analytics.main`).
    let dotted = client
        .list_objects(cid, vec![main.name.clone()], vec![])
        .await
        .expect("list objects by dotted name");
    assert!(
        dotted.is_empty(),
        "a dotted schema name must not be treated as a single segment"
    );

    // Describe works through the same path.
    let users_ref = objects
        .iter()
        .find(|o| o.reference.name == "users")
        .unwrap()
        .reference
        .clone();
    let details = client
        .describe_objects(cid, vec![users_ref])
        .await
        .expect("describe users");
    assert_eq!(details[0].columns.len(), 2);
    assert!(details[0]
        .columns
        .iter()
        .any(|c| c.name == "id" && c.is_primary_key));

    supervisor.shutdown().await.ok();
}

#[tokio::test]
async fn browse_sql_quotes_each_namespace_segment_the_engine_accepts_it() {
    let (mut supervisor, client, cid, _dir) = connected_file().await;

    client
        .execute(
            cid,
            "CREATE TABLE users (id BIGINT PRIMARY KEY, name VARCHAR)",
        )
        .await
        .expect("create users");

    // The shape `browse_table` now produces — each segment quoted separately
    // — is accepted by the engine.
    client
        .execute(cid, r#"SELECT * FROM "analytics"."main"."users""#)
        .await
        .expect("segment-quoted select must work");

    // The old shape — the dotted display name quoted as ONE identifier —
    // must keep failing loudly (pins the bug the explorer used to ship).
    let err = client
        .execute(cid, r#"SELECT * FROM "analytics.main"."users""#)
        .await
        .expect_err("a dotted schema quoted as one identifier must not resolve");
    assert!(
        err.contains("does not exist"),
        "expected a catalog error naming the missing schema: {err}"
    );

    supervisor.shutdown().await.ok();
}
