use regex::Regex;
use std::sync::LazyLock;

use crate::notebook::types::{CellError, CellModel};

/// Execution-order references. The literal `cell` prefix cannot collide with a
/// stable cell id (ids are `[a-f0-9]{8}` and `l` is not a hex digit), nor with
/// PostgreSQL placeholders (`$1`) or dollar-quoted strings (`$$...$$`).
static EXEC_REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\$\{cell(\d+)\}|\$cell(\d+)\.([a-z_][a-z0-9_]*)").unwrap());

fn id_for_order(order: u32, cells: &[CellModel]) -> Option<&str> {
    cells
        .iter()
        .find(|c| c.execution_order == Some(order))
        .map(|c| c.id.as_str())
}

pub fn resolve_exec_refs(source: &str, cells: &[CellModel]) -> Result<String, CellError> {
    let mut failure: Option<CellError> = None;

    let out = EXEC_REF_RE
        .replace_all(source, |caps: &regex::Captures| {
            let (order_str, column) = match caps.get(1) {
                Some(m) => (m.as_str(), None),
                None => (
                    caps.get(2).unwrap().as_str(),
                    Some(caps.get(3).unwrap().as_str()),
                ),
            };
            let order: u32 = match order_str.parse() {
                Ok(n) => n,
                Err(_) => return caps.get(0).unwrap().as_str().to_string(),
            };
            match id_for_order(order, cells) {
                Some(id) => match column {
                    None => format!("${{{id}}}"),
                    Some(col) => format!("${id}.{col}"),
                },
                None => {
                    if failure.is_none() {
                        failure = Some(CellError::UnresolvedRef {
                            cell_id: format!("cell{order}"),
                            ref_name: format!("cell{order}"),
                            hint: format!(
                                "no cell has execution order {order} — run that cell first"
                            ),
                        });
                    }
                    caps.get(0).unwrap().as_str().to_string()
                }
            }
        })
        .to_string();

    match failure {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

pub fn resolve_exec_refs_in_cells(cells: &[CellModel]) -> Result<Vec<CellModel>, CellError> {
    cells
        .iter()
        .map(|c| {
            let source = resolve_exec_refs(&c.source, cells)?;
            Ok(CellModel {
                source,
                ..c.clone()
            })
        })
        .collect()
}

#[cfg(test)]
mod unit_tests {
    use super::{resolve_exec_refs, resolve_exec_refs_in_cells};
    use crate::notebook::types::*;

    fn cell(id: &str, source: &str, order: Option<u32>) -> CellModel {
        CellModel {
            id: id.into(),
            kind: CellKind::Sql,
            source: source.into(),
            alias: None,
            collapsed: false,
            outputs: None,
            status: CellStatus::Ok,
            execution_order: order,
            duration_ms: None,
            error: None,
            stale_since: None,
            ai_state: None,
        }
    }

    #[test]
    fn resolves_table_ref_by_execution_order() {
        let cells = vec![
            cell("a1b2c3d4", "SELECT 1", Some(3)),
            cell("e5f6a7b8", "SELECT * FROM ${cell3}", None),
        ];
        let out = resolve_exec_refs("SELECT * FROM ${cell3}", &cells).unwrap();
        assert_eq!(out, "SELECT * FROM ${a1b2c3d4}");
    }

    #[test]
    fn resolves_column_ref_by_execution_order() {
        let cells = vec![cell("a1b2c3d4", "SELECT 1", Some(7))];
        let out = resolve_exec_refs("WHERE x = $cell7.region", &cells).unwrap();
        assert_eq!(out, "WHERE x = $a1b2c3d4.region");
    }

    #[test]
    fn leaves_stable_id_refs_untouched() {
        let cells = vec![cell("a1b2c3d4", "SELECT 1", Some(1))];
        let src = "SELECT * FROM ${a1b2c3d4} WHERE y = $a1b2c3d4.col";
        assert_eq!(resolve_exec_refs(src, &cells).unwrap(), src);
    }

    #[test]
    fn is_idempotent() {
        let cells = vec![cell("a1b2c3d4", "SELECT 1", Some(3))];
        let once = resolve_exec_refs("SELECT * FROM ${cell3}", &cells).unwrap();
        let twice = resolve_exec_refs(&once, &cells).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn unknown_execution_order_is_unresolved_ref() {
        let cells = vec![cell("a1b2c3d4", "SELECT 1", Some(1))];
        let err = resolve_exec_refs("SELECT * FROM ${cell99}", &cells).unwrap_err();
        match err {
            CellError::UnresolvedRef { ref_name, .. } => assert_eq!(ref_name, "cell99"),
            other => panic!("expected UnresolvedRef, got {other:?}"),
        }
    }

    #[test]
    fn does_not_match_postgres_placeholders_or_dollar_quotes() {
        let cells = vec![cell("a1b2c3d4", "SELECT 1", Some(1))];
        let src = "SELECT $1, $$body$$, $user FROM t";
        assert_eq!(resolve_exec_refs(src, &cells).unwrap(), src);
    }

    #[test]
    fn resolves_every_cell_source_in_place() {
        let cells = vec![
            cell("a1b2c3d4", "SELECT 1", Some(2)),
            cell("e5f6a7b8", "SELECT * FROM ${cell2}", None),
        ];
        let out = resolve_exec_refs_in_cells(&cells).unwrap();
        assert_eq!(out[1].source, "SELECT * FROM ${a1b2c3d4}");
        assert_eq!(out[0].source, "SELECT 1");
    }
}
