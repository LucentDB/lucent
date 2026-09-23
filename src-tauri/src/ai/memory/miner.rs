//! Offline frequent-join-pattern miner over query history.
//!
//! Feed this the SQL of *successful* history entries (see
//! [`crate::query_history::successful_sqls`]) and it extracts
//! `(left_table, right_table, join_condition)` triples from the AST, counting
//! the join shapes that recur across queries. The memory subsystem can then
//! propose learned join relationships the schema graph never declared.
//!
//! Parsing is best-effort: dialect-specific syntax, template placeholders and
//! otherwise unparseable statements are skipped, never fatal. This is an
//! advisory miner, not a guard.

use sqlparser::ast::{
    JoinConstraint, JoinOperator, Query, SetExpr, Statement, TableFactor, TableWithJoins,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use std::collections::HashMap;

/// A recurring join relationship: `left_table` joined to `right_table` by the
/// same textual `join_condition`, observed `occurrences` times.
#[derive(Debug, Clone)]
pub struct MinedPattern {
    pub left_table: String,
    pub right_table: String,
    pub join_condition: String,
    pub occurrences: usize,
}

/// Mines recurring join relationships from `queries`.
///
/// A pattern is a `(left_table, right_table, join_condition)` triple. Identical
/// triples seen at least `min_count` times are returned sorted by descending
/// occurrence count, then by left/right table and condition, so the order is
/// stable across runs.
pub fn mine_join_patterns(queries: &[&str], min_count: usize) -> Vec<MinedPattern> {
    let dialect = GenericDialect {};
    let mut counts: HashMap<(String, String, String), usize> = HashMap::new();

    for q in queries {
        let Ok(statements) = Parser::parse_sql(&dialect, q) else {
            continue;
        };
        for stmt in statements {
            if let Statement::Query(query) = stmt {
                collect_query_joins(&query, &mut counts);
            }
        }
    }

    let mut patterns: Vec<MinedPattern> = counts
        .into_iter()
        .filter(|(_, occurrences)| *occurrences >= min_count)
        .map(
            |((left_table, right_table, join_condition), occurrences)| MinedPattern {
                left_table,
                right_table,
                join_condition,
                occurrences,
            },
        )
        .collect();

    patterns.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.left_table.cmp(&b.left_table))
            .then_with(|| a.right_table.cmp(&b.right_table))
            .then_with(|| a.join_condition.cmp(&b.join_condition))
    });

    patterns
}

/// Walks a query, including CTE bodies, collecting join triples.
fn collect_query_joins(query: &Query, counts: &mut HashMap<(String, String, String), usize>) {
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            collect_query_joins(&cte.query, counts);
        }
    }
    collect_set_expr_joins(&query.body, counts);
}

/// Walks the query body: a SELECT, a parenthesized query, or a set operation.
fn collect_set_expr_joins(body: &SetExpr, counts: &mut HashMap<(String, String, String), usize>) {
    match body {
        SetExpr::Select(select) => {
            for twj in &select.from {
                collect_table_with_joins(twj, counts);
            }
        }
        SetExpr::Query(inner) => collect_query_joins(inner, counts),
        SetExpr::SetOperation { left, right, .. } => {
            collect_set_expr_joins(left, counts);
            collect_set_expr_joins(right, counts);
        }
        _ => {}
    }
}

/// Records every explicit join on a `FROM` item against its left relation.
fn collect_table_with_joins(
    twj: &TableWithJoins,
    counts: &mut HashMap<(String, String, String), usize>,
) {
    let Some(left_table) = table_name(&twj.relation) else {
        return;
    };
    for join in &twj.joins {
        let Some(right_table) = table_name(&join.relation) else {
            continue;
        };
        let Some(join_condition) = join_condition_text(&join.join_operator) else {
            continue;
        };
        *counts
            .entry((left_table.clone(), right_table, join_condition))
            .or_insert(0) += 1;
    }
}

/// The underlying table name of a table factor, or `None` for derived tables,
/// table functions, CTE references and other non-`Table` factors.
fn table_name(factor: &TableFactor) -> Option<String> {
    if let TableFactor::Table { name, .. } = factor {
        Some(name.to_string())
    } else {
        None
    }
}

/// The textual join condition carried by a join operator, or `None` when the
/// join expresses no condition (e.g. `CROSS APPLY`, or an unconstrained join).
///
/// `USING (a, b)` and `NATURAL` are preserved in a normalized textual form so
/// equal join shapes still hash together.
fn join_condition_text(op: &JoinOperator) -> Option<String> {
    use JoinOperator::*;

    let constraint = match op {
        Join(c) | Inner(c) | Left(c) | LeftOuter(c) | Right(c) | RightOuter(c) | FullOuter(c)
        | CrossJoin(c) | Semi(c) | LeftSemi(c) | RightSemi(c) | Anti(c) | LeftAnti(c)
        | RightAnti(c) | StraightJoin(c) => c,
        AsOf { constraint, .. } => constraint,
        CrossApply | OuterApply | ArrayJoin | LeftArrayJoin | InnerArrayJoin => return None,
    };

    match constraint {
        JoinConstraint::On(expr) => Some(expr.to_string()),
        JoinConstraint::Using(columns) => {
            let cols: Vec<String> = columns.iter().map(|c| c.to_string()).collect();
            Some(format!("USING ({})", cols.join(", ")))
        }
        JoinConstraint::Natural => Some("NATURAL".to_string()),
        JoinConstraint::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mine_recurring_join_patterns() {
        let sqls = vec![
            "SELECT * FROM orders o JOIN users u ON o.user_id = u.id",
            "SELECT o.id, u.name FROM orders o JOIN users u ON o.user_id = u.id WHERE o.total > 100",
            "SELECT count(*) FROM orders o JOIN users u ON o.user_id = u.id",
        ];

        let patterns = mine_join_patterns(&sqls, 2);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].left_table, "orders");
        assert_eq!(patterns[0].right_table, "users");
        assert!(patterns[0].join_condition.contains("user_id = u.id"));
    }

    /// R20 seam: real history flows through `successful_sqls` into the miner,
    /// and a failed statement must never contribute a pattern.
    #[test]
    fn test_successful_history_feeds_the_miner() {
        use crate::query_history::{successful_sqls, QueryHistoryEntry};

        let entry = |sql: &str, status: &str| QueryHistoryEntry {
            id: sql.into(),
            connection_id: "conn".into(),
            connection_name: "conn".into(),
            database: "db".into(),
            sql: sql.into(),
            duration_ms: 1,
            row_count: None,
            status: status.into(),
            error: None,
            executed_at: String::new(),
            favorite: false,
        };
        let sql = "SELECT * FROM orders o JOIN users u ON o.user_id = u.id";
        let entries = vec![
            entry(sql, "success"),
            entry(sql, "success"),
            entry(sql, "error"),
        ];

        let sqls = successful_sqls(&entries);
        assert_eq!(sqls.len(), 2, "failed statements must not teach patterns");

        let refs: Vec<&str> = sqls.iter().map(String::as_str).collect();
        assert_eq!(mine_join_patterns(&refs, 2).len(), 1);
    }
}
