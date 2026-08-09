use lucent_protocol::SqlDialect;
use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

use crate::notebook::cte::{compose, validate_referenceable};
use crate::notebook::exec_refs::resolve_exec_refs_in_cells;
use crate::notebook::types::*;

static CELL_REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{([a-f0-9]{8})\}|\$([a-f0-9]{8})\.([a-z_][a-z0-9_]*)").unwrap()
});

pub struct CellRef {
    pub cell_id: String,
    pub column: Option<String>,
}

pub fn extract_refs(source: &str) -> Vec<CellRef> {
    CELL_REF_RE
        .captures_iter(source)
        .map(|cap| {
            if let Some(brace_id) = cap.get(1) {
                CellRef {
                    cell_id: brace_id.as_str().to_string(),
                    column: None,
                }
            } else {
                CellRef {
                    cell_id: cap.get(2).unwrap().as_str().to_string(),
                    column: Some(cap.get(3).unwrap().as_str().to_string()),
                }
            }
        })
        .collect()
}

pub fn build_dag(
    cell_id: &str,
    cells: &[CellModel],
    dialect: SqlDialect,
) -> Result<HashMap<String, Vec<String>>, CellError> {
    // Execution-order refs become stable ids before any DAG work, so the rest of
    // the pipeline only ever sees `[a-f0-9]{8}` ids. Idempotent, so the nested
    // call from rewrite_sql is a no-op.
    let cells = resolve_exec_refs_in_cells(cells)?;
    let mut dag = HashMap::new();
    let mut visited = Vec::new();
    build_dag_recursive(cell_id, &cells, &mut dag, &mut visited, dialect)?;
    Ok(dag)
}

fn build_dag_recursive(
    cell_id: &str,
    cells: &[CellModel],
    dag: &mut HashMap<String, Vec<String>>,
    visited: &mut Vec<String>,
    dialect: SqlDialect,
) -> Result<(), CellError> {
    if visited.contains(&cell_id.to_string()) {
        let mut cycle = visited.clone();
        cycle.push(cell_id.to_string());
        return Err(CellError::CyclicDependency {
            cycle,
            hint: "circular cell reference detected".into(),
        });
    }

    let cell = cells
        .iter()
        .find(|c| c.id == cell_id)
        .ok_or_else(|| CellError::UnresolvedRef {
            cell_id: cell_id.into(),
            ref_name: cell_id.into(),
            hint: "referenced cell not found in notebook".into(),
        })?;

    let refs = extract_refs(&cell.source);
    let ref_ids: Vec<String> = refs.iter().map(|r| r.cell_id.clone()).collect();

    dag.insert(cell_id.to_string(), ref_ids.clone());
    visited.push(cell_id.to_string());

    for ref_id in &ref_ids {
        let ref_cell =
            cells
                .iter()
                .find(|c| &c.id == ref_id)
                .ok_or_else(|| CellError::UnresolvedRef {
                    cell_id: ref_id.clone(),
                    ref_name: ref_id.clone(),
                    hint: "referenced cell not found".into(),
                })?;

        if let CellKind::Markdown = &ref_cell.kind {
            return Err(CellError::NotExecutable {
                cell_id: ref_id.clone(),
                message: "markdown cells cannot be referenced".into(),
            });
        }

        if ref_cell.status != CellStatus::Ok {
            if ref_cell.stale_since.is_some() {
                return Err(CellError::StaleReference {
                    cell_id: ref_id.clone(),
                    hint: format!("cell '{}' was modified — re-run it first", ref_id),
                });
            }
            return Err(CellError::NotExecuted {
                cell_id: ref_id.clone(),
                hint: format!("cell '{}' has not been executed", ref_id),
            });
        }

        match &ref_cell.kind {
            CellKind::Sql => {
                // Subsumes the old is_dml_or_ddl check: "not a single Statement::Query"
                // already covers DML, DDL, and multi-statement bodies. Validates the
                // SUBSTITUTED source: a chained cell's raw source contains `${id}`
                // table names, which sqlparser rejects — after substitution they are
                // ordinary `_cell_*` identifiers, which is what actually runs.
                validate_referenceable(ref_id, &substitute_refs(&ref_cell.source), dialect)?;
            }
            CellKind::Ai => {
                if let Some(CellOutput::Text(_)) = &ref_cell.outputs {
                    return Err(CellError::TextNotReferencable {
                        cell_id: ref_id.clone(),
                        message: format!("cell '{}' produced text, not a table", ref_id),
                    });
                }
            }
            _ => {}
        }

        build_dag_recursive(ref_id, cells, dag, visited, dialect)?;
    }

    visited.pop();
    Ok(())
}

