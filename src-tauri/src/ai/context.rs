use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::ai::mschema::{self, ContextTier};
use crate::ai::schema_graph::SchemaGraph;

const LINE_BUDGET: usize = 150;

#[derive(Clone, Debug, Serialize)]
pub struct SchemaTree {
    pub database_name: String,
    pub server_version: String,
    pub schemas: Vec<SchemaNode>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SchemaNode {
    pub name: String,
    pub tables: Vec<String>,
    pub views: Vec<String>,
    pub functions: Vec<String>,
}

/// Build the system prompt from a schema tree.
/// Static content (tools, RULES) comes FIRST before dynamic schema content
/// so prompt caching sees a stable prefix across different connected databases.
/// Build the system prompt. Static content (tools, RULES) comes FIRST before
/// dynamic schema content so prompt caching sees a stable prefix. When a
/// SchemaGraph is available, the dynamic section is tier-selected:
/// Push = full M-Schema, Hybrid = compact index, Pull = counts.
// The static prefix is authored as line-by-line `push` calls so each prompt
// line stays readable and diffable; converting to a single `vec![]` literal
// would hurt maintainability without changing behavior.
#[allow(clippy::vec_init_then_push)]
pub fn build_system_prompt(
    schema: &SchemaTree,
    graph: Option<&SchemaGraph>,
    _send_results_to_ai: bool,
) -> String {
    let mut lines: Vec<String> = vec![];

    // ── Static prefix (unchanged across databases — cache-friendly) ─────
    lines.push("AVAILABLE TOOLS:".into());
    lines.push(String::new());
    lines.push("1. search_schema — Search database schema by meaning or by name. Use".into());
    lines.push("   mode=\"semantic\" for natural-language questions about what data lives".into());
    lines.push("   where (e.g. \"which table has unpaid invoices\"); mode=\"keyword\" when".into());
    lines.push(
        "   you know part of an exact table or column name; mode=\"hybrid\" (default)".into(),
    );
    lines.push("   tries both.".into());
    lines.push("   Args: {\"query\":\"search term\", \"mode\":\"semantic\"}".into());
    lines.push(String::new());
    lines.push("2. get_objects_info — Get columns, types, constraints for tables/views.".into());
    lines.push("   Args: {\"objects\":[{\"schema\":\"public\",\"kind\":\"table\",\"name\":\"tablename\"}]}".into());
    lines.push(String::new());
    lines.push(
        "3. run_readonly_query — Execute SELECT/WITH/EXPLAIN (read-only). Results auto-display."
            .into(),
    );
    lines.push(String::new());
    lines.push(
        "4. preview_dml — Preview INSERT/UPDATE/DELETE (never executes). Pauses for user approval."
            .into(),
    );
    lines.push(String::new());
    lines.push("RULES:".into());
    lines.push(
        "- Never query information_schema or pg_catalog directly. Use the tools above.".into(),
    );
    lines.push("- For INSERT/UPDATE/DELETE use preview_dml only — user must confirm.".into());
    lines.push(
        "- One DML statement per preview_dml call. Never submit multi-statement batches.".into(),
    );
    lines.push(
        "- After run_readonly_query you receive a Markdown table preview of the data.".into(),
    );
    lines.push(
        "- Read the preview carefully. If it contains the data you need, ANSWER the user.".into(),
    );
    lines.push("- DO NOT re-query the same data. You already have it in the preview.".into());
    lines.push("- EFFICIENCY: Each tool call costs time and tokens. Call tools only when".into());
    lines.push("  you genuinely need new information. If you can answer from what you".into());
    lines.push("  already know or from the data preview, provide your answer immediately.".into());
    lines.push("- DECISION FLOW: After receiving tool results, check: do I have enough".into());
    lines.push(
        "  information to answer the user's question? If YES, answer now. If NO, call".into(),
    );
    lines.push("  one more tool. Never call a tool that repeats a previous query.".into());
    lines.push("- COMPLETE QUERIES: When a question implies more than one related metric".into());
    lines.push(
        "  (e.g. \"delay\" usually means both departure AND arrival delay), write ONE".into(),
    );
    lines.push("  query covering all of them. Do not run a narrow query first and then a".into());
    lines.push("  near-duplicate query to add one more column — decide what the complete".into());
    lines.push("  answer needs before writing SQL.".into());
    lines.push(
        "- PARALLEL TOOL CALLS: If you need information from more than one independent".into(),
    );
    lines.push(
        "  source (e.g. schema details for several unrelated tables, or a schema lookup".into(),
    );
    lines.push(
        "  alongside a data preview), request all of them in the same turn instead of".into(),
    );
    lines.push(
        "  one at a time. Only go sequential when a later call genuinely depends on an".into(),
    );
    lines.push("  earlier call's result.".into());
    lines.push(
        "- ONE well-chosen search_schema call beats several narrow ones: the tool already".into(),
    );
    lines.push(
        "  expands to related tables via foreign keys, so a single broader query (e.g.".into(),
    );
    lines.push(
        "  \"ticket passenger and flight segments\" instead of two separate queries for".into(),
    );
    lines.push(
        "  \"passenger name\" and \"flight segments\") usually surfaces everything needed".into(),
    );
    lines.push("  in one call.".into());
    lines.push("- AMBIGUOUS METRICS: When a question has more than one reasonable".into());
    lines.push("  interpretation (e.g. \"busiest\" could mean most flights or most".into());
    lines.push("  passengers), pick the most direct interpretation, run ONE query for it,".into());
    lines.push("  state in your answer which interpretation you used, and offer to compute".into());
    lines.push("  the alternative. Do NOT run queries for every interpretation unprompted.".into());
    lines.push("- TIES: In top-N questions, anticipate ties by using DENSE_RANK() in your".into());
    lines.push("  FIRST ranking query. If results come back tied anyway, report the tie".into());
    lines.push("  as-is — never re-run near-duplicate variants of a query to investigate".into());
    lines.push("  a tie you can already describe from the data you have.".into());
    lines.push("- JOIN DISCIPLINE: Join tables ONLY along the foreign-key paths listed in".into());
    lines.push("  the schema. An equality join on same-named columns that is NOT a listed".into());
    lines.push("  FK (e.g. joining two tables on seat_no) is almost always semantically".into());
    lines.push("  wrong. If you genuinely need one, justify it explicitly in your answer.".into());
    lines.push("- TIME-VERSIONED TABLES: A table with a range column (e.g. validity".into());
    lines.push("  tstzrange) stores multiple historical versions per key. EVERY join to it".into());
    lines
        .push("  must constrain the range (validity @> <event timestamp>) or rows silently".into());
    lines.push("  multiply and every aggregate downstream is wrong.".into());
    lines.push("- NO REFORMAT RE-RUNS: Never re-run a query just to change ORDER BY, add".into());
    lines.push("  derived columns (percentages, ranks), or add comments. Present from data".into());
    lines.push("  you already retrieved — the UI shows full results.".into());
    lines.push("- TRUST THE SCHEMA: Sample values shown in the schema are real data from".into());
    lines.push("  this database. Do not run exploratory LIMIT queries to see what values".into());
    lines.push("  look like — you already have them.".into());
    lines.push(String::new());
    lines.push(String::new());

    // ── Tier-specific guidance ────────────────────────────────────────────
    let tier = graph.map(mschema::select_tier);
    match tier.as_ref().map(|(t, _)| t) {
        Some(ContextTier::Push) => {
            lines.push("SCHEMA MODE: The complete schema — every table, column, type, key,".into());
            lines.push(
                "and sample values — is included below. You do not need search_schema".into(),
            );
            lines.push("or get_objects_info for schema structure:".into());
            lines.push("write SQL directly with run_readonly_query. Only reach for those".into());
            lines.push("tools if something below seems missing or ambiguous.".into());
        }
        Some(ContextTier::Hybrid) => {
            lines
                .push("SCHEMA MODE: A one-line index of every table is below, and detailed".into());
            lines.push(
                "context for the tables most relevant to each question is attached to".into(),
            );
            lines.push(
                "the user's message. Use search_schema only when you need detail on a".into(),
            );
            lines.push("table that wasn't included.".into());
        }
        Some(ContextTier::Pull) | None => { /* existing behavior, no extra guidance */ }
    }
    lines.push(String::new());

    // ── Dynamic content (changes per database / per connection) ─────────
    lines.push(format!(
        "You are connected to database \"{}\" ({}).",
        schema.database_name, schema.server_version
    ));
    lines.push(String::new());

    match tier {
        Some((_, block)) => {
            lines.push("Database schema:".into());
            lines.push(block);
        }
        None => {
            // legacy SchemaTree rendering (no graph available)
            let use_compact = count_verbose_lines(schema) > LINE_BUDGET;
            if use_compact {
                lines.push("Database structure (large — use search_schema to find tables):".into());
                lines.push(String::new());
                for n in &schema.schemas {
                    lines.push(format!(
                        "  Schema \"{}\": {} tables, {} views, {} functions",
                        n.name,
                        n.tables.len(),
                        n.views.len(),
                        n.functions.len()
                    ));
                }
            } else {
                lines.push("Database structure:".into());
                for n in &schema.schemas {
                    lines.push(format!("Schema \"{}\":", n.name));
                    if !n.tables.is_empty() {
                        lines.push(format!("  Tables: {}", n.tables.join(", ")));
                    }
                    if !n.views.is_empty() {
                        lines.push(format!("  Views: {}", n.views.join(", ")));
                    }
                    if !n.functions.is_empty() {
                        lines.push("  Functions:".into());
                        for f in &n.functions {
                            lines.push(format!("    {f}"));
                        }
                    }
                }
            }
        }
    }

    lines.join("\n")
}

/// Derive a SchemaTree from the in-memory SchemaGraph. Used when the
/// TTL-bound tree cache has expired but the graph (which has no TTL and
/// strictly more information) is available — the system prompt must NEVER
/// degrade to "context not loaded" while a graph exists. Views/functions
/// are omitted: with a graph present, the dynamic prompt body comes from
/// the tier renderer, and the tree only contributes the header line.
pub fn tree_from_graph(
    database_name: String,
    graph: &crate::ai::schema_graph::SchemaGraph,
) -> SchemaTree {
    let mut by_schema: std::collections::BTreeMap<&str, Vec<String>> =
        std::collections::BTreeMap::new();
    for t in &graph.tables {
        by_schema
            .entry(t.schema.as_str())
            .or_default()
            .push(t.name.clone());
    }
    SchemaTree {
        database_name,
        server_version: String::new(),
        schemas: by_schema
            .into_iter()
            .map(|(name, tables)| SchemaNode {
                name: name.to_string(),
                tables,
                views: vec![],
                functions: vec![],
            })
            .collect(),
    }
}

fn count_verbose_lines(schema: &SchemaTree) -> usize {
    let mut c = 1;
    for n in &schema.schemas {
        c += 1;
        if !n.tables.is_empty() {
            c += 1;
        }
        if !n.views.is_empty() {
            c += 1;
        }
        if !n.functions.is_empty() {
            c += 1 + n.functions.len();
        }
    }
    c
}

/// Per-connection schema cache with TTL.
pub struct SchemaCache {
    inner: Mutex<HashMap<String, CacheEntry>>,
    ttl: Duration,
}

struct CacheEntry {
    tree: SchemaTree,
    fetched_at: Instant,
}

impl SchemaCache {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    pub fn get(&self, conn_id: &str) -> Option<SchemaTree> {
        let g = self.inner.lock().ok()?;
        let e = g.get(conn_id)?;
        let valid = e.fetched_at.elapsed() < self.ttl;
        if !valid {
            log::debug!("Schema cache expired for {conn_id}");
        }
        valid.then(|| e.tree.clone())
    }

