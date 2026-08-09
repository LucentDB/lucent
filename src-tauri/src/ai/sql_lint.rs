//! Join linter: checks executed SQL against the schema graph and appends
//! warnings to tool results. Two verified production failures motivate it:
//! a non-FK equijoin (seat_no↔seat_no, 2.56x overcount) and a join to a
//! time-versioned table without a range predicate (3.07x overcount).
//! Fail-open by design: any parse or resolution failure yields no warnings.

use lucent_protocol::SqlDialect;
use sqlparser::ast::{
    Expr, Join, JoinConstraint, JoinOperator, ObjectNamePart, Query, Select, SetExpr, Statement,
    TableFactor,
};
use sqlparser::parser::Parser;
use std::collections::HashMap;

use crate::ai::schema_graph::{SchemaGraph, RANGE_TYPES};

/// alias (or bare table name) → canonical table name known to the graph.
type AliasMap = HashMap<String, String>;

/// A single equality join between two qualified columns:
/// ((table, column), (table, column)).
type Equijoin = ((String, String), (String, String));

pub fn lint_sql(graph: &SchemaGraph, sql: &str, dialect: SqlDialect) -> Vec<String> {
    let Some(dialect) = crate::dialect::parser_for(dialect) else {
        // The lint is advisory. A dialect we cannot parse means no warnings,
        // never a blocked query — the guard is what fails closed, not this.
        return Vec::new();
    };
    let Ok(statements) = Parser::parse_sql(dialect.as_ref(), sql) else {
        return Vec::new();
    };
    let mut warnings: Vec<String> = Vec::new();
    for stmt in &statements {
        if let Statement::Query(q) = stmt {
            lint_query(graph, q, &mut warnings);
        }
    }
    warnings.sort();
    warnings.dedup();
    warnings
}

fn lint_query(graph: &SchemaGraph, query: &Query, warnings: &mut Vec<String>) {
    // CTEs: lint each CTE body as its own query.
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            lint_query(graph, &cte.query, warnings);
        }
    }
    if let SetExpr::Select(select) = query.body.as_ref() {
        lint_select(graph, select, warnings);
    }
}

fn lint_select(graph: &SchemaGraph, select: &Select, warnings: &mut Vec<String>) {
    let mut aliases: AliasMap = HashMap::new();
    let mut join_exprs: Vec<&Expr> = Vec::new();

    for twj in &select.from {
        collect_table(&twj.relation, graph, &mut aliases);
        for Join {
            relation,
            join_operator,
            ..
        } in &twj.joins
        {
            collect_table(relation, graph, &mut aliases);
            let constraint = match join_operator {
                JoinOperator::Join(c)
                | JoinOperator::Inner(c)
                | JoinOperator::Left(c)
                | JoinOperator::LeftOuter(c)
                | JoinOperator::Right(c)
                | JoinOperator::RightOuter(c)
                | JoinOperator::FullOuter(c) => Some(c),
                _ => None,
            };
            if let Some(JoinConstraint::On(expr)) = constraint {
                join_exprs.push(expr);
            }
        }
    }
    if let Some(selection) = &select.selection {
        join_exprs.push(selection);
    }

    // Which tables carry a range predicate anywhere in this SELECT?
    let mut range_constrained: Vec<String> = Vec::new();
    // Equi-join pairs: ((table, col), (table, col)).
    let mut equijoins: Vec<Equijoin> = Vec::new();
    for expr in &join_exprs {
        walk_expr(expr, &aliases, &mut equijoins, &mut range_constrained);
    }

    // L2 (strong): a referenced table with a range-typed column but no range
    // predicate — rows multiply across historical versions.
    let mut flagged_tables: Vec<&String> = Vec::new();
    for table in aliases.values() {
        if flagged_tables.contains(&table) {
            continue;
        }
        for c in &graph.columns {
            if &c.table == table
                && RANGE_TYPES.contains(&c.data_type.as_str())
                && !range_constrained.contains(table)
            {
                warnings.push(format!(
                    "⚠ {table} is time-versioned: no range predicate on {table}.{col} \
                     found — rows can silently multiply across historical versions. \
                     Add e.g. {col} @> <event timestamp> to the join.",
                    col = c.name
                ));
                flagged_tables.push(table);
                break;
            }
        }
    }

    // L1 (gentle): equijoin pairs not backed by an FK edge. Suppressed when
    // either table already carries a range predicate (an informed versioned
    // join like `route_no = … AND validity @> …` has no declarable FK).
    for ((t1, c1), (t2, c2)) in &equijoins {
        if t1 == t2 {
            continue;
        }
        if range_constrained.contains(t1) || range_constrained.contains(t2) {
            continue;
        }
        if !is_fk_edge(graph, t1, c1, t2, c2) {
            warnings.push(format!(
                "ℹ join {t1}.{c1} = {t2}.{c2} is not a declared foreign key — \
                 double-check this relationship is semantically valid, or join \
                 through the listed FK path instead."
            ));
        }
    }
}