pub fn topological_sort(dag: &HashMap<String, Vec<String>>) -> Vec<String> {
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut queue: Vec<&str> = Vec::new();
    let mut result: Vec<String> = Vec::new();

    // Kahn's algorithm
    for node in dag.keys() {
        in_degree.entry(node.as_str()).or_insert(0);
        for dep in &dag[node] {
            *in_degree.entry(dep.as_str()).or_insert(0) += 1;
        }
    }

    for (node, &deg) in &in_degree {
        if deg == 0 {
            queue.push(node);
        }
    }

    while let Some(node) = queue.pop() {
        if dag.contains_key(node) {
            result.push(node.to_string());
            for dep in &dag[node] {
                let entry = in_degree.get_mut(dep.as_str()).unwrap();
                *entry -= 1;
                if *entry == 0 {
                    queue.push(dep);
                }
            }
        }
    }

    result.reverse(); // leaf cells first
    result
}

pub fn rewrite_sql(
    cell_id: &str,
    cells: &[CellModel],
    dialect: SqlDialect,
) -> Result<String, CellError> {
    let cells = resolve_exec_refs_in_cells(cells)?;
    let dag = build_dag(cell_id, &cells, dialect)?;

    let cell = cells
        .iter()
        .find(|c| c.id == cell_id)
        .ok_or_else(|| CellError::UnresolvedRef {
            cell_id: cell_id.into(),
            ref_name: cell_id.into(),
            hint: "cell not found in notebook".into(),
        })?;

    let order = topological_sort(&dag);
    let mut cte_defs = Vec::new();

    for dep_id in &order {
        if dep_id == cell_id {
            continue;
        }
        let dep_cell =
            cells
                .iter()
                .find(|c| &c.id == dep_id)
                .ok_or_else(|| CellError::UnresolvedRef {
                    cell_id: dep_id.clone(),
                    ref_name: dep_id.clone(),
                    hint: "referenced cell not found".into(),
                })?;

        let raw_sql = match &dep_cell.kind {
            CellKind::Ai => dep_cell
                .ai_state
                .as_ref()
                .and_then(|s| s.final_sql.clone())
                .unwrap_or_else(|| dep_cell.source.clone()),
            _ => dep_cell.source.clone(),
        };

        // Substitute first, then validate: a chained cell's raw source contains
        // `${id}` table names, which sqlparser rejects ("Expected: identifier");
        // the substituted `_cell_*` form is what actually executes.
        let inlined = substitute_refs(&raw_sql);
        let body = validate_referenceable(dep_id, &inlined, dialect)?;
        cte_defs.push(format!("_cell_{dep_id} AS (\n{body}\n)"));
    }

    let body = substitute_refs(&cell.source);
    Ok(compose(&body, &cte_defs, dialect))
}

/// Replaces `${id}` with the CTE name and `$id.col` with a scalar subquery.
fn substitute_refs(sql: &str) -> String {
    CELL_REF_RE
        .replace_all(sql, |caps: &regex::Captures| {
            if let Some(brace_id) = caps.get(1) {
                format!("_cell_{}", brace_id.as_str())
            } else {
                let id = caps.get(2).unwrap().as_str();
                let col = caps.get(3).unwrap().as_str();
                format!("(SELECT {col} FROM _cell_{id} LIMIT 1)")
            }
        })
        .to_string()
}

#[cfg(test)]
mod unit_tests {
    use lucent_protocol::SqlDialect;

    use super::*;

    const PG: SqlDialect = SqlDialect::PostgreSql;

    #[test]
    fn test_extract_refs_empty() {
        assert!(extract_refs("SELECT 1").is_empty());
    }

    #[test]
    fn test_topological_sort_empty() {
        let dag = HashMap::new();
        assert!(topological_sort(&dag).is_empty());
    }

