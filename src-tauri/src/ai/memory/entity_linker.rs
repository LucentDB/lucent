use sqlparser::ast::{
    Expr, GroupByExpr, JoinConstraint, JoinOperator, ObjectNamePart, OrderByKind, Query, Select,
    SelectItem, SelectItemQualifiedWildcardKind, SetExpr, Statement, TableFactor,
};
use sqlparser::dialect::GenericDialect;
use sqlparser::parser::Parser;
use std::collections::HashSet;

use crate::ai::schema_graph::SchemaGraph;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EntityRef {
    pub schema_name: String,
    pub table_name: String,
    pub column_name: Option<String>,
    pub data_type: String,
    pub is_nullable: bool,
    pub entity_fingerprint: String,
}

/// Computes the deterministic entity fingerprint as specified in spec §4.1:
/// blake3("entity-v1:{schema}:{table}:{column}:{data_type}:{is_nullable}")
pub fn compute_entity_fingerprint(
    schema: &str,
    table: &str,
    column: Option<&str>,
    data_type: &str,
    is_nullable: bool,
) -> String {
    let col = column.unwrap_or("");
    let formatted = format!("entity-v1:{schema}:{table}:{col}:{data_type}:{is_nullable}");
    blake3::hash(formatted.as_bytes()).to_hex().to_string()
}

/// Extracts entity links from a memory's rule text and optional SQL snippet,
/// resolving them against the current active SchemaGraph.
pub fn extract_and_link_entities(
    rule_text: &str,
    sql_snippet: Option<&str>,
    graph: &SchemaGraph,
) -> Vec<EntityRef> {
    let mut found_tables: HashSet<(Option<String>, String)> = HashSet::new();
    let mut found_columns: HashSet<String> = HashSet::new();

    // 1. If SQL is provided, parse AST using sqlparser
    if let Some(sql) = sql_snippet {
        let dialect = GenericDialect {};
        if let Ok(statements) = Parser::parse_sql(&dialect, sql) {
            for stmt in &statements {
                extract_from_statement(stmt, &mut found_tables, &mut found_columns);
            }
        }
    }

    // 2. Also search rule_text for known table and column names in the schema
    let lower_rule = rule_text.to_lowercase();
    for t in &graph.tables {
        let t_name_lower = t.name.to_lowercase();
        // Check if table name appears as a whole word in rule_text
        if contains_word(&lower_rule, &t_name_lower) {
            found_tables.insert((Some(t.schema.clone()), t.name.clone()));
        }
    }

    for c in &graph.columns {
        let c_name_lower = c.name.to_lowercase();
        if contains_word(&lower_rule, &c_name_lower) {
            found_columns.insert(c.name.clone());
        }
    }

    // 3. Resolve extracted names against SchemaGraph
    let mut results: HashSet<EntityRef> = HashSet::new();

    for (schema_opt, table_name) in found_tables {
        // Find matching table in graph
        let matching_tables: Vec<_> = graph
            .tables
            .iter()
            .filter(|t| {
                t.name.eq_ignore_ascii_case(&table_name)
                    && (schema_opt.is_none()
                        || schema_opt
                            .as_ref()
                            .map(|s| s.eq_ignore_ascii_case(&t.schema))
                            .unwrap_or(true))
            })
            .collect();

        for t in matching_tables {
            let cols = graph.columns_for_table(t.id);
            let mut matched_any_column = false;

            for col in cols {
                if found_columns
                    .iter()
                    .any(|fc| fc.eq_ignore_ascii_case(&col.name))
                {
                    matched_any_column = true;
                    let fp = compute_entity_fingerprint(
                        &t.schema,
                        &t.name,
                        Some(&col.name),
                        &col.data_type,
                        col.is_nullable,
                    );
                    results.insert(EntityRef {
                        schema_name: t.schema.clone(),
                        table_name: t.name.clone(),
                        column_name: Some(col.name.clone()),
                        data_type: col.data_type.clone(),
                        is_nullable: col.is_nullable,
                        entity_fingerprint: fp,
                    });
                }
            }

            // If no specific column matched, link at table level
            if !matched_any_column {
                let fp = compute_entity_fingerprint(&t.schema, &t.name, None, "", false);
                results.insert(EntityRef {
                    schema_name: t.schema.clone(),
                    table_name: t.name.clone(),
                    column_name: None,
                    data_type: String::new(),
                    is_nullable: false,
                    entity_fingerprint: fp,
                });
            }
        }
    }

    results.into_iter().collect()
}

fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() || haystack.is_empty() {
        return false;
    }
    haystack
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| word.eq_ignore_ascii_case(needle))
}

