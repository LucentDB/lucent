use lucent_protocol::{ConnectionId, QueryId};
use std::collections::HashMap;
use std::time::Duration;

use crate::ai::cache_store::{
    EmbeddingRow, PersistentVectorCache, DOC_TEXT_FORMAT_VERSION, MODEL_NAME,
};
use crate::ai::embed::Embedder;
use crate::ai::single_flight::SingleFlightEmbedder;
use crate::ai::truncate_utf8;
use crate::client::ConnectorClient;

/// Raw per-column tuple gathered while building the graph:
/// (column name, data type, schema, table, is_primary_key).
type ColumnTuple = (String, String, String, String, bool);
/// Columns grouped by owning table id.
type TableColumns = HashMap<usize, Vec<ColumnTuple>>;

/// How much of the schema index has been materialized for this connection.
/// Tier-1 is metadata only (fast, inside connect()); Tier-2 adds sampled
/// values + embeddings (background). Serialized into the persisted graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexingTier {
    #[default]
    MetadataOnly,
    FullyEnriched,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ColumnEntry {
    pub id: usize,
    pub table_id: usize,
    pub schema: String,
    pub table: String,
    pub name: String,
    pub data_type: String,
    pub is_primary_key: bool,
    #[serde(default)]
    pub sample_values: Vec<String>,
    #[serde(default)]
    pub fk_ref: Option<String>,
    #[serde(default)]
    pub embedding: Vec<f32>,
    pub doc_text: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TableEntry {
    pub id: usize,
    pub schema: String,
    pub name: String,
    /// Estimated row count from the driver's catalog; 0 when unknown.
    #[serde(default)]
    pub row_count_estimate: i64,
    /// "PARTITIONED BY RANGE (created_at) — 84 partitions" for partitioned parents.
    #[serde(default)]
    pub partition_info: Option<String>,
}

/// Human-readable partition annotation for a partitioned parent table.
pub fn partition_annotation(partkey: Option<&str>, partition_count: i64) -> Option<String> {
    partkey.map(|k| format!("PARTITIONED BY {k} — {partition_count} partitions"))
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct FkEdge {
    pub from_column: usize,
    pub to_column: usize,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SchemaGraph {
    pub tables: Vec<TableEntry>,
    pub columns: Vec<ColumnEntry>,
    #[serde(default)]
    pub columns_by_table: HashMap<usize, Vec<usize>>,
    #[serde(default)]
    pub fk_edges: Vec<FkEdge>,
    #[serde(default)]
    pub table_adjacency: HashMap<usize, Vec<usize>>,
    /// Epoch seconds (serializable — replaces the old `std::time::Instant`).
    #[serde(default)]
    pub built_at_unix: i64,
    /// Metadata-only until the background indexer finishes enrich().
    #[serde(default)]
    pub tier: IndexingTier,
}

pub struct SchemaIndexer;

/// Turn normalized catalog results into the indexer's working structures.
///
/// Pure: no I/O, no async. Everything provider-specific already happened below
/// the `Connector` seam.
pub(crate) fn harvest_to_entries(
    objects: Vec<lucent_protocol::ObjectSummary>,
    details: Vec<lucent_protocol::ObjectDetail>,
) -> (Vec<TableEntry>, TableColumns) {
    let mut tables: Vec<TableEntry> = Vec::new();
    let mut table_map: HashMap<(String, String), usize> = HashMap::new();

    for object in objects {
        // Partition children are collapsed into the parent: indexing 84
        // near-identical partitions poisons retrieval and blows the context
        // budget while adding zero schema information.
        if object.is_partition_child {
            continue;
        }
        let schema = object.reference.namespace.join(".");
        let name = object.reference.name;
        let id = tables.len();
        table_map.insert((schema.clone(), name.clone()), id);
        tables.push(TableEntry {
            id,
            schema,
            name,
            // The field is an i64 with a "0 when unknown" contract; the
            // Option is where the real fidelity lives if this ever widens.
            row_count_estimate: object.est_rows.unwrap_or(0) as i64,
            partition_info: object
                .partition
                .as_ref()
                .map(|p| partition_annotation(p.key.as_deref(), p.child_count as i64))
                .unwrap_or(None),
        });
    }

    let mut table_columns: TableColumns = HashMap::new();
    for detail in details {
        let schema = detail.reference.namespace.join(".");
        let table = detail.reference.name;
        // A detail for a table the listing excluded (a partition child) must
        // not conjure a phantom table entry.
        let Some(&tid) = table_map.get(&(schema.clone(), table.clone())) else {
            continue;
        };
        for c in detail.columns {
            table_columns.entry(tid).or_default().push((
                c.name,
                c.type_name,
                schema.clone(),
                table.clone(),
                c.is_primary_key,
            ));
        }
    }

    (tables, table_columns)
}

pub(crate) const RANGE_TYPES: &[&str] = &[
    "tstzrange",
    "tsrange",
    "daterange",
    "int4range",
    "int8range",
    "numrange",
];

/// Build the text that gets embedded for a column. Range-typed columns (tstzrange,
/// daterange, etc.) get an explicit join-hazard hint: these typically mean the table
/// is time-versioned (e.g. a route's airplane assignment changes over time), so a
/// naive join on the table's other key columns alone can silently fan out across
/// historical periods — this was directly observed corrupting a join result (the same
/// flight_id resolving to two different airplane_codes) before this hint existed.
///
/// Deliberately metadata-only: sample values are NEVER part of the embedded text.
/// The sampler is non-deterministic (LIMIT without ORDER BY), so values in the hash
/// would silently invalidate the cache on data churn and break the differential
/// re-index claim. `ColumnEntry.sample_values` remains a separate field for value hints.
pub(crate) fn doc_text_for(schema: &str, table: &str, name: &str, data_type: &str) -> String {
    let mut parts = vec![format!("{schema}.{table}.{name} {data_type}")];
    if RANGE_TYPES.contains(&data_type) {
        parts.push(format!(
            "(time-versioned — joins on {table}'s other key columns alone may fan out \
             across historical periods; also filter on {name})"
        ));
    }
    parts.join(" ")
}

/// Types whose values carry no semantic-search signal (or are unbounded blobs).
const UNSAMPLEABLE_TYPES: &[&str] = &[
    "bytea", "json", "jsonb", "oid", "uuid", "point", "line", "lseg", "box", "path", "polygon",
    "circle",
];

/// Build the batched value-sampling query as ONE statement (the prepared-statement
/// execution path rejects multi-statement strings — this exact bug silently emptied
/// every column's sample_values). Each column contributes:
///   (SELECT {tid}, '{col}', val FROM (bounded scan of 1000 non-null values) GROUP BY val LIMIT 20)
/// The inner LIMIT bounds the scan so a 100M-row table costs a 1000-row read,
/// not a full-table hash aggregate.
fn build_sampling_sql(
    tables: &[TableEntry],
    table_columns: &TableColumns,
    builder: &dyn crate::sql_builder::SqlBuilder,
    driver_id: &str,
) -> Option<String> {
    let mut subqueries: Vec<String> = Vec::new();
    for table_entry in tables {
        let qualified = format!(
            "{}.{}",
            builder.quote_identifier(&table_entry.schema),
            builder.quote_identifier(&table_entry.name)
        );
        let Some(cols) = table_columns.get(&table_entry.id) else {
            continue;
        };
        for (name, data_type, _, _, is_pk) in cols {
            if *is_pk || UNSAMPLEABLE_TYPES.contains(&data_type.as_str()) {
                // jsonb objects hide the most human-searched values in this
                // domain (aircraft models, airport/city names). Extract their
                // scalar string values via jsonb_each_text — guarded by
                // jsonb_typeof, which would otherwise error on arrays/scalars
                // and kill the whole batch. These are PostgreSQL functions: a
                // driver without JSON sampling simply gets fewer sample values,
                // which degrades retrieval quality without breaking it.
                if data_type == "jsonb" && driver_id == "postgres" {
                    let quoted_col = builder.quote_identifier(name);
                    let literal_col = name.replace('\'', "''");
                    subqueries.push(format!(
                        "(SELECT {tid} AS tid, '{literal_col}' AS col, val FROM \
                          (SELECT DISTINCT v.value AS val \
                           FROM (SELECT {quoted_col} AS j FROM {qualified} \
                                 WHERE {quoted_col} IS NOT NULL \
                                   AND jsonb_typeof({quoted_col}) = 'object' \
                                 LIMIT 200) _src, \
                                LATERAL jsonb_each_text(_src.j) v \
                           WHERE length(v.value) BETWEEN 2 AND 60 \
                           LIMIT 20) _j)",
                        tid = table_entry.id,
                    ));
                }
                continue;
            }
            let quoted_col = builder.quote_identifier(name);
            let literal_col = name.replace('\'', "''");
            let cast_col = builder.cast_to_text(&quoted_col);
            subqueries.push(format!(
                "(SELECT {tid} AS tid, '{literal_col}' AS col, val \
                  FROM (SELECT {cast_col} AS val FROM {qualified} \
                        WHERE {quoted_col} IS NOT NULL LIMIT 1000) _bounded \
                  GROUP BY val LIMIT 20)",
                tid = table_entry.id,
            ));
        }
    }
    if subqueries.is_empty() {
        None
    } else {
        Some(subqueries.join(" UNION ALL "))
    }
}

/// Cap a sampled column value at 197 bytes plus an ellipsis suffix.
fn truncate_sample_value(val_str: &str) -> String {
    if val_str.len() > 200 {
        format!("{}...", truncate_utf8(val_str, 197))
    } else {
        val_str.to_string()
    }
}

/// Canonical metadata snapshot of a schema, used for fingerprinting. The
/// snapshot types derive `Ord` so the caller sorts before hashing — ordering
/// must not affect the fingerprint.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CatalogSnapshot {
    pub format_version: u32,
    pub tables: Vec<SnapshotTable>,
    pub columns: Vec<SnapshotColumn>,
    pub fks: Vec<SnapshotFk>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct SnapshotTable {
    pub schema: String,
    pub name: String,
    pub row_count_estimate: i64,
    pub partition_info: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct SnapshotColumn {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub data_type: String,
    pub is_primary_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct SnapshotFk {
    pub from_schema: String,
    pub from_table: String,
    pub from_column: String,
    pub to_schema: String,
    pub to_table: String,
    pub to_column: String,
}

/// SHA256 over the bincode of the canonically-sorted snapshot. The snapshot
/// types derive Ord so the caller sorts before hashing (see `from_catalog`);
/// the sort is repeated here on a clone so order-stability holds even for an
/// unsorted snapshot passed directly.
pub fn compute_schema_hash(snapshot: &CatalogSnapshot) -> String {
    use sha2::Digest;
    let mut canonical = snapshot.clone();
    canonical.tables.sort();
    canonical.columns.sort();
    canonical.fks.sort();
    // Hash ONLY the stable identity fields. The table row-count estimate
    // moves on every autovacuum and `partition_info` changes on partition
    // layout edits — neither reflects a schema identity change, and hashing
    // them would invalidate the persisted Tier-2 graph on reconnect for
    // exactly the active schemas the differential cache targets.
    let tables: Vec<(&str, &str)> = canonical
        .tables
        .iter()
        .map(|t| (t.schema.as_str(), t.name.as_str()))
        .collect();
    let columns: Vec<(&str, &str, &str, &str, bool)> = canonical
        .columns
        .iter()
        .map(|c| {
            (
                c.schema.as_str(),
                c.table.as_str(),
                c.name.as_str(),
                c.data_type.as_str(),
                c.is_primary_key,
            )
        })
        .collect();
    let fks: Vec<(&str, &str, &str, &str, &str, &str)> = canonical
        .fks
        .iter()
        .map(|f| {
            (
                f.from_schema.as_str(),
                f.from_table.as_str(),
                f.from_column.as_str(),
                f.to_schema.as_str(),
                f.to_table.as_str(),
                f.to_column.as_str(),
            )
        })
        .collect();
    let bytes = bincode::serialize(&(canonical.format_version, tables, columns, fks))
        .expect("snapshot serializes");
    format!("{:x}", sha2::Sha256::digest(&bytes))
}

/// Version tag for the persisted Tier-2 graph blob. Bumped whenever the
/// serialized `SchemaGraph` layout changes so stale blobs are dropped and
/// re-indexed instead of failing forever on the same bytes (bincode is
/// order-sensitive — any field removal/rename breaks old blobs deliberately).
pub const GRAPH_FORMAT_VERSION: u32 = 1;

/// Serialize a Tier-2 graph for the connection cache: `(version, graph)` so a
/// loader can reject a blob written by an older/newer layout before
/// attempting (and failing) a bincode decode.
pub(crate) fn encode_persisted_graph(graph: &SchemaGraph) -> Result<Vec<u8>, String> {
    bincode::serialize(&(GRAPH_FORMAT_VERSION, graph)).map_err(|e| e.to_string())
}

/// Decode a persisted Tier-2 graph, rejecting mismatched format versions with
/// a typed error the caller can treat as "stale, re-index".
pub(crate) fn decode_persisted_graph(blob: &[u8]) -> Result<SchemaGraph, String> {
    let (version, graph): (u32, SchemaGraph) =
        bincode::deserialize(blob).map_err(|e| format!("corrupt cached graph: {e}"))?;
    if version != GRAPH_FORMAT_VERSION {
        return Err(format!(
            "cached graph format v{version} != current v{GRAPH_FORMAT_VERSION}"
        ));
    }
    Ok(graph)
}

/// Epoch seconds — the serializable replacement for `std::time::Instant`.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Phases the background indexer reports through `on_progress`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexingStage {
    Sampling,
    Embedding,
    Complete,
}

impl SchemaGraph {
    /// Tier-1: metadata only, no sampling, no embeddings. Fast enough to run
    /// inside connect(). Returns the canonical snapshot for fingerprinting.
    pub async fn from_catalog(
        connection_id: ConnectionId,
        client: &ConnectorClient,
        _capabilities: &lucent_protocol::DriverCapabilities,
    ) -> Result<(SchemaGraph, CatalogSnapshot), String> {
        // Harvest tables and columns through the catalog seam. Two requests
        // replace two hand-written Postgres queries; the FK edges come from a
        // third and are applied after embedding, as before.
        let objects = client
            .list_all_objects(connection_id, vec![lucent_protocol::ObjectKind::Table])
            .await
            .map_err(|e| format!("table metadata: {e}"))?;

        let refs: Vec<lucent_protocol::ObjectRef> = objects
            .iter()
            .filter(|o| !o.is_partition_child)
            .map(|o| o.reference.clone())
            .collect();

        let details = client
            .describe_objects(connection_id, refs)
            .await
            .map_err(|e| format!("columns: {e}"))?;

        let (tables, table_columns) = harvest_to_entries(objects, details);

        log::info!(
            "SchemaGraph::from_catalog: {} tables, {} columns from the catalog",
            tables.len(),
            table_columns.values().map(Vec::len).sum::<usize>()
        );

        // Build columns (stable metadata-only doc_text, empty embeddings) + FK edges.
        let mut columns: Vec<ColumnEntry> = Vec::new();
        let mut columns_by_table: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut col_lookup: HashMap<(String, String, String), usize> = HashMap::new();

        for (tid, col_infos) in &table_columns {
            let ids = columns_by_table.entry(*tid).or_default();
            for (name, data_type, schema, table, is_pk) in col_infos {
                let cid = columns.len();
                let doc_text = doc_text_for(schema, table, name, data_type);
                columns.push(ColumnEntry {
                    id: cid,
                    table_id: *tid,
                    schema: schema.clone(),
                    table: table.clone(),
                    name: name.clone(),
                    data_type: data_type.clone(),
                    is_primary_key: *is_pk,
                    sample_values: vec![],
                    fk_ref: None,
                    embedding: vec![],
                    doc_text,
                });
                ids.push(cid);
                col_lookup.insert((schema.clone(), table.clone(), name.clone()), cid);
            }
        }

        // Fetch FK constraints through the catalog seam — the driver answers
        // with normalized ForeignKey { from: ColumnPath, to: ColumnPath }.
        let fk_rows = client.list_foreign_keys(connection_id).await?;
        let mut fk_edges: Vec<FkEdge> = Vec::new();
        let mut table_adjacency: HashMap<usize, Vec<usize>> = HashMap::new();

        for fk in &fk_rows {
            let from_key = (
                fk.from.namespace.join("."),
                fk.from.table.clone(),
                fk.from.column.clone(),
            );
            let to_key = (
                fk.to.namespace.join("."),
                fk.to.table.clone(),
                fk.to.column.clone(),
            );
            if let Some(&from_cid) = col_lookup.get(&from_key) {
                if let Some(&to_cid) = col_lookup.get(&to_key) {
                    let from_tid = columns[from_cid].table_id;
                    let to_tid = columns[to_cid].table_id;
                    fk_edges.push(FkEdge {
                        from_column: from_cid,
                        to_column: to_cid,
                    });
                    table_adjacency.entry(from_tid).or_default().push(to_tid);
                    if from_tid != to_tid {
                        table_adjacency.entry(to_tid).or_default().push(from_tid);
                    }
                }
            }
        }
        // Deduplicate adjacency lists
        for v in table_adjacency.values_mut() {
            v.sort();
            v.dedup();
        }

        // Annotate columns with their FK reference targets
        for fk in &fk_edges {
            let to = &columns[fk.to_column];
            columns[fk.from_column].fk_ref = Some(format!("{}.{}", to.table, to.name));
        }

        log::info!(
            "SchemaGraph::from_catalog: {} FK edges, {} table adjacencies",
            fk_edges.len(),
            table_adjacency.len()
        );

        // Canonical snapshot for fingerprinting — sorted so query order never
        // changes the hash.
        let mut snapshot = CatalogSnapshot {
            format_version: DOC_TEXT_FORMAT_VERSION,
            tables: tables
                .iter()
                .map(|t| SnapshotTable {
                    schema: t.schema.clone(),
                    name: t.name.clone(),
                    row_count_estimate: t.row_count_estimate,
                    partition_info: t.partition_info.clone(),
                })
                .collect(),
            columns: columns
                .iter()
                .map(|c| SnapshotColumn {
                    schema: c.schema.clone(),
                    table: c.table.clone(),
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                    is_primary_key: c.is_primary_key,
                })
                .collect(),
            fks: fk_edges
                .iter()
                .map(|e| {
                    let from = &columns[e.from_column];
                    let to = &columns[e.to_column];
                    SnapshotFk {
                        from_schema: from.schema.clone(),
                        from_table: from.table.clone(),
                        from_column: from.name.clone(),
                        to_schema: to.schema.clone(),
                        to_table: to.table.clone(),
                        to_column: to.name.clone(),
                    }
                })
                .collect(),
        };
        snapshot.tables.sort();
        snapshot.columns.sort();
        snapshot.fks.sort();

        let graph = SchemaGraph {
            tables,
            columns,
            columns_by_table,
            fk_edges,
            table_adjacency,
            built_at_unix: now_unix(),
            tier: IndexingTier::MetadataOnly,
        };
        Ok((graph, snapshot))
    }
}

impl SchemaIndexer {
    /// TEMPORARY shim (removed by T2.4): the connect path still calls
    /// `build_index` with this exact shape until IndexingManager::start lands.
    /// Internally it now runs the Tier-1 harvest (from_catalog) and then the
    /// old inline sampling + embedding — behavior identical to the pre-T2.3
    /// connect-time build, but no persistent cache (the connect path has no
    /// connection key until T2.4 wires the manager).
    pub async fn build_index(
        connection_id: ConnectionId,
        client: &ConnectorClient,
        embedder: &Embedder,
        include_sample_values: bool,
        capabilities: &lucent_protocol::DriverCapabilities,
    ) -> Result<SchemaGraph, String> {
        let start = std::time::Instant::now();
        let (mut graph, _snapshot) =
            SchemaGraph::from_catalog(connection_id, client, capabilities).await?;

        // Old Step 3: fetch sample values — ONE combined query across all
        // tables (per-table round trips caused a ~23x regression: 214ms → 4.88s
        // on a 12-table / 77-column schema). Bounded scan LIMIT 1000, dedup via
        // GROUP BY, capped at 20 values per column.
        if include_sample_values {
            let table_columns = graph_columns_by_table(&graph);
            let builder = crate::sql_builder::for_driver(capabilities);
            if let Some(sql) = build_sampling_sql(
                &graph.tables,
                &table_columns,
                builder.as_ref(),
                &capabilities.id,
            ) {
                // Session-level timeout as its own statement (SET LOCAL needs a
                // transaction and multi-statement strings are rejected outright).
                let _ = client
                    .execute(connection_id, "SET statement_timeout = 3000")
                    .await;
                let query_result = client.execute(connection_id, &sql).await;
                let _ = client
                    .execute(connection_id, "SET statement_timeout = 0")
                    .await;
                match query_result {
                    Ok(res) => apply_sample_values(&mut graph, &res),
                    Err(e) => {
                        log::warn!(
                            "Value sampling failed for the whole batch, continuing with name+type only: {e}"
                        );
                    }
                }
            }
        }

        // Old Step 6: embed all column documents.
        let doc_texts: Vec<&str> = graph.columns.iter().map(|c| c.doc_text.as_str()).collect();
        let embeddings = match embedder.embed(&doc_texts).await {
            Ok(embs) => embs,
            Err(e) => {
                log::warn!(
                    "Batch embed failed ({e}), retrying column-by-column to isolate bad input"
                );
                let mut embs = Vec::with_capacity(graph.columns.len());
                for (i, c) in graph.columns.iter().enumerate() {
                    match embedder.embed(&[&c.doc_text]).await {
                        Ok(mut v) => {
                            embs.push(v.pop().unwrap_or_default());
                        }
                        Err(col_err) => {
                            log::warn!(
                                "Skipping column {} due to embed error: {col_err}",
                                c.doc_text
                            );
                            embs.push(vec![]);
                        }
                    }
                    if (i + 1) % 250 == 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                    }
                }
                embs
            }
        };

        for (c, emb) in graph.columns.iter_mut().zip(embeddings) {
            c.embedding = emb;
        }
        graph.tier = IndexingTier::FullyEnriched;

        let elapsed = start.elapsed();
        log::info!(
            "SchemaGraph built in {elapsed:?}: {} tables, {} columns",
            graph.tables.len(),
            graph.columns.len()
        );
        Ok(graph)
    }

    /// Tier-2: enrich a Tier-1 graph in the background. Fast path loads the
    /// persisted Tier-2 graph on an unchanged-schema fingerprint hit (zero
    /// catalog queries, zero ONNX). Otherwise: chunked value sampling on the
    /// sampling connection (session B, 5s client timeout + cancel backstop),
    /// bulk BLAKE3 cache lookup, single-flight embedding of ONLY the misses,
    /// persistence of embeddings + graph blob + fingerprint, and a copy-on-write
    /// swap of the enriched graph.
    ///
    /// The 11-parameter signature is plan-mandated (used verbatim by T2.4's
    /// IndexingManager) — a context struct would churn every caller for no
    /// behavior change, so the lint is allowed at the function level.
    #[allow(clippy::too_many_arguments)]
    pub async fn enrich(
        _connection_id: ConnectionId,
        snapshot: &CatalogSnapshot,
        graph: &SchemaGraph,
        client: Option<&ConnectorClient>,
        sampling_connection_id: Option<ConnectionId>,
        embedder: &SingleFlightEmbedder,
        cache: &PersistentVectorCache,
        connection_key: &str,
        sample_values: bool,
        capabilities: &lucent_protocol::DriverCapabilities,
        on_progress: &(dyn Fn(IndexingStage, usize, usize, usize, usize) + Send + Sync),
    ) -> Result<SchemaGraph, String> {
        let total = graph.tables.len();
        let column_count = graph.columns.len();

        // Fast path: unchanged schema with a persisted Tier-2 graph.
        if let Some(entry) = cache.get_connection_cache(connection_key).await? {
            if entry.schema_hash == compute_schema_hash(snapshot) {
                match decode_persisted_graph(&entry.graph_blob) {
                    Ok(mut tier2) => {
                        tier2.built_at_unix = now_unix();
                        on_progress(IndexingStage::Complete, total, total, column_count, 0);
                        return Ok(tier2);
                    }
                    Err(e) => {
                        // Corrupt or format-stale blob: drop it and fall through
                        // to re-index instead of failing forever on the same
                        // bad bytes (bincode is order-sensitive, so any layout
                        // change breaks old blobs deliberately).
                        log::warn!(
                            "cached graph for {connection_key} unreadable ({e}); re-indexing"
                        );
                        let _ = cache.delete_connection_cache(connection_key).await;
                    }
                }
            }
        }

        let mut tier2 = graph.clone();
        let table_columns = graph_columns_by_table(graph);

        // 1. Sampling — chunked 10 tables/statement on session B ONLY. The
        //    statement timeout is scoped to a short transaction (BEGIN → SET
        //    LOCAL → query → COMMIT/ROLLBACK) so it can never leak onto a
        //    shared session, and there is NO fallback to the editor connection:
        //    sampling the user's active session would mutate its timeout.
        if sample_values {
            if let Some(sampling_conn_id) = sampling_connection_id {
                if let Some(client) = client {
                    let chunks: Vec<&[TableEntry]> = graph.tables.chunks(10).collect();
                    for (ci, chunk) in chunks.iter().enumerate() {
                        on_progress(IndexingStage::Sampling, ci * 10 + chunk.len(), total, 0, 0);
                        let builder = crate::sql_builder::for_driver(capabilities);
                        let Some(sql) = build_sampling_sql(
                            chunk,
                            &table_columns,
                            builder.as_ref(),
                            &capabilities.id,
                        ) else {
                            continue;
                        };
                        // The transaction is the timeout-scoping mechanism; on
                        // ANY failure path we roll back so session B never
                        // retains an open sampling transaction.
                        if let Err(e) = client.execute(sampling_conn_id, "BEGIN").await {
                            log::warn!("sampling chunk {ci}: BEGIN failed: {e}; skipping chunk");
                            break;
                        }
                        if let Err(e) = client
                            .execute(sampling_conn_id, "SET LOCAL statement_timeout = 3000")
                            .await
                        {
                            log::warn!("sampling chunk {ci}: SET LOCAL failed: {e}; rolling back");
                            if let Err(rb) = client.execute(sampling_conn_id, "ROLLBACK").await {
                                log::warn!("sampling chunk {ci}: ROLLBACK failed: {rb}");
                            }
                            break;
                        }
                        let query_id = QueryId(uuid::Uuid::new_v4());
                        let result = tokio::time::timeout(
                            Duration::from_secs(5),
                            client.execute_with_id(query_id, sampling_conn_id, &sql, None),
                        )
                        .await;
                        match result {
                            Ok(Ok((res, _qid))) => {
                                if let Err(e) = client.execute(sampling_conn_id, "COMMIT").await {
                                    log::warn!(
                                        "sampling chunk {ci}: COMMIT failed: {e}; rolling back"
                                    );
                                    if let Err(rb) =
                                        client.execute(sampling_conn_id, "ROLLBACK").await
                                    {
                                        log::warn!("sampling chunk {ci}: ROLLBACK failed: {rb}");
                                    }
                                    break;
                                }
                                apply_sample_values(&mut tier2, &res);
                            }
                            Ok(Err(e)) => {
                                log::warn!("sampling chunk {ci} failed: {e}");
                                if let Err(rb) = client.execute(sampling_conn_id, "ROLLBACK").await
                                {
                                    log::warn!("sampling chunk {ci}: ROLLBACK failed: {rb}");
                                }
                                break;
                            }
                            Err(_elapsed) => {
                                log::warn!("sampling chunk {ci} timed out; cancelling");
                                let _ = client.cancel(sampling_conn_id, query_id).await;
                                if let Err(rb) = client.execute(sampling_conn_id, "ROLLBACK").await
                                {
                                    log::warn!("sampling chunk {ci}: ROLLBACK failed: {rb}");
                                }
                                break;
                            }
                        }
                    }
                } else {
                    log::info!("no DB client available; skipping value sampling");
                }
            } else {
                log::info!("no dedicated session B available; skipping value sampling this run");
            }
        }

        // 2. Hashes + bulk cache lookup.
        let doc_texts: Vec<String> = tier2.columns.iter().map(|c| c.doc_text.clone()).collect();
        let hashes: Vec<String> = doc_texts
            .iter()
            .map(|t| PersistentVectorCache::compute_doc_hash(t))
            .collect();
        let cached = cache.get_embeddings(&hashes).await?;

        // 3. Embed ONLY the misses.
        let mut missing: Vec<(usize, String)> = Vec::new();
        for (i, h) in hashes.iter().enumerate() {
            if !cached.contains_key(h) {
                missing.push((i, h.clone()));
            }
        }
        if !missing.is_empty() {
            on_progress(
                IndexingStage::Embedding,
                cached.len(),
                hashes.len(),
                cached.len(),
                missing.len(),
            );
            let missing_texts: Vec<String> =
                missing.iter().map(|(i, _)| doc_texts[*i].clone()).collect();
            match embedder.embed_missing(&missing_texts).await {
                Ok(new_embeddings) => {
                    let rows: Vec<EmbeddingRow> = missing
                        .iter()
                        .zip(new_embeddings.iter())
                        .filter(|(_, e)| !e.is_empty())
                        .map(|((i, h), e)| EmbeddingRow {
                            doc_hash: h.clone(),
                            model_name: MODEL_NAME.into(),
                            doc_text: doc_texts[*i].clone(),
                            embedding: e.clone(),
                        })
                        .collect();
                    if let Err(e) = cache.put_embeddings(&rows).await {
                        log::warn!(
                            "persisting {} embeddings failed ({e}); continuing with in-memory vectors",
                            rows.len()
                        );
                    }
                    for ((i, _), e) in missing.iter().zip(new_embeddings.iter()) {
                        tier2.columns[*i].embedding = e.clone();
                    }
                }
                Err(e) => {
                    // Degrade, don't fail the whole graph (spec contract: a
                    // failed embedding must not fail the graph). The tier-2
                    // clone keeps whatever resolved from the cache and is
                    // still swapped; the blob is NOT persisted so the next
                    // reconnect re-attempts the missing columns.
                    log::warn!(
                        "embedding {} missing columns failed ({e}); degrading to cached embeddings only",
                        missing.len()
                    );
                }
            }
        }
        for (i, h) in hashes.iter().enumerate() {
            if let Some(v) = cached.get(h) {
                tier2.columns[i].embedding = v.clone();
            }
        }

        // 4. Persist the Tier-2 graph + fingerprint — only when every missing
        //    column was embedded. A degraded run (embed failure) is swapped but
        //    not pinned, so the next reconnect re-attempts it. The persisted
        //    blob is version-tagged and carries NO sample values: live row
        //    values are privacy-sensitive and never belong in the on-disk
        //    cache (they stay in-memory for the current session only).
        let embedded_count = tier2
            .columns
            .iter()
            .filter(|c| !c.embedding.is_empty())
            .count();
        if embedded_count == hashes.len() {
            tier2.tier = IndexingTier::FullyEnriched;
            let mut persist_graph = tier2.clone();
            for col in &mut persist_graph.columns {
                col.sample_values.clear();
            }
            let blob = encode_persisted_graph(&persist_graph).map_err(|e| e.to_string())?;
            if let Err(e) = cache
                .put_connection_cache(connection_key, &compute_schema_hash(snapshot), &blob)
                .await
            {
                log::warn!("persisting the tier-2 graph failed ({e}); skipping cache write");
            }
        } else {
            log::warn!(
                "tier-2 graph incomplete ({embedded_count}/{} columns embedded); swapping without persisting",
                hashes.len()
            );
        }
        on_progress(
            IndexingStage::Complete,
            total,
            total,
            cached.len(),
            missing.len(),
        );
        Ok(tier2)
    }
}

/// Rebuild the per-table column map (name, data_type, schema, table, is_pk)
/// from a graph — used by sampling in both the temporary shim and enrich.
fn graph_columns_by_table(graph: &SchemaGraph) -> TableColumns {
    let mut table_columns: TableColumns = HashMap::new();
    for col in &graph.columns {
        table_columns.entry(col.table_id).or_default().push((
            col.name.clone(),
            col.data_type.clone(),
            col.schema.clone(),
            col.table.clone(),
            col.is_primary_key,
        ));
    }
    table_columns
}

/// Attach sampled values to the tier-2 clone. Row shape from
/// build_sampling_sql's UNION ALL: (tid, col, val) as JSON values.
fn apply_sample_values(tier2: &mut SchemaGraph, res: &crate::client::ExecuteResult) {
    let mut col_lookup: HashMap<(usize, String), usize> = HashMap::new();
    for (cid, col) in tier2.columns.iter().enumerate() {
        col_lookup.insert((col.table_id, col.name.clone()), cid);
    }
    for row in &res.rows {
        if let (Some(tid_f), Some(col_str), Some(val_str)) =
            (row[0].as_i64(), row[1].as_str(), row[2].as_str())
        {
            let truncated = truncate_sample_value(val_str);
            if let Some(cid) = col_lookup.get(&(tid_f as usize, col_str.to_string())) {
                let values = &mut tier2.columns[*cid].sample_values;
                if values.len() < 20 && !values.contains(&truncated) {
                    values.push(truncated);
                }
            }
        }
    }
}

#[cfg(test)]
mod sampling_sql_tests {
    use crate::sql_builder::PostgresSqlBuilder;

    use super::*;
    use std::collections::HashMap;

    fn pg() -> PostgresSqlBuilder {
        PostgresSqlBuilder
    }

    fn fixture() -> (Vec<TableEntry>, TableColumns) {
        let tables = vec![TableEntry {
            id: 0,
            schema: "public".into(),
            name: "invoices".into(),
            row_count_estimate: 0,
            partition_info: None,
        }];
        let mut cols = HashMap::new();
        cols.insert(
            0,
            vec![
                (
                    "id".to_string(),
                    "integer".to_string(),
                    "public".to_string(),
                    "invoices".to_string(),
                    true,
                ),
                (
                    "status".to_string(),
                    "text".to_string(),
                    "public".to_string(),
                    "invoices".to_string(),
                    false,
                ),
                (
                    "payload".to_string(),
                    "jsonb".to_string(),
                    "public".to_string(),
                    "invoices".to_string(),
                    false,
                ),
                (
                    "blob".to_string(),
                    "bytea".to_string(),
                    "public".to_string(),
                    "invoices".to_string(),
                    false,
                ),
            ],
        );
        (tables, cols)
    }

    #[test]
    fn sampling_sql_is_a_single_statement() {
        let (tables, cols) = fixture();
        let sql =
            build_sampling_sql(&tables, &cols, &pg(), "postgres").expect("has sampleable columns");
        assert!(
            !sql.contains(';'),
            "must be ONE statement — multi-statement strings \
                 are rejected by the prepared-statement path: {sql}"
        );
        assert!(
            !sql.to_uppercase().contains("SET LOCAL"),
            "timeout must not be inlined"
        );
    }

    #[test]
    fn sampling_sql_bounds_the_scan_before_grouping() {
        let (tables, cols) = fixture();
        let sql = build_sampling_sql(&tables, &cols, &pg(), "postgres").unwrap();
        assert!(
            sql.contains("LIMIT 1000"),
            "inner scan must be bounded so huge tables \
                 don't get a full-table hash aggregate: {sql}"
        );
        assert!(
            sql.contains("GROUP BY val"),
            "dedup happens over the bounded sample"
        );
    }

    #[test]
    fn sampling_sql_skips_pk_and_unsampleable_types() {
        let (tables, cols) = fixture();
        let sql = build_sampling_sql(&tables, &cols, &pg(), "postgres").unwrap();
        assert!(sql.contains("\"status\""), "text column is sampleable");
        assert!(!sql.contains("\"id\""), "PK columns are skipped");
        assert!(!sql.contains("\"blob\""), "bytea columns are skipped");
    }

    #[test]
    fn jsonb_object_columns_are_sampled_via_each_text() {
        let (tables, cols) = fixture();
        let sql = build_sampling_sql(&tables, &cols, &pg(), "postgres").unwrap();
        assert!(
            sql.contains("jsonb_each_text"),
            "jsonb values must be grounded: {sql}"
        );
        assert!(
            sql.contains("jsonb_typeof(\"payload\") = 'object'"),
            "non-object jsonb would crash jsonb_each_text — must be guarded: {sql}"
        );
        assert!(!sql.contains(';'), "still a single statement");
    }

    #[test]
    fn jsonb_sampling_is_skipped_for_non_postgres_drivers() {
        // jsonb_each_text / jsonb_typeof are PostgreSQL functions. A driver
        // without them must not emit them — it simply gets fewer sample values,
        // which degrades retrieval quality without breaking it.
        let (tables, cols) = fixture();
        let sql = build_sampling_sql(&tables, &cols, &pg(), "duckdb")
            .expect("non-jsonb columns are still sampleable");
        assert!(
            !sql.contains("jsonb_each_text"),
            "jsonb functions must not be emitted for a non-postgres driver: {sql}"
        );
        assert!(!sql.contains("jsonb_typeof"), "{sql}");
        assert!(
            sql.contains("\"status\""),
            "text columns still sampled: {sql}"
        );
    }

    #[test]
    fn sampling_sql_returns_none_when_nothing_qualifies() {
        let tables = vec![TableEntry {
            id: 0,
            schema: "public".into(),
            name: "blobs".into(),
            row_count_estimate: 0,
            partition_info: None,
        }];
        let mut cols = HashMap::new();
        cols.insert(
            0,
            vec![(
                "data".to_string(),
                "bytea".to_string(),
                "public".to_string(),
                "blobs".to_string(),
                false,
            )],
        );
        assert!(build_sampling_sql(&tables, &cols, &pg(), "postgres").is_none());
    }

    #[test]
    fn sampling_sql_escapes_identifiers_and_literals() {
        let tables = vec![TableEntry {
            id: 0,
            schema: "public".into(),
            name: "weird\"tbl".into(),
            row_count_estimate: 0,
            partition_info: None,
        }];
        let mut cols = HashMap::new();
        cols.insert(
            0,
            vec![(
                "odd'col".to_string(),
                "text".to_string(),
                "public".to_string(),
                "weird\"tbl".to_string(),
                false,
            )],
        );
        let sql = build_sampling_sql(&tables, &cols, &pg(), "postgres").unwrap();
        assert!(
            sql.contains("weird\"\"tbl"),
            "double quotes doubled in identifiers"
        );
        assert!(
            sql.contains("odd''col"),
            "single quotes doubled in the tag literal"
        );
    }
}

#[cfg(test)]
mod partition_tests {
    use super::*;

    #[test]
    fn partition_annotation_formats_strategy_and_count() {
        assert_eq!(
            partition_annotation(Some("RANGE (created_at)"), 84),
            Some("PARTITIONED BY RANGE (created_at) — 84 partitions".to_string()),
        );
    }

    #[test]
    fn partition_annotation_none_for_plain_tables() {
        assert_eq!(partition_annotation(None, 0), None);
    }

    #[test]
    fn table_entry_carries_row_estimate_and_partition_info() {
        let t = TableEntry {
            id: 0,
            schema: "bookings".into(),
            name: "events".into(),
            row_count_estimate: 215_000,
            partition_info: Some("PARTITIONED BY RANGE (created_at) — 84 partitions".into()),
        };
        assert_eq!(t.row_count_estimate, 215_000);
        assert!(t.partition_info.is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use uuid::Uuid;

    #[test]
    fn long_multibyte_sample_value_never_panics() {
        let val = "é".repeat(3000);
        let cut = truncate_sample_value(&val);
        assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
        assert!(cut.len() <= 200, "197 bytes plus '...' suffix: {cut}");
    }

    #[test]
    fn short_sample_value_passes_through_untouched() {
        assert_eq!(truncate_sample_value("CAN"), "CAN");
    }

    fn make_col_graph(columns: Vec<ColumnEntry>, fk_edges: Vec<FkEdge>) -> SchemaGraph {
        let mut tables = Vec::new();
        let mut columns_by_table: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut table_adjacency: HashMap<usize, Vec<usize>> = HashMap::new();

        for c in &columns {
            let tid = c.table_id;
            columns_by_table.entry(tid).or_default().push(c.id);
        }

        for fk in &fk_edges {
            let from_tid = columns[fk.from_column].table_id;
            let to_tid = columns[fk.to_column].table_id;
            table_adjacency.entry(from_tid).or_default().push(to_tid);
            if from_tid != to_tid {
                table_adjacency.entry(to_tid).or_default().push(from_tid);
            }
        }
        for v in table_adjacency.values_mut() {
            v.sort();
            v.dedup();
        }

        // Infer tables from columns
        let mut seen: HashMap<usize, bool> = HashMap::new();
        for c in &columns {
            if let std::collections::hash_map::Entry::Vacant(e) = seen.entry(c.table_id) {
                e.insert(true);
                tables.push(TableEntry {
                    id: c.table_id,
                    schema: c.schema.clone(),
                    name: c.table.clone(),
                    row_count_estimate: 0,
                    partition_info: None,
                });
            }
        }

        SchemaGraph {
            tables,
            columns,
            columns_by_table,
            fk_edges,
            table_adjacency,
            built_at_unix: 0,
            tier: IndexingTier::MetadataOnly,
        }
    }

    fn fake_col(
        id: usize,
        tid: usize,
        name: &str,
        data_type: &str,
        is_pk: bool,
        embedding: Vec<f32>,
    ) -> ColumnEntry {
        ColumnEntry {
            id,
            table_id: tid,
            schema: "public".into(),
            table: format!("t{tid}"),
            name: name.into(),
            data_type: data_type.into(),
            is_primary_key: is_pk,
            sample_values: vec![],
            fk_ref: None,
            embedding,
            doc_text: format!("public.t{tid}.{name} {data_type}"),
        }
    }

    #[test]
    fn test_column_count_and_properties() {
        let graph = make_col_graph(
            vec![
                fake_col(0, 0, "id", "INTEGER", true, vec![]),
                fake_col(1, 0, "name", "TEXT", false, vec![]),
                fake_col(2, 1, "id", "INTEGER", true, vec![]),
            ],
            vec![],
        );
        assert_eq!(graph.columns.len(), 3);
        assert_eq!(graph.tables.len(), 2);
        assert!(graph.columns[0].is_primary_key);
        assert!(!graph.columns[1].is_primary_key);
    }

    #[test]
    fn test_fk_edges_resolve_correctly() {
        let graph = make_col_graph(
            vec![
                fake_col(0, 0, "id", "INTEGER", true, vec![]),
                fake_col(1, 0, "org_id", "INTEGER", false, vec![]),
                fake_col(2, 1, "id", "INTEGER", true, vec![]),
            ],
            vec![FkEdge {
                from_column: 1,
                to_column: 2,
            }],
        );
        assert_eq!(graph.fk_edges.len(), 1);
        assert_eq!(graph.fk_edges[0].from_column, 1);
        assert_eq!(graph.fk_edges[0].to_column, 2);
        // table_adjacency should have both tables connected
        assert!(graph.table_adjacency.contains_key(&0));
        assert!(graph.table_adjacency.contains_key(&1));
        assert!(graph.table_adjacency[&0].contains(&1));
    }

    #[test]
    fn test_composite_fk_no_cross_join() {
        // Two-column FK (org_id, region_id) -> (id, region_id)
        // Should give exactly 2 edges, NOT a 2x2=4 cross product
        let graph = make_col_graph(
            vec![
                fake_col(0, 0, "a_id", "INTEGER", true, vec![]),
                fake_col(1, 0, "org_id", "INTEGER", false, vec![]),
                fake_col(2, 0, "region_id", "INTEGER", false, vec![]),
                fake_col(3, 1, "id", "INTEGER", true, vec![]),
                fake_col(4, 1, "region_id", "INTEGER", false, vec![]),
            ],
            vec![
                FkEdge {
                    from_column: 1,
                    to_column: 3,
                },
                FkEdge {
                    from_column: 2,
                    to_column: 4,
                },
            ],
        );
        assert_eq!(
            graph.fk_edges.len(),
            2,
            "composite FK should produce exactly 2 edges, not 4"
        );
        assert_eq!(graph.fk_edges[0].from_column, 1);
        assert_eq!(graph.fk_edges[0].to_column, 3);
        assert_eq!(graph.fk_edges[1].from_column, 2);
        assert_eq!(graph.fk_edges[1].to_column, 4);
    }

    #[test]
    fn test_columns_by_table_groups() {
        let graph = make_col_graph(
            vec![
                fake_col(0, 0, "id", "INTEGER", true, vec![]),
                fake_col(1, 0, "name", "TEXT", false, vec![]),
                fake_col(2, 1, "id", "INTEGER", true, vec![]),
            ],
            vec![],
        );
        assert_eq!(graph.columns_by_table[&0].len(), 2);
        assert_eq!(graph.columns_by_table[&1].len(), 1);
        assert!(graph.columns_by_table[&0].contains(&0));
        assert!(graph.columns_by_table[&0].contains(&1));
    }

    #[test]
    fn test_no_values_when_flag_false() {
        // When include_sample_values is false, sample_values should be empty
        // and doc_text should not contain "values:"
        let graph = make_col_graph(
            vec![fake_col(0, 0, "status", "TEXT", false, vec![])],
            vec![],
        );
        assert!(graph.columns[0].sample_values.is_empty());
        assert!(!graph.columns[0].doc_text.contains("values:"));
    }

    #[test]
    fn test_table_with_no_fk() {
        let graph = make_col_graph(vec![fake_col(0, 0, "id", "INTEGER", true, vec![])], vec![]);
        assert_eq!(graph.tables.len(), 1);
        assert_eq!(graph.columns.len(), 1);
        // table_adjacency should be empty (no FK edges)
        assert!(graph.table_adjacency.is_empty());
    }

    #[test]
    fn test_empty_schema() {
        let graph = make_col_graph(vec![], vec![]);
        assert_eq!(graph.tables.len(), 0);
        assert_eq!(graph.columns.len(), 0);
        assert!(graph.fk_edges.is_empty());
        assert!(graph.table_adjacency.is_empty());
    }

    #[test]
    fn test_skip_bad_column_on_embed_failure() {
        // Column with empty embedding is skipped in retrieval but the graph still builds
        let graph = make_col_graph(
            vec![
                fake_col(0, 0, "good", "TEXT", false, vec![0.1, 0.2, 0.3]),
                fake_col(1, 0, "bad", "TEXT", false, vec![]),
            ],
            vec![],
        );
        assert_eq!(graph.columns.len(), 2);
        assert!(!graph.columns[0].embedding.is_empty());
        assert!(graph.columns[1].embedding.is_empty());
    }

    #[test]
    fn range_typed_column_doc_text_includes_join_hazard_hint() {
        let col = doc_text_for("public", "routes", "validity", "tstzrange");
        assert!(
            col.contains("time-versioned") || col.contains("may fan out"),
            "range-typed columns must warn that a naive join on the table's other \
             columns can multiply rows across historical periods, got: {col}"
        );
    }

    #[test]
    fn plain_typed_column_doc_text_has_no_hazard_hint() {
        let col = doc_text_for("public", "users", "email", "text");
        assert!(
            !col.contains("time-versioned"),
            "non-range columns should not get this hint"
        );
    }

    #[tokio::test]
    async fn value_sampling_queries_run_concurrently_not_sequentially() {
        use std::time::Duration;
        use tokio::time::Instant;

        async fn fake_query(delay_ms: u64) -> u64 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            delay_ms
        }

        let start = Instant::now();
        let delays = vec![50u64, 50, 50, 50];
        let results = futures::future::join_all(delays.into_iter().map(fake_query)).await;
        let elapsed = start.elapsed();

        assert_eq!(results.len(), 4);
        assert!(
            elapsed < Duration::from_millis(150),
            "4 concurrent 50ms operations should take ~50ms total, not ~200ms — took {elapsed:?}. \
             This proves join_all-style concurrency is available and fast in this codebase's \
             async runtime; Step 3 applies the same pattern to the real per-table queries."
        );
    }

    use lucent_protocol::{
        ColumnDetail, ForeignKeyTarget, ObjectDetail, ObjectKind, ObjectRef, ObjectSummary,
        PartitionInfo,
    };

    fn summary(name: &str, est: Option<u64>) -> ObjectSummary {
        ObjectSummary {
            reference: ObjectRef {
                namespace: vec!["public".into()],
                name: name.into(),
                kind: ObjectKind::Table,
            },
            est_rows: est,
            comment: None,
            partition: None,
            is_partition_child: false,
        }
    }

    fn detail(name: &str, columns: Vec<ColumnDetail>) -> ObjectDetail {
        ObjectDetail {
            reference: ObjectRef {
                namespace: vec!["public".into()],
                name: name.into(),
                kind: ObjectKind::Table,
            },
            columns,
            comment: None,
        }
    }

    fn column(name: &str, ty: &str, pk: bool) -> ColumnDetail {
        ColumnDetail {
            name: name.into(),
            type_name: ty.into(),
            nullable: !pk,
            is_primary_key: pk,
            ordinal: 1,
            default: None,
            comment: None,
            foreign_key: None,
        }
    }

    #[test]
    fn builds_table_entries_from_summaries_preserving_estimates() {
        let (tables, _) = super::harvest_to_entries(
            vec![summary("users", Some(4200)), summary("orders", None)],
            vec![],
        );
        let users = tables.iter().find(|t| t.name == "users").unwrap();
        assert_eq!(users.row_count_estimate, 4200);
        // Unknown collapses to 0 for this field's existing i64 contract.
        let orders = tables.iter().find(|t| t.name == "orders").unwrap();
        assert_eq!(orders.row_count_estimate, 0);
    }

    #[test]
    fn partition_children_are_excluded_and_parents_keep_their_annotation() {
        let parent = ObjectSummary {
            partition: Some(PartitionInfo {
                key: Some("RANGE (created_at)".into()),
                child_count: 84,
            }),
            ..summary("events", Some(1))
        };
        let child = ObjectSummary {
            is_partition_child: true,
            ..summary("events_2026", Some(1))
        };
        let (tables, _) = super::harvest_to_entries(vec![parent, child], vec![]);
        assert_eq!(tables.len(), 1, "children must not be indexed: {tables:?}");
        assert_eq!(
            tables[0].partition_info.as_deref(),
            Some("PARTITIONED BY RANGE (created_at) — 84 partitions")
        );
    }

    #[test]
    fn columns_attach_to_their_table_with_pk_and_fk_intact() {
        let mut user_id = column("user_id", "bigint", false);
        user_id.foreign_key = Some(ForeignKeyTarget {
            namespace: vec!["public".into()],
            table: "users".into(),
            column: "id".into(),
        });

        let (tables, columns) = super::harvest_to_entries(
            vec![summary("orders", Some(1))],
            vec![detail(
                "orders",
                vec![column("id", "bigint", true), user_id],
            )],
        );

        assert_eq!(tables.len(), 1);
        let cols = columns.get(&tables[0].id).expect("orders has columns");
        assert_eq!(cols.len(), 2);
        // (name, data_type, schema, table, is_pk)
        assert!(cols.iter().any(|c| c.0 == "id" && c.4));
        assert!(cols.iter().any(|c| c.0 == "user_id" && !c.4));
    }

    #[test]
    fn a_column_whose_table_was_filtered_out_is_dropped_not_orphaned() {
        // DescribeObjects could return a table that ListAllObjects excluded
        // (a partition child). Its columns must not create a phantom table.
        let (tables, columns) = super::harvest_to_entries(
            vec![],
            vec![detail("ghost", vec![column("x", "int", false)])],
        );
        assert!(tables.is_empty());
        assert!(columns.is_empty());
    }

    // ─── T2.3: stable doc text, schema fingerprint, two-tier enrich ───────────

    #[test]
    fn doc_text_excludes_sample_values_and_keeps_range_hints() {
        let text = doc_text_for("public", "routes", "airplane_code", "tstzrange");
        assert!(text.starts_with("public.routes.airplane_code tstzrange"));
        assert!(text.contains("time-versioned"), "range hint retained");
        assert!(
            !text.contains("values:"),
            "samples must not enter the hash input"
        );
        let plain = doc_text_for("public", "users", "status", "text");
        assert_eq!(plain, "public.users.status text");
    }

    #[test]
    fn schema_hash_is_order_stable_and_content_sensitive() {
        let mut a = CatalogSnapshot {
            format_version: DOC_TEXT_FORMAT_VERSION,
            tables: vec![
                SnapshotTable {
                    schema: "public".into(),
                    name: "b".into(),
                    row_count_estimate: 0,
                    partition_info: None,
                },
                SnapshotTable {
                    schema: "public".into(),
                    name: "a".into(),
                    row_count_estimate: 0,
                    partition_info: None,
                },
            ],
            columns: vec![],
            fks: vec![],
        };
        let h1 = compute_schema_hash(&a);
        a.tables.reverse();
        assert_eq!(h1, compute_schema_hash(&a), "ordering must not matter");
        a.tables.push(SnapshotTable {
            schema: "public".into(),
            name: "c".into(),
            row_count_estimate: 0,
            partition_info: None,
        });
        assert_ne!(
            h1,
            compute_schema_hash(&a),
            "schema change must change the hash"
        );
    }

    #[tokio::test]
    async fn enrich_loads_cached_tier2_with_zero_embeds() {
        let dir = std::env::temp_dir().join(format!("lucent-enrich-cached-{}", std::process::id()));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db")).unwrap();
        let (graph, snapshot) = test_graph_and_snapshot();
        let mut tier2 = graph.clone();
        tier2.tier = IndexingTier::FullyEnriched;
        tier2.columns[0].embedding = vec![0.5, 0.5];
        let blob = encode_persisted_graph(&tier2).unwrap();
        let key = "test-connection-key".to_string();
        cache
            .put_connection_cache(&key, &compute_schema_hash(&snapshot), &blob)
            .await
            .unwrap();

        let calls = Arc::new(AtomicUsize::new(0));
        let embedder = SingleFlightEmbedder::new(Arc::new(CountingEmbed2 {
            calls: calls.clone(),
        }));
        let out = SchemaIndexer::enrich(
            ConnectionId(Uuid::new_v4()),
            &snapshot,
            &graph,
            None,
            None,
            &embedder,
            &cache,
            &key,
            false,
            &fake_capabilities(),
            &|_s, _p, _t, _h, _c| {},
        )
        .await
        .unwrap();
        assert_eq!(out.tier, IndexingTier::FullyEnriched);
        assert_eq!(out.columns[0].embedding, vec![0.5, 0.5]);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "cached path must not embed"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn enrich_embeds_exactly_the_missing_columns() {
        let dir =
            std::env::temp_dir().join(format!("lucent-enrich-missing-{}", std::process::id()));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db")).unwrap();
        let (graph, snapshot) = test_graph_and_snapshot();
        // Prime the cache for column 0 only.
        let h0 = PersistentVectorCache::compute_doc_hash(&graph.columns[0].doc_text);
        cache
            .put_embeddings(&[EmbeddingRow {
                doc_hash: h0,
                model_name: MODEL_NAME.into(),
                doc_text: graph.columns[0].doc_text.clone(),
                embedding: vec![1.0, 0.0],
            }])
            .await
            .unwrap();

        let calls = Arc::new(AtomicUsize::new(0));
        let embedder = SingleFlightEmbedder::new(Arc::new(CountingEmbed2 {
            calls: calls.clone(),
        }));
        let out = SchemaIndexer::enrich(
            ConnectionId(Uuid::new_v4()),
            &snapshot,
            &graph,
            None,
            None,
            &embedder,
            &cache,
            "key2",
            false,
            &fake_capabilities(),
            &|_s, _p, _t, _h, _c| {},
        )
        .await
        .unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "exactly one ONNX invocation"
        );
        assert_eq!(out.columns[0].embedding, vec![1.0, 0.0]);
        assert!(!out.columns[1].embedding.is_empty());
        // Persisted: a second enrich for the same fingerprint is a cache hit.
        let out2 = SchemaIndexer::enrich(
            ConnectionId(Uuid::new_v4()),
            &snapshot,
            &graph,
            None,
            None,
            &embedder,
            &cache,
            "key2",
            false,
            &fake_capabilities(),
            &|_s, _p, _t, _h, _c| {},
        )
        .await
        .unwrap();
        assert_eq!(out2.columns[1].embedding, out.columns[1].embedding);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "second run is a full cache hit"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn enrich_falls_through_on_a_corrupt_cached_blob_and_deletes_it() {
        let dir =
            std::env::temp_dir().join(format!("lucent-enrich-corrupt-{}", std::process::id()));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db")).unwrap();
        let (graph, snapshot) = test_graph_and_snapshot();
        let key = "corrupt-key".to_string();
        // A valid fingerprint with garbage bytes — bincode cannot decode it.
        cache
            .put_connection_cache(&key, &compute_schema_hash(&snapshot), b"not-a-graph")
            .await
            .unwrap();

        let calls = Arc::new(AtomicUsize::new(0));
        let embedder = SingleFlightEmbedder::new(Arc::new(CountingEmbed2 {
            calls: calls.clone(),
        }));
        let out = SchemaIndexer::enrich(
            ConnectionId(Uuid::new_v4()),
            &snapshot,
            &graph,
            None,
            None,
            &embedder,
            &cache,
            &key,
            false,
            &fake_capabilities(),
            &|_s, _p, _t, _h, _c| {},
        )
        .await
        .unwrap();
        assert_eq!(
            out.tier,
            IndexingTier::FullyEnriched,
            "corrupt blob falls through to a fresh re-index"
        );
        assert!(
            calls.load(Ordering::SeqCst) >= 1,
            "re-index embedded the columns"
        );
        // The corrupt row was deleted and replaced by a valid versioned blob.
        let entry = cache.get_connection_cache(&key).await.unwrap().unwrap();
        let decoded = decode_persisted_graph(&entry.graph_blob).unwrap();
        assert_eq!(decoded.tier, IndexingTier::FullyEnriched);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn enrich_persists_sample_values_but_never_at_rest() {
        let dir =
            std::env::temp_dir().join(format!("lucent-enrich-samples-{}", std::process::id()));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db")).unwrap();
        let (mut graph, snapshot) = test_graph_and_snapshot();
        // Give the tier-1 fixture live sample values (as sampling would).
        graph.columns[0].sample_values = vec!["alice".to_string(), "bob".to_string()];
        graph.columns[1].sample_values = vec!["active".to_string()];

        let calls = Arc::new(AtomicUsize::new(0));
        let embedder = SingleFlightEmbedder::new(Arc::new(CountingEmbed2 {
            calls: calls.clone(),
        }));
        let out = SchemaIndexer::enrich(
            ConnectionId(Uuid::new_v4()),
            &snapshot,
            &graph,
            None,
            None,
            &embedder,
            &cache,
            "samples-key",
            false,
            &fake_capabilities(),
            &|_s, _p, _t, _h, _c| {},
        )
        .await
        .unwrap();
        // The in-memory swapped graph keeps the samples for this session.
        assert!(!out.columns[0].sample_values.is_empty());
        assert!(!out.columns[1].sample_values.is_empty());

        // The persisted blob must carry NO sample values (privacy at rest).
        let entry = cache
            .get_connection_cache("samples-key")
            .await
            .unwrap()
            .unwrap();
        let decoded = decode_persisted_graph(&entry.graph_blob).unwrap();
        assert!(
            decoded.columns.iter().all(|c| c.sample_values.is_empty()),
            "persisted graph must not contain live row values"
        );
        // And a cache-hit reload yields an in-memory graph with empty samples.
        let out2 = SchemaIndexer::enrich(
            ConnectionId(Uuid::new_v4()),
            &snapshot,
            &graph,
            None,
            None,
            &embedder,
            &cache,
            "samples-key",
            false,
            &fake_capabilities(),
            &|_s, _p, _t, _h, _c| {},
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "cache hit after first run");
        assert!(
            out2.columns[0].sample_values.is_empty(),
            "cache-hit connections load the persisted graph with empty sample_values — the privacy fix strips samples from the blob, and the cache-hit fast path never re-harvests, so value hints stay empty until a schema change invalidates the fingerprint and triggers re-indexing"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_graph_and_snapshot() -> (SchemaGraph, CatalogSnapshot) {
        let tables = vec![TableEntry {
            id: 0,
            schema: "public".into(),
            name: "users".into(),
            row_count_estimate: 0,
            partition_info: None,
        }];
        let columns = vec![
            ColumnEntry {
                id: 0,
                table_id: 0,
                schema: "public".into(),
                table: "users".into(),
                name: "id".into(),
                data_type: "int4".into(),
                is_primary_key: true,
                sample_values: vec![],
                fk_ref: None,
                embedding: vec![],
                doc_text: doc_text_for("public", "users", "id", "int4"),
            },
            ColumnEntry {
                id: 1,
                table_id: 0,
                schema: "public".into(),
                table: "users".into(),
                name: "status".into(),
                data_type: "text".into(),
                is_primary_key: false,
                sample_values: vec![],
                fk_ref: None,
                embedding: vec![],
                doc_text: doc_text_for("public", "users", "status", "text"),
            },
        ];
        let graph = SchemaGraph {
            tables: tables.clone(),
            columns: columns.clone(),
            columns_by_table: HashMap::from([(0usize, vec![0usize, 1usize])]),
            fk_edges: vec![],
            table_adjacency: HashMap::new(),
            tier: IndexingTier::MetadataOnly,
            built_at_unix: 0,
        };
        let snapshot = CatalogSnapshot {
            format_version: DOC_TEXT_FORMAT_VERSION,
            tables: vec![SnapshotTable {
                schema: "public".into(),
                name: "users".into(),
                row_count_estimate: 0,
                partition_info: None,
            }],
            columns: vec![
                SnapshotColumn {
                    schema: "public".into(),
                    table: "users".into(),
                    name: "id".into(),
                    data_type: "int4".into(),
                    is_primary_key: true,
                },
                SnapshotColumn {
                    schema: "public".into(),
                    table: "users".into(),
                    name: "status".into(),
                    data_type: "text".into(),
                    is_primary_key: false,
                },
            ],
            fks: vec![],
        };
        (graph, snapshot)
    }

    struct CountingEmbed2 {
        calls: Arc<AtomicUsize>,
    }
    impl crate::ai::single_flight::Embed for CountingEmbed2 {
        fn embed<'a>(
            &'a self,
            texts: &'a [String],
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>, String>> + Send + 'a>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                Ok(texts
                    .iter()
                    .map(|t| vec![t.len() as f32, 1.0, 0.0])
                    .collect())
            })
        }
    }

    fn fake_capabilities() -> lucent_protocol::DriverCapabilities {
        lucent_protocol::DriverCapabilities {
            id: "fake".into(),
            display_name: "Fake".into(),
            sql_dialect: lucent_protocol::SqlDialect::PostgreSql,
            namespace_model: lucent_protocol::NamespaceModel::DbSchemaObject,
            readonly: lucent_protocol::ReadOnlyMode::TransactionScoped,
            statement_timeout: lucent_protocol::TimeoutSupport::Statement,
            cancel: lucent_protocol::CancelMode::Native,
            paging: lucent_protocol::PagingStyle::LimitOffset,
            identifier_quote: '"',
            string_literal: lucent_protocol::StringLiteralStyle::StandardConforming,
            auth: lucent_protocol::AuthModel::UserPassword,
        }
    }

    /// Manual cold-start diagnostic (NOT CI): builds a 100-table tier-1
    /// fixture and runs `enrich` against the REAL ONNX embedder. Timings are
    /// logged as diagnostics only — never asserted. Run with:
    /// `cargo test -p lucent --lib ai::schema_graph::tests::test_cold_100_tables -- --ignored`
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires ONNX model download; timing is diagnostic"]
    async fn test_cold_100_tables() {
        let dir = std::env::temp_dir().join(format!("lucent-cold-start-{}", std::process::id()));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db"))
            .expect("cache opens at temp dir");

        let mut tables = Vec::with_capacity(100);
        let mut columns = Vec::with_capacity(100);
        let mut columns_by_table = HashMap::new();
        for i in 0..100 {
            let name = format!("t{i:03}");
            tables.push(TableEntry {
                id: i,
                schema: "public".into(),
                name: name.clone(),
                row_count_estimate: 0,
                partition_info: None,
            });
            columns.push(ColumnEntry {
                id: i,
                table_id: i,
                schema: "public".into(),
                table: name,
                name: "id".into(),
                data_type: "int4".into(),
                is_primary_key: true,
                sample_values: vec![],
                fk_ref: None,
                embedding: vec![],
                doc_text: doc_text_for("public", &format!("t{i:03}"), "id", "int4"),
            });
            columns_by_table.insert(i, vec![i]);
        }
        let graph = SchemaGraph {
            tables: tables.clone(),
            columns: columns.clone(),
            columns_by_table,
            fk_edges: vec![],
            table_adjacency: HashMap::new(),
            tier: IndexingTier::MetadataOnly,
            built_at_unix: 0,
        };
        let snapshot = CatalogSnapshot {
            format_version: DOC_TEXT_FORMAT_VERSION,
            tables: tables
                .iter()
                .map(|t| SnapshotTable {
                    schema: t.schema.clone(),
                    name: t.name.clone(),
                    row_count_estimate: t.row_count_estimate,
                    partition_info: t.partition_info.clone(),
                })
                .collect(),
            columns: columns
                .iter()
                .map(|c| SnapshotColumn {
                    schema: c.schema.clone(),
                    table: c.table.clone(),
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                    is_primary_key: c.is_primary_key,
                })
                .collect(),
            fks: vec![],
        };

        let embedder = SingleFlightEmbedder::new(Arc::new(
            crate::ai::embed::Embedder::new().expect("real ONNX embedder initializes"),
        ));
        let started = std::time::Instant::now();
        let out = SchemaIndexer::enrich(
            ConnectionId(Uuid::new_v4()),
            &snapshot,
            &graph,
            None,
            None,
            &embedder,
            &cache,
            "cold-start-diagnostic-key",
            false,
            &fake_capabilities(),
            &|_s, _p, _t, _h, _c| {},
        )
        .await
        .expect("enrich completes on a cold cache");
        let elapsed = started.elapsed();

        assert_eq!(out.tier, IndexingTier::FullyEnriched);
        assert_eq!(out.columns.len(), 100);
        let embedded = out
            .columns
            .iter()
            .filter(|c| !c.embedding.is_empty())
            .count();
        let ratio = if out.columns.is_empty() {
            0.0
        } else {
            embedded as f64 / out.columns.len() as f64
        };
        log::info!(
            "[diagnostic] cold-start 100-table enrich: {elapsed:?} elapsed, {embedded}/{} columns embedded (cache-hit ratio {ratio:.2})",
            out.columns.len()
        );
        assert_eq!(embedded, out.columns.len(), "all columns embedded");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
