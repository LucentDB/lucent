//! Renders the SchemaGraph into LLM-facing context blocks and picks the
//! context tier for a connection.
//!
//! Tiers (thresholds are enriched-token estimates, chars/4):
//! - Push:   full M-Schema fits the budget → inject everything; the model
//!   writes SQL without any schema-exploration round trips.
//! - Hybrid: too big to push, but a one-line-per-table index fits → inject
//!   the index; per-question detail arrives via pre-flight retrieval.
//! - Pull:   even the index is too big → per-schema counts only; the model
//!   relies on search_schema.

use crate::ai::schema_graph::SchemaGraph;

pub const PUSH_BUDGET_TOKENS: usize = 15_000;
pub const HYBRID_BUDGET_TOKENS: usize = 8_000;
/// Wide-table cap: PK/FK columns always render; the rest render in ordinal
/// order up to this total, then an elision marker points at get_objects_info.
pub const MAX_COLS_PER_TABLE: usize = 40;
const MAX_EXAMPLES_PER_COL: usize = 5;
const MAX_EXAMPLE_CHARS: usize = 40;

#[derive(Clone, Debug, PartialEq)]
pub enum ContextTier {
    Push,
    Hybrid,
    Pull,
}

/// Cheap token estimate (~4 chars/token for English + SQL identifiers).
pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4
}

fn header(t: &crate::ai::schema_graph::TableEntry) -> String {
    let mut h = format!(
        "# Table: {}.{} — ~{} rows",
        t.schema, t.name, t.row_count_estimate
    );
    if let Some(p) = &t.partition_info {
        h.push_str(&format!(" [{p}]"));
    }
    h
}

fn column_line(c: &crate::ai::schema_graph::ColumnEntry) -> String {
    let mut parts = vec![format!("{}: {}", c.name, c.data_type)];
    if c.is_primary_key {
        parts.push("PK".into());
    }
    if let Some(fk) = &c.fk_ref {
        parts.push(format!("FK → {fk}"));
    }
    if crate::ai::schema_graph::RANGE_TYPES.contains(&c.data_type.as_str()) {
        parts.push(format!(
            "time-versioned — every join to {} MUST also constrain this column \
             (e.g. {} @> <event timestamp>) or rows silently multiply",
            c.table, c.name
        ));
    }
    if !c.sample_values.is_empty() {
        let shown: Vec<String> = c
            .sample_values
            .iter()
            .take(MAX_EXAMPLES_PER_COL)
            .map(|v| {
                if v.len() > MAX_EXAMPLE_CHARS {
                    format!(
                        "{}…",
                        &v[..v
                            .char_indices()
                            .take(MAX_EXAMPLE_CHARS)
                            .last()
                            .map(|(i, ch)| i + ch.len_utf8())
                            .unwrap_or(0)]
                    )
                } else {
                    v.clone()
                }
            })
            .collect();
        parts.push(format!("examples: {}", shown.join(", ")));
    }
    format!("({})", parts.join(", "))
}

/// Full M-Schema: every logical table with typed, key-flagged, value-grounded
/// column tuples, followed by the complete FK edge list.
pub fn render_m_schema(graph: &SchemaGraph) -> String {
    let mut out: Vec<String> = Vec::new();
    for t in &graph.tables {
        out.push(header(t));
        let Some(col_ids) = graph.columns_by_table.get(&t.id) else {
            continue;
        };

        // Keys always render; the remainder renders in ordinal order up to the cap.
        let (keys, rest): (Vec<usize>, Vec<usize>) = col_ids.iter().copied().partition(|&cid| {
            let c = &graph.columns[cid];
            c.is_primary_key || c.fk_ref.is_some()
        });
        let budget_for_rest = MAX_COLS_PER_TABLE.saturating_sub(keys.len());
        let elided = rest.len().saturating_sub(budget_for_rest);

        for &cid in keys.iter().chain(rest.iter().take(budget_for_rest)) {
            out.push(column_line(&graph.columns[cid]));
        }
        if elided > 0 {
            out.push(format!(
                "… +{elided} more columns — call get_objects_info for the full list"
            ));
        }
        out.push(String::new());
    }

    if !graph.fk_edges.is_empty() {
        out.push("Foreign keys:".into());
        for fk in &graph.fk_edges {
            let from = &graph.columns[fk.from_column];
            let to = &graph.columns[fk.to_column];
            out.push(format!(
                "{}.{} → {}.{}",
                from.table, from.name, to.table, to.name
            ));
        }
    }
    out.join("\n")
}

