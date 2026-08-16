use crate::sql_builder::SqlBuilder;
use lucent_protocol::SqlDialect;
use serde::Deserialize;
use sqlparser::ast::{Query, SetExpr, Statement};
use sqlparser::dialect::Dialect;
use sqlparser::parser::Parser;

/// Whether `sql` is a single `SELECT`/`WITH`/`VALUES`-shaped statement that
/// can be safely wrapped as `SELECT * FROM (<sql>) AS _lucent_page ...`.
///
/// Multi-statement input, non-query statements, and SQL this build cannot parse
/// all return false — those execute unwrapped, unpaginated, exactly as today.
pub fn is_wrappable_query(sql: &str, dialect: SqlDialect) -> bool {
    is_wrappable_with_parser(sql, crate::dialect::parser_for(dialect))
}

pub(crate) fn is_wrappable_with_parser(sql: &str, dialect: Option<Box<dyn Dialect>>) -> bool {
    let Some(dialect) = dialect else {
        return false;
    };
    match Parser::parse_sql(dialect.as_ref(), sql) {
        Ok(statements) if statements.len() == 1 => {
            matches!(statements[0], Statement::Query(ref q) if query_is_wrappable(q))
        }
        _ => false,
    }
}

/// A query is wrappable only if PostgreSQL can legally wrap it as a
/// subquery: no row-locking clauses (`Query.locks`) and no `SELECT … INTO`
/// (creates a table; `Select.into`). Both parse as plain `Statement::Query`
/// but the server rejects them inside a subquery — wrapping them turned a
/// valid query into a syntax error, silently disabling paging for those
/// shapes (C6).
fn query_is_wrappable(q: &Query) -> bool {
    if !q.locks.is_empty() {
        return false;
    }
    !setexpr_has_into(&q.body)
}

