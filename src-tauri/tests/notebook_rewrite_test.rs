use lucent_lib::notebook::rewrite::{build_dag, extract_refs, rewrite_sql, topological_sort};
use lucent_lib::notebook::types::*;

use lucent_protocol::SqlDialect;

const PG: SqlDialect = SqlDialect::PostgreSql;

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

#[test]
fn test_extract_braced_ref() {
    let refs = extract_refs("SELECT * FROM ${a1b2c3d4}");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].cell_id, "a1b2c3d4");
    assert_eq!(refs[0].column, None);
}

#[test]
fn test_extract_column_ref() {
    let refs = extract_refs("WHERE region IN ($a1b2c3d4.region)");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].cell_id, "a1b2c3d4");
    assert_eq!(refs[0].column, Some("region".into()));
}

#[test]
fn test_ignores_postgres_syntax() {
    // Dollar-quoted strings, function params, system vars — NOT cell refs
    let refs = extract_refs("SELECT $$body$$, $1, $user FROM t");
    assert!(refs.is_empty());
}

#[test]
fn test_ignores_non_8char_hex() {
    // Only 8-char hex patterns match
    let refs = extract_refs("SELECT ${cell} FROM $mycell.col");
    assert!(refs.is_empty());
}

#[test]
fn test_multiple_refs() {
    let refs =
        extract_refs("SELECT * FROM ${a1b2c3d4} JOIN ${e5f6a7b8} ON $a1b2c3d4.id = $e5f6a7b8.id");
    assert_eq!(refs.len(), 4);
}

#[test]
fn test_single_dependency() {
    let cells = vec![
        make_cell("a1b2c3d4", CellKind::Sql, "SELECT 1", CellStatus::Ok),
        make_cell(
            "e5f6a7b8",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Pending,
        ),
    ];
    let dag = build_dag("e5f6a7b8", &cells, PG).unwrap();
    assert!(dag.contains_key("a1b2c3d4"));
    assert_eq!(dag["e5f6a7b8"].len(), 1);
}

#[test]
fn test_transitive_dependency() {
    let cells = vec![
        make_cell("a1b2c3d4", CellKind::Sql, "SELECT 1", CellStatus::Ok),
        make_cell(
            "b2c3d4e5",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Ok,
        ),
        make_cell(
            "c3d4e5f6",
            CellKind::Sql,
            "SELECT * FROM ${b2c3d4e5}",
            CellStatus::Pending,
        ),
    ];
    let dag = build_dag("c3d4e5f6", &cells, PG).unwrap();
    let order = topological_sort(&dag);
    assert_eq!(&order[..2], &["a1b2c3d4", "b2c3d4e5"]);
}

#[test]
fn test_cycle_detection() {
    let cells = vec![
        make_cell(
            "a1b2c3d4",
            CellKind::Sql,
            "SELECT * FROM ${b2c3d4e5}",
            CellStatus::Ok,
        ),
        make_cell(
            "b2c3d4e5",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Ok,
        ),
    ];
    let result = build_dag("a1b2c3d4", &cells, PG);
    assert!(result.is_err());
    match result.unwrap_err() {
        CellError::CyclicDependency { cycle, .. } => {
            assert!(cycle.contains(&"a1b2c3d4".into()));
            assert!(cycle.contains(&"b2c3d4e5".into()));
        }
        _ => panic!("expected CyclicDependency"),
    }
}

#[test]
fn test_not_executed_error() {
    let cells = vec![
        make_cell("a1b2c3d4", CellKind::Sql, "SELECT 1", CellStatus::Pending),
        make_cell(
            "e5f6a7b8",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Pending,
        ),
    ];
    let result = build_dag("e5f6a7b8", &cells, PG);
    match result.unwrap_err() {
        CellError::NotExecuted { cell_id, .. } => assert_eq!(cell_id, "a1b2c3d4"),
        _ => panic!("expected NotExecuted"),
    }
}

#[test]
fn test_text_not_referencable() {
    let cells = vec![
        {
            let mut c = make_cell("a1b2c3d4", CellKind::Ai, "explain this", CellStatus::Ok);
            c.outputs = Some(CellOutput::Text(TextOutput {
                content: "hello".into(),
            }));
            c.ai_state = Some(AiCellState {
                conversation_id: "conv1".into(),
                final_sql: None,
                response: None,
                messages: vec![],
                tool_calls: vec![],
            });
            c
        },
        make_cell(
            "e5f6a7b8",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Pending,
        ),
    ];
    let result = build_dag("e5f6a7b8", &cells, PG);
    match result.unwrap_err() {
        CellError::TextNotReferencable { cell_id, .. } => assert_eq!(cell_id, "a1b2c3d4"),
        _ => panic!("expected TextNotReferencable"),
    }
}

#[test]
fn test_dml_not_referencable() {
    let cells = vec![
        make_cell(
            "a1b2c3d4",
            CellKind::Sql,
            "INSERT INTO foo VALUES (1)",
            CellStatus::Ok,
        ),
        make_cell(
            "e5f6a7b8",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Pending,
        ),
    ];
    let result = build_dag("e5f6a7b8", &cells, PG);
    match result.unwrap_err() {
        CellError::NotATable { cell_id, .. } => assert_eq!(cell_id, "a1b2c3d4"),
        _ => panic!("expected NotATable"),
    }
}

