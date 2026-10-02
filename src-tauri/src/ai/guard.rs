use lucent_protocol::SqlDialect;
use sqlparser::ast::{
    Expr, FromTable, FunctionArg, FunctionArgExpr, FunctionArguments, JoinConstraint, JoinOperator,
    ObjectName, Query, SelectItem, SetExpr, Statement, TableFactor, TableWithJoins, UtilityOption,
};
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
    #[error("Forbidden function or replacement scan: {0}")]
    ForbiddenFunction(String),
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
        Statement::Explain {
            analyze, options, ..
        } if !analyze && !explain_options_contain_analyze(options) => Ok(()),
        Statement::Explain { .. } => Err(GuardError::ExplainAnalyze),
        _ => Err(GuardError::NotReadOnly),
    }
}

/// Recursively verify a `Query` — including its CTEs and set-operation branches —
/// contains no data-modifying statements.
fn check_query_readonly(q: &Query) -> Result<(), GuardError> {
    if let Some(with) = &q.with {
        for cte in &with.cte_tables {
            check_query_readonly(&cte.query)?;
        }
    }
    check_setexpr_readonly(&q.body)
}

fn explain_options_contain_analyze(options: &Option<Vec<UtilityOption>>) -> bool {
    options
        .as_ref()
        .map(|opts| {
            opts.iter()
                .any(|o| o.name.value.eq_ignore_ascii_case("analyze"))
        })
        .unwrap_or(false)
}

const FORBIDDEN_FUNCTIONS: &[&str] = &[
    "read_text",
    "read_csv",
    "read_parquet",
    "read_json",
    "read_blob",
    "glob",
    "scan_parquet",
    "scan_csv",
    "httpfs",
    "iceberg_scan",
    "delta_scan",
    "write_csv",
    "copy_to",
    "pg_read_file",
    "pg_read_binary_file",
    "pg_ls_dir",
    "pg_stat_file",
    "dblink",
    "dblink_exec",
];

fn is_forbidden_func(name: &str) -> bool {
    FORBIDDEN_FUNCTIONS.contains(&name)
}

fn check_table_name(name: &ObjectName) -> Result<(), GuardError> {
    let name_str = name.to_string();
    let last_str = name
        .0
        .last()
        .map(|p| match p.as_ident() {
            Some(id) => id.value.clone(),
            None => p.to_string(),
        })
        .unwrap_or_default();
    let last_lower = last_str.to_ascii_lowercase();

    if is_forbidden_func(&last_lower) {
        return Err(GuardError::ForbiddenFunction(name_str));
    }

    let cleaned = last_str.trim_matches('\'').trim_matches('"');
    let lower_cleaned = cleaned.to_ascii_lowercase();
    if lower_cleaned.ends_with(".csv")
        || lower_cleaned.ends_with(".parquet")
        || lower_cleaned.ends_with(".json")
        || lower_cleaned.ends_with(".env")
        || lower_cleaned.ends_with(".txt")
        || cleaned.contains('/')
        || cleaned.contains('\\')
        || name_str.contains('/')
        || name_str.contains('\\')
    {
        return Err(GuardError::ForbiddenFunction(name_str));
    }

    Ok(())
}

fn check_function_arg_expr(arg_expr: &FunctionArgExpr) -> Result<(), GuardError> {
    match arg_expr {
        FunctionArgExpr::Expr(e) => check_expr(e),
        _ => Ok(()),
    }
}

fn check_function_arg(arg: &FunctionArg) -> Result<(), GuardError> {
    match arg {
        FunctionArg::Named { arg, .. } => check_function_arg_expr(arg),
        FunctionArg::ExprNamed { name, arg, .. } => {
            check_expr(name)?;
            check_function_arg_expr(arg)
        }
        FunctionArg::Unnamed(arg) => check_function_arg_expr(arg),
    }
}

