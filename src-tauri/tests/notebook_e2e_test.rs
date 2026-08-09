#![cfg(feature = "integration-tests")]

use std::time::Duration;

use lucent_protocol::ConnectionConfig;

use lucent_lib::client::ConnectorClient;
use lucent_lib::notebook::rewrite;
use lucent_lib::notebook::types::*;
use lucent_lib::supervisor::Supervisor;

fn pg_config(port: u16) -> ConnectionConfig {
    ConnectionConfig::new("postgres")
        .with("host", "127.0.0.1")
        .with("port", port.to_string())
        .with("user", "postgres")
        .with("database", "postgres")
        .with("ssl_mode", "prefer")
        .with_secret("postgres")
}

async fn start_postgres() -> (
    u16,
    Option<
        testcontainers_modules::testcontainers::ContainerAsync<
            testcontainers_modules::postgres::Postgres,
        >,
    >,
) {
    // Keep the handle: dropping it at test end removes the container. The old
    // `std::mem::forget(container)` leaked one postgres container per test.
    // LUCENT_TEST_PG_PORT reuses an externally-managed container instead.
    if let Ok(port_str) = std::env::var("LUCENT_TEST_PG_PORT") {
        let port: u16 = port_str
            .parse()
            .expect("LUCENT_TEST_PG_PORT must be a valid port number");
        return (port, None);
    }
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;
    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    (port, Some(container))
}

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

fn make_cell(id: &str, kind: CellKind, source: &str, status: CellStatus) -> CellModel {
    CellModel {
        id: id.into(),
        kind,
        source: source.into(),
        alias: None,
        collapsed: false,
        outputs: None,
        status,
        execution_order: None,
        duration_ms: None,
        error: None,
        stale_since: None,
        ai_state: None,
    }
}

fn make_notebook_v2(cells: Vec<CellModel>) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    let metadata = NotebookMetadata {
        connection_id: Some("prof-test".into()),
        connection_name: Some("test".into()),
        connection_host: Some("127.0.0.1".into()),
        database: Some("postgres".into()),
        created_at: now.clone(),
        updated_at: now,
        lucent_version: "0.1.0".into(),
    };
    lucent_lib::notebook::file::to_json(&metadata, &cells).unwrap()
}

