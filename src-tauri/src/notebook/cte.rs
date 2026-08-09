use lucent_protocol::SqlDialect;
use sqlparser::ast::Statement;
use sqlparser::parser::Parser;

use crate::notebook::types::CellError;
use crate::query_paging::{is_wrappable_query, normalize_sql_body};

/// A cell body may be referenced only if it is a single `SELECT`/`WITH`/`VALUES`
/// statement. This one predicate subsumes three checks that were previously
/// separate or missing: parseable, single-statement, and not DML/DDL.
pub fn validate_referenceable(
    cell_id: &str,
    sql: &str,
    dialect: SqlDialect,
) -> Result<String, CellError> {
    let body = normalize_sql_body(sql).to_string();
    if body.is_empty() {
        return Err(CellError::NotATable {
            cell_id: cell_id.into(),
            message: format!("cell '{cell_id}' is empty and cannot be referenced"),
        });
    }
    if !is_wrappable_query(&body, dialect) {
        return Err(CellError::NotATable {
            cell_id: cell_id.into(),
            message: format!(
                "cell '{cell_id}' is not a single SELECT statement and cannot be referenced"
            ),
        });
    }
    Ok(body)
}

/// Builds `Cte` nodes without ever constructing `sqlparser` AST structs by hand:
/// a throwaway `WITH ... SELECT 1` is parsed and its `with` clause lifted. Field
/// shapes change between sqlparser minor versions; parsing does not.
fn parse_scaffold_with(cte_defs: &[String], dialect: SqlDialect) -> Option<sqlparser::ast::With> {
    let scaffold = format!("WITH {} SELECT 1", cte_defs.join(", "));
    let parser = crate::dialect::parser_for(dialect)?;
    let mut ast = Parser::parse_sql(parser.as_ref(), &scaffold).ok()?;
    if ast.len() != 1 {
        return None;
    }
    match ast.remove(0) {
        Statement::Query(q) => q.with,
        _ => None,
    }
}

fn concat_fallback(body: &str, cte_defs: &[String]) -> String {
    format!("WITH {}\n{}", cte_defs.join(",\n  "), body)
}