fn check_expr(expr: &Expr) -> Result<(), GuardError> {
    match expr {
        Expr::Function(func) => {
            let func_name = func.name.to_string();
            let last_str = func
                .name
                .0
                .last()
                .map(|p| match p.as_ident() {
                    Some(id) => id.value.clone(),
                    None => p.to_string(),
                })
                .unwrap_or_default();
            let last_lower = last_str.to_ascii_lowercase();
            if is_forbidden_func(&last_lower) {
                return Err(GuardError::ForbiddenFunction(func_name));
            }
            match &func.args {
                FunctionArguments::List(arg_list) => {
                    for arg in &arg_list.args {
                        check_function_arg(arg)?;
                    }
                }
                FunctionArguments::Subquery(subquery) => {
                    check_query_readonly(subquery)?;
                }
                _ => {}
            }
            Ok(())
        }
        Expr::Subquery(subquery) => check_query_readonly(subquery),
        Expr::InSubquery { subquery, expr, .. } => {
            check_expr(expr)?;
            check_query_readonly(subquery)
        }
        Expr::Exists { subquery, .. } => check_query_readonly(subquery),
        Expr::BinaryOp { left, right, .. } => {
            check_expr(left)?;
            check_expr(right)
        }
        Expr::UnaryOp { expr, .. } => check_expr(expr),
        Expr::Nested(e) => check_expr(e),
        Expr::Case {
            operand,
            conditions,
            else_result,
            ..
        } => {
            if let Some(op) = operand {
                check_expr(op)?;
            }
            for c in conditions {
                check_expr(&c.condition)?;
                check_expr(&c.result)?;
            }
            if let Some(el) = else_result {
                check_expr(el)?;
            }
            Ok(())
        }
        Expr::Cast { expr, .. } => check_expr(expr),
        Expr::InList { expr, list, .. } => {
            check_expr(expr)?;
            for item in list {
                check_expr(item)?;
            }
            Ok(())
        }
        Expr::Between {
            expr,
            low,
            high,
            ..
        } => {
            check_expr(expr)?;
            check_expr(low)?;
            check_expr(high)
        }
        Expr::IsNull(e)
        | Expr::IsNotNull(e)
        | Expr::IsTrue(e)
        | Expr::IsNotTrue(e)
        | Expr::IsFalse(e)
        | Expr::IsNotFalse(e)
        | Expr::IsUnknown(e)
        | Expr::IsNotUnknown(e) => check_expr(e),
        Expr::Tuple(exprs) => {
            for e in exprs {
                check_expr(e)?;
            }
            Ok(())
        }
        Expr::Array(arr) => {
            for e in &arr.elem {
                check_expr(e)?;
            }
            Ok(())
        }
        Expr::Like { expr, pattern, .. }
        | Expr::ILike { expr, pattern, .. }
        | Expr::SimilarTo { expr, pattern, .. } => {
            check_expr(expr)?;
            check_expr(pattern)
        }
        _ => Ok(()),
    }
}

fn check_table_factor(tf: &TableFactor) -> Result<(), GuardError> {
    match tf {
        TableFactor::Table { name, args, .. } => {
            check_table_name(name)?;
            if let Some(targs) = args {
                for arg in &targs.args {
                    check_function_arg(arg)?;
                }
            }
            Ok(())
        }
        TableFactor::Function { name, args, .. } => {
            check_table_name(name)?;
            for arg in args {
                check_function_arg(arg)?;
            }
            Ok(())
        }
        TableFactor::TableFunction { expr, .. } => check_expr(expr),
        TableFactor::Derived { subquery, .. } => check_query_readonly(subquery),
        TableFactor::NestedJoin {
            table_with_joins, ..
        } => check_table_with_joins(table_with_joins),
        TableFactor::Pivot { table, .. } | TableFactor::Unpivot { table, .. } => {
            check_table_factor(table)
        }
        _ => Ok(()),
    }
}

fn check_table_with_joins(twj: &TableWithJoins) -> Result<(), GuardError> {
    check_table_factor(&twj.relation)?;
    for join in &twj.joins {
        check_table_factor(&join.relation)?;
        match &join.join_operator {
            JoinOperator::Join(c)
            | JoinOperator::Inner(c)
            | JoinOperator::Left(c)
            | JoinOperator::LeftOuter(c)
            | JoinOperator::Right(c)
            | JoinOperator::RightOuter(c)
            | JoinOperator::FullOuter(c)
            | JoinOperator::CrossJoin(c)
            | JoinOperator::Semi(c)
            | JoinOperator::LeftSemi(c)
            | JoinOperator::RightSemi(c)
            | JoinOperator::Anti(c)
            | JoinOperator::LeftAnti(c)
            | JoinOperator::RightAnti(c) => match c {
                JoinConstraint::On(expr) => check_expr(expr)?,
                _ => {}
            },
            _ => {}
        }
    }
    Ok(())
}

