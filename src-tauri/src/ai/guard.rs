use lucent_protocol::SqlDialect;
use sqlparser::ast::{FromTable, Query, SetExpr, Statement};
use sqlparser::dialect::Dialect;
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
    #[error(
        "This build cannot parse the connection's SQL dialect, so it cannot \
             prove the statement is read-only"
    )]
    UnknownDialect,
}

/// Layer 1 of read-only enforcement: syntactic AST check.
///
/// Layer 2 (an engine-enforced read-only transaction) is applied by
/// `crate::readonly` **only when the driver's capabilities support it**. When
/// they do not, this function is the only protection there is.
pub fn validate_readonly(sql: &str, dialect: SqlDialect) -> Result<(), GuardError> {
    validate_readonly_with_parser(sql, crate::dialect::parser_for(dialect))
}

/// Split out so the fail-closed path is directly testable without inventing a
/// protocol variant this build does not have.
pub(crate) fn validate_readonly_with_parser(
    sql: &str,
    dialect: Option<Box<dyn Dialect>>,
) -> Result<(), GuardError> {
    let Some(dialect) = dialect else {
        return Err(GuardError::UnknownDialect);
    };

    let statements =
        Parser::parse_sql(dialect.as_ref(), sql).map_err(|e| GuardError::Parse(e.to_string()))?;

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
/// Returns None for INSERT, for DELETE/UPDATE without WHERE, and for a dialect
/// this build cannot parse.
pub fn extract_where_for_count(sql: &str, dialect: SqlDialect) -> Option<String> {
    let dialect = crate::dialect::parser_for(dialect)?;
    let mut stmts = Parser::parse_sql(dialect.as_ref(), sql).ok()?;
    match stmts.pop()? {
        Statement::Delete(d) => d.selection.as_ref().map(|e| e.to_string()),
        Statement::Update(u) => u.selection.as_ref().map(|e| e.to_string()),
        _ => None,
    }
}

/// Extract the primary table name from a DML statement.
pub fn extract_table_name(sql: &str, dialect: SqlDialect) -> Option<String> {
    let dialect = crate::dialect::parser_for(dialect)?;
    let mut stmts = Parser::parse_sql(dialect.as_ref(), sql).ok()?;
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
    use lucent_protocol::SqlDialect;

    const PG: SqlDialect = SqlDialect::PostgreSql;

    #[test]
    fn an_unresolvable_dialect_rejects_the_sql_rather_than_guessing() {
        // Fail closed. A guard that falls back to a permissive parser when it
        // does not recognise the dialect is not a guard.
        //
        // `SqlDialect` is #[non_exhaustive], so this simulates a driver on a
        // newer protocol declaring a dialect this build cannot parse.
        let err = validate_readonly_with_parser("SELECT 1", None).unwrap_err();
        assert!(matches!(err, GuardError::UnknownDialect));
    }

    #[test]
    fn duckdb_sql_is_validated_with_the_duckdb_parser() {
        // DuckDB's SELECT * EXCLUDE (...) is not Postgres syntax. Under the
        // Postgres parser this fails to parse and is rejected as a parse
        // error — which would make a legal read-only query unrunnable.
        let sql = "SELECT * EXCLUDE (secret) FROM users";
        assert!(
            validate_readonly(sql, SqlDialect::DuckDb).is_ok(),
            "a valid DuckDB SELECT must pass the guard"
        );
    }

    #[test]
    fn the_guard_still_rejects_dml_under_every_dialect() {
        for d in [SqlDialect::PostgreSql, SqlDialect::DuckDb] {
            assert!(
                validate_readonly("DELETE FROM users", d).is_err(),
                "DML must be rejected under {d:?}"
            );
            assert!(
                validate_readonly("WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x", d)
                    .is_err(),
                "writing CTEs must be rejected under {d:?}"
            );
        }
    }

    #[test]
    fn rejects_insert() {
        assert!(matches!(
            validate_readonly("INSERT INTO users VALUES (1)", PG).unwrap_err(),
            GuardError::NotReadOnly
        ));
    }

    #[test]
    fn rejects_update() {
        assert!(matches!(
            validate_readonly("UPDATE users SET name = 'x'", PG).unwrap_err(),
            GuardError::NotReadOnly
        ));
    }

    #[test]
    fn rejects_delete() {
        assert!(matches!(
            validate_readonly("DELETE FROM users", PG).unwrap_err(),
            GuardError::NotReadOnly
        ));
    }

    #[test]
    fn rejects_explain_analyze() {
        assert!(matches!(
            validate_readonly("EXPLAIN ANALYZE SELECT 1", PG).unwrap_err(),
            GuardError::ExplainAnalyze
        ));
    }

    #[test]
    fn rejects_multi_statement() {
        assert!(matches!(
            validate_readonly("SELECT 1; DELETE FROM users", PG).unwrap_err(),
            GuardError::MultiStatement
        ));
    }

    #[test]
    fn accepts_select() {
        assert!(validate_readonly("SELECT * FROM users", PG).is_ok());
    }

    #[test]
    fn accepts_cte() {
        assert!(validate_readonly("WITH r AS (SELECT * FROM o) SELECT * FROM r", PG).is_ok());
    }

    #[test]
    fn rejects_cte_dml() {
        assert!(matches!(
            validate_readonly(
                "WITH x AS (DELETE FROM users RETURNING *) SELECT * FROM x",
                PG
            )
            .unwrap_err(),
            GuardError::NotReadOnly
        ));
    }

    #[test]
    fn rejects_cte_update() {
        assert!(validate_readonly(
            "WITH b AS (UPDATE t SET x = 1 RETURNING *) SELECT * FROM b",
            PG
        )
        .is_err());
    }

    #[test]
    fn rejects_cte_insert() {
        assert!(validate_readonly(
            "WITH b AS (INSERT INTO t (n) VALUES (1) RETURNING *) SELECT * FROM b",
            PG
        )
        .is_err());
    }

    #[test]
    fn rejects_nested_cte_dml() {
        assert!(validate_readonly(
            "WITH a AS (SELECT 1), b AS (DELETE FROM t RETURNING *) SELECT * FROM b",
            PG
        )
        .is_err());
    }

    #[test]
    fn accepts_readonly_cte_still() {
        assert!(validate_readonly("WITH r AS (SELECT * FROM o) SELECT * FROM r", PG).is_ok());
    }

    #[test]
    fn accepts_explain_no_analyze() {
        assert!(validate_readonly("EXPLAIN SELECT * FROM users", PG).is_ok());
    }

    #[test]
    fn extract_where_delete() {
        let c =
            extract_where_for_count("DELETE FROM orders WHERE status = 'cancelled'", PG).unwrap();
        assert!(c.contains("status") && c.contains("cancelled"));
    }

    #[test]
    fn extract_where_update() {
        let c = extract_where_for_count(
            "UPDATE users SET active=false WHERE last_login < '2020-01-01'",
            PG,
        )
        .unwrap();
        assert!(c.contains("last_login"));
    }

    #[test]
    fn extract_where_insert_none() {
        assert!(extract_where_for_count("INSERT INTO t (n) VALUES ('x')", PG).is_none());
    }

    #[test]
    fn extract_where_delete_no_where_none() {
        assert!(extract_where_for_count("DELETE FROM users", PG).is_none());
    }

    #[test]
    fn extract_table_delete() {
        assert_eq!(
            extract_table_name("DELETE FROM orders WHERE id=1", PG).as_deref(),
            Some("orders")
        );
    }
}