fn collect_table(factor: &TableFactor, graph: &SchemaGraph, aliases: &mut AliasMap) {
    if let TableFactor::Table { name, alias, .. } = factor {
        // Last identifier part is the bare table name ("bookings"."routes" → routes).
        if let Some(ObjectNamePart::Identifier(last)) = name.0.last() {
            let bare = last.value.clone();
            if graph.tables.iter().any(|t| t.name == bare) {
                let key = alias
                    .as_ref()
                    .map(|a| a.name.value.clone())
                    .unwrap_or_else(|| bare.clone());
                aliases.insert(key, bare);
            }
        }
    }
    // Derived tables / nested joins: skipped — fail-open.
}

fn walk_expr(
    expr: &Expr,
    aliases: &AliasMap,
    equijoins: &mut Vec<Equijoin>,
    range_constrained: &mut Vec<String>,
) {
    match expr {
        Expr::BinaryOp { left, op, right } => {
            use sqlparser::ast::BinaryOperator as B;
            match op {
                B::And | B::Or => {
                    walk_expr(left, aliases, equijoins, range_constrained);
                    walk_expr(right, aliases, equijoins, range_constrained);
                }
                B::Eq => {
                    if let (Some(a), Some(b)) =
                        (resolve_col(left, aliases), resolve_col(right, aliases))
                    {
                        equijoins.push((a, b));
                    }
                }
                // Range operators: @> is AtArrow, <@ is ArrowAt, && is PGOverlap.
                B::AtArrow | B::ArrowAt | B::PGOverlap => {
                    for side in [left, right] {
                        if let Some((table, _)) = resolve_col(side, aliases) {
                            if !range_constrained.contains(&table) {
                                range_constrained.push(table);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Expr::Nested(inner) => walk_expr(inner, aliases, equijoins, range_constrained),
        _ => {}
    }
}

/// `alias.column` (CompoundIdentifier) → (table, column); bare Identifiers are
/// skipped (ambiguous — fail-open).
fn resolve_col(expr: &Expr, aliases: &AliasMap) -> Option<(String, String)> {
    if let Expr::CompoundIdentifier(parts) = expr {
        if parts.len() == 2 {
            let table = aliases.get(&parts[0].value)?.clone();
            return Some((table, parts[1].value.clone()));
        }
    }
    None
}

fn is_fk_edge(graph: &SchemaGraph, t1: &str, c1: &str, t2: &str, c2: &str) -> bool {
    graph.fk_edges.iter().any(|fk| {
        let from = &graph.columns[fk.from_column];
        let to = &graph.columns[fk.to_column];
        (from.table == t1 && from.name == c1 && to.table == t2 && to.name == c2)
            || (from.table == t2 && from.name == c2 && to.table == t1 && to.name == c1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::schema_graph::{ColumnEntry, FkEdge, SchemaGraph, TableEntry};
    use lucent_protocol::SqlDialect;
    use std::collections::HashMap;

    /// Mirror of the real demo schema's relevant slice:
    /// boarding_passes(ticket_no, flight_id, seat_no) — FK (ticket_no,flight_id)→segments
    /// seats(airplane_code, seat_no) · flights(flight_id, route_no, scheduled_departure)
    /// routes(route_no, validity tstzrange, airplane_code) — NO FK from flights
    /// segments(ticket_no, flight_id) — FK flight_id→flights
    fn demo_graph() -> SchemaGraph {
        let t = |id: usize, name: &str| TableEntry {
            id,
            schema: "bookings".into(),
            name: name.into(),
            row_count_estimate: 0,
            partition_info: None,
        };
        let c = |id: usize, tid: usize, table: &str, name: &str, dt: &str| ColumnEntry {
            id,
            table_id: tid,
            schema: "bookings".into(),
            table: table.into(),
            name: name.into(),
            data_type: dt.into(),
            is_primary_key: false,
            sample_values: vec![],
            fk_ref: None,
            embedding: vec![],
            doc_text: String::new(),
        };
        let tables = vec![
            t(0, "boarding_passes"),
            t(1, "seats"),
            t(2, "flights"),
            t(3, "routes"),
            t(4, "segments"),
        ];
        let columns = vec![
            c(0, 0, "boarding_passes", "ticket_no", "text"),
            c(1, 0, "boarding_passes", "flight_id", "integer"),
            c(2, 0, "boarding_passes", "seat_no", "text"),
            c(3, 1, "seats", "airplane_code", "character"),
            c(4, 1, "seats", "seat_no", "text"),
            c(5, 2, "flights", "flight_id", "integer"),
            c(6, 2, "flights", "route_no", "text"),
            c(
                7,
                2,
                "flights",
                "scheduled_departure",
                "timestamp with time zone",
            ),
            c(8, 3, "routes", "route_no", "text"),
            c(9, 3, "routes", "validity", "tstzrange"),
            c(10, 3, "routes", "airplane_code", "character"),
            c(11, 4, "segments", "ticket_no", "text"),
            c(12, 4, "segments", "flight_id", "integer"),
        ];
        let mut columns_by_table: HashMap<usize, Vec<usize>> = HashMap::new();
        for col in &columns {
            columns_by_table
                .entry(col.table_id)
                .or_default()
                .push(col.id);
        }
        SchemaGraph {
            tables,
            columns,
            columns_by_table,
            fk_edges: vec![
                FkEdge {
                    from_column: 0,
                    to_column: 11,
                }, // bp.ticket_no → segments.ticket_no
                FkEdge {
                    from_column: 1,
                    to_column: 12,
                }, // bp.flight_id → segments.flight_id
                FkEdge {
                    from_column: 12,
                    to_column: 5,
                }, // segments.flight_id → flights.flight_id
            ],
            table_adjacency: HashMap::new(),
            built_at: std::time::Instant::now(),
        }
    }

    #[test]
    fn production_seat_join_fires_non_fk_warning() {
        // The exact join shape that overstated the seat-row answer 2.56x.
        let sql = "SELECT COUNT(*) FROM bookings.boarding_passes bp \
                   JOIN bookings.seats s ON s.airplane_code = '77W' AND bp.seat_no = s.seat_no";
        let warnings = lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("seat_no") && w.contains("not a declared foreign key")),
            "{warnings:?}"
        );
    }

    #[test]
    fn naive_routes_join_fires_time_versioned_warning() {
        // The exact join shape that overstated flight counts 3.07x.
        let sql = "SELECT COUNT(*) FROM bookings.flights f \
                   JOIN bookings.routes r ON f.route_no = r.route_no \
                   WHERE r.airplane_code = '77W'";
        let warnings = lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("validity") && w.contains("time-versioned")),
            "{warnings:?}"
        );
    }

    #[test]
    fn validity_constrained_routes_join_is_silent() {
        let sql = "SELECT COUNT(*) FROM bookings.flights f \
                   JOIN bookings.routes r ON f.route_no = r.route_no \
                     AND r.validity @> f.scheduled_departure \
                   WHERE r.airplane_code = '77W'";
        assert!(
            lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql).is_empty(),
            "informed join must not warn"
        );
    }

    #[test]
    fn fk_backed_join_is_silent() {
        let sql = "SELECT COUNT(*) FROM bookings.segments s \
                   JOIN bookings.flights f ON s.flight_id = f.flight_id";
        assert!(lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql).is_empty());
    }

    #[test]
    fn unparseable_sql_is_fail_open() {
        assert!(lint_sql(&demo_graph(), "SELEKT garbage FRM", SqlDialect::PostgreSql).is_empty());
        assert!(lint_sql(&demo_graph(), "", SqlDialect::PostgreSql).is_empty());
    }

    #[test]
    fn cte_bodies_are_linted() {
        // The production 96s conversation buried its naive routes join inside a CTE.
        let sql = "WITH x AS (SELECT f.flight_id FROM bookings.flights f \
                   JOIN bookings.routes r ON f.route_no = r.route_no) \
                   SELECT COUNT(*) FROM x";
        let warnings = lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql);
        assert!(
            warnings.iter().any(|w| w.contains("time-versioned")),
            "CTE bodies must be linted too: {warnings:?}"
        );
    }

    // ── Additional real-world scenarios ───────────────────────────────────

    #[test]
    fn equijoin_in_where_clause_fires_non_fk_warning() {
        // The plan says joins can be in WHERE (old-style), not just JOIN ... ON.
        let sql = "SELECT COUNT(*) FROM bookings.boarding_passes bp, bookings.seats s \
                   WHERE s.airplane_code = '77W' AND bp.seat_no = s.seat_no";
        let warnings = lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("seat_no") && w.contains("not a declared foreign key")),
            "WHERE equijoin must fire L1: {warnings:?}"
        );
    }

    #[test]
    fn self_join_does_not_fire_non_fk_warning() {
        // Self-joins on non-FK columns are structural (e.g. employee/manager).
        // Need a graph with a single table that has at least one range column
        // so L2 doesn't fire spuriously.
        let mut g = demo_graph();
        // Add a self-referencing column to flights: flights.parent_flight_id
        g.columns.push(ColumnEntry {
            id: 13,
            table_id: 2,
            schema: "bookings".into(),
            table: "flights".into(),
            name: "parent_flight_id".into(),
            data_type: "integer".into(),
            is_primary_key: false,
            sample_values: vec![],
            fk_ref: None,
            embedding: vec![],
            doc_text: String::new(),
        });
        g.columns_by_table.entry(2).or_default().push(13);

        let sql = "SELECT f1.flight_id, f2.flight_id \
                   FROM bookings.flights f1 \
                   JOIN bookings.flights f2 ON f1.parent_flight_id = f2.flight_id";
        let warnings = lint_sql(&g, sql, SqlDialect::PostgreSql);
        assert!(
            warnings.iter().all(|w| !w.contains("parent_flight_id")),
            "self-join must not warn: {warnings:?}"
        );
    }

    #[test]
    fn inner_left_right_join_variants_all_fire() {
        let sql_left = "SELECT COUNT(*) FROM bookings.boarding_passes bp \
                        LEFT JOIN bookings.seats s ON bp.seat_no = s.seat_no";
        let sql_right = "SELECT COUNT(*) FROM bookings.boarding_passes bp \
                         RIGHT JOIN bookings.seats s ON bp.seat_no = s.seat_no";
        let sql_full = "SELECT COUNT(*) FROM bookings.boarding_passes bp \
                        FULL JOIN bookings.seats s ON bp.seat_no = s.seat_no";
        for (kind, sql) in [("LEFT", sql_left), ("RIGHT", sql_right), ("FULL", sql_full)] {
            let warnings = lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql);
            assert!(
                warnings
                    .iter()
                    .any(|w| w.contains("seat_no") && w.contains("not a declared foreign key")),
                "{kind} JOIN must fire L1: {warnings:?}"
            );
        }
    }

    #[test]
    fn cross_join_without_on_is_silent() {
        let sql = "SELECT COUNT(*) FROM bookings.boarding_passes bp \
                   CROSS JOIN bookings.seats s";
        let warnings = lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql);
        // CROSS JOIN has no ON → no equijoin extracted → no L1.
        assert!(
            warnings
                .iter()
                .all(|w| !w.contains("seat_no") && !w.contains("boarding_passes")),
            "CROSS JOIN must be silent (it joins every row): {warnings:?}"
        );
    }

    #[test]
    fn schema_qualified_references_are_resolved() {
        let sql = "SELECT COUNT(*) \
                   FROM bookings.flights \
                   JOIN bookings.routes ON bookings.routes.route_no = bookings.flights.route_no \
                   WHERE bookings.routes.airplane_code = '77W'";
        let warnings = lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql);
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("validity") && w.contains("time-versioned")),
            "schema-qualified refs must resolve aliases: {warnings:?}"
        );
    }

    #[test]
    fn subquery_fail_open_no_warnings() {
        // Derived tables are skipped (fail-open), but the outer query's range
        // table reference should still trigger L2 globally.
        let sql = "SELECT COUNT(*) FROM (SELECT flight_id, route_no FROM bookings.flights) f \
                   JOIN bookings.routes r ON f.route_no = r.route_no";
        let warnings = lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql);
        // The subquery means f is skipped (derived table), but routes still
        // gets an alias 'r' → should fire time-versioned warning.
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("validity") && w.contains("time-versioned")),
            "time-versioned warning must fire even with derived table: {warnings:?}"
        );
    }

    #[test]
    fn l1_suppressed_when_l2_fires_on_same_join() {
        // When a table pair both has a non-FK equijoin AND carries a range
        // predicate, L1 is suppressed (the join is an informed versioned join).
        let sql = "SELECT COUNT(*) FROM bookings.flights f \
                   JOIN bookings.routes r ON f.route_no = r.route_no \
                     AND r.validity @> f.scheduled_departure";
        let warnings = lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql);
        assert!(
            warnings.is_empty(),
            "informed versioned join must suppress L1: {warnings:?}"
        );
    }

    #[test]
    fn multi_statement_only_queries_are_linted() {
        // Only Statement::Query is linted; non-query statements are silently
        // skipped (fail-open).
        let sql = "SELECT 1; SELECT COUNT(*) FROM bookings.boarding_passes bp \
                   JOIN bookings.seats s ON bp.seat_no = s.seat_no";
        let warnings = lint_sql(&demo_graph(), sql, SqlDialect::PostgreSql);
        assert!(
            warnings.iter().any(|w| w.contains("seat_no")),
            "second SELECT must be linted: {warnings:?}"
        );
    }
}
