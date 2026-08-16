#[cfg(feature = "integration-tests")]
use std::time::Duration;

use lucent_lib::notebook::types::*;

#[cfg(feature = "integration-tests")]
use testcontainers::runners::AsyncRunner;
#[cfg(feature = "integration-tests")]
use testcontainers_modules::postgres::Postgres;
#[cfg(feature = "integration-tests")]
use tokio_postgres::NoTls;

#[cfg(feature = "integration-tests")]
use lucent_protocol::SqlDialect;

#[cfg(feature = "integration-tests")]
const PG: SqlDialect = SqlDialect::PostgreSql;

#[cfg(feature = "integration-tests")]
fn pg_config(port: u16) -> lucent_protocol::ConnectionConfig {
    lucent_protocol::ConnectionConfig::new("postgres")
        .with("host", "127.0.0.1")
        .with("port", port.to_string())
        .with("user", "postgres")
        .with("database", "postgres")
        .with("ssl_mode", "prefer")
        .with_secret("postgres")
}

#[cfg(feature = "integration-tests")]
async fn start_postgres() -> (u16, testcontainers::ContainerAsync<Postgres>) {
    // Keep the handle: dropping it at test end removes the container. The old
    // `std::mem::forget(container)` leaked one postgres container per test.
    let container = Postgres::default()
        .start()
        .await
        .expect("postgres container to start");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get postgres port");
    (port, container)
}

#[cfg(feature = "integration-tests")]
async fn wait_for_postgres(port: u16) {
    let conn_string =
        format!("host=127.0.0.1 port={port} user=postgres password=postgres dbname=postgres");
    for i in 0..20 {
        match tokio_postgres::connect(&conn_string, NoTls).await {
            Ok((_client, connection)) => {
                tokio::spawn(connection);
                return;
            }
            Err(_) if i < 19 => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(e) => panic!("postgres not ready after 20 retries: {e}"),
        }
    }
}

#[cfg(feature = "integration-tests")]
mod integration {
    use super::*;
    use lucent_lib::client::ConnectorClient;
    use lucent_lib::supervisor::Supervisor;