pub fn compose(body: &str, cte_defs: &[String], dialect: SqlDialect) -> String {
    if cte_defs.is_empty() {
        return body.to_string();
    }

    let scaffold = match parse_scaffold_with(cte_defs, dialect) {
        Some(w) => w,
        None => {
            log::warn!("notebook cte: generated CTEs failed to parse; using concatenation");
            return concat_fallback(body, cte_defs);
        }
    };

    let Some(parser) = crate::dialect::parser_for(dialect) else {
        log::warn!("notebook cte: unsupported SQL dialect; using concatenation");
        return concat_fallback(body, cte_defs);
    };
    let mut ast = match Parser::parse_sql(parser.as_ref(), body) {
        Ok(a) if a.len() == 1 => a,
        Ok(_) => {
            log::warn!("notebook cte: body contains multiple statements; using concatenation");
            return concat_fallback(body, cte_defs);
        }
        Err(e) => {
            log::warn!("notebook cte: body failed to parse ({e}); using concatenation");
            return concat_fallback(body, cte_defs);
        }
    };

    match &mut ast[0] {
        Statement::Query(query) => {
            match query.with.as_mut() {
                // Prepend, so a user CTE may reference generated `_cell_*` names.
                Some(existing) => {
                    let generated = scaffold.cte_tables;
                    existing.cte_tables.splice(0..0, generated);
                }
                None => query.with = Some(scaffold),
            }
            ast[0].to_string()
        }
        _ => {
            log::warn!("notebook cte: body is not a Query statement; using concatenation");
            concat_fallback(body, cte_defs)
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use lucent_protocol::SqlDialect;

    use super::*;

    const PG: SqlDialect = SqlDialect::PostgreSql;

    fn norm(s: &str) -> String {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_uppercase()
    }

    #[test]
    fn validate_accepts_plain_select_and_strips_semicolon() {
        let body = validate_referenceable("a1b2c3d4", "SELECT * FROM t LIMIT 10;", PG).unwrap();
        assert_eq!(body, "SELECT * FROM t LIMIT 10");
    }

    #[test]
    fn validate_accepts_with_query() {
        let body = validate_referenceable("a1b2c3d4", "WITH x AS (SELECT 1) SELECT * FROM x;", PG)
            .unwrap();
        assert!(body.ends_with("SELECT * FROM x"));
    }

    #[test]
    fn validate_rejects_multi_statement() {
        let err = validate_referenceable("a1b2c3d4", "SELECT 1; SELECT 2", PG).unwrap_err();
        assert!(matches!(err, CellError::NotATable { .. }));
    }

    #[test]
    fn validate_rejects_dml() {
        let err = validate_referenceable("a1b2c3d4", "INSERT INTO t VALUES (1)", PG).unwrap_err();
        assert!(matches!(err, CellError::NotATable { .. }));
    }

    #[test]
    fn validate_allows_semicolon_inside_string_literal() {
        let body = validate_referenceable("a1b2c3d4", "SELECT ';' AS s", PG).unwrap();
        assert!(body.contains(';'));
    }

    #[test]
    fn compose_adds_with_clause_to_plain_select() {
        let out = compose(
            "SELECT * FROM _cell_a1b2c3d4",
            &["_cell_a1b2c3d4 AS (SELECT 1 AS n)".to_string()],
            PG,
        );
        let n = norm(&out);
        assert!(n.starts_with("WITH _CELL_A1B2C3D4 AS ("), "got {out}");
        assert!(n.contains("SELECT * FROM _CELL_A1B2C3D4"), "got {out}");
    }

    #[test]
    fn compose_merges_into_existing_user_with() {
        let out = compose(
            "WITH mine AS (SELECT 2) SELECT * FROM mine, _cell_a1b2c3d4",
            &["_cell_a1b2c3d4 AS (SELECT 1)".to_string()],
            PG,
        );
        let n = norm(&out);
        // Exactly one WITH keyword, and generated CTE precedes the user's.
        assert_eq!(n.matches("WITH ").count(), 1, "got {out}");
        let gen_at = n.find("_CELL_A1B2C3D4 AS").unwrap();
        let mine_at = n.find("MINE AS").unwrap();
        assert!(
            gen_at < mine_at,
            "generated CTE must precede user CTE: {out}"
        );
    }

    #[test]
    fn compose_is_unconfused_by_select_inside_string_literal() {
        // The old find("SELECT ") splice broke here.
        let out = compose(
            "WITH mine AS (SELECT 'select one' AS s) SELECT * FROM mine, _cell_a1b2c3d4",
            &["_cell_a1b2c3d4 AS (SELECT 1)".to_string()],
            PG,
        );
        assert_eq!(norm(&out).matches("WITH ").count(), 1, "got {out}");
        assert!(out.contains("select one"), "literal must survive: {out}");
    }

    #[test]
    fn compose_is_unconfused_by_nested_subquery_select() {
        let out = compose(
            "WITH mine AS (SELECT * FROM (SELECT 1 AS n) inner_q) SELECT * FROM mine",
            &["_cell_a1b2c3d4 AS (SELECT 1)".to_string()],
            PG,
        );
        assert_eq!(norm(&out).matches("WITH ").count(), 1, "got {out}");
    }

    #[test]
    fn compose_preserves_recursive_flag() {
        let out = compose(
            "WITH RECURSIVE r AS (SELECT 1 AS n) SELECT * FROM r",
            &["_cell_a1b2c3d4 AS (SELECT 1)".to_string()],
            PG,
        );
        assert!(norm(&out).contains("WITH RECURSIVE"), "got {out}");
    }

    #[test]
    fn compose_with_no_ctes_returns_body_unchanged() {
        assert_eq!(compose("SELECT 1", &[], PG), "SELECT 1");
    }

    #[test]
    fn compose_falls_back_when_body_is_unparseable() {
        // Use genuinely unparseable SQL (not valid for sqlparser under the
        // PostgreSQL dialect).
        // concat_fallback preserves body verbatim; AST path would normalize it via to_string().
        let body = "SELECT * FROM (";
        let out = compose(body, &["_cell_a1b2c3d4 AS (SELECT 1)".to_string()], PG);
        // Verify fallback format (WITH <cte>\n<body>): CTE present AND body preserved verbatim.
        assert!(out.contains("_cell_a1b2c3d4 AS"), "got {out}");
        assert!(
            out.ends_with(body),
            "fallback must preserve body verbatim; got {out}"
        );
    }
}