fn extract_from_statement(
    stmt: &Statement,
    tables: &mut HashSet<(Option<String>, String)>,
    columns: &mut HashSet<String>,
) {
    if let Statement::Query(q) = stmt {
        extract_from_query(q, tables, columns);
    }
}

fn extract_from_query(
    query: &Query,
    tables: &mut HashSet<(Option<String>, String)>,
    columns: &mut HashSet<String>,
) {
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            extract_from_query(&cte.query, tables, columns);
        }
    }
    if let SetExpr::Select(select) = query.body.as_ref() {
        extract_from_select(select, tables, columns);
    }
    // B-I6: ORDER BY lives on the Query, not the Select.
    if let Some(order_by) = &query.order_by {
        if let OrderByKind::Expressions(exprs) = &order_by.kind {
            for e in exprs {
                extract_from_expr(&e.expr, columns);
            }
        }
    }
}

fn extract_from_select(
    select: &Select,
    tables: &mut HashSet<(Option<String>, String)>,
    columns: &mut HashSet<String>,
) {
    for twj in &select.from {
        extract_from_table_factor(&twj.relation, tables);
        for join in &twj.joins {
            extract_from_table_factor(&join.relation, tables);
            // B-I6: a column referenced only by a join's ON condition was
            // invisible to the old WHERE-only walk.
            extract_from_join_operator(&join.join_operator, columns);
        }
    }
    // B-I6: projection columns (SELECT a, b ...) were never walked.
    for item in &select.projection {
        match item {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                extract_from_expr(expr, columns);
            }
            SelectItem::ExprWithAliases { expr, .. } => {
                extract_from_expr(expr, columns);
            }
            SelectItem::QualifiedWildcard(SelectItemQualifiedWildcardKind::Expr(expr), _) => {
                extract_from_expr(expr, columns);
            }
            SelectItem::QualifiedWildcard(SelectItemQualifiedWildcardKind::ObjectName(_), _)
            | SelectItem::Wildcard(_) => {}
        }
    }
    if let Some(sel) = &select.selection {
        extract_from_expr(sel, columns);
    }
    // B-I6: GROUP BY columns were never walked.
    if let GroupByExpr::Expressions(exprs, _) = &select.group_by {
        for e in exprs {
            extract_from_expr(e, columns);
        }
    }
}

/// Walks the constraint of a join operator, extracting column references from
/// its `ON` expression. `ASOF` joins carry both a match condition and an
/// additional constraint, so both are visited.
fn extract_from_join_operator(op: &JoinOperator, columns: &mut HashSet<String>) {
    use JoinOperator::*;
    let constraint = match op {
        Join(c) | Inner(c) | Left(c) | LeftOuter(c) | Right(c) | RightOuter(c) | FullOuter(c)
        | CrossJoin(c) | Semi(c) | LeftSemi(c) | RightSemi(c) | Anti(c) | LeftAnti(c)
        | RightAnti(c) | StraightJoin(c) => Some(c),
        AsOf {
            match_condition,
            constraint,
        } => {
            extract_from_expr(match_condition, columns);
            Some(constraint)
        }
        CrossApply | OuterApply | ArrayJoin | LeftArrayJoin | InnerArrayJoin => None,
    };
    if let Some(JoinConstraint::On(expr)) = constraint {
        extract_from_expr(expr, columns);
    }
}

/// Extracts referenced table names from a SQL query string.
pub fn extract_tables_from_sql(sql: &str) -> Vec<String> {
    let dialect = GenericDialect {};
    let mut found_tables = HashSet::new();
    let mut dummy_columns = HashSet::new();
    if let Ok(statements) = Parser::parse_sql(&dialect, sql) {
        for stmt in &statements {
            extract_from_statement(stmt, &mut found_tables, &mut dummy_columns);
        }
    }
    found_tables.into_iter().map(|(_, t)| t).collect()
}

fn extract_from_table_factor(factor: &TableFactor, tables: &mut HashSet<(Option<String>, String)>) {
    if let TableFactor::Table { name, .. } = factor {
        let parts: Vec<String> = name
            .0
            .iter()
            .filter_map(|p| match p {
                ObjectNamePart::Identifier(ident) => Some(ident.value.clone()),
                _ => None,
            })
            .collect();
        if parts.len() == 1 {
            tables.insert((None, parts[0].clone()));
        } else if parts.len() >= 2 {
            tables.insert((
                Some(parts[parts.len() - 2].clone()),
                parts[parts.len() - 1].clone(),
            ));
        }
    }
}

