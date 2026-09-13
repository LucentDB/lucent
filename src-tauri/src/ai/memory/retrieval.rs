use rusqlite::Connection;
use std::collections::HashMap;

use super::decay::{calculate_retention, calculate_viability};
use super::drift::validate_live_schema_gate;
use super::{GoldenQuery, MemoryItem, MemoryStatus, HOT_TIER_TOKEN_BUDGET};
use crate::ai::rerank::Rerank;
use crate::ai::schema_graph::SchemaGraph;

pub const RRF_K: f32 = 60.0;
pub const MAX_RETRIEVED_MEMORIES: usize = 3;
pub const MAX_RETRIEVED_GOLDEN_QUERIES: usize = 2;

#[derive(Debug, Clone)]
pub struct ScoredCandidate {
    pub memory: MemoryItem,
    pub rrf_score: f32,
    pub viability: f32,
    pub final_score: f32,
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom > 0.0 {
        (dot / denom).clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Formats a user natural query into a safe SQLite FTS5 match expression.
pub fn format_fts5_query(query: &str) -> Option<String> {
    let words: Vec<&str> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .map(|w| w.trim())
        .filter(|w| !w.is_empty() && w.len() > 1)
        .collect();

    if words.is_empty() {
        return None;
    }

    // Combine with OR to maximize recall in the lexical retrieval leg
    let escaped: Vec<String> = words.iter().map(|w| format!("\"{w}\"*")).collect();
    Some(escaped.join(" OR "))
}

/// Executes the two-stage hybrid retrieval pipeline:
/// Stage 1: Dense Semantic Search (fastembed) + Sparse BM25 Search (FTS5) merged via RRF (k=60)
/// Stage 2: Ebbinghaus Viability Gating + Live Schema Validation Gate
/// Stage 3: Jina Cross-Encoder Reranking (if candidates >= 10 and reranker available)
/// Returns top memories capped under the 800-token working budget.
/// Stage 1: Dense Semantic Search (fastembed) + Sparse BM25 Search (FTS5) merged via RRF (k=60)
/// Stage 2: Ebbinghaus Viability Gating + Live Schema Validation Gate
/// Synchronous operation against the SQLite connection.
pub fn score_hybrid_candidates(
    query: &str,
    connection_key: &str,
    query_embedding: Option<&[f32]>,
    graph: Option<&SchemaGraph>,
    active_memories: &[MemoryItem],
    conn: &Connection,
) -> Vec<ScoredCandidate> {
    if active_memories.is_empty() {
        return Vec::new();
    }

    let now = chrono::Utc::now().timestamp();

    // 1. Dense Semantic Ranking (strictly positive cosine similarity)
    let mut dense_ranks: HashMap<String, usize> = HashMap::new();
    if let Some(q_vec) = query_embedding {
        let mut scored_dense: Vec<(&MemoryItem, f32)> = active_memories
            .iter()
            .map(|m| (m, cosine_similarity(q_vec, &m.embedding)))
            .filter(|(_, sim)| *sim > 0.0)
            .collect();
        scored_dense.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        for (rank_idx, (m, _)) in scored_dense.into_iter().enumerate() {
            dense_ranks.insert(m.id.clone(), rank_idx + 1);
        }
    }

    // 2. Sparse BM25 Ranking via FTS5 (scoped to connection/global active memories)
    let mut bm25_ranks: HashMap<String, usize> = HashMap::new();
    if let Some(fts_expr) = format_fts5_query(query) {
        // FTS5 auxiliary functions (`bm25`) bind to the FTS table's name, not
        // a query-level alias: `bm25(f)` fails to prepare, silently emptying
        // the sparse ranking map. Reference the table by name throughout.
        let stmt = conn.prepare(
            "SELECT memories_fts.id, bm25(memories_fts) as rank
             FROM memories_fts
             JOIN memories m ON m.id = memories_fts.id
             WHERE memories_fts MATCH ?1 AND (m.connection_key = ?2 OR m.scope = 'global') AND m.status = 'ACTIVE' AND m.tombstone = 0
             ORDER BY rank ASC
             LIMIT 50",
        );
        if let Ok(mut s) = stmt {
            if let Ok(rows) = s.query_map(rusqlite::params![&fts_expr, connection_key], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            }) {
                for (rank, row_res) in (1..).zip(rows.flatten()) {
                    bm25_ranks.insert(row_res.0, rank);
                }
            }
        }
    }

    // 3. Reciprocal Rank Fusion & Gating
    let mut candidates: Vec<ScoredCandidate> = Vec::new();

    for m in active_memories {
        if m.status != MemoryStatus::Active || m.tombstone {
            continue;
        }
        if let Some(valid_until) = m.valid_until {
            if valid_until <= now {
                continue;
            }
        }

        // RRF computation
        let r_dense = dense_ranks.get(&m.id).copied().unwrap_or(1000) as f32;
        let r_sparse = bm25_ranks.get(&m.id).copied().unwrap_or(1000) as f32;

        let dense_part = if dense_ranks.contains_key(&m.id) {
            1.0 / (RRF_K + r_dense)
        } else {
            0.0
        };

        let sparse_part = if bm25_ranks.contains_key(&m.id) {
            1.0 / (RRF_K + r_sparse)
        } else {
            0.0
        };

        let rrf_score = dense_part + sparse_part;
        if rrf_score <= 0.0 {
            continue;
        }

        // Evaluate live schema gate (Gate 2). B-I2: this is a per-memory
        // `memory_entity_links` query, so it must run only for candidates that
        // actually scored — running it first made retrieval O(N) DB round
        // trips across *every* active memory, including ones that can never be
        // selected.
        if let Some(g) = graph {
            if !validate_live_schema_gate(&m.id, g, conn) {
                continue;
            }
        }

        // Viability calculation
        let elapsed_hours = (now - m.last_accessed_at).max(0) as f32 / 3600.0;
        let retention = calculate_retention(elapsed_hours, m.stability_hours);
        let viability = calculate_viability(retention, m.importance);

        let final_score = rrf_score * viability;
        if final_score > 0.0 {
            candidates.push(ScoredCandidate {
                memory: m.clone(),
                rrf_score,
                viability,
                final_score,
            });
        }
    }

    // Sort by final score descending
    candidates.sort_by(|a, b| {
        b.final_score
            .partial_cmp(&a.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    candidates
}

/// Stage 3: Jina Cross-Encoder Reranking (if candidates >= 10 and reranker available)
/// Enforces working token budget under 800 tokens.
/// Pure async operation that does not reference Connection.
pub async fn finalize_hybrid_memories<R: Rerank>(
    query: &str,
    reranker: Option<&R>,
    mut candidates: Vec<ScoredCandidate>,
) -> Vec<MemoryItem> {
    if candidates.is_empty() {
        return Vec::new();
    }

    // Stage 3: Jina Cross-Encoder Reranking
    if candidates.len() >= 10 {
        // Clamp the cross-encoder pool to the spec's top 20 before reranking.
        // Jina cross-attention is O(N) on CPU, so handing it hundreds of RRF
        // candidates pins Tokio workers for tens of seconds. Candidates are
        // already sorted by final score, so truncation keeps the best 20.
        candidates.truncate(20);
        if let Some(rr) = reranker {
            let texts: Vec<String> = candidates
                .iter()
                .map(|c| format!("{}: {}", c.memory.key_phrase, c.memory.rule_text))
                .collect();
            if let Ok(rerank_results) = rr.rerank(query, &texts).await {
                let mut reranked = Vec::with_capacity(candidates.len());
                for (orig_idx, _) in rerank_results {
                    if orig_idx < candidates.len() {
                        reranked.push(candidates[orig_idx].clone());
                    }
                }
                candidates = reranked;
            }
        }
    }

    // Select Top-3 under working token budget (~3200 characters)
    let max_budget_chars = HOT_TIER_TOKEN_BUDGET * 4;
    let mut current_chars = 0;
    let mut selected = Vec::new();

    for c in candidates.into_iter().take(MAX_RETRIEVED_MEMORIES) {
        let text_len = c.memory.rule_text.len()
            + c.memory.key_phrase.len()
            + c.memory.sql_snippet.as_ref().map(|s| s.len()).unwrap_or(0);
        if current_chars + text_len > max_budget_chars && !selected.is_empty() {
            break;
        }
        current_chars += text_len;
        selected.push(c.memory);
    }

    selected
}

/// Retrieves Top-2 verified Golden Queries matching the natural query.
pub fn retrieve_golden_queries(
    query_embedding: Option<&[f32]>,
    golden_queries: &[GoldenQuery],
) -> Vec<GoldenQuery> {
    if golden_queries.is_empty() {
        return Vec::new();
    }
    let Some(q_vec) = query_embedding else {
        return golden_queries
            .iter()
            .filter(|g| g.verified)
            .take(MAX_RETRIEVED_GOLDEN_QUERIES)
            .cloned()
            .collect();
    };

    let mut scored: Vec<(&GoldenQuery, f32)> = golden_queries
        .iter()
        .filter(|g| g.verified)
        .map(|g| (g, cosine_similarity(q_vec, &g.embedding)))
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    scored
        .into_iter()
        .take(MAX_RETRIEVED_GOLDEN_QUERIES)
        .map(|(g, _)| g.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-4);

        let c = vec![0.0, 1.0, 0.0];
        assert_eq!(cosine_similarity(&a, &c), 0.0);
    }

    #[test]
    fn test_format_fts5_query() {
        let q = "unpaid invoices in public schema";
        let formatted = format_fts5_query(q).unwrap();
        assert!(formatted.contains("\"unpaid\"*"));
        assert!(formatted.contains("\"invoices\"*"));
    }

    use crate::ai::memory::{
        MemoryCategory, MemoryItem, MemoryScope, SourceTrust, MEMORY_FORMAT_VERSION,
        MEMORY_MODEL_NAME,
    };

    fn sample_candidate(idx: usize, final_score: f32) -> ScoredCandidate {
        ScoredCandidate {
            memory: MemoryItem {
                id: format!("mem_{idx}"),
                connection_key: "conn".into(),
                scope: MemoryScope::Connection,
                scope_key: "conn".into(),
                category: MemoryCategory::Metric,
                key_phrase: format!("mem_{idx}"),
                rule_text: format!("rule {idx}"),
                sql_snippet: None,
                importance: 0.5,
                stability_hours: 720.0,
                last_accessed_at: 0,
                access_count: 0,
                source_trust: SourceTrust::UserExplicit,
                source_conv_id: None,
                source_turn_id: None,
                source_tool_id: None,
                status: MemoryStatus::Active,
                supersedes_id: None,
                valid_from: 0,
                valid_until: None,
                learned_at: 0,
                tombstone: false,
                tombstoned_at: None,
                doc_hash: String::new(),
                embedding_model: MEMORY_MODEL_NAME.into(),
                embedding_version: MEMORY_FORMAT_VERSION,
                embedding: Vec::new(),
                created_at: 0,
                updated_at: 0,
            },
            rrf_score: final_score,
            viability: 1.0,
            final_score,
        }
    }

    /// Records the size and contents of every candidate slice it is handed, and
    /// returns scores that preserve the incoming order.
    #[derive(Default)]
    struct RecordingReranker {
        seen_lengths: std::sync::Mutex<Vec<usize>>,
        seen_texts: std::sync::Mutex<Vec<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Rerank for RecordingReranker {
        async fn rerank(
            &self,
            _query: &str,
            candidates: &[String],
        ) -> Result<Vec<(usize, f32)>, String> {
            self.seen_lengths.lock().unwrap().push(candidates.len());
            self.seen_texts.lock().unwrap().push(candidates.to_vec());
            Ok(candidates
                .iter()
                .enumerate()
                .map(|(i, _)| (i, (candidates.len() - i) as f32))
                .collect())
        }
    }

    #[tokio::test]
    async fn reranker_pool_is_clamped_to_top_20_before_reranking() {
        // 200 RRF-scored candidates, best score first (as score_hybrid_candidates emits).
        let candidates: Vec<ScoredCandidate> = (0..200)
            .map(|i| sample_candidate(i, (200 - i) as f32))
            .collect();

        let fake = RecordingReranker::default();
        let selected = finalize_hybrid_memories("query", Some(&fake), candidates).await;

        let lengths = fake.seen_lengths.lock().unwrap();
        assert_eq!(lengths.len(), 1, "reranker must be invoked exactly once");
        assert_eq!(
            lengths[0], 20,
            "cross-encoder pool must be clamped to the top 20, got {}",
            lengths[0]
        );

        // The 20 handed over are the top 20 by RRF/final score, in order.
        let texts = fake.seen_texts.lock().unwrap();
        let expected: Vec<String> = (0..20).map(|i| format!("mem_{i}: rule {i}")).collect();
        assert_eq!(texts[0], expected, "must rerank the top-scoring 20");

        assert_eq!(
            selected.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["mem_0", "mem_1", "mem_2"]
        );
    }

    #[tokio::test]
    async fn reranker_skipped_below_ten_candidates() {
        let candidates: Vec<ScoredCandidate> = (0..9)
            .map(|i| sample_candidate(i, (9 - i) as f32))
            .collect();

        let fake = RecordingReranker::default();
        let selected = finalize_hybrid_memories("query", Some(&fake), candidates).await;

        assert!(
            fake.seen_lengths.lock().unwrap().is_empty(),
            "the < 10 threshold must not invoke the reranker"
        );
        assert_eq!(selected.len(), 3);
    }

    fn sample_memory(id: &str) -> MemoryItem {
        MemoryItem {
            id: id.into(),
            connection_key: "conn".into(),
            scope: MemoryScope::Connection,
            scope_key: "conn".into(),
            category: MemoryCategory::Metric,
            key_phrase: id.into(),
            rule_text: format!("rule {id}"),
            sql_snippet: None,
            importance: 0.5,
            stability_hours: 720.0,
            last_accessed_at: chrono::Utc::now().timestamp(),
            access_count: 1,
            source_trust: SourceTrust::UserExplicit,
            source_conv_id: None,
            source_turn_id: None,
            source_tool_id: None,
            status: MemoryStatus::Active,
            supersedes_id: None,
            valid_from: 0,
            valid_until: None,
            learned_at: 0,
            tombstone: false,
            tombstoned_at: None,
            doc_hash: String::new(),
            embedding_model: MEMORY_MODEL_NAME.into(),
            embedding_version: MEMORY_FORMAT_VERSION,
            embedding: Vec::new(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn empty_graph() -> SchemaGraph {
        SchemaGraph {
            tables: vec![],
            columns: vec![],
            columns_by_table: HashMap::new(),
            fk_edges: vec![],
            table_adjacency: HashMap::new(),
            built_at_unix: 0,
            tier: crate::ai::schema_graph::IndexingTier::MetadataOnly,
        }
    }

    fn graph_with_users() -> SchemaGraph {
        use crate::ai::schema_graph::TableEntry;
        SchemaGraph {
            tables: vec![TableEntry {
                id: 0,
                schema: "public".into(),
                name: "users".into(),
                kind: "table".into(),
                row_count_estimate: 0,
                partition_info: None,
            }],
            columns: vec![],
            columns_by_table: HashMap::new(),
            fk_edges: vec![],
            table_adjacency: HashMap::new(),
            built_at_unix: 0,
            tier: crate::ai::schema_graph::IndexingTier::MetadataOnly,
        }
    }

    /// B-I2: Gate 2 ran a `memory_entity_links` query for *every* active
    /// memory before RRF scores were even computed. Candidates that scored
    /// zero can never be selected, so those queries were pure N+1 storm.
    #[test]
    fn b_i2_zero_score_candidates_never_reach_the_live_gate() {
        use crate::ai::memory::{live_gate_call_count, reset_live_gate_call_count};
        let conn = Connection::open_in_memory().unwrap();
        let graph = empty_graph();
        let memories: Vec<MemoryItem> = (0..25)
            .map(|i| sample_memory(&format!("mem_{i}")))
            .collect();

        reset_live_gate_call_count();
        // No embedding and no FTS table → every candidate's RRF score is 0.
        let out =
            score_hybrid_candidates("nomatchterm", "conn", None, Some(&graph), &memories, &conn);

        assert!(out.is_empty(), "no candidate scored, so none is selected");
        assert_eq!(
            live_gate_call_count(),
            0,
            "zero-scoring candidates must not each spend a Gate 2 DB query"
        );
    }

    /// Guards the fix against the degenerate version: candidates that *do*
    /// score must still be live-validated, and the gate must still be able to
    /// reject one whose linked table vanished.
    #[test]
    fn b_i2_live_gate_still_applies_to_scoring_candidates() {
        use crate::ai::memory::{live_gate_call_count, reset_live_gate_call_count};
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE memory_entity_links (
                 memory_id TEXT, schema_name TEXT, table_name TEXT, column_name TEXT);
             INSERT INTO memory_entity_links VALUES ('mem_0', 'public', 'ghost', '');",
        )
        .unwrap();

        let graph = graph_with_users();
        let mut memories = vec![sample_memory("mem_0"), sample_memory("mem_1")];
        for m in &mut memories {
            m.embedding = vec![1.0, 0.0, 0.0];
        }

        reset_live_gate_call_count();
        let out = score_hybrid_candidates(
            "anything",
            "conn",
            Some(&[1.0, 0.0, 0.0]),
            Some(&graph),
            &memories,
            &conn,
        );

        assert_eq!(
            live_gate_call_count(),
            2,
            "every candidate that scored must still be live-validated"
        );
        let ids: Vec<&str> = out.iter().map(|c| c.memory.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["mem_1"],
            "the memory whose linked table vanished must still be rejected"
        );
    }
}
