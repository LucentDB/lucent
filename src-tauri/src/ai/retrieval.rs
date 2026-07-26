use std::collections::{HashMap, HashSet, VecDeque};

#[cfg(test)]
use crate::ai::schema_graph::FkEdge;
use crate::ai::schema_graph::{ColumnEntry, SchemaGraph};

#[derive(Clone, Debug)]
pub struct ScoredColumn {
    pub column_id: usize,
    pub score: f32,
}

#[derive(Clone, Debug)]
pub struct RetrievedContext {
    pub matched: Vec<ScoredColumn>,
    pub tables: Vec<TableContext>,
    pub relationships: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TableContext {
    pub id: usize,
    pub schema: String,
    pub table: String,
    pub columns: Vec<ColumnEntry>,
}

pub struct Retriever;

impl Retriever {
    /// Find top-K columns by cosine similarity, then expand to FK-neighbor tables
    /// via bounded BFS. Pure synchronous — no I/O.
    pub fn retrieve(
        graph: &SchemaGraph,
        query_embedding: &[f32],
        top_k: usize,
        max_hops: usize,
        max_cluster_tables: usize,
    ) -> RetrievedContext {
        let matched = Self::top_matches(graph, query_embedding, top_k);
        Self::cluster_from_matches(graph, matched, max_hops, max_cluster_tables)
    }