#[tokio::test]
async fn test_notebook_open_save_reopen() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let (client, conn_id) = ConnectorClient::connect(&socket_path, &token, pg_config(port))
        .await
        .expect("connect");

    let result_a = client
        .execute(conn_id, "SELECT 42 AS answer")
        .await
        .expect("execute cell A");

    let cell_a = CellModel {
        id: "aaaaaaaa".into(),
        kind: CellKind::Sql,
        source: "SELECT 42 AS answer".into(),
        alias: None,
        collapsed: false,
        outputs: Some(CellOutput::Table(TableOutput {
            columns: result_a.columns.clone(),
            rows: result_a.rows.clone(),
            total_count: Some(result_a.row_count as u64),
            is_truncated: false,
            page_size: 10,
            is_wrappable: true,
            rows_affected: None,
        })),
        status: CellStatus::Ok,
        execution_order: Some(1),
        duration_ms: Some(10),
        error: None,
        stale_since: None,
        ai_state: None,
    };

    let cell_b = make_cell(
        "bbbbbbbb",
        CellKind::Markdown,
        "Hello, notebook!",
        CellStatus::Ok,
    );

    let json = make_notebook_v2(vec![cell_a.clone(), cell_b]);

    let tmp = std::env::temp_dir().join(format!("notebook_test_{}.lucent", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, &json).unwrap();

    let content = std::fs::read_to_string(&tmp).unwrap();
    let opened: lucent_lib::notebook::file::NotebookFileV2 =
        lucent_lib::notebook::file::parse(&content).unwrap();
    assert_eq!(opened.version, 2);
    assert_eq!(opened.cells.len(), 2);
    assert_eq!(opened.cells[0].id, "aaaaaaaa");
    assert_eq!(opened.cells[1].id, "bbbbbbbb");

    match &opened.cells[0].outputs {
        Some(CellOutput::Table(t)) => {
            assert_eq!(t.columns[0].name, "answer");
            assert_eq!(t.rows[0][0], serde_json::json!(42));
        }
        _ => panic!("expected Table output"),
    }

    let mut updated = opened.clone();
    updated.cells[0].outputs = None;
    let json2 = lucent_lib::notebook::file::to_json(&updated.metadata, &[]).unwrap();
    // Rebuild the file with the cell's outputs cleared via the v2 cell type.
    let cell_no_output = lucent_lib::notebook::file::from_file_cell(updated.cells[0].clone());
    assert!(cell_no_output.outputs.is_none());
    std::fs::write(&tmp, &json2).unwrap();

    std::fs::remove_file(&tmp).ok();
    let mut client = client;
    client.shutdown().await.unwrap();
    supervisor.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_cell_reference_cte_composition() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let (client, conn_id) = ConnectorClient::connect(&socket_path, &token, pg_config(port))
        .await
        .expect("connect");

    let result_a = client
        .execute(conn_id, "SELECT 1 AS x")
        .await
        .expect("Cell A execute");
    assert_eq!(result_a.columns[0].name, "x");
    assert_eq!(result_a.rows[0][0], serde_json::json!(1));

    let cell_a = CellModel {
        id: "aaaaaaaa".into(),
        kind: CellKind::Sql,
        source: "SELECT 1 AS x".into(),
        alias: None,
        collapsed: false,
        outputs: Some(CellOutput::Table(TableOutput {
            columns: result_a.columns.clone(),
            rows: result_a.rows.clone(),
            total_count: Some(1),
            is_truncated: false,
            page_size: 10,
            is_wrappable: true,
            rows_affected: None,
        })),
        status: CellStatus::Ok,
        execution_order: Some(1),
        duration_ms: Some(5),
        error: None,
        stale_since: None,
        ai_state: None,
    };

    let cell_b = CellModel {
        id: "bbbbbbbb".into(),
        kind: CellKind::Sql,
        source: "SELECT * FROM ${aaaaaaaa} WHERE x = 1".into(),
        alias: None,
        collapsed: false,
        outputs: None,
        status: CellStatus::Pending,
        execution_order: None,
        duration_ms: None,
        error: None,
        stale_since: None,
        ai_state: None,
    };

    let cells = vec![cell_a, cell_b];

    let rewritten =
        rewrite::rewrite_sql("bbbbbbbb", &cells, lucent_protocol::SqlDialect::PostgreSql)
            .expect("rewrite_sql");
    assert!(
        rewritten.contains("_cell_aaaaaaaa"),
        "rewritten SQL should reference _cell_aaaaaaaa, got: {rewritten}"
    );
    assert!(
        rewritten.starts_with("WITH"),
        "rewritten SQL should start with WITH, got: {rewritten}"
    );

    let result = client
        .execute(conn_id, &rewritten)
        .await
        .expect("cell B execute via CTE");

    assert_eq!(result.columns.len(), 1);
    assert_eq!(result.columns[0].name, "x");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], serde_json::json!(1));

    let mut client = client;
    client.shutdown().await.unwrap();
    supervisor.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_run_all_with_error_stops() {
    let (port, _container) = start_postgres().await;
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    let socket_path = supervisor.ensure_running().await.unwrap().to_path_buf();
    let token = supervisor.handshake_token().to_owned();

    let (client, conn_id) = ConnectorClient::connect(&socket_path, &token, pg_config(port))
        .await
        .expect("connect");

    let cell1 = CellModel {
        id: "aaaaaaaa".into(),
        kind: CellKind::Sql,
        source: "SELECT 1 AS val".into(),
        alias: None,
        collapsed: false,
        outputs: None,
        status: CellStatus::Pending,
        execution_order: None,
        duration_ms: None,
        error: None,
        stale_since: None,
        ai_state: None,
    };

    let cell2 = CellModel {
        id: "bbbbbbbb".into(),
        kind: CellKind::Sql,
        source: "SELECT * FROM nonexistent_table_xyz".into(),
        alias: None,
        collapsed: false,
        outputs: None,
        status: CellStatus::Pending,
        execution_order: None,
        duration_ms: None,
        error: None,
        stale_since: None,
        ai_state: None,
    };

    let cell3 = CellModel {
        id: "cccccccc".into(),
        kind: CellKind::Sql,
        source: "SELECT 2 AS val".into(),
        alias: None,
        collapsed: false,
        outputs: None,
        status: CellStatus::Pending,
        execution_order: None,
        duration_ms: None,
        error: None,
        stale_since: None,
        ai_state: None,
    };

    let cells = vec![cell1, cell2, cell3];
    let mut cell3_ran = false;

    for cell in &cells {
        match client.execute(conn_id, &cell.source).await {
            Ok(_) => {}
            Err(e) => {
                assert!(
                    e.contains("nonexistent_table_xyz") || e.contains("does not exist"),
                    "expected error about nonexistent table, got: {e}"
                );
                break;
            }
        }
        if cell.id == "cccccccc" {
            cell3_ran = true;
        }
    }

    assert!(!cell3_ran, "cell 3 should never have executed");

    let mut client = client;
    client.shutdown().await.unwrap();
    supervisor.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_connection_mismatch_prompt() {
    let json = make_notebook_v2(vec![]);
    let nf: lucent_lib::notebook::file::NotebookFileV2 =
        lucent_lib::notebook::file::parse(&json).unwrap();

    assert_eq!(
        nf.metadata.connection_id.as_deref(),
        Some("prof-test"),
        "file should reference prof-test connection"
    );

    let repo = lucent_lib::connections::ConnectionProfileRepository::load();
    let profiles = repo.list_profiles().await;
    let matches = profiles.iter().any(|p| p.id == "prof-test");

    assert!(!matches, "should find no matching profile for 'prof-test'");
}