/// One line per table: qualified name, row estimate, column count, key columns.
pub fn render_compact_index(graph: &SchemaGraph) -> String {
    let mut out: Vec<String> = Vec::new();
    for t in &graph.tables {
        let col_ids = graph
            .columns_by_table
            .get(&t.id)
            .cloned()
            .unwrap_or_default();
        let keys: Vec<String> = col_ids
            .iter()
            .filter_map(|&cid| {
                let c = &graph.columns[cid];
                if c.is_primary_key {
                    Some(format!("{} PK", c.name))
                } else {
                    c.fk_ref.as_ref().map(|fk| format!("{}→{}", c.name, fk))
                }
            })
            .collect();
        let mut line = format!(
            "{}.{} — ~{} rows, {} cols",
            t.schema,
            t.name,
            t.row_count_estimate,
            col_ids.len()
        );
        if !keys.is_empty() {
            line.push_str(&format!(" ({})", keys.join(", ")));
        }
        if let Some(p) = &t.partition_info {
            line.push_str(&format!(" [{p}]"));
        }
        out.push(line);
    }
    out.join("\n")
}

/// Per-schema table counts — the last resort when even the index is too big.
fn render_counts(graph: &SchemaGraph) -> String {
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for t in &graph.tables {
        *counts.entry(t.schema.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(s, n)| format!("Schema \"{s}\": {n} tables"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Pick the context tier and return its rendered dynamic schema block.
pub fn select_tier(graph: &SchemaGraph) -> (ContextTier, String) {
    let m = render_m_schema(graph);
    if estimate_tokens(&m) <= PUSH_BUDGET_TOKENS {
        return (ContextTier::Push, m);
    }
    let index = render_compact_index(graph);
    if estimate_tokens(&index) <= HYBRID_BUDGET_TOKENS {
        return (ContextTier::Hybrid, index);
    }
    (ContextTier::Pull, render_counts(graph))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::schema_graph::{ColumnEntry, FkEdge, SchemaGraph, TableEntry};
    use std::collections::HashMap;

    #[allow(clippy::too_many_arguments)] // test helper — arg count is inherent
    fn col(
        id: usize,
        tid: usize,
        table: &str,
        name: &str,
        dt: &str,
        pk: bool,
        fk: Option<&str>,
        vals: Vec<&str>,
    ) -> ColumnEntry {
        ColumnEntry {
            id,
            table_id: tid,
            schema: "bookings".into(),
            table: table.into(),
            name: name.into(),
            data_type: dt.into(),
            is_primary_key: pk,
            sample_values: vals.into_iter().map(String::from).collect(),
            fk_ref: fk.map(String::from),
            embedding: vec![],
            doc_text: String::new(),
        }
    }

    fn graph(
        tables: Vec<TableEntry>,
        columns: Vec<ColumnEntry>,
        fk_edges: Vec<FkEdge>,
    ) -> SchemaGraph {
        let mut columns_by_table: HashMap<usize, Vec<usize>> = HashMap::new();
        for c in &columns {
            columns_by_table.entry(c.table_id).or_default().push(c.id);
        }
        SchemaGraph {
            tables,
            columns,
            columns_by_table,
            fk_edges,
            table_adjacency: HashMap::new(),
            built_at_unix: 0,
            tier: crate::ai::schema_graph::IndexingTier::MetadataOnly,
        }
    }

    fn small_graph() -> SchemaGraph {
        graph(
            vec![
                TableEntry {
                    id: 0,
                    schema: "bookings".into(),
                    name: "flights".into(),
                    row_count_estimate: 214_867,
                    partition_info: None,
                },
                TableEntry {
                    id: 1,
                    schema: "bookings".into(),
                    name: "routes".into(),
                    row_count_estimate: 710,
                    partition_info: None,
                },
            ],
            vec![
                col(0, 0, "flights", "flight_id", "integer", true, None, vec![]),
                col(
                    1,
                    0,
                    "flights",
                    "route_no",
                    "character",
                    false,
                    Some("routes.route_no"),
                    vec![],
                ),
                col(
                    2,
                    0,
                    "flights",
                    "status",
                    "text",
                    false,
                    None,
                    vec!["Arrived", "Cancelled", "Delayed"],
                ),
                col(3, 1, "routes", "route_no", "character", true, None, vec![]),
            ],
            vec![FkEdge {
                from_column: 1,
                to_column: 3,
            }],
        )
    }

    #[test]
    fn m_schema_lists_columns_with_types_keys_and_examples() {
        let out = render_m_schema(&small_graph());
        assert!(
            out.contains("# Table: bookings.flights"),
            "table header: {out}"
        );
        assert!(out.contains("~214867 rows"), "row estimate in header");
        assert!(out.contains("(flight_id: integer, PK)"));
        assert!(out.contains("(route_no: character, FK \u{2192} routes.route_no)"));
        assert!(out.contains("(status: text, examples: Arrived, Cancelled, Delayed)"));
    }

    #[test]
    fn m_schema_includes_foreign_key_edge_list() {
        let out = render_m_schema(&small_graph());
        assert!(out.contains("Foreign keys:"));
        assert!(out.contains("flights.route_no \u{2192} routes.route_no"));
    }

    #[test]
    fn m_schema_shows_partition_annotation_in_header() {
        let mut g = small_graph();
        g.tables[0].partition_info =
            Some("PARTITIONED BY RANGE (created_at) — 84 partitions".into());
        let out = render_m_schema(&g);
        assert!(
            out.contains("PARTITIONED BY RANGE"),
            "partition strategy visible: {out}"
        );
    }

    #[test]
    fn m_schema_caps_wide_tables_with_keys_first() {
        let mut columns = vec![
            col(0, 0, "wide", "pk_col", "integer", true, None, vec![]),
            col(
                1,
                0,
                "wide",
                "fk_col",
                "integer",
                false,
                Some("other.id"),
                vec![],
            ),
        ];
        for i in 2..60 {
            columns.push(col(
                i,
                0,
                "wide",
                &format!("col_{i}"),
                "text",
                false,
                None,
                vec![],
            ));
        }
        let g = graph(
            vec![TableEntry {
                id: 0,
                schema: "bookings".into(),
                name: "wide".into(),
                row_count_estimate: 10,
                partition_info: None,
            }],
            columns,
            vec![],
        );
        let out = render_m_schema(&g);
        assert!(out.contains("pk_col"), "PK always shown");
        assert!(out.contains("fk_col"), "FK always shown");
        assert!(
            out.contains("more columns"),
            "elision marker for a 60-column table"
        );
        assert!(
            out.contains("+20 more columns"),
            "60 cols with cap 40 elides exactly 20: {out}"
        );
        assert!(
            out.contains("get_objects_info"),
            "tells the model how to get the rest"
        );
    }

    #[test]
    fn compact_index_is_one_line_per_table() {
        let out = render_compact_index(&small_graph());
        let flights_lines: Vec<&str> = out
            .lines()
            .filter(|l| l.contains("bookings.flights"))
            .collect();
        assert_eq!(flights_lines.len(), 1, "exactly one line per table: {out}");
        assert!(flights_lines[0].contains("~214867 rows"));
        assert!(flights_lines[0].contains("3 cols"));
    }

    #[test]
    fn tiny_schema_selects_push_with_full_m_schema() {
        let (tier, block) = select_tier(&small_graph());
        assert_eq!(tier, ContextTier::Push);
        assert!(
            block.contains("(status: text"),
            "push tier carries full column detail"
        );
    }

    #[test]
    fn large_schema_selects_hybrid_with_compact_index() {
        let mut tables = vec![];
        let mut columns = vec![];
        let mut cid = 0;
        for t in 0..400 {
            tables.push(TableEntry {
                id: t,
                schema: "public".into(),
                name: format!("table_number_{t}"),
                row_count_estimate: 100,
                partition_info: None,
            });
            for c in 0..12 {
                columns.push(col(
                    cid,
                    t,
                    &format!("table_number_{t}"),
                    &format!("column_name_{c}"),
                    "text",
                    c == 0,
                    None,
                    vec![],
                ));
                cid += 1;
            }
        }
        let g = graph(tables, columns, vec![]);
        assert!(
            estimate_tokens(&render_m_schema(&g)) > PUSH_BUDGET_TOKENS,
            "fixture must exceed push budget"
        );
        let (tier, block) = select_tier(&g);
        assert_eq!(tier, ContextTier::Hybrid);
        assert!(
            block.contains("table_number_399"),
            "hybrid tier lists every table by name"
        );
        assert!(
            !block.contains("column_name_5"),
            "hybrid tier does not carry per-column detail"
        );
    }

    #[test]
    fn estimate_tokens_is_chars_over_four() {
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    #[test]
    fn range_typed_column_renders_time_versioned_warning_into_prompt() {
        let g = graph(
            vec![TableEntry {
                id: 0,
                schema: "bookings".into(),
                name: "routes".into(),
                row_count_estimate: 710,
                partition_info: None,
            }],
            vec![col(
                0,
                0,
                "routes",
                "validity",
                "tstzrange",
                false,
                None,
                vec![],
            )],
            vec![],
        );
        let out = render_m_schema(&g);
        assert!(
            out.contains("time-versioned") && out.contains("validity @>"),
            "the join hazard must reach the MODEL, not just the embeddings — \
             a naive routes join overcounted 3.07x in production: {out}"
        );
    }
}