    pub fn set(&self, conn_id: String, tree: SchemaTree) {
        log::debug!(
            "Schema cache updated for {conn_id} ({} schemas)",
            tree.schemas.len()
        );
        if let Ok(mut g) = self.inner.lock() {
            g.insert(
                conn_id,
                CacheEntry {
                    tree,
                    fetched_at: Instant::now(),
                },
            );
        }
    }

    pub fn invalidate(&self, conn_id: &str) {
        if let Ok(mut g) = self.inner.lock() {
            g.remove(conn_id);
        }
    }

    /// Clear all cached schema trees.
    pub fn clear(&self) {
        if let Ok(mut g) = self.inner.lock() {
            g.clear();
        }
    }

    /// Fetch the schema tree from a live database client and cache it.
    pub async fn refresh(
        &self,
        conn_id: String,
        client: &mut crate::client::ConnectorClient,
    ) -> Result<SchemaTree, String> {
        // Get schemas
        let schema_rows = client
            .execute(
                "SELECT s.schema_name \
                 FROM information_schema.schemata s \
                 WHERE s.schema_name NOT IN ('pg_catalog', 'information_schema', 'pg_toast') \
                 ORDER BY s.schema_name",
            )
            .await
            .map_err(|e| format!("failed to fetch schemas: {e}"))?;

        let mut schemas = Vec::new();

        for row in &schema_rows.rows {
            let schema_name = row[0].as_str().unwrap_or("public").to_string();

            // Tables (skip partition children — they're collapsed into the parent)
            let tables = client
                .execute(&format!(
                    "SELECT c.relname FROM pg_class c \
                     JOIN pg_namespace n ON n.oid = c.relnamespace \
                     LEFT JOIN pg_inherits i ON i.inhrelid = c.oid \
                     WHERE n.nspname = '{}' AND c.relkind IN ('r', 'p') \
                       AND i.inhrelid IS NULL \
                     ORDER BY c.relname",
                    schema_name.replace('\'', "''"),
                ))
                .await
                .map_err(|e| format!("failed to fetch tables for {schema_name}: {e}"))?;
            let table_names: Vec<String> = tables
                .rows
                .iter()
                .filter_map(|r| r[0].as_str().map(String::from))
                .collect();

            // Views
            let views = client
                .execute(&format!(
                    "SELECT table_name FROM information_schema.views \
                     WHERE table_schema = '{}' ORDER BY table_name",
                    schema_name.replace('\'', "''"),
                ))
                .await
                .map_err(|e| format!("failed to fetch views for {schema_name}: {e}"))?;
            let view_names: Vec<String> = views
                .rows
                .iter()
                .filter_map(|r| r[0].as_str().map(String::from))
                .collect();

            // Functions
            let funcs = client
                .execute(&format!(
                    "SELECT p.proname FROM pg_proc p \
                     JOIN pg_namespace n ON p.pronamespace = n.oid \
                     WHERE n.nspname = '{}' AND p.prokind = 'f' \
                     ORDER BY p.proname",
                    schema_name.replace('\'', "''"),
                ))
                .await
                .map_err(|e| format!("failed to fetch functions for {schema_name}: {e}"))?;
            let func_names: Vec<String> = funcs
                .rows
                .iter()
                .filter_map(|r| r[0].as_str().map(String::from))
                .collect();

            schemas.push(SchemaNode {
                name: schema_name,
                tables: table_names,
                views: view_names,
                functions: func_names,
            });
        }

        let tree = SchemaTree {
            database_name: conn_id.rsplit('/').next().unwrap_or(&conn_id).to_string(),
            server_version: String::new(),
            schemas,
        };

        log::info!(
            "Schema cache refreshed for {conn_id}: {} schemas, {} total objects",
            tree.schemas.len(),
            tree.schemas
                .iter()
                .map(|s| s.tables.len() + s.views.len() + s.functions.len())
                .sum::<usize>(),
        );
        self.set(conn_id, tree.clone());
        Ok(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> SchemaTree {
        SchemaTree {
            database_name: "testdb".into(),
            server_version: "PostgreSQL 16".into(),
            schemas: vec![SchemaNode {
                name: "public".into(),
                tables: vec!["users".into(), "orders".into()],
                views: vec!["active_users".into()],
                functions: vec!["calc_discount(amount decimal) -> decimal".into()],
            }],
        }
    }

    fn graph_for_prompt() -> crate::ai::schema_graph::SchemaGraph {
        use crate::ai::schema_graph::{ColumnEntry, SchemaGraph, TableEntry};
        use std::collections::HashMap;
        let columns = vec![ColumnEntry {
            id: 0,
            table_id: 0,
            schema: "public".into(),
            table: "invoices".into(),
            name: "status".into(),
            data_type: "text".into(),
            is_primary_key: false,
            sample_values: vec!["pending".into(), "paid".into()],
            fk_ref: None,
            embedding: vec![],
            doc_text: String::new(),
        }];
        SchemaGraph {
            tables: vec![TableEntry {
                id: 0,
                schema: "public".into(),
                name: "invoices".into(),
                row_count_estimate: 42,
                partition_info: None,
            }],
            columns_by_table: HashMap::from([(0, vec![0])]),
            columns,
            fk_edges: vec![],
            table_adjacency: HashMap::new(),
            built_at: std::time::Instant::now(),
        }
    }

    #[test]
    fn push_tier_injects_full_column_detail_and_direct_sql_guidance() {
        let g = graph_for_prompt();
        let p = build_system_prompt(&small(), Some(&g), false);
        assert!(
            p.contains("(status: text, examples: pending, paid)"),
            "push tier must carry the full M-Schema: {p}"
        );
        assert!(
            p.contains("write SQL directly with run_readonly_query"),
            "push tier must tell the model NOT to explore first"
        );
    }

    #[test]
    fn tree_from_graph_groups_tables_by_schema() {
        use crate::ai::schema_graph::{SchemaGraph, TableEntry};
        use std::collections::HashMap;
        let graph = SchemaGraph {
            tables: vec![
                TableEntry {
                    id: 0,
                    schema: "bookings".into(),
                    name: "flights".into(),
                    row_count_estimate: 1,
                    partition_info: None,
                },
                TableEntry {
                    id: 1,
                    schema: "bookings".into(),
                    name: "routes".into(),
                    row_count_estimate: 1,
                    partition_info: None,
                },
                TableEntry {
                    id: 2,
                    schema: "public".into(),
                    name: "notes".into(),
                    row_count_estimate: 1,
                    partition_info: None,
                },
            ],
            columns: vec![],
            columns_by_table: HashMap::new(),
            fk_edges: vec![],
            table_adjacency: HashMap::new(),
            built_at: std::time::Instant::now(),
        };
        let tree = tree_from_graph("demo".into(), &graph);
        assert_eq!(tree.database_name, "demo");
        assert_eq!(tree.schemas.len(), 2, "one node per schema");
        let bookings = tree.schemas.iter().find(|s| s.name == "bookings").unwrap();
        assert_eq!(
            bookings.tables,
            vec!["flights".to_string(), "routes".to_string()]
        );
    }

    #[test]
    fn no_graph_falls_back_to_tree_rendering() {
        let p = build_system_prompt(&small(), None, false);
        assert!(
            p.contains("users") && p.contains("orders"),
            "without a graph the legacy tree listing still works"
        );
        assert!(
            !p.contains("write SQL directly with run_readonly_query"),
            "fallback keeps exploration guidance since no detail was injected"
        );
    }

    #[test]
    fn static_prefix_still_precedes_dynamic_content_with_graph() {
        let g = graph_for_prompt();
        let p = build_system_prompt(&small(), Some(&g), false);
        let tools_pos = p.find("AVAILABLE TOOLS").expect("tools section");
        let schema_pos = p
            .find("# Table: public.invoices")
            .expect("m-schema section");
        assert!(
            tools_pos < schema_pos,
            "cache-friendly ordering must survive tiering"
        );
    }

    #[test]
    fn small_schema_verbose_lists_names() {
        let p = build_system_prompt(&small(), None, false);
        assert!(p.contains("users") && p.contains("orders") && p.contains("testdb"));
    }

    #[test]
    fn large_schema_compact_shows_count_not_names() {
        let schemas: Vec<SchemaNode> = (0..80)
            .map(|i| SchemaNode {
                name: format!("schema_{i}"),
                tables: (0..10).map(|j| format!("table_{i}_{j}")).collect(),
                views: vec![],
                functions: vec![],
            })
            .collect();
        let schema = SchemaTree {
            database_name: "bigdb".into(),
            server_version: "PG16".into(),
            schemas,
        };
        let p = build_system_prompt(&schema, None, false);
        assert!(p.contains("10 tables"), "each schema must show count");
        assert!(!p.contains("table_0_0"), "must NOT list individual names");
    }

    #[test]
    fn tools_and_rules_precede_database_structure_for_cache_friendliness() {
        let p = build_system_prompt(&small(), None, false);
        let tools_pos = p.find("AVAILABLE TOOLS").expect("tools section present");
        let structure_pos = p
            .find("Database schema")
            .or(p.find("Database structure"))
            .expect("schema section present");
        assert!(
            tools_pos < structure_pos,
            "static tool/RULES block must precede dynamic schema content"
        );
    }

    #[test]
    fn compact_schema_hint_references_current_tool_name() {
        let schemas: Vec<SchemaNode> = (0..80)
            .map(|i| SchemaNode {
                name: format!("schema_{i}"),
                tables: (0..10).map(|j| format!("table_{i}_{j}")).collect(),
                views: vec![],
                functions: vec![],
            })
            .collect();
        let schema = SchemaTree {
            database_name: "bigdb".into(),
            server_version: "PG16".into(),
            schemas,
        };
        let p = build_system_prompt(&schema, None, false);
        assert!(
            p.contains("use search_schema to find tables"),
            "compact-schema hint must point at the tool that actually exists"
        );
        assert!(
            !p.contains("search_objects"),
            "search_objects was removed — this tool no longer exists"
        );
    }

    #[test]
    fn no_dead_tool_call_marker_syntax() {
        let p = build_system_prompt(&small(), None, false);
        assert!(
            !p.contains("[TOOL_CALL]"),
            "no parser for this marker exists — teaching the model dead syntax wastes a turn"
        );
    }

    #[test]
    fn rules_include_complete_query_and_parallel_call_guidance() {
        let p = build_system_prompt(&small(), None, false);
        assert!(p.contains("COMPLETE QUERIES"), "must nudge the model toward one comprehensive query over iterative single-metric queries");
        assert!(
            p.contains("PARALLEL TOOL CALLS"),
            "must nudge the model to batch independent tool calls in one turn"
        );
    }

    #[test]
    fn cache_stores_and_invalidates() {
        let c = SchemaCache::new(9999);
        c.set("conn-1".into(), small());
        assert!(c.get("conn-1").is_some());
        c.invalidate("conn-1");
        assert!(c.get("conn-1").is_none());
    }

    #[test]
    fn send_results_flag_adds_analysis_hint() {
        // Data preview hint is always included — preview is sent regardless of flag.
        let p = build_system_prompt(&small(), None, true);
        assert!(p.contains("Markdown table preview"));
        let p = build_system_prompt(&small(), None, false);
        assert!(p.contains("Markdown table preview"));
    }

    #[test]
    fn rules_cover_ambiguous_metrics_and_ties() {
        let p = build_system_prompt(&small(), None, false);
        assert!(
            p.contains("AMBIGUOUS METRICS"),
            "must instruct: pick one interpretation, state it, offer the alternative"
        );
        assert!(
            p.contains("DENSE_RANK()"),
            "must instruct: anticipate ties in the FIRST ranking query"
        );
        assert!(
            p.contains("as-is — never re-run"),
            "must forbid re-running near-duplicate queries to investigate ties"
        );
    }

    #[test]
    fn rules_cover_join_discipline_reformat_and_schema_trust() {
        let p = build_system_prompt(&small(), None, false);
        assert!(p.contains("JOIN DISCIPLINE"), "FK-paths-only rule missing");
        assert!(
            p.contains("TIME-VERSIONED TABLES"),
            "range-predicate rule missing"
        );
        assert!(p.contains("NO REFORMAT RE-RUNS"), "reformat rule missing");
        assert!(p.contains("TRUST THE SCHEMA"), "no-peek rule missing");
    }

    #[test]
    fn rules_discourage_redundant_parallel_semantic_searches() {
        let p = build_system_prompt(&small(), None, false);
        assert!(
            p.contains("ONE well-chosen search_schema call")
                || p.contains("already expands to related tables"),
            "must nudge the model away from firing multiple near-duplicate search_schema \
             queries in one turn when a single broader query would already surface the \
             FK-clustered tables"
        );
    }
}