#[test]
fn test_ai_cell_uses_final_sql() {
    let cells = vec![
        {
            let mut c = make_cell("a1b2c3d4", CellKind::Ai, "find top regions", CellStatus::Ok);
            c.outputs = Some(CellOutput::Table(TableOutput {
                columns: vec![],
                rows: vec![],
                total_count: Some(0),
                is_truncated: false,
                page_size: 10,
                is_wrappable: true,
                rows_affected: None,
            }));
            c.ai_state = Some(AiCellState {
                conversation_id: "conv1".into(),
                final_sql: Some("SELECT region FROM sales LIMIT 10".into()),
                response: None,
                messages: vec![],
                tool_calls: vec![],
            });
            c
        },
        make_cell(
            "e5f6a7b8",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Pending,
        ),
    ];
    let _dag = build_dag("e5f6a7b8", &cells, PG).unwrap();
    let rewritten = rewrite_sql("e5f6a7b8", &cells, PG).unwrap();
    // The composed SQL round-trips through sqlparser (Task 3/4), so assert the
    // CTE content rather than exact whitespace of the raw interpolation.
    assert!(rewritten.contains("_cell_a1b2c3d4 AS ("), "got {rewritten}");
    assert!(
        rewritten.contains("SELECT region FROM sales LIMIT 10"),
        "got {rewritten}"
    );
}

#[test]
fn test_empty_sql_no_refs() {
    let refs = extract_refs("");
    assert!(refs.is_empty());
}

#[test]
fn test_mixed_case_id_matching() {
    let refs = extract_refs("SELECT * FROM ${A1B2C3D4}");
    assert!(refs.is_empty());
}

#[test]
fn test_self_reference_detected() {
    let cells = vec![make_cell(
        "a1b2c3d4",
        CellKind::Sql,
        "SELECT * FROM ${a1b2c3d4}",
        CellStatus::Ok,
    )];
    let result = build_dag("a1b2c3d4", &cells, PG);
    assert!(matches!(
        result.unwrap_err(),
        CellError::CyclicDependency { .. }
    ));
}

#[test]
fn test_deep_transitive_chain() {
    let cells = vec![
        make_cell("a1b2c3d4", CellKind::Sql, "SELECT 1", CellStatus::Ok),
        make_cell(
            "b2c3d4e5",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Ok,
        ),
        make_cell(
            "c3d4e5f6",
            CellKind::Sql,
            "SELECT * FROM ${b2c3d4e5}",
            CellStatus::Ok,
        ),
        make_cell(
            "d4e5f6a7",
            CellKind::Sql,
            "SELECT * FROM ${c3d4e5f6}",
            CellStatus::Ok,
        ),
        make_cell(
            "e5f6a7b8",
            CellKind::Sql,
            "SELECT * FROM ${d4e5f6a7}",
            CellStatus::Pending,
        ),
    ];
    let dag = build_dag("e5f6a7b8", &cells, PG).unwrap();
    let order = topological_sort(&dag);
    assert_eq!(
        &order[..4],
        &["a1b2c3d4", "b2c3d4e5", "c3d4e5f6", "d4e5f6a7"]
    );
}

#[test]
fn test_multiple_dependencies_on_same_cell() {
    let cells = vec![
        make_cell("c1c2c3c4", CellKind::Sql, "SELECT 1", CellStatus::Ok),
        make_cell(
            "a1a2a3a4",
            CellKind::Sql,
            "SELECT * FROM ${c1c2c3c4}",
            CellStatus::Ok,
        ),
        make_cell(
            "b1b2b3b4",
            CellKind::Sql,
            "SELECT * FROM ${c1c2c3c4}",
            CellStatus::Ok,
        ),
        make_cell(
            "d1d2d3d4",
            CellKind::Sql,
            "SELECT * FROM ${a1a2a3a4} JOIN ${b1b2b3b4}",
            CellStatus::Pending,
        ),
    ];
    let rewritten = rewrite_sql("d1d2d3d4", &cells, PG).unwrap();
    let count = rewritten.matches("_cell_c1c2c3c4 AS (").count();
    assert_eq!(count, 1);
}

#[test]
fn test_rewrite_preserves_user_limit() {
    let cells = vec![
        make_cell(
            "a1b2c3d4",
            CellKind::Sql,
            "SELECT * FROM huge_table LIMIT 50",
            CellStatus::Ok,
        ),
        make_cell(
            "e5f6a7b8",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Pending,
        ),
    ];
    let rewritten = rewrite_sql("e5f6a7b8", &cells, PG).unwrap();
    assert!(rewritten.contains("LIMIT 50"));
}

#[test]
fn test_ai_cell_without_final_sql_falls_back_to_source() {
    let cells = vec![
        {
            let mut c = make_cell("a1b2c3d4", CellKind::Ai, "SELECT 1", CellStatus::Ok);
            c.outputs = Some(CellOutput::Table(TableOutput {
                columns: vec![],
                rows: vec![],
                total_count: Some(0),
                is_truncated: false,
                page_size: 10,
                is_wrappable: true,
                rows_affected: None,
            }));
            c.ai_state = Some(AiCellState {
                conversation_id: "c1".into(),
                final_sql: None,
                response: None,
                messages: vec![],
                tool_calls: vec![],
            });
            c
        },
        make_cell(
            "e5f6a7b8",
            CellKind::Sql,
            "SELECT * FROM ${a1b2c3d4}",
            CellStatus::Pending,
        ),
    ];
    let rewritten = rewrite_sql("e5f6a7b8", &cells, PG).unwrap();
    assert!(rewritten.contains("SELECT 1"));
}
