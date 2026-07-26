use crate::commands::{quote_identifier, quote_string};
use serde::Deserialize;
use sqlparser::ast::Statement;
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;

/// Whether `sql` is a single `SELECT`/`WITH`/`VALUES`-shaped statement that
/// can be safely wrapped as `SELECT * FROM (<sql>) AS _lucent_page ...`.
/// Multi-statement input and anything that isn't a Statement::Query
/// (INSERT/UPDATE/DELETE/DDL) returns false — those execute unwrapped,
/// unpaginated, exactly as today.
pub fn is_wrappable_query(sql: &str) -> bool {
    let dialect = PostgreSqlDialect {};
    match Parser::parse_sql(&dialect, sql) {
        Ok(statements) if statements.len() == 1 => matches!(statements[0], Statement::Query(_)),
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

/// Escapes literal `%`, `_`, and `\` in a user-supplied value before it goes
/// into an ILIKE pattern, so a search for "50%" matches the literal percent
/// sign instead of being read as a wildcard.
fn escape_like_pattern(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Renders the WHERE clause a set of filters produces, or an empty string when
/// there are none. Shared by the query path and the UI's SQL preview, so the
/// SQL shown to the user is the SQL that runs.
pub fn filters_to_where_clause(filters: &[FilterSpec]) -> String {
    if filters.is_empty() {
        return String::new();
    }
    let predicates: Vec<String> = filters.iter().map(filter_to_sql).collect();
    format!("WHERE {}", predicates.join(" AND "))
}

pub fn wrap_for_count(base_sql: &str, filters: &[FilterSpec]) -> String {
    let trimmed = base_sql.trim().trim_end_matches(';').trim_end();
    let mut sql = format!("SELECT COUNT(*) FROM ({trimmed}) AS _lucent_count_base");
    if !filters.is_empty() {
        sql.push(' ');
        sql.push_str(&filters_to_where_clause(filters));
    }
    sql
}

pub fn wrap_for_page(
    base_sql: &str,
    sort: &Option<SortSpec>,
    filters: &[FilterSpec],
    limit: i64,
    offset: i64,
) -> String {
    let trimmed = base_sql.trim().trim_end_matches(';').trim_end();
    let mut sql = format!("SELECT * FROM ({trimmed}) AS _lucent_page");

    if !filters.is_empty() {
        sql.push(' ');
        sql.push_str(&filters_to_where_clause(filters));
    }

    if let Some(s) = sort {
        let dir = if s.direction.eq_ignore_ascii_case("desc") {
            "DESC"
        } else {
            "ASC"
        };
        sql.push_str(&format!(" ORDER BY {} {dir}", quote_identifier(&s.column)));
    }

    sql.push_str(&format!(" LIMIT {} OFFSET {}", limit.max(0), offset.max(0)));
    sql
}

pub fn filter_to_sql(filter: &FilterSpec) -> String {
    let col = quote_identifier(&filter.column);
    let val = filter.value.as_deref().unwrap_or("");
    match filter.operator.as_str() {
        "eq" => format!("{col} = {}", quote_string(val)),
        "neq" => format!("{col} != {}", quote_string(val)),
        "contains" => format!(
            "{col}::text ILIKE {} ESCAPE '\\'",
            quote_string(&format!("%{}%", escape_like_pattern(val)))
        ),
        "starts" => format!(
            "{col}::text ILIKE {} ESCAPE '\\'",
            quote_string(&format!("{}%", escape_like_pattern(val)))
        ),
        "ends" => format!(
            "{col}::text ILIKE {} ESCAPE '\\'",
            quote_string(&format!("%{}", escape_like_pattern(val)))
        ),
        "ncontains" => format!(
            "({col}::text NOT ILIKE {} ESCAPE '\\' OR {col} IS NULL)",
            quote_string(&format!("%{}%", escape_like_pattern(val)))
        ),
        "gt" => format!("{col} > {}", quote_string(val)),
        "gte" => format!("{col} >= {}", quote_string(val)),
        "lt" => format!("{col} < {}", quote_string(val)),
        "lte" => format!("{col} <= {}", quote_string(val)),
        "istrue" => format!("{col} IS TRUE"),
        "isfalse" => format!("{col} IS FALSE"),
        "null" => format!("{col} IS NULL"),
        "notnull" => format!("{col} IS NOT NULL"),
        _ => "TRUE".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrappable_for_plain_select() {
        assert!(is_wrappable_query("SELECT * FROM users"));
    }

    #[test]
    fn wrappable_for_cte() {
        assert!(is_wrappable_query(
            "WITH r AS (SELECT * FROM orders) SELECT * FROM r"
        ));
    }

    #[test]
    fn not_wrappable_for_insert() {
        assert!(!is_wrappable_query("INSERT INTO users (name) VALUES ('x')"));
    }

    #[test]
    fn not_wrappable_for_delete() {
        assert!(!is_wrappable_query("DELETE FROM users WHERE id = 1"));
    }

    #[test]
    fn not_wrappable_for_ddl() {
        assert!(!is_wrappable_query("CREATE TABLE t (id int)"));
    }

    #[test]
    fn not_wrappable_for_multi_statement() {
        assert!(!is_wrappable_query("SELECT 1; SELECT 2"));
    }

    #[test]
    fn not_wrappable_for_unparseable_sql() {
        assert!(!is_wrappable_query("this is not sql"));
    }

    #[test]
    fn not_wrappable_for_empty_string() {
        assert!(!is_wrappable_query(""));
    }
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    #[test]
    fn eq_quotes_column_and_value() {
        let f = FilterSpec {
            column: "name".into(),
            operator: "eq".into(),
            value: Some("Bob".into()),
        };
        assert_eq!(filter_to_sql(&f), r#""name" = 'Bob'"#);
    }

    #[test]
    fn neq_builds_not_equal() {
        let f = FilterSpec {
            column: "status".into(),
            operator: "neq".into(),
            value: Some("archived".into()),
        };
        assert_eq!(filter_to_sql(&f), r#""status" != 'archived'"#);
    }

    #[test]
    fn contains_wraps_value_in_percent_and_escapes_like_wildcards() {
        let f = FilterSpec {
            column: "email".into(),
            operator: "contains".into(),
            value: Some("50%_off".into()),
        };
        assert_eq!(
            filter_to_sql(&f),
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
        assert_eq!(filter_to_sql(&f), r#""name"::text ILIKE 'Ac%' ESCAPE '\'"#);
    }

    #[test]
    fn null_ignores_value() {
        let f = FilterSpec {
            column: "deleted_at".into(),
            operator: "null".into(),
            value: None,
        };
        assert_eq!(filter_to_sql(&f), r#""deleted_at" IS NULL"#);
    }

    #[test]
    fn notnull_ignores_value() {
        let f = FilterSpec {
            column: "deleted_at".into(),
            operator: "notnull".into(),
            value: None,
        };
        assert_eq!(filter_to_sql(&f), r#""deleted_at" IS NOT NULL"#);
    }

    #[test]
    fn column_names_are_identifier_escaped() {
        let f = FilterSpec {
            column: r#"weird"col"#.into(),
            operator: "eq".into(),
            value: Some("x".into()),
        };
        assert!(filter_to_sql(&f).starts_with(r#""weird""col""#));
    }

    #[test]
    fn value_quotes_are_escaped() {
        let f = FilterSpec {
            column: "name".into(),
            operator: "eq".into(),
            value: Some("O'Brien".into()),
        };
        assert_eq!(filter_to_sql(&f), r#""name" = 'O''Brien'"#);
    }

    #[test]
    fn unknown_operator_is_a_safe_no_op() {
        let f = FilterSpec {
            column: "x".into(),
            operator: "bogus".into(),
            value: Some("y".into()),
        };
        assert_eq!(filter_to_sql(&f), "TRUE");
    }

    #[test]
    fn ncontains_includes_null_rows() {
        let f = FilterSpec {
            column: "name".into(),
            operator: "ncontains".into(),
            value: Some("Ac".into()),
        };
        assert_eq!(
            filter_to_sql(&f),
            r#"("name"::text NOT ILIKE '%Ac%' ESCAPE '\' OR "name" IS NULL)"#
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
            filter_to_sql(&f),
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
            filter_to_sql(&f),
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
            filter_to_sql(&f),
            r#"("code"::text NOT ILIKE '%100\%%' ESCAPE '\' OR "code" IS NULL)"#
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
            assert_eq!(filter_to_sql(&f), format!(r#""age" {sql_op} '30'"#));
        }
    }

    #[test]
    fn truthiness_operators_need_no_value() {
        let t = FilterSpec {
            column: "active".into(),
            operator: "istrue".into(),
            value: None,
        };
        assert_eq!(filter_to_sql(&t), r#""active" IS TRUE"#);

        let f = FilterSpec {
            column: "active".into(),
            operator: "isfalse".into(),
            value: None,
        };
        assert_eq!(filter_to_sql(&f), r#""active" IS FALSE"#);
    }

    #[test]
    fn unknown_operator_still_falls_back_to_true() {
        let f = FilterSpec {
            column: "x".into(),
            operator: "bogus".into(),
            value: None,
        };
        assert_eq!(filter_to_sql(&f), "TRUE");
    }

    #[test]
    fn where_clause_is_empty_without_filters() {
        assert_eq!(filters_to_where_clause(&[]), "");
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
            filters_to_where_clause(&filters),
            r#"WHERE "status" = 'active' AND "age" >= '30'"#
        );
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::*;

    #[test]
    fn wraps_with_limit_and_offset_only() {
        let sql = wrap_for_page("SELECT * FROM users", &None, &[], 200, 0);
        assert_eq!(
            sql,
            r#"SELECT * FROM (SELECT * FROM users) AS _lucent_page LIMIT 200 OFFSET 0"#
        );
    }

    #[test]
    fn strips_trailing_semicolon_and_whitespace_from_base() {
        let sql = wrap_for_page("SELECT * FROM users;  ", &None, &[], 200, 0);
        assert!(sql.starts_with("SELECT * FROM (SELECT * FROM users) AS _lucent_page"));
    }

    #[test]
    fn includes_order_by_when_sort_given() {
        let sort = Some(SortSpec {
            column: "created_at".into(),
            direction: "desc".into(),
        });
        let sql = wrap_for_page("SELECT * FROM users", &sort, &[], 200, 0);
        assert_eq!(
            sql,
            r#"SELECT * FROM (SELECT * FROM users) AS _lucent_page ORDER BY "created_at" DESC LIMIT 200 OFFSET 0"#
        );
    }

    #[test]
    fn non_desc_direction_defaults_to_asc() {
        let sort = Some(SortSpec {
            column: "id".into(),
            direction: "whatever".into(),
        });
        let sql = wrap_for_page("SELECT * FROM users", &sort, &[], 200, 0);
        assert!(sql.contains(r#"ORDER BY "id" ASC"#));
    }

    #[test]
    fn includes_where_when_filters_given() {
        let filters = vec![FilterSpec {
            column: "active".into(),
            operator: "eq".into(),
            value: Some("true".into()),
        }];
        let sql = wrap_for_page("SELECT * FROM users", &None, &filters, 200, 0);
        assert_eq!(
            sql,
            r#"SELECT * FROM (SELECT * FROM users) AS _lucent_page WHERE "active" = 'true' LIMIT 200 OFFSET 0"#
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
        let sql = wrap_for_page("SELECT * FROM users", &None, &filters, 200, 0);
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
        let sql = wrap_for_page("SELECT * FROM users", &sort, &filters, 50, 100);
        assert_eq!(
            sql,
            r#"SELECT * FROM (SELECT * FROM users) AS _lucent_page WHERE "active" = 'true' ORDER BY "id" ASC LIMIT 50 OFFSET 100"#
        );
    }

    #[test]
    fn negative_limit_and_offset_are_clamped_to_zero() {
        let sql = wrap_for_page("SELECT * FROM users", &None, &[], -5, -10);
        assert!(sql.ends_with("LIMIT 0 OFFSET 0"));
    }
}

#[cfg(test)]
mod count_tests {
    use super::*;

    #[test]
    fn wraps_with_count_star() {
        let sql = wrap_for_count("SELECT * FROM users", &[]);
        assert_eq!(
            sql,
            r#"SELECT COUNT(*) FROM (SELECT * FROM users) AS _lucent_count_base"#
        );
    }

    #[test]
    fn includes_where_when_filters_given() {
        let filters = vec![FilterSpec {
            column: "active".into(),
            operator: "eq".into(),
            value: Some("true".into()),
        }];
        let sql = wrap_for_count("SELECT * FROM users", &filters);
        assert_eq!(
            sql,
            r#"SELECT COUNT(*) FROM (SELECT * FROM users) AS _lucent_count_base WHERE "active" = 'true'"#
        );
    }

    #[test]
    fn strips_trailing_semicolon() {
        let sql = wrap_for_count("SELECT * FROM users;", &[]);
        assert!(!sql.contains(';'));
    }
}