fn check_setexpr_readonly(body: &SetExpr) -> Result<(), GuardError> {
    match body {
        SetExpr::Select(sel) => {
            if sel.into.is_some() {
                // `SELECT … INTO table` creates a table — a write, even
                // though the statement parses as a Select (C4).
                return Err(GuardError::NotReadOnly);
            }
            for twj in &sel.from {
                check_table_with_joins(twj)?;
            }
            for item in &sel.projection {
                match item {
                    SelectItem::UnnamedExpr(e) | SelectItem::ExprWithAlias { expr: e, .. } => {
                        check_expr(e)?;
                    }
                    _ => {}
                }
            }
            if let Some(selection) = &sel.selection {
                check_expr(selection)?;
            }
            if let Some(having) = &sel.having {
                check_expr(having)?;
            }
            Ok(())
        }
        SetExpr::Values(_) | SetExpr::Table(_) => Ok(()),
        SetExpr::Query(q) => check_query_readonly(q),
        SetExpr::SetOperation { left, right, .. } => {
            check_setexpr_readonly(left)?;
            check_setexpr_readonly(right)
        }
        // DML inside a query body (writing CTEs) — reject. Anything
        // unrecognised is rejected too: a read-only guard must fail closed.
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
    fn rejects_select_into_which_creates_a_table() {
        // C4: `SELECT … INTO` parses as a Select with `into: Some(..)` — it
        // creates a table, a write. The old guard matched `SetExpr::Select(_)`
        // without inspecting `into` and let it through; on Postgres only the
        // engine layer (BEGIN READ ONLY) caught it, and a GuardOnly driver had
        // no defense at all.
        assert!(matches!(
            validate_readonly("SELECT * INTO copied_users FROM users", PG).unwrap_err(),
            GuardError::NotReadOnly
        ));
    }

    #[test]
    fn rejects_explain_with_analyze_in_parenthesized_options() {
        // C4: `EXPLAIN (ANALYZE true) …` actually EXECUTES the statement.
        // sqlparser 0.62 stores ANALYZE in the parenthesized `options` list and
        // leaves the `analyze` field false, so the field-only check let
        // `EXPLAIN (ANALYZE true) DELETE FROM t` through. Verified against the
        // vendored parser: `parse_explain` routes `(…)` to
        // `parse_utility_options`.
        assert!(matches!(
            validate_readonly("EXPLAIN (ANALYZE true) DELETE FROM t", PG).unwrap_err(),
            GuardError::ExplainAnalyze
        ));
        // Bare `EXPLAIN (ANALYZE)` is the same as ANALYZE true.
        assert!(matches!(
            validate_readonly("EXPLAIN (ANALYZE) SELECT 1", PG).unwrap_err(),
            GuardError::ExplainAnalyze
        ));
    }

    #[test]
    fn explain_without_analyze_still_passes() {
        // Guard against over-blocking: plain EXPLAIN and non-ANALYZE options
        // plan but never execute.
        assert!(validate_readonly("EXPLAIN SELECT * FROM users", PG).is_ok());
        assert!(validate_readonly("EXPLAIN (COSTS false) SELECT 1", PG).is_ok());
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

    #[test]
    fn guard_rejects_filesystem_and_exfiltration_functions_and_replacement_scans() {
        let forbidden_queries = [
            "SELECT * FROM read_text('/etc/passwd')",
            "SELECT * FROM main.read_text('/etc/passwd')",
            "SELECT * FROM duckdb.read_csv('data.csv')",
            "SELECT * FROM 'data.csv'",
            "SELECT * FROM '/etc/hosts'",
            "SELECT count(*) FROM glob('/*')",
            "SELECT * FROM pg_read_file('config.json')",
            "SELECT * FROM pg_ls_dir('/tmp')",
        ];

        for q in forbidden_queries {
            let err = validate_readonly(q, SqlDialect::DuckDb).unwrap_err();
            assert!(
                matches!(err, GuardError::ForbiddenFunction(_)),
                "query {q} should fail with ForbiddenFunction, got: {err:?}"
            );
        }
    }
}
