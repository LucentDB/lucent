use serde_json::json;

use super::{AiToolContext, ToolError, ToolOutput};

#[derive(Clone)]
pub struct SearchSchema {
    ctx: AiToolContext,
}

impl SearchSchema {
    pub fn new(ctx: AiToolContext) -> Self {
        Self { ctx }
    }

    pub fn description(&self) -> String {
        "Search the database schema by meaning or by name. Use mode=\"semantic\" for \
         natural-language questions about what data lives where (e.g. \"which table has \
         unpaid invoices\"); mode=\"keyword\" when you know part of an exact table or \
         column name; mode=\"hybrid\" (default) tries both. Returns matching columns with \
         similarity scores (low scores mean a weak match — consider trying a different \
         mode or phrasing), plus related tables discovered via foreign keys."
            .into()
    }

    pub fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "mode": {
                    "type": "string",
                    "enum": ["semantic", "keyword", "hybrid"],
                    "default": "hybrid"
                }
            },
            "required": ["query"]
        })
    }

    pub async fn call(
        &self,
        args: serde_json::Value,
        _ctx: &AiToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let query = args["query"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs("missing 'query'".into()))?;
        let mode = args["mode"].as_str().unwrap_or("hybrid");

        log::info!("Tool 'search_schema' called — query={query:?}, mode={mode:?}");

        let mut sections: Vec<String> = Vec::new();

        // Semantic mode
        if mode == "semantic" || mode == "hybrid" {
            let graph_guard = self.ctx.schema_graph.lock().await;
            let emb_guard = self.ctx.embedder.lock().await;
            match (graph_guard.as_ref(), emb_guard.as_ref()) {
                (Some(graph), Some(embedder)) => match embedder.embed_query(query).await {
                    Ok(query_vec) => {
                        const CANDIDATE_POOL: usize = 20;
                        const FINAL_K: usize = 10;
                        let (max_hops, max_cluster_tables) = scaled_bfs_bounds(graph.tables.len());

                        let candidates = crate::ai::retrieval::Retriever::top_matches(
                            graph,
                            &query_vec,
                            CANDIDATE_POOL,
                        );

                        let reranker_guard = self.ctx.reranker.lock().await;
                        let active_reranker = reranker_guard
                            .as_ref()
                            .filter(|_| should_rerank(graph.columns.len()));
                        let final_matched = if let Some(reranker) = active_reranker {
                            let doc_texts: Vec<String> = candidates
                                .iter()
                                .filter_map(|sc| {
                                    graph
                                        .columns
                                        .iter()
                                        .find(|c| c.id == sc.column_id)
                                        .map(|c| c.doc_text.clone())
                                })
                                .collect();
                            match reranker.rerank(query, &doc_texts).await {
                                Ok(ranked) => {
                                    log::debug!(
                                            "search_schema: reranked {} candidates for {query:?}, top result column_id={:?}",
                                            ranked.len(),
                                            ranked.first().map(|(idx, _)| candidates[*idx].column_id),
                                        );
                                    ranked
                                        .into_iter()
                                        .take(FINAL_K)
                                        .map(|(idx, score)| crate::ai::retrieval::ScoredColumn {
                                            column_id: candidates[idx].column_id,
                                            score,
                                        })
                                        .collect()
                                }
                                Err(e) => {
                                    log::warn!(
                                        "Reranking failed, falling back to raw cosine order: {e}"
                                    );
                                    candidates.into_iter().take(FINAL_K).collect()
                                }
                            }
                        } else {
                            candidates.into_iter().take(FINAL_K).collect()
                        };
                        drop(reranker_guard);

                        let retrieved = crate::ai::retrieval::Retriever::cluster_from_matches(
                            graph,
                            final_matched,
                            max_hops,
                            max_cluster_tables,
                        );
                        sections.push(format_semantic_section(&retrieved));
                    }
                    Err(e) => {
                        sections.push(format!("Semantic search error: {e}"));
                    }
                },
                _ => {
                    sections.push(
                        "Semantic search unavailable (schema index not built). \
                         Falling back to keyword search only."
                            .into(),
                    );
                }
            }
        }

        // Keyword mode
        if mode == "keyword" || mode == "hybrid" {
            let conn_id = match self.ctx.connection_id {
                Some(c) => c,
                None => return Err(ToolError::NotConnected),
            };
            let client = self
                .ctx
                .db
                .lock()
                .await
                .clone()
                .ok_or(ToolError::NotConnected)?;
            match super::objects::keyword_search_objects(&client, conn_id, query, None, None).await
            {
                Ok(items) => {
                    sections.push(format_keyword_section(&items));
                }
                Err(e) => {
                    sections.push(format!("Keyword search error: {e}"));
                }
            }
        }

        Ok(ToolOutput::Text {
            content: sections.join("\n\n"),
        })
    }
}

