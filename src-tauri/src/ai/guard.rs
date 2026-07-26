use sqlparser::ast::{FromTable, Query, SetExpr, Statement};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GuardError {
    #[error("SQL parse error: {0}")]
    Parse(String),
    #[error("Only SELECT, WITH, VALUES, EXPLAIN (without ANALYZE) are allowed")]
    NotReadOnly,
    #[error("EXPLAIN ANALYZE executes the underlying statement — not allowed")]
    ExplainAnalyze,
    #[error("Multi-statement SQL is not allowed")]
    MultiStatement,
}

/// Layer 1 of read-only enforcement: syntactic AST check.
/// Layer 2 (READ ONLY transaction wrap) is applied at the DB driver level.
pub fn validate_readonly(sql: &str) -> Result<(), GuardError> {
    let dialect = PostgreSqlDialect {};
    let statements =
        Parser::parse_sql(&dialect, sql).map_err(|e| GuardError::Parse(e.to_string()))?;

    if statements.is_empty() {
        return Err(GuardError::Parse("Empty SQL".into()));
    }
    if statements.len() > 1 {
        return Err(GuardError::MultiStatement);
    }

    match &statements[0] {
        Statement::Query(q) => check_query_readonly(q),
        Statement::Explain { analyze, .. } if !analyze => Ok(()),
        Statement::Explain { analyze: true, .. } => Err(GuardError::ExplainAnalyze),
        _ => Err(GuardError::NotReadOnly),
    }
}

/// Recursively verify a `Query` — including its CTEs and set-operation branches —
/// contains no data-modifying statements.
///
/// A writing CTE such as `WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x`
/// parses as `Statement::Query`, but the CTE's body is a DML `SetExpr`
/// (`Insert`/`Update`/`Delete`). A flat `matches!(stmt, Statement::Query(_))`
/// check is one level too shallow and would let such statements through.
fn check_query_readonly(q: &Query) -> Result<(), GuardError> {
    if let Some(with) = &q.with {
        for cte in &with.cte_tables {
            check_query_readonly(&cte.query)?;
        }
    }
    check_setexpr_readonly(&q.body)
}

fn check_setexpr_readonly(body: &SetExpr) -> Result<(), GuardError> {
    match body {
        SetExpr::Select(_) | SetExpr::Values(_) | SetExpr::Table(_) => Ok(()),
        SetExpr::Query(q) => check_query_readonly(q),
        SetExpr::SetOperation { left, right, .. } => {
            check_setexpr_readonly(left)?;
            check_setexpr_readonly(right)
        }
        // DML inside a query body (writing CTEs) — reject. Anything unrecognised
        // is rejected too: a read-only guard must fail closed.
        _ => Err(GuardError::NotReadOnly),
    }
}

/// Extract the WHERE clause from a DML statement as a string.
/// Returns None for INSERT, or DELETE/UPDATE without WHERE.
pub fn extract_where_for_count(sql: &str) -> Option<String> {
    let mut stmts = Parser::parse_sql(&PostgreSqlDialect {}, sql).ok()?;
    match stmts.pop()? {
        Statement::Delete(d) => d.selection.as_ref().map(|e| e.to_string()),
        Statement::Update(u) => u.selection.as_ref().map(|e| e.to_string()),
        _ => None,
    }
}

/// Extract the primary table name from a DML statement.
pub fn extract_table_name(sql: &str) -> Option<String> {
    let mut stmts = Parser::parse_sql(&PostgreSqlDialect {}, sql).ok()?;
    match stmts.pop()? {
        Statement::Delete(d) => match d.from {
            FromTable::WithFromKeyword(tables) | FromTable::WithoutKeyword(tables) => {
                tables.first().map(|t| t.relation.to_string())
            }
        },
        Statement::Update(u) => Some(u.table.relation.to_string()),
        Statement::Insert(i) => Some(i.table.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_insert() {
        assert!(matches!(
            validate_readonly("INSERT INTO users VALUES (1)").unwrap_err(),
            GuardError::NotReadOnly
        ));
    }

    #[test]
    fn rejects_update() {
        assert!(matches!(
            validate_readonly("UPDATE users SET name = 'x'").unwrap_err(),
            GuardError::NotReadOnly
        ));
    }

    #[test]
    fn rejects_delete() {
        assert!(matches!(
            validate_readonly("DELETE FROM users").unwrap_err(),
            GuardError::NotReadOnly
        ));
    }

    #[test]
    fn rejects_explain_analyze() {
        assert!(matches!(
            validate_readonly("EXPLAIN ANALYZE SELECT 1").unwrap_err(),
            GuardError::ExplainAnalyze
        ));
    }

    #[test]
    fn rejects_multi_statement() {
        assert!(matches!(
            validate_readonly("SELECT 1; DELETE FROM users").unwrap_err(),
            GuardError::MultiStatement
        ));
    }

    #[test]
    fn accepts_select() {
        assert!(validate_readonly("SELECT * FROM users").is_ok());
    }

    #[test]
    fn accepts_cte() {
        assert!(validate_readonly("WITH r AS (SELECT * FROM o) SELECT * FROM r").is_ok());
    }

    #[test]
    fn rejects_cte_dml() {
        assert!(matches!(
            validate_readonly("WITH x AS (DELETE FROM users RETURNING *) SELECT * FROM x")
                .unwrap_err(),
            GuardError::NotReadOnly
        ));
    }

    #[test]
    fn rejects_cte_update() {
        assert!(
            validate_readonly("WITH b AS (UPDATE t SET x = 1 RETURNING *) SELECT * FROM b")
                .is_err()
        );
    }

    #[test]
    fn rejects_cte_insert() {
        assert!(validate_readonly(
            "WITH b AS (INSERT INTO t (n) VALUES (1) RETURNING *) SELECT * FROM b"
        )
        .is_err());
    }

    #[test]
    fn rejects_nested_cte_dml() {
        assert!(validate_readonly(
            "WITH a AS (SELECT 1), b AS (DELETE FROM t RETURNING *) SELECT * FROM b"
        )
        .is_err());
    }

    #[test]
    fn accepts_readonly_cte_still() {
        assert!(validate_readonly("WITH r AS (SELECT * FROM o) SELECT * FROM r").is_ok());
    }

    #[test]
    fn accepts_explain_no_analyze() {
        assert!(validate_readonly("EXPLAIN SELECT * FROM users").is_ok());
    }

    #[test]
    fn extract_where_delete() {
        let c = extract_where_for_count("DELETE FROM orders WHERE status = 'cancelled'").unwrap();
        assert!(c.contains("status") && c.contains("cancelled"));
    }

    #[test]
    fn extract_where_update() {
        let c = extract_where_for_count(
            "UPDATE users SET active=false WHERE last_login < '2020-01-01'",
        )
        .unwrap();
        assert!(c.contains("last_login"));
    }

    #[test]
    fn extract_where_insert_none() {
        assert!(extract_where_for_count("INSERT INTO t (n) VALUES ('x')").is_none());
    }

    #[test]
    fn extract_where_delete_no_where_none() {
        assert!(extract_where_for_count("DELETE FROM users").is_none());
    }

    #[test]
    fn extract_table_delete() {
        assert_eq!(
            extract_table_name("DELETE FROM orders WHERE id=1").as_deref(),
            Some("orders")
        );
    }
}