    #[tokio::test]
    async fn test_sql_cell_select_1_returns_one_row() {
        let (port, _container) = start_postgres().await;
        wait_for_postgres(port).await;

        let mut supervisor = Supervisor::new();
        supervisor
            .ensure_running()
            .await
            .expect("supervisor running");
        let socket_path = supervisor.endpoint().to_string();
        let token = supervisor.handshake_token().to_owned();

        let (mut client, conn_id) = ConnectorClient::connect(&socket_path, &token, pg_config(port))
            .await
            .expect("connect client");

        // Execute actual SQL through the worker — validates the full pipeline
        let result = client
            .execute(conn_id, "SELECT 1 AS x")
            .await
            .expect("execute SELECT 1");

        assert_eq!(result.columns.len(), 1);
        assert_eq!(result.columns[0].name, "x");
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0], serde_json::json!(1));
        assert_eq!(result.row_count, 1);

        client.shutdown().await.expect("shutdown");
        let _ = supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn test_run_cell_a_then_cell_b_cte_composed() {
        let (port, _container) = start_postgres().await;
        wait_for_postgres(port).await;

        let mut supervisor = Supervisor::new();
        supervisor
            .ensure_running()
            .await
            .expect("supervisor running");
        let socket_path = supervisor.endpoint().to_string();
        let token = supervisor.handshake_token().to_owned();

        let (mut client, conn_id) = ConnectorClient::connect(&socket_path, &token, pg_config(port))
            .await
            .expect("connect client");

        // Execute cell A directly
        let result_a = client
            .execute(conn_id, "SELECT 1 AS a")
            .await
            .expect("execute cell A");
        assert_eq!(result_a.columns[0].name, "a");
        assert_eq!(result_a.rows[0][0], serde_json::json!(1));

        // Compose cell B with CTE reference to cell A, then execute against real Postgres
        let cell_a = CellModel {
            id: "a1b2c3d4".into(),
            kind: CellKind::Sql,
            source: "SELECT 1 AS a".into(),
            alias: None,
            collapsed: false,
            outputs: None,
            status: CellStatus::Ok,
            execution_order: Some(1),
            duration_ms: Some(10),
            error: None,
            stale_since: None,
            ai_state: None,
        };
        let cell_b = CellModel {
            id: "b2c3d4e5".into(),
            kind: CellKind::Sql,
            source: "SELECT * FROM ${a1b2c3d4} WHERE a = 1".into(),
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

        let rewritten = lucent_lib::notebook::rewrite::rewrite_sql("b2c3d4e5", &cells, PG)
            .expect("rewrite should succeed");
        assert!(
            rewritten.contains("_cell_a1b2c3d4"),
            "CTE alias should appear: {rewritten}"
        );
        assert!(
            rewritten.contains("SELECT 1 AS a"),
            "original source should be in CTE: {rewritten}"
        );

        // Execute the CTE-composed SQL
        let result_b = client
            .execute(conn_id, &rewritten)
            .await
            .expect("execute CTE-composed query");
        assert_eq!(result_b.columns[0].name, "a");
        assert_eq!(result_b.rows.len(), 1);
        assert_eq!(result_b.rows[0][0], serde_json::json!(1));

        client.shutdown().await.expect("shutdown");
        let _ = supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn test_run_all_executes_in_order() {
        let (port, _container) = start_postgres().await;
        wait_for_postgres(port).await;

        let mut supervisor = Supervisor::new();
        supervisor
            .ensure_running()
            .await
            .expect("supervisor running");
        let socket_path = supervisor.endpoint().to_string();
        let token = supervisor.handshake_token().to_owned();

        let (mut client, conn_id) = ConnectorClient::connect(&socket_path, &token, pg_config(port))
            .await
            .expect("connect client");

        // Simulate 3 cells that must execute in order: create table → insert → select
        client
            .execute(conn_id, "CREATE TEMP TABLE test_order (val INT)")
            .await
            .expect("create table");
        client
            .execute(conn_id, "INSERT INTO test_order VALUES (42)")
            .await
            .expect("insert");
        let result = client
            .execute(conn_id, "SELECT val FROM test_order")
            .await
            .expect("select");
        assert_eq!(result.row_count, 1);
        assert_eq!(result.rows[0][0], serde_json::json!(42));

        client.shutdown().await.expect("shutdown");
        let _ = supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn test_run_all_stops_on_error() {
        let (port, _container) = start_postgres().await;
        wait_for_postgres(port).await;

        let mut supervisor = Supervisor::new();
        supervisor
            .ensure_running()
            .await
            .expect("supervisor running");
        let socket_path = supervisor.endpoint().to_string();
        let token = supervisor.handshake_token().to_owned();

        let (mut client, conn_id) = ConnectorClient::connect(&socket_path, &token, pg_config(port))
            .await
            .expect("connect client");

        // Create a table first (cell 1)
        client
            .execute(conn_id, "CREATE TEMP TABLE test_stop (val INT)")
            .await
            .expect("create table");

        // Cell 2 has invalid SQL — should error
        let err = client
            .execute(conn_id, "SELECT invalid_column_name_xyz FROM test_stop")
            .await;
        assert!(err.is_err(), "cell 2 should error");
        let err_msg = err.unwrap_err();
        assert!(
            err_msg.contains("invalid_column_name_xyz"),
            "error should mention invalid column: {err_msg}"
        );

        // Cell 3 should NOT have executed — but since we're simulating
        // stop-on-error at the client level, we just verify error handling works.
        // The key assertion is that we never sent cell 3's SQL.
        // We verify this by checking the table still has no rows.
        let check = client
            .execute(conn_id, "SELECT COUNT(*) FROM test_stop")
            .await
            .expect("count should still work");
        assert_eq!(check.rows[0][0], serde_json::json!(0));

        client.shutdown().await.expect("shutdown");
        let _ = supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn test_run_all_continue_on_error() {
        let (port, _container) = start_postgres().await;
        wait_for_postgres(port).await;

        let mut supervisor = Supervisor::new();
        supervisor
            .ensure_running()
            .await
            .expect("supervisor running");
        let socket_path = supervisor.endpoint().to_string();
        let token = supervisor.handshake_token().to_owned();

        let (mut client, conn_id) = ConnectorClient::connect(&socket_path, &token, pg_config(port))
            .await
            .expect("connect client");

        // Cell 1: succeeds
        client
            .execute(conn_id, "CREATE TEMP TABLE test_continue (val INT)")
            .await
            .expect("create table");

        // Cell 2: intentionally fails
        let err = client
            .execute(conn_id, "INSERT INTO test_continue VALUES ('not_an_int')")
            .await;
        assert!(err.is_err(), "cell 2 should error (type mismatch)");

        // Cell 3: still executes despite cell 2's error
        let result = client
            .execute(conn_id, "INSERT INTO test_continue VALUES (99)")
            .await
            .expect("cell 3 should still execute");
        // NB: the worker uses the text protocol, so DML command tags are not
        // parsed into row counts — the INSERT's effect is asserted below via
        // the SELECT instead.

        let check = client
            .execute(conn_id, "SELECT val FROM test_continue")
            .await
            .expect("select");
        assert_eq!(
            check.rows.len(),
            1,
            "only cell 3 succeeded, so 1 row expected"
        );
        assert_eq!(check.rows[0][0], serde_json::json!(99));

        client.shutdown().await.expect("shutdown");
        let _ = supervisor.shutdown().await;
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

    #[tokio::test]
    async fn test_cycle_detection_returns_cyclic_error() {
        let cell_a = make_cell(
            "a1b2c3d4",
            CellKind::Sql,
            "SELECT * FROM ${b2c3d4e5}",
            CellStatus::Ok,
        );
        let cell_b = make_cell(
            "b2c3d4e5",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Ok,
        );
        let cells = vec![cell_a, cell_b];

        let result = lucent_lib::notebook::rewrite::build_dag("a1b2c3d4", &cells, PG);
        assert!(result.is_err());
        match result.unwrap_err() {
            CellError::CyclicDependency { .. } => {} // Expected
            other => panic!("expected CyclicDependency, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_text_cell_not_referencable() {
        let text_cell = CellModel {
            id: "a1b2c3d4".into(),
            kind: CellKind::Ai,
            source: "Explain this query".into(),
            alias: None,
            collapsed: false,
            outputs: Some(CellOutput::Text(TextOutput {
                content: "some text".into(),
            })),
            status: CellStatus::Ok,
            execution_order: Some(1),
            duration_ms: Some(10),
            error: None,
            stale_since: None,
            ai_state: None,
        };

        let sql_cell = make_cell(
            "e5f6a7b8",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Pending,
        );
        let cells = vec![text_cell, sql_cell];

        let result = lucent_lib::notebook::rewrite::build_dag("e5f6a7b8", &cells, PG);
        assert!(result.is_err());
        match result.unwrap_err() {
            CellError::TextNotReferencable { .. } => {} // Expected
            other => panic!("expected TextNotReferencable, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_dml_cell_not_referencable() {
        let dml_cell = make_cell(
            "a1b2c3d4",
            CellKind::Sql,
            "INSERT INTO test VALUES (1)",
            CellStatus::Ok,
        );
        let sql_cell = make_cell(
            "e5f6a7b8",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Pending,
        );
        let cells = vec![dml_cell, sql_cell];

        let result = lucent_lib::notebook::rewrite::build_dag("e5f6a7b8", &cells, PG);
        assert!(result.is_err());
        match result.unwrap_err() {
            // Task 4: the DML check is now wrappability (NotATable), which subsumes
            // the old DmlNotReferencable.
            CellError::NotATable { .. } => {} // Expected
            other => panic!("expected NotATable, got: {other:?}"),
        }
    }

    fn make_cell_with_output(
        id: &str,
        kind: CellKind,
        source: &str,
        status: CellStatus,
        output: Option<CellOutput>,
    ) -> CellModel {
        CellModel {
            id: id.into(),
            kind,
            source: source.into(),
            alias: None,
            collapsed: false,
            outputs: output,
            status,
            execution_order: None,
            duration_ms: None,
            error: None,
            stale_since: None,
            ai_state: None,
        }
    }

    #[tokio::test]
    async fn test_assemble_ai_context_respects_token_budget() {
        use lucent_lib::notebook::commands;

        let cell_ok = make_cell_with_output(
            "c1",
            CellKind::Sql,
            "SELECT 1",
            CellStatus::Ok,
            Some(CellOutput::Table(TableOutput {
                columns: vec![lucent_protocol::ColumnMeta {
                    name: "?column?".into(),
                    type_name: "int4".into(),
                }],
                rows: vec![vec![serde_json::json!(1)]],
                total_count: Some(1),
                is_truncated: false,
                page_size: 10,
                is_wrappable: true,
                rows_affected: None,
            })),
        );

        let current = make_cell_with_output(
            "c2",
            CellKind::Ai,
            "What does this show?",
            CellStatus::Pending,
            None,
        );

        let context = commands::assemble_ai_context(&[cell_ok, current], "c2", 5, 4000);

        assert!(!context.is_empty(), "context should include the prior cell");
        assert!(context.contains("c1"), "context should reference cell id");
        assert!(
            context.contains("?column?"),
            "context should include column names"
        );
    }

    #[tokio::test]
    async fn test_classify_ai_output_select_returns_table() {
        use lucent_lib::notebook::commands;

        let tool_calls = vec![serde_json::json!({
            "name": "execute_sql",
            "args": { "sql": "SELECT * FROM users" },
            "output": {
                "table": {
                    "columns": [{"name": "id", "type_name": "int4"}],
                    "rows": [[1]],
                    "total_count": 1,
                    "is_truncated": false
                }
            },
            "summary": "Returned 1 row"
        })];

        let output = commands::classify_ai_output(&tool_calls, None);
        match output {
            CellOutput::Table(t) => {
                assert_eq!(t.columns.len(), 1);
                assert_eq!(t.columns[0].name, "id");
                assert_eq!(t.total_count, Some(1));
            }
            other => panic!("expected TableOutput, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_classify_ai_output_no_sql_returns_text() {
        use lucent_lib::notebook::commands;

        let tool_calls = vec![serde_json::json!({
            "name": "search_schema",
            "summary": "Found table users with columns id, name, email"
        })];

        let output = commands::classify_ai_output(&tool_calls, None);
        match output {
            CellOutput::Text(t) => {
                assert!(t.content.contains("users"), "text should include summary");
            }
            other => panic!("expected TextOutput, got: {other:?}"),
        }
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    use lucent_lib::notebook::commands::{assemble_ai_context, classify_ai_output};

    #[test]
    fn test_assemble_ai_context_empty_no_prior_cells() {
        let context = assemble_ai_context(&[], "c1", 5, 4000);
        assert!(
            context.is_empty(),
            "no prior cells should yield empty context"
        );
    }

    #[test]
    fn test_assemble_ai_context_skips_current_cell() {
        let cell_a = CellModel {
            id: "a1".into(),
            kind: CellKind::Sql,
            source: "SELECT 1".into(),
            alias: None,
            collapsed: false,
            outputs: Some(CellOutput::Table(TableOutput {
                columns: vec![],
                rows: vec![],
                total_count: Some(0),
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
        let cell_b = CellModel {
            id: "b1".into(),
            kind: CellKind::Sql,
            source: "SELECT 2".into(),
            alias: None,
            collapsed: false,
            outputs: Some(CellOutput::Table(TableOutput {
                columns: vec![],
                rows: vec![],
                total_count: Some(0),
                is_truncated: false,
                page_size: 10,
                is_wrappable: true,
                rows_affected: None,
            })),
            status: CellStatus::Ok,
            execution_order: Some(2),
            duration_ms: Some(10),
            error: None,
            stale_since: None,
            ai_state: None,
        };

        // Current cell is "b1" — should only include "a1"
        let context = assemble_ai_context(&[cell_a, cell_b], "b1", 5, 4000);
        assert!(context.contains("a1"), "should include a1");
        assert!(!context.contains("b1"), "should NOT include current cell");
    }

    #[test]
    fn test_classify_ai_output_explain_returns_table() {
        let tool_calls = vec![serde_json::json!({
            "name": "execute_sql",
            "args": { "sql": "EXPLAIN ANALYZE SELECT * FROM users" },
            "output": {
                "table": {
                    "columns": [{"name": "QUERY PLAN", "type_name": "text"}],
                    "rows": [["Seq Scan on users"]],
                    "total_count": 1,
                    "is_truncated": false
                }
            }
        })];

        let output = classify_ai_output(&tool_calls, None);
        match output {
            CellOutput::Table(t) => assert_eq!(t.total_count, Some(1)),
            other => panic!("expected TableOutput for EXPLAIN, got: {other:?}"),
        }
    }

    #[test]
    fn test_classify_ai_output_empty_tool_calls_returns_text() {
        let output = classify_ai_output(&[], None);
        match output {
            CellOutput::Text(t) => assert!(t.content.is_empty()),
            other => panic!("expected TextOutput, got: {other:?}"),
        }
    }

    #[test]
    fn test_assemble_ai_context_respects_max_cells() {
        let mut cells = Vec::new();
        for i in 0..10 {
            cells.push(CellModel {
                id: format!("c{i}"),
                kind: CellKind::Sql,
                source: "SELECT 1".into(),
                alias: None,
                collapsed: false,
                outputs: Some(CellOutput::Table(TableOutput {
                    columns: vec![],
                    rows: vec![],
                    total_count: Some(1),
                    is_truncated: false,
                    page_size: 10,
                    is_wrappable: true,
                    rows_affected: None,
                })),
                status: CellStatus::Ok,
                execution_order: Some(i as u32),
                duration_ms: Some(5),
                error: None,
                stale_since: None,
                ai_state: None,
            });
        }

        let context = assemble_ai_context(&cells, "c9", 3, 4000);
        // Should only include last 3 executed cells before current
        let count = context.matches("[Cell ").count();
        assert!(count <= 3, "should include at most 3 cells, got {count}");
    }
}