pub(crate) fn format_semantic_section(
    retrieved: &crate::ai::retrieval::RetrievedContext,
) -> String {
    let mut out = String::from("=== Semantic Search Results ===\n\n");

    if retrieved.matched.is_empty() {
        out.push_str("No semantically matching columns found.\n");
        return out;
    }

    out.push_str(&format!(
        "Top {} matching columns:\n",
        retrieved.matched.len()
    ));
    for sc in &retrieved.matched {
        for t in &retrieved.tables {
            if let Some(c) = t.columns.iter().find(|c| c.id == sc.column_id) {
                out.push_str(&format!(
                    "  {}.{} (score: {:.4})\n",
                    c.table, c.name, sc.score
                ));
            }
        }
    }

    out.push_str(&format!(
        "\nFull column detail for {} clustered table(s) — use this instead of a \
         follow-up get_objects_info call for schema info. If you need actual row \
         samples to see what data looks like, call get_objects_info with sample_rows:N:\n",
        retrieved.tables.len()
    ));
    for t in &retrieved.tables {
        out.push_str(&format!("\n  {}.{}:\n", t.schema, t.table));
        for c in &t.columns {
            let pk = if c.is_primary_key { " PK" } else { "" };
            let fk = c
                .fk_ref
                .as_ref()
                .map(|r| format!(" → {r}"))
                .unwrap_or_default();
            let values = if c.sample_values.is_empty() {
                String::new()
            } else {
                let shown: Vec<&str> = c.sample_values.iter().take(5).map(String::as_str).collect();
                format!(" [values: {}]", shown.join(", "))
            };
            out.push_str(&format!(
                "    {} {}{}{}{}\n",
                c.name, c.data_type, pk, fk, values
            ));
        }
    }

    if !retrieved.relationships.is_empty() {
        out.push_str("\nRelationships:\n");
        for r in &retrieved.relationships {
            out.push_str(&format!("  {r}\n"));
        }
    }

    out
}

fn format_keyword_section(items: &[serde_json::Value]) -> String {
    let mut out = String::from("=== Keyword Search Results ===\n\n");
    if items.is_empty() {
        out.push_str("No matching objects found.\n");
        return out;
    }
    for item in items {
        let schema = item["schema"].as_str().unwrap_or("");
        let name = item["name"].as_str().unwrap_or("");
        let kind = item["kind"].as_str().unwrap_or("");
        let score = item["score"].as_f64().unwrap_or(0.0);
        out.push_str(&format!("  {schema}.{name} ({kind}, score: {score:.4})\n"));
    }
    out
}

/// Scale the BFS cluster bound to schema size. A fixed bound of 15 tables is a
/// no-op on a schema with 12 tables total (the "bounded" cluster ends up being
/// almost the whole schema) — this keeps the original generous bound for large
/// schemas where it was never the problem, while giving small schemas a genuinely
/// tighter bound so retrieval stays selective.
/// Reranking only pays for itself when candidates come from a corpus large
/// enough that raw cosine ordering gets noisy. Below this, skip the
/// cross-encoder pass entirely.
const RERANK_MIN_CORPUS_COLUMNS: usize = 100;

fn should_rerank(total_columns: usize) -> bool {
    total_columns >= RERANK_MIN_CORPUS_COLUMNS
}

pub(crate) fn scaled_bfs_bounds(total_tables: usize) -> (usize, usize) {
    const MAX_HOPS: usize = 3;
    const MAX_CLUSTER_CAP: usize = 15;
    const MIN_CLUSTER: usize = 2;

    let scaled = (total_tables / 2).max(MIN_CLUSTER);
    (MAX_HOPS, scaled.min(MAX_CLUSTER_CAP))
}

#[cfg(test)]
mod format_tests {
    use super::format_semantic_section;
    use crate::ai::retrieval::{RetrievedContext, ScoredColumn, TableContext};
    use crate::ai::schema_graph::ColumnEntry;

    fn fake_column(
        id: usize,
        name: &str,
        data_type: &str,
        is_pk: bool,
        values: Vec<&str>,
    ) -> ColumnEntry {
        ColumnEntry {
            id,
            table_id: 0,
            schema: "public".into(),
            table: "invoices".into(),
            name: name.into(),
            data_type: data_type.into(),
            is_primary_key: is_pk,
            sample_values: values.into_iter().map(String::from).collect(),
            fk_ref: None,
            embedding: vec![],
            doc_text: String::new(),
        }
    }

    #[test]
    fn clustered_table_output_includes_real_column_detail_not_just_a_count() {
        let ctx = RetrievedContext {
            matched: vec![ScoredColumn {
                column_id: 1,
                score: 0.81,
            }],
            tables: vec![TableContext {
                id: 0,
                schema: "public".into(),
                table: "invoices".into(),
                columns: vec![
                    fake_column(0, "id", "INTEGER", true, vec![]),
                    fake_column(
                        1,
                        "status",
                        "TEXT",
                        false,
                        vec!["pending", "paid", "overdue"],
                    ),
                ],
            }],
            relationships: vec![],
        };
        let out = format_semantic_section(&ctx);
        assert!(out.contains("status"), "must list real column names");
        assert!(out.contains("TEXT"), "must list real column types");
        assert!(
            out.contains("pending"),
            "must include sample values so a follow-up get_objects_info isn't needed"
        );
        assert!(
            !out.contains("(2 columns)"),
            "must not fall back to a bare column count"
        );
        assert!(
            out.contains("get_objects_info with sample_rows"),
            "must tell the agent how to get sample data"
        );
    }