fn extract_from_expr(expr: &Expr, columns: &mut HashSet<String>) {
    match expr {
        Expr::Identifier(ident) => {
            columns.insert(ident.value.clone());
        }
        Expr::CompoundIdentifier(idents) => {
            if let Some(last) = idents.last() {
                columns.insert(last.value.clone());
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            extract_from_expr(left, columns);
            extract_from_expr(right, columns);
        }
        Expr::UnaryOp { expr, .. } => {
            extract_from_expr(expr, columns);
        }
        Expr::Nested(e) => {
            extract_from_expr(e, columns);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_entity_fingerprint_deterministic() {
        let fp1 = compute_entity_fingerprint("public", "users", Some("email"), "varchar", false);
        let fp2 = compute_entity_fingerprint("public", "users", Some("email"), "varchar", false);
        assert_eq!(fp1, fp2);

        // Nullability change changes fingerprint
        let fp_nullable =
            compute_entity_fingerprint("public", "users", Some("email"), "varchar", true);
        assert_ne!(fp1, fp_nullable);

        // Column None creates table-level fingerprint
        let fp_table = compute_entity_fingerprint("public", "users", None, "", false);
        assert_ne!(fp1, fp_table);
    }

    #[test]
    fn test_contains_word() {
        assert!(contains_word("select from orders where id = 1", "orders"));
        assert!(contains_word("join orders_items on id", "orders_items"));
        assert!(!contains_word("select from sub_orders", "orders"));
    }

    use crate::ai::schema_graph::{ColumnEntry, SchemaGraph, TableEntry};
    use std::collections::HashMap;

    /// `public.orders(id, user_id, created_at)` + `public.users(id, email)`.
    fn link_graph() -> SchemaGraph {
        let tables = vec![
            TableEntry {
                id: 0,
                schema: "public".into(),
                name: "orders".into(),
                kind: "table".into(),
                row_count_estimate: 0,
                partition_info: None,
            },
            TableEntry {
                id: 1,
                schema: "public".into(),
                name: "users".into(),
                kind: "table".into(),
                row_count_estimate: 0,
                partition_info: None,
            },
        ];
        let mk = |table_id: usize, table: &str, name: &str| ColumnEntry {
            id: 0,
            table_id,
            schema: "public".into(),
            table: table.into(),
            name: name.into(),
            data_type: "text".into(),
            is_primary_key: false,
            is_nullable: true,
            sample_values: vec![],
            fk_ref: None,
            embedding: vec![],
            doc_text: String::new(),
        };
        let columns = vec![
            mk(0, "orders", "id"),
            mk(0, "orders", "user_id"),
            mk(0, "orders", "created_at"),
            mk(1, "users", "id"),
            mk(1, "users", "email"),
        ];
        SchemaGraph {
            tables,
            columns,
            columns_by_table: HashMap::from([(0, vec![0, 1, 2]), (1, vec![3, 4])]),
            fk_edges: vec![],
            table_adjacency: HashMap::new(),
            built_at_unix: 0,
            tier: crate::ai::schema_graph::IndexingTier::MetadataOnly,
        }
    }

    fn assert_links(sql: &str, table: &str, column: &str) {
        let graph = link_graph();
        let links = extract_and_link_entities("", Some(sql), &graph);
        assert!(
            links
                .iter()
                .any(|e| e.table_name == table && e.column_name.as_deref() == Some(column)),
            "column {table}.{column} from `{sql}` must be linked; got {links:?}"
        );
    }

    /// B-I6: the AST walk only read the WHERE clause, so columns that appear
    /// solely in projections, GROUP BY, ORDER BY, or a join's ON condition were
    /// never linked to their entities.
    #[test]
    fn projection_columns_are_linked() {
        assert_links("SELECT created_at FROM orders", "orders", "created_at");
    }

    #[test]
    fn group_by_columns_are_linked() {
        assert_links(
            "SELECT 1 FROM orders GROUP BY created_at",
            "orders",
            "created_at",
        );
    }

    #[test]
    fn order_by_columns_are_linked() {
        assert_links(
            "SELECT 1 FROM orders ORDER BY created_at",
            "orders",
            "created_at",
        );
    }

    #[test]
    fn join_on_columns_are_linked() {
        assert_links(
            "SELECT 1 FROM orders o JOIN users u ON o.user_id = u.email",
            "users",
            "email",
        );
        // Both sides of the ON condition are column references.
        assert_links(
            "SELECT 1 FROM orders o JOIN users u ON o.user_id = u.email",
            "orders",
            "user_id",
        );
    }
}