    /// Score all columns by cosine similarity against `query_embedding`,
    /// return the top-K sorted descending. No clustering.
    pub fn top_matches(
        graph: &SchemaGraph,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Vec<ScoredColumn> {
        let mut scored: Vec<ScoredColumn> = graph
            .columns
            .iter()
            .filter(|c| !c.embedding.is_empty())
            .map(|c| ScoredColumn {
                column_id: c.id,
                score: cosine_similarity(query_embedding, &c.embedding),
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(top_k);
        scored
    }

    /// Given an already-selected set of matched columns, expand to FK-neighbor
    /// tables via bounded BFS and assemble the final RetrievedContext.
    pub fn cluster_from_matches(
        graph: &SchemaGraph,
        matched: Vec<ScoredColumn>,
        max_hops: usize,
        max_cluster_tables: usize,
    ) -> RetrievedContext {
        // Seed the cluster with the tables of matched columns
        let mut cluster: HashSet<usize> = HashSet::new();
        for s in &matched {
            if s.column_id < graph.columns.len() {
                cluster.insert(graph.columns[s.column_id].table_id);
            }
        }

        // Bounded BFS over table_adjacency
        if !cluster.is_empty() {
            let mut queue: VecDeque<(usize, usize)> = VecDeque::new();
            let start_hops: HashMap<usize, usize> =
                cluster.iter().map(|&tid| (tid, 0usize)).collect();
            for (&tid, &hop) in &start_hops {
                queue.push_back((tid, hop));
            }

            let mut visited: HashSet<usize> = cluster.clone();

            while let Some((current, hop)) = queue.pop_front() {
                if hop >= max_hops {
                    continue;
                }
                if cluster.len() >= max_cluster_tables {
                    break;
                }
                if let Some(neighbors) = graph.table_adjacency.get(&current) {
                    for &n in neighbors {
                        if cluster.len() >= max_cluster_tables {
                            break;
                        }
                        if visited.insert(n) {
                            cluster.insert(n);
                            queue.push_back((n, hop + 1));
                        }
                    }
                    if cluster.len() >= max_cluster_tables {
                        break;
                    }
                }
            }
        }

        // Build TableContext for each clustered table
        let mut tables: Vec<TableContext> = Vec::new();
        for &tid in &cluster {
            if let Some(col_ids) = graph.columns_by_table.get(&tid) {
                let entry = &graph.tables[tid];
                let cols: Vec<ColumnEntry> = col_ids
                    .iter()
                    .map(|&cid| graph.columns[cid].clone())
                    .collect();
                tables.push(TableContext {
                    id: tid,
                    schema: entry.schema.clone(),
                    table: entry.name.clone(),
                    columns: cols,
                });
            }
        }
        tables.sort_by_key(|a| a.id);

        // Build human-readable relationships
        let cluster_set: HashSet<usize> = cluster;
        let mut relationships: Vec<String> = Vec::new();
        for fk in &graph.fk_edges {
            let from = &graph.columns[fk.from_column];
            let to = &graph.columns[fk.to_column];
            if cluster_set.contains(&from.table_id) && cluster_set.contains(&to.table_id) {
                relationships.push(format!(
                    "{}.{} → {}.{}",
                    from.table, from.name, to.table, to.name,
                ));
            }
        }

        RetrievedContext {
            matched,
            tables,
            relationships,
        }
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum();
    let nb: f32 = b.iter().map(|x| x * x).sum();
    let denom = na.sqrt() * nb.sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        dot / denom
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::schema_graph::ColumnEntry;

    fn fake_col(id: usize, tid: usize, name: &str, embedding: Vec<f32>) -> ColumnEntry {
        ColumnEntry {
            id,
            table_id: tid,
            schema: "public".into(),
            table: format!("t{tid}"),
            name: name.into(),
            data_type: "TEXT".into(),
            is_primary_key: false,
            sample_values: vec![],
            fk_ref: None,
            embedding,
            doc_text: format!("public.t{tid}.{name} TEXT"),
        }
    }

    fn make_graph(
        columns: Vec<ColumnEntry>,
        fk_triples: Vec<(usize, usize, usize, usize)>,
    ) -> SchemaGraph {
        let mut tables = std::collections::HashMap::new();
        let mut columns_by_table: std::collections::HashMap<usize, Vec<usize>> =
            std::collections::HashMap::new();
        let mut fk_edges = Vec::new();
        let mut table_adjacency: std::collections::HashMap<usize, Vec<usize>> =
            std::collections::HashMap::new();

        for c in &columns {
            tables
                .entry(c.table_id)
                .or_insert_with(|| crate::ai::schema_graph::TableEntry {
                    id: c.table_id,
                    schema: c.schema.clone(),
                    name: c.table.clone(),
                    row_count_estimate: 0,
                    partition_info: None,
                });
            columns_by_table.entry(c.table_id).or_default().push(c.id);
        }

        for &(from_col, to_col, from_tid, to_tid) in &fk_triples {
            fk_edges.push(FkEdge {
                from_column: from_col,
                to_column: to_col,
            });
            table_adjacency.entry(from_tid).or_default().push(to_tid);
            if from_tid != to_tid {
                table_adjacency.entry(to_tid).or_default().push(from_tid);
            }
        }
        for v in table_adjacency.values_mut() {
            v.sort();
            v.dedup();
        }

        let tables: Vec<_> = {
            let mut t: Vec<_> = tables.into_values().collect();
            t.sort_by_key(|a| a.id);
            t
        };

        SchemaGraph {
            tables,
            columns,
            columns_by_table,
            fk_edges,
            table_adjacency,
            built_at: std::time::Instant::now(),
        }
    }

    #[test]
    fn test_cosine_top_k_returns_correct_order() {
        let graph = make_graph(
            vec![
                fake_col(0, 0, "status", vec![1.0, 0.0, 0.0]),
                fake_col(1, 0, "name", vec![0.0, 1.0, 0.0]),
                fake_col(2, 1, "amount", vec![0.0, 0.0, 1.0]),
            ],
            vec![],
        );
        let query = vec![0.9, 0.1, 0.1];
        let result = Retriever::retrieve(&graph, &query, 2, 2, 10);
        assert_eq!(result.matched.len(), 2);
        // First match should be column 0 (status) — most similar to query
        assert_eq!(result.matched[0].column_id, 0);
        assert!(result.matched[0].score > result.matched[1].score);
    }

    #[test]
    fn test_bfs_stops_at_max_hops() {
        // Chain: t0 - t1 - t2 - t3
        let graph = make_graph(
            vec![
                fake_col(0, 0, "id", vec![1.0, 0.0]),
                fake_col(1, 1, "id", vec![0.9, 0.1]),
                fake_col(2, 1, "t0_id", vec![0.8, 0.2]),
                fake_col(3, 2, "id", vec![0.7, 0.3]),
                fake_col(4, 2, "t1_id", vec![0.6, 0.4]),
                fake_col(5, 3, "id", vec![0.5, 0.5]),
            ],
            vec![(0, 1, 0, 1), (2, 3, 1, 2), (4, 5, 2, 3)],
        );
        let query = vec![1.0, 0.0];
        // top_k picks column 0 (t0.id). max_hops=1 means only t0 and immediate neighbors (t1).
        let result = Retriever::retrieve(&graph, &query, 1, 1, 10);
        let table_ids: Vec<usize> = result.tables.iter().map(|t| t.id).collect();
        assert!(table_ids.contains(&0), "seed table should be in cluster");
        assert!(
            table_ids.contains(&1),
            "1-hop neighbor should be in cluster"
        );
        assert!(
            !table_ids.contains(&2),
            "2-hop table should NOT be in cluster"
        );
        assert!(
            !table_ids.contains(&3),
            "3-hop table should NOT be in cluster"
        );
    }

    #[test]
    fn test_max_cluster_tables_caps() {
        // Hub t0 with 5 neighbors
        let mut columns = vec![fake_col(0, 0, "id", vec![1.0, 0.0])];
        let mut fks = vec![];
        for i in 1..=5 {
            let cid = columns.len();
            columns.push(fake_col(cid, i, "id", vec![0.5, 0.5]));
            let fk_cid = columns.len();
            columns.push(fake_col(fk_cid, i, "t0_id", vec![0.5, 0.5]));
            fks.push((0, cid, 0, i)); // t0 -> ti
        }
        let graph = make_graph(columns, fks);
        let query = vec![1.0, 0.0];
        let result = Retriever::retrieve(&graph, &query, 1, 3, 3);
        // Cluster should be capped at 3 tables (t0 + 2 neighbors)
        assert_eq!(
            result.tables.len(),
            3,
            "should be capped at max_cluster_tables=3"
        );
        assert!(
            result.tables.iter().any(|t| t.id == 0),
            "seed table always included"
        );
    }

    #[test]
    fn test_no_nearby_columns_still_returns_top_k() {
        let graph = make_graph(
            vec![
                fake_col(0, 0, "a", vec![1.0, 0.0, 0.0]),
                fake_col(1, 0, "b", vec![0.0, 1.0, 0.0]),
            ],
            vec![],
        );
        let query = vec![0.0, 0.0, 1.0];
        let result = Retriever::retrieve(&graph, &query, 5, 2, 10);
        // Neither column matches the query well; should still return min(columns, top_k) results
        assert_eq!(result.matched.len(), 2);
        assert!(
            result.matched[0].score < 0.05,
            "scores should be near zero for orthogonal vectors"
        );
    }

    #[test]
    fn test_relationships_only_for_clustered_tables() {
        // t0 -> t1 (in cluster), t0 -> t2 (outside cluster)
        let graph = make_graph(
            vec![
                fake_col(0, 0, "id", vec![1.0, 0.0]),
                fake_col(1, 0, "t1_id", vec![0.5, 0.5]),
                fake_col(2, 1, "id", vec![0.9, 0.1]),
                fake_col(3, 0, "t2_id", vec![0.0, 0.0]),
                fake_col(4, 2, "id", vec![0.0, 0.0]),
            ],
            vec![
                (1, 2, 0, 1), // t0.t1_id -> t1.id
                (3, 4, 0, 2), // t0.t2_id -> t2.id
            ],
        );
        let query = vec![1.0, 0.0];
        let result = Retriever::retrieve(&graph, &query, 1, 1, 10);
        // t0 matched, t1 is 1-hop neighbor. t2 is also 1-hop (same hop boundary)
        let rels = result.relationships.clone();
        assert!(
            rels.iter().any(|r| r.contains("t1_id")),
            "should include t0->t1 FK"
        );
    }

    #[test]
    fn test_empty_embedding_skipped() {
        let mut col = fake_col(0, 0, "empty_emb", vec![]);
        col.embedding = vec![];
        let graph = make_graph(vec![col, fake_col(1, 0, "normal", vec![1.0, 0.0])], vec![]);
        let query = vec![1.0, 0.0];
        let result = Retriever::retrieve(&graph, &query, 10, 2, 10);
        // Only column 1 should be in results (column 0 has empty embedding, filtered out)
        assert_eq!(result.matched.len(), 1);
        assert_eq!(result.matched[0].column_id, 1);
    }

    #[test]
    fn top_matches_and_cluster_from_matches_compose_to_same_result_as_retrieve() {
        let graph = make_graph(
            vec![
                fake_col(0, 0, "status", vec![1.0, 0.0, 0.0]),
                fake_col(1, 0, "name", vec![0.0, 1.0, 0.0]),
                fake_col(2, 1, "amount", vec![0.0, 0.0, 1.0]),
            ],
            vec![],
        );
        let query = vec![0.9, 0.1, 0.1];

        let direct = Retriever::retrieve(&graph, &query, 2, 2, 10);
        let matched = Retriever::top_matches(&graph, &query, 2);
        let composed = Retriever::cluster_from_matches(&graph, matched, 2, 10);

        assert_eq!(direct.matched.len(), composed.matched.len());
        assert_eq!(direct.matched[0].column_id, composed.matched[0].column_id);
        assert_eq!(direct.tables.len(), composed.tables.len());
    }
}