    #[test]
    fn primary_key_column_is_flagged() {
        let ctx = RetrievedContext {
            matched: vec![ScoredColumn {
                column_id: 0,
                score: 0.95,
            }],
            tables: vec![TableContext {
                id: 0,
                schema: "public".into(),
                table: "invoices".into(),
                columns: vec![fake_column(0, "id", "INTEGER", true, vec![])],
            }],
            relationships: vec![],
        };
        let out = format_semantic_section(&ctx);
        assert!(
            out.contains("PK"),
            "primary key columns must be visually flagged"
        );
    }

    #[test]
    fn fk_reference_shown_inline_not_just_in_relationships_section() {
        let mut col = fake_column(0, "org_id", "INTEGER", false, vec![]);
        col.fk_ref = Some("organizations.id".into());
        let ctx = RetrievedContext {
            matched: vec![ScoredColumn {
                column_id: 0,
                score: 0.9,
            }],
            tables: vec![TableContext {
                id: 0,
                schema: "public".into(),
                table: "users".into(),
                columns: vec![col],
            }],
            relationships: vec![],
        };
        let out = format_semantic_section(&ctx);
        assert!(
            out.contains("→ organizations.id"),
            "FK ref must appear inline next to the column"
        );
    }
}

#[cfg(test)]
mod bfs_bound_scaling_tests {
    use super::scaled_bfs_bounds;

    #[test]
    fn small_schema_gets_a_tight_bound_not_the_whole_schema() {
        let (max_hops, max_cluster_tables) = scaled_bfs_bounds(12);
        assert!(
            max_cluster_tables < 12,
            "on a 12-table schema, the bound must be tighter than the whole schema \
             (got {max_cluster_tables}) — otherwise 'bounded' clustering is a no-op"
        );
        assert!(max_hops >= 1, "must allow at least 1 hop of FK expansion");
    }

    #[test]
    fn large_schema_gets_the_original_generous_bound() {
        let (max_hops, max_cluster_tables) = scaled_bfs_bounds(500);
        assert_eq!(max_hops, 3);
        assert_eq!(
            max_cluster_tables, 15,
            "large schemas keep the original bound — it was never the problem there"
        );
    }

    #[test]
    fn tiny_schema_still_gets_at_least_a_few_tables() {
        let (_max_hops, max_cluster_tables) = scaled_bfs_bounds(3);
        assert!(
            max_cluster_tables >= 2,
            "must never bound below 2 (a single-table cluster can't show any FK relationships)"
        );
    }
}

#[cfg(test)]
mod rerank_gating_tests {
    use super::should_rerank;

    #[test]
    fn small_corpus_skips_reranking() {
        assert!(
            !should_rerank(69),
            "cosine over a tiny corpus is already clean — the cross-encoder pass \
             is pure latency there"
        );
    }

    #[test]
    fn large_corpus_engages_reranking() {
        assert!(should_rerank(500));
    }

    #[test]
    fn threshold_boundary() {
        assert!(!should_rerank(99));
        assert!(should_rerank(100));
    }
}

#[cfg(test)]
mod rerank_wiring_tests {
    use crate::ai::retrieval::{Retriever, ScoredColumn};
    use crate::ai::schema_graph::{ColumnEntry, SchemaGraph, TableEntry};
    use std::collections::HashMap;

    fn make_col(id: usize, name: &str, embedding: Vec<f32>) -> ColumnEntry {
        ColumnEntry {
            id,
            table_id: 0,
            schema: "public".into(),
            table: "t0".into(),
            name: name.into(),
            data_type: "TEXT".into(),
            is_primary_key: false,
            sample_values: vec![],
            fk_ref: None,
            embedding,
            doc_text: format!("public.t0.{name} TEXT"),
        }
    }

    #[test]
    fn candidate_pool_is_wider_than_final_k_before_rerank_truncation() {
        // Prove the widen-then-narrow shape: with CANDIDATE_POOL=20 and FINAL_K=10,
        // top_matches(pool) must be called with the WIDER number, and only truncated
        // to the narrower number after reranking — never widened AFTER truncation.
        let columns: Vec<ColumnEntry> = (0..20)
            .map(|i| make_col(i, &format!("col{i}"), vec![1.0 - (i as f32) * 0.01, 0.0]))
            .collect();
        let graph = SchemaGraph {
            tables: vec![TableEntry {
                id: 0,
                schema: "public".into(),
                name: "t0".into(),
                row_count_estimate: 0,
                partition_info: None,
            }],
            columns_by_table: HashMap::from([(0, (0..20).collect())]),
            columns,
            fk_edges: vec![],
            table_adjacency: HashMap::new(),
            built_at: std::time::Instant::now(),
        };
        let query = vec![1.0, 0.0];
        let pool: Vec<ScoredColumn> = Retriever::top_matches(&graph, &query, 20);
        assert_eq!(
            pool.len(),
            20,
            "candidate pool must be the WIDE count, not the final narrow count"
        );
    }
}