fn setexpr_has_into(body: &SetExpr) -> bool {
    match body {
        SetExpr::Select(sel) => sel.into.is_some(),
        SetExpr::Query(q) => query_is_wrappable(q),
        SetExpr::SetOperation { left, right, .. } => {
            setexpr_has_into(left) || setexpr_has_into(right)
        }
        _ => false,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilterSpec {
    pub column: String,
    pub operator: String, // "eq" | "neq" | "contains" | "starts" | "ends" | "ncontains" | "gt" | "gte" | "lt" | "lte" | "null" | "notnull" | "istrue" | "isfalse"
    pub value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SortSpec {
    pub column: String,
    pub direction: String, // "asc" | "desc"
}

/// Renders the WHERE clause a set of filters produces, or an empty string when
/// there are none. Shared by the query path and the UI's SQL preview, so the
/// SQL shown to the user is the SQL that runs.
pub fn filters_to_where_clause(filters: &[FilterSpec], builder: &dyn SqlBuilder) -> String {
    if filters.is_empty() {
        return String::new();
    }
    let predicates: Vec<String> = filters.iter().map(|f| filter_to_sql(f, builder)).collect();
    format!("WHERE {}", predicates.join(" AND "))
}

/// Trims surrounding whitespace and every trailing statement terminator, so a
/// body can be safely wrapped in `(...)` as a subquery or CTE. Interior
/// semicolons are untouched — `is_wrappable_query` is what rejects genuinely
/// multi-statement input.
pub fn normalize_sql_body(sql: &str) -> &str {
    let mut s = sql.trim();
    while s.ends_with(';') {
        s = s[..s.len() - 1].trim_end();
    }
    s
}

pub fn wrap_for_count(base_sql: &str, filters: &[FilterSpec], builder: &dyn SqlBuilder) -> String {
    let trimmed = normalize_sql_body(base_sql);
    // The newline after the body terminates a trailing `--` comment so it
    // cannot swallow the closing paren (C6).
    let mut sql = format!("SELECT COUNT(*) FROM ({trimmed}\n) AS _lucent_count_base");
    if !filters.is_empty() {
        sql.push(' ');
        sql.push_str(&filters_to_where_clause(filters, builder));
    }
    sql
}

pub fn wrap_for_page(
    base_sql: &str,
    sort: &Option<SortSpec>,
    filters: &[FilterSpec],
    limit: i64,
    offset: i64,
    builder: &dyn SqlBuilder,
) -> String {
    let trimmed = normalize_sql_body(base_sql);
    // The newline after the body terminates a trailing `--` comment (C6).
    let mut sql = format!("SELECT * FROM ({trimmed}\n) AS _lucent_page");

    if !filters.is_empty() {
        sql.push(' ');
        sql.push_str(&filters_to_where_clause(filters, builder));
    }

    if let Some(s) = sort {
        let dir = if s.direction.eq_ignore_ascii_case("desc") {
            "DESC"
        } else {
            "ASC"
        };
        sql.push_str(&format!(
            " ORDER BY {} {dir}",
            builder.quote_identifier(&s.column)
        ));
    }

    // `page` re-emits the whole SQL plus the window clause.
    sql = builder.page(&sql, limit, offset);
    sql
}

pub fn filter_to_sql(filter: &FilterSpec, builder: &dyn SqlBuilder) -> String {
    let col = builder.quote_identifier(&filter.column);
    let val = filter.value.as_deref().unwrap_or("");
    let text = builder.cast_to_text(&col);
    match filter.operator.as_str() {
        "eq" => format!("{col} = {}", builder.quote_string(val)),
        "neq" => format!("{col} != {}", builder.quote_string(val)),
        "contains" => builder.case_insensitive_contains(&col, val),
        "starts" => builder.case_insensitive_starts_with(&col, val),
        "ends" => builder.case_insensitive_ends_with(&col, val),
        "ncontains" => format!(
            "(NOT ({}) OR {col} IS NULL)",
            builder.case_insensitive_contains(&col, val)
        ),
        "gt" => format!("{col} > {}", builder.quote_string(val)),
        "gte" => format!("{col} >= {}", builder.quote_string(val)),
        "lt" => format!("{col} < {}", builder.quote_string(val)),
        "lte" => format!("{col} <= {}", builder.quote_string(val)),
        "istrue" => format!("{col} IS TRUE"),
        "isfalse" => format!("{col} IS FALSE"),
        "null" => format!("{col} IS NULL"),
        "notnull" => format!("{col} IS NOT NULL"),
        _ => {
            let _ = text;
            "TRUE".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use lucent_protocol::SqlDialect;

    use super::*;

    const PG: SqlDialect = SqlDialect::PostgreSql;

    #[test]
    fn wrappable_for_plain_select() {
        assert!(is_wrappable_query("SELECT * FROM users", PG));
    }

    #[test]
    fn wrappable_for_cte() {
        assert!(is_wrappable_query(
            "WITH r AS (SELECT * FROM orders) SELECT * FROM r",
            PG
        ));
    }

    #[test]
    fn not_wrappable_for_insert() {
        assert!(!is_wrappable_query(
            "INSERT INTO users (name) VALUES ('x')",
            PG
        ));
    }

    #[test]
    fn not_wrappable_for_delete() {
        assert!(!is_wrappable_query("DELETE FROM users WHERE id = 1", PG));
    }

    #[test]
    fn not_wrappable_for_ddl() {
        assert!(!is_wrappable_query("CREATE TABLE t (id int)", PG));
    }

    #[test]
    fn not_wrappable_for_multi_statement() {
        assert!(!is_wrappable_query("SELECT 1; SELECT 2", PG));
    }

    #[test]
    fn not_wrappable_for_unparseable_sql() {
        assert!(!is_wrappable_query("this is not sql", PG));
    }

    #[test]
    fn not_wrappable_for_empty_string() {
        assert!(!is_wrappable_query("", PG));
    }

    #[test]
    fn not_wrappable_for_row_locking_clauses() {
        // C6: PostgreSQL rejects FOR UPDATE/FOR SHARE inside a subquery, so
        // wrapping turned a valid query into a syntax error. These must run
        // unwrapped instead.
        assert!(!is_wrappable_query("SELECT * FROM users FOR UPDATE", PG));
        assert!(!is_wrappable_query("SELECT * FROM users FOR SHARE", PG));
        assert!(!is_wrappable_query(
            "SELECT * FROM users FOR NO KEY UPDATE",
            PG
        ));
    }

    #[test]
    fn not_wrappable_for_select_into() {
        // C6: SELECT INTO is rejected inside a subquery too.
        assert!(!is_wrappable_query(
            "SELECT * INTO copied_users FROM users",
            PG
        ));
    }

    #[test]
    fn an_unparseable_dialect_makes_nothing_wrappable() {
        // Not fail-closed in the safety sense — an unwrappable query simply
        // executes unpaged, exactly as a multi-statement script does today.
        // But it must never wrap SQL it could not parse.
        assert!(!super::is_wrappable_with_parser("SELECT 1", None));
    }

    #[test]
    fn duckdb_specific_selects_are_wrappable_under_the_duckdb_dialect() {
        assert!(
            is_wrappable_query("SELECT * EXCLUDE (secret) FROM t", SqlDialect::DuckDb),
            "a valid DuckDB SELECT must be pageable"
        );
    }

    #[test]
    fn dml_is_still_not_wrappable_under_any_dialect() {
        for d in [SqlDialect::PostgreSql, SqlDialect::DuckDb] {
            assert!(!is_wrappable_query("DELETE FROM t", d));
            assert!(!is_wrappable_query("SELECT 1; SELECT 2", d));
        }
    }

    #[test]
    fn normalize_sql_body_strips_trailing_semicolons_and_space() {
        assert_eq!(normalize_sql_body("SELECT 1"), "SELECT 1");
        assert_eq!(normalize_sql_body("SELECT 1;"), "SELECT 1");
        assert_eq!(normalize_sql_body("  SELECT 1 ;  "), "SELECT 1");
        assert_eq!(normalize_sql_body("SELECT 1;;;"), "SELECT 1");
        assert_eq!(normalize_sql_body("SELECT 1 ; ; "), "SELECT 1");
    }

    #[test]
    fn normalize_sql_body_keeps_semicolon_inside_string_literal() {
        // A trailing quote means the semicolon is inside a literal, not a terminator.
        assert_eq!(normalize_sql_body("SELECT ';'"), "SELECT ';'");
    }
}

#[cfg(test)]
mod filter_tests {
    use crate::sql_builder::PostgresSqlBuilder;

    use super::*;

    fn pg() -> PostgresSqlBuilder {
        PostgresSqlBuilder
    }

    #[test]
    fn eq_quotes_column_and_value() {
        let f = FilterSpec {
            column: "name".into(),
            operator: "eq".into(),
            value: Some("Bob".into()),
        };
        assert_eq!(filter_to_sql(&f, &pg()), r#""name" = 'Bob'"#);
    }

    #[test]
    fn neq_builds_not_equal() {
        let f = FilterSpec {
            column: "status".into(),
            operator: "neq".into(),
            value: Some("archived".into()),
        };
        assert_eq!(filter_to_sql(&f, &pg()), r#""status" != 'archived'"#);
    }

    #[test]
    fn contains_wraps_value_in_percent_and_escapes_like_wildcards() {
        let f = FilterSpec {
            column: "email".into(),
            operator: "contains".into(),
            value: Some("50%_off".into()),
        };
        assert_eq!(
            filter_to_sql(&f, &pg()),
            r#""email"::text ILIKE '%50\%\_off%' ESCAPE '\'"#
        );
    }

    #[test]
    fn starts_anchors_pattern_at_start() {
        let f = FilterSpec {
            column: "name".into(),
            operator: "starts".into(),
            value: Some("Ac".into()),
        };
        assert_eq!(
            filter_to_sql(&f, &pg()),
            r#""name"::text ILIKE 'Ac%' ESCAPE '\'"#
        );
    }

    #[test]
    fn null_ignores_value() {
        let f = FilterSpec {
            column: "deleted_at".into(),
            operator: "null".into(),
            value: None,
        };
        assert_eq!(filter_to_sql(&f, &pg()), r#""deleted_at" IS NULL"#);
    }

    #[test]
    fn notnull_ignores_value() {
        let f = FilterSpec {
            column: "deleted_at".into(),
            operator: "notnull".into(),
            value: None,
        };
        assert_eq!(filter_to_sql(&f, &pg()), r#""deleted_at" IS NOT NULL"#);
    }

    #[test]
    fn column_names_are_identifier_escaped() {
        let f = FilterSpec {
            column: r#"weird"col"#.into(),
            operator: "eq".into(),
            value: Some("x".into()),
        };
        assert!(filter_to_sql(&f, &pg()).starts_with(r#""weird""col""#));
    }

    #[test]
    fn value_quotes_are_escaped() {
        let f = FilterSpec {
            column: "name".into(),
            operator: "eq".into(),
            value: Some("O'Brien".into()),
        };
        assert_eq!(filter_to_sql(&f, &pg()), r#""name" = 'O''Brien'"#);
    }

    #[test]
    fn unknown_operator_is_a_safe_no_op() {
        let f = FilterSpec {
            column: "x".into(),
            operator: "bogus".into(),
            value: Some("y".into()),
        };
        assert_eq!(filter_to_sql(&f, &pg()), "TRUE");
    }

    #[test]
    fn ncontains_includes_null_rows() {
        let f = FilterSpec {
            column: "name".into(),
            operator: "ncontains".into(),
            value: Some("Ac".into()),
        };
        assert_eq!(
            filter_to_sql(&f, &pg()),
            r#"(NOT ("name"::text ILIKE '%Ac%' ESCAPE '\') OR "name" IS NULL)"#
        );
    }

    #[test]
    fn ends_anchors_the_pattern_at_the_end() {
        let f = FilterSpec {
            column: "email".into(),
            operator: "ends".into(),
            value: Some("@example.com".into()),
        };
        assert_eq!(
            filter_to_sql(&f, &pg()),
            r#""email"::text ILIKE '%@example.com' ESCAPE '\'"#
        );
    }

    #[test]
    fn ends_escapes_like_wildcards_in_the_value() {
        let f = FilterSpec {
            column: "code".into(),
            operator: "ends".into(),
            value: Some("50%_x".into()),
        };
        assert_eq!(
            filter_to_sql(&f, &pg()),
            r#""code"::text ILIKE '%50\%\_x' ESCAPE '\'"#
        );
    }

    #[test]
    fn ncontains_escapes_like_wildcards_in_the_value() {
        let f = FilterSpec {
            column: "code".into(),
            operator: "ncontains".into(),
            value: Some("100%".into()),
        };
        assert_eq!(
            filter_to_sql(&f, &pg()),
            r#"(NOT ("code"::text ILIKE '%100\%%' ESCAPE '\') OR "code" IS NULL)"#
        );
    }

    #[test]
    fn comparison_operators_quote_the_value() {
        let cases = [("gt", ">"), ("gte", ">="), ("lt", "<"), ("lte", "<=")];
        for (operator, sql_op) in cases {
            let f = FilterSpec {
                column: "age".into(),
                operator: operator.into(),
                value: Some("30".into()),
            };
            assert_eq!(filter_to_sql(&f, &pg()), format!(r#""age" {sql_op} '30'"#));
        }
    }

    #[test]
    fn truthiness_operators_need_no_value() {
        let t = FilterSpec {
            column: "active".into(),
            operator: "istrue".into(),
            value: None,
        };
        assert_eq!(filter_to_sql(&t, &pg()), r#""active" IS TRUE"#);

        let f = FilterSpec {
            column: "active".into(),
            operator: "isfalse".into(),
            value: None,
        };
        assert_eq!(filter_to_sql(&f, &pg()), r#""active" IS FALSE"#);
    }

    #[test]
    fn unknown_operator_still_falls_back_to_true() {
        let f = FilterSpec {
            column: "x".into(),
            operator: "bogus".into(),
            value: None,
        };
        assert_eq!(filter_to_sql(&f, &pg()), "TRUE");
    }

    #[test]
    fn where_clause_is_empty_without_filters() {
        assert_eq!(filters_to_where_clause(&[], &pg()), "");
    }

    #[test]
    fn where_clause_joins_predicates_with_and() {
        let filters = vec![
            FilterSpec {
                column: "status".into(),
                operator: "eq".into(),
                value: Some("active".into()),
            },
            FilterSpec {
                column: "age".into(),
                operator: "gte".into(),
                value: Some("30".into()),
            },
        ];
        assert_eq!(
            filters_to_where_clause(&filters, &pg()),
            r#"WHERE "status" = 'active' AND "age" >= '30'"#
        );
    }
}

#[cfg(test)]
mod wrap_tests {
    use crate::sql_builder::PostgresSqlBuilder;

    use super::*;

    fn pg() -> PostgresSqlBuilder {
        PostgresSqlBuilder
    }

    #[test]
    fn a_trailing_line_comment_cannot_swallow_the_closing_paren() {
        // C6: the body's trailing `-- done` used to comment out the closing
        // paren, producing a syntax error on every paged query with a trailing
        // comment. The newline after the body terminates the comment.
        let sql = wrap_for_page("SELECT 1 -- done", &None, &[], 200, 0, &pg());
        assert!(
            sql.starts_with("SELECT * FROM (SELECT 1 -- done\n) AS _lucent_page"),
            "closing paren must not be inside the comment: {sql}"
        );
        let count_sql = wrap_for_count("SELECT 1 -- done", &[], &pg());
        assert!(
            count_sql
                .starts_with("SELECT COUNT(*) FROM (SELECT 1 -- done\n) AS _lucent_count_base"),
            "count wrap must not be inside the comment: {count_sql}"
        );
    }

    #[test]
    fn wraps_with_limit_and_offset_only() {
        let sql = wrap_for_page("SELECT * FROM users", &None, &[], 200, 0, &pg());
        assert_eq!(
            sql,
            r#"SELECT * FROM (SELECT * FROM users
) AS _lucent_page LIMIT 200 OFFSET 0"#
        );
    }

    #[test]
    fn strips_trailing_semicolon_and_whitespace_from_base() {
        let sql = wrap_for_page("SELECT * FROM users;  ", &None, &[], 200, 0, &pg());
        assert!(sql.starts_with("SELECT * FROM (SELECT * FROM users\n) AS _lucent_page"));
    }

    #[test]
    fn includes_order_by_when_sort_given() {
        let sort = Some(SortSpec {
            column: "created_at".into(),
            direction: "desc".into(),
        });
        let sql = wrap_for_page("SELECT * FROM users", &sort, &[], 200, 0, &pg());
        assert_eq!(
            sql,
            r#"SELECT * FROM (SELECT * FROM users
) AS _lucent_page ORDER BY "created_at" DESC LIMIT 200 OFFSET 0"#
        );
    }

    #[test]
    fn non_desc_direction_defaults_to_asc() {
        let sort = Some(SortSpec {
            column: "id".into(),
            direction: "whatever".into(),
        });
        let sql = wrap_for_page("SELECT * FROM users", &sort, &[], 200, 0, &pg());
        assert!(sql.contains(r#"ORDER BY "id" ASC"#));
    }

    #[test]
    fn includes_where_when_filters_given() {
        let filters = vec![FilterSpec {
            column: "active".into(),
            operator: "eq".into(),
            value: Some("true".into()),
        }];
        let sql = wrap_for_page("SELECT * FROM users", &None, &filters, 200, 0, &pg());
        assert_eq!(
            sql,
            r#"SELECT * FROM (SELECT * FROM users
) AS _lucent_page WHERE "active" = 'true' LIMIT 200 OFFSET 0"#
        );
    }

    #[test]
    fn multiple_filters_are_joined_with_and() {
        let filters = vec![
            FilterSpec {
                column: "active".into(),
                operator: "eq".into(),
                value: Some("true".into()),
            },
            FilterSpec {
                column: "role".into(),
                operator: "neq".into(),
                value: Some("guest".into()),
            },
        ];
        let sql = wrap_for_page("SELECT * FROM users", &None, &filters, 200, 0, &pg());
        assert!(sql.contains(r#"WHERE "active" = 'true' AND "role" != 'guest'"#));
    }

    #[test]
    fn where_and_order_by_and_pagination_compose_together() {
        let sort = Some(SortSpec {
            column: "id".into(),
            direction: "asc".into(),
        });
        let filters = vec![FilterSpec {
            column: "active".into(),
            operator: "eq".into(),
            value: Some("true".into()),
        }];
        let sql = wrap_for_page("SELECT * FROM users", &sort, &filters, 50, 100, &pg());
        assert_eq!(
            sql,
            r#"SELECT * FROM (SELECT * FROM users
) AS _lucent_page WHERE "active" = 'true' ORDER BY "id" ASC LIMIT 50 OFFSET 100"#
        );
    }

    #[test]
    fn negative_limit_and_offset_are_clamped_to_zero() {
        let sql = wrap_for_page("SELECT * FROM users", &None, &[], -5, -10, &pg());
        assert!(sql.ends_with("LIMIT 0 OFFSET 0"));
    }
}

#[cfg(test)]
mod count_tests {
    use crate::sql_builder::PostgresSqlBuilder;

    use super::*;

    fn pg() -> PostgresSqlBuilder {
        PostgresSqlBuilder
    }

    #[test]
    fn wraps_with_count_star() {
        let sql = wrap_for_count("SELECT * FROM users", &[], &pg());
        assert_eq!(
            sql,
            r#"SELECT COUNT(*) FROM (SELECT * FROM users
) AS _lucent_count_base"#
        );
    }

    #[test]
    fn includes_where_when_filters_given() {
        let filters = vec![FilterSpec {
            column: "active".into(),
            operator: "eq".into(),
            value: Some("true".into()),
        }];
        let sql = wrap_for_count("SELECT * FROM users", &filters, &pg());
        assert_eq!(
            sql,
            r#"SELECT COUNT(*) FROM (SELECT * FROM users
) AS _lucent_count_base WHERE "active" = 'true'"#
        );
    }

    #[test]
    fn strips_trailing_semicolon() {
        let sql = wrap_for_count("SELECT * FROM users;", &[], &pg());
        assert!(!sql.contains(';'));
    }
}