    #[test]
    fn test_rewrite_no_deps() {
        let cells = vec![make_cell(
            "a1b2c3d4",
            CellKind::Sql,
            "SELECT 1",
            CellStatus::Ok,
        )];
        let result = rewrite_sql("a1b2c3d4", &cells, PG).unwrap();
        assert_eq!(result, "SELECT 1");
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

    fn make_cell_ord(
        id: &str,
        kind: CellKind,
        source: &str,
        status: CellStatus,
        order: Option<u32>,
    ) -> CellModel {
        CellModel {
            execution_order: order,
            ..make_cell(id, kind, source, status)
        }
    }

    #[test]
    fn rewrite_strips_trailing_semicolon_from_dependency() {
        let cells = vec![
            make_cell(
                "a1b2c3d4",
                CellKind::Sql,
                "SELECT 1 AS n LIMIT 1000;",
                CellStatus::Ok,
            ),
            make_cell(
                "e5f6a7b8",
                CellKind::Sql,
                "SELECT * FROM ${a1b2c3d4}",
                CellStatus::Ok,
            ),
        ];
        let out = rewrite_sql("e5f6a7b8", &cells, PG).unwrap();
        assert!(
            !out.contains(";"),
            "composed query must not contain a semicolon: {out}"
        );
        assert!(out.to_uppercase().contains("WITH"), "got {out}");
    }

    #[test]
    fn rewrite_supports_chained_references() {
        // Regression: a cell whose source references another cell via ${id} must
        // itself be referenceable. Its raw source contains `${id}` table names,
        // which sqlparser rejects — validation must run on the substituted form.
        let cells = vec![
            make_cell("a1b2c3d4", CellKind::Sql, "SELECT 1 AS n", CellStatus::Ok),
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
        ];
        let out = rewrite_sql("c3d4e5f6", &cells, PG).unwrap();
        assert!(out.contains("_cell_b2c3d4e5"), "got {out}");
        assert!(out.contains("_cell_a1b2c3d4"), "got {out}");
        let parsed = sqlparser::parser::Parser::parse_sql(
            crate::dialect::parser_for(SqlDialect::PostgreSql)
                .unwrap()
                .as_ref(),
            &out,
        )
        .unwrap_or_else(|e| panic!("composed SQL failed to parse ({e}): {out}"));
        assert_eq!(parsed.len(), 1, "expected exactly one statement, got {out}");
    }

    #[test]
    fn rewrite_rejects_multi_statement_dependency() {
        let cells = vec![
            make_cell(
                "a1b2c3d4",
                CellKind::Sql,
                "SELECT 1; SELECT 2",
                CellStatus::Ok,
            ),
            make_cell(
                "e5f6a7b8",
                CellKind::Sql,
                "SELECT * FROM ${a1b2c3d4}",
                CellStatus::Ok,
            ),
        ];
        let err = rewrite_sql("e5f6a7b8", &cells, PG).unwrap_err();
        assert!(matches!(err, CellError::NotATable { .. }), "got {err:?}");
    }

    #[test]
    fn rewrite_resolves_execution_order_reference() {
        let cells = vec![
            make_cell_ord(
                "a1b2c3d4",
                CellKind::Sql,
                "SELECT 1 AS n",
                CellStatus::Ok,
                Some(3),
            ),
            make_cell_ord(
                "e5f6a7b8",
                CellKind::Sql,
                "SELECT * FROM ${cell3}",
                CellStatus::Ok,
                None,
            ),
        ];
        let out = rewrite_sql("e5f6a7b8", &cells, PG).unwrap();
        assert!(out.contains("_cell_a1b2c3d4"), "got {out}");
    }

    #[test]
    fn rewrite_merges_into_user_with_containing_select_literal() {
        let cells = vec![
            make_cell("a1b2c3d4", CellKind::Sql, "SELECT 1 AS n", CellStatus::Ok),
            make_cell(
                "e5f6a7b8",
                CellKind::Sql,
                "WITH mine AS (SELECT 'select one' AS s) SELECT * FROM mine, ${a1b2c3d4}",
                CellStatus::Ok,
            ),
        ];
        let out = rewrite_sql("e5f6a7b8", &cells, PG).unwrap();
        assert_eq!(out.to_uppercase().matches("WITH ").count(), 1, "got {out}");
        assert!(out.contains("select one"), "got {out}");

        // The two assertions above survive even the old buggy string-offset splice
        // (`upper.find("SELECT ")` matches the SELECT inside the `'select one'`
        // literal's subquery and splits mid-CTE, producing invalid SQL that still
        // contains one "WITH " and the literal text). Parsing is the discriminator
        // that actually distinguishes a correct AST merge from that mangled output.
        let parsed = sqlparser::parser::Parser::parse_sql(
            crate::dialect::parser_for(SqlDialect::PostgreSql)
                .unwrap()
                .as_ref(),
            &out,
        )
        .unwrap_or_else(|e| panic!("composed SQL failed to parse ({e}): {out}"));
        assert_eq!(parsed.len(), 1, "expected exactly one statement, got {out}");
    }
}
