use lucent_protocol::ConnectionId;
use std::collections::HashMap;

use crate::ai::embed::Embedder;
use crate::ai::truncate_utf8;
use crate::client::ConnectorClient;

/// Raw per-column tuple gathered while building the graph:
/// (column name, data type, schema, table, is_primary_key).
type ColumnTuple = (String, String, String, String, bool);
/// Columns grouped by owning table id.
type TableColumns = HashMap<usize, Vec<ColumnTuple>>;

#[derive(Clone, Debug)]
pub struct ColumnEntry {
    pub id: usize,
    pub table_id: usize,
    pub schema: String,
    pub table: String,
    pub name: String,
    pub data_type: String,
    pub is_primary_key: bool,
    pub sample_values: Vec<String>,
    pub fk_ref: Option<String>,
    pub embedding: Vec<f32>,
    pub doc_text: String,
}

#[derive(Clone, Debug)]
pub struct TableEntry {
    pub id: usize,
    pub schema: String,
    pub name: String,
    /// Estimated row count from the driver's catalog; 0 when unknown.
    pub row_count_estimate: i64,
    /// "PARTITIONED BY RANGE (created_at) — 84 partitions" for partitioned parents.
    pub partition_info: Option<String>,
}

/// Human-readable partition annotation for a partitioned parent table.
pub fn partition_annotation(partkey: Option<&str>, partition_count: i64) -> Option<String> {
    partkey.map(|k| format!("PARTITIONED BY {k} — {partition_count} partitions"))
}

#[derive(Clone, Debug)]
pub struct FkEdge {
    pub from_column: usize,
    pub to_column: usize,
}

#[derive(Clone, Debug)]
pub struct SchemaGraph {
    pub tables: Vec<TableEntry>,
    pub columns: Vec<ColumnEntry>,
    pub columns_by_table: HashMap<usize, Vec<usize>>,
    pub fk_edges: Vec<FkEdge>,
    pub table_adjacency: HashMap<usize, Vec<usize>>,
    pub built_at: std::time::Instant,
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
fn doc_text_for(
    schema: &str,
    table: &str,
    name: &str,
    data_type: &str,
    _is_pk: bool,
    values: &[String],
) -> String {
    let mut parts = vec![format!("{schema}.{table}.{name} {data_type}")];
    if RANGE_TYPES.contains(&data_type) {
        parts.push(format!(
            "(time-versioned — joins on {table}'s other key columns alone may fan out \
             across historical periods; also filter on {name})"
        ));
    }
    if !values.is_empty() {
        parts.push(format!("values: {}", values.join(", ")));
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

impl SchemaIndexer {
    pub async fn build_index(
        connection_id: ConnectionId,
        client: &ConnectorClient,
        embedder: &Embedder,
        include_sample_values: bool,
        capabilities: &lucent_protocol::DriverCapabilities,
    ) -> Result<SchemaGraph, String> {
        let start = std::time::Instant::now();

        // Step 0-2: harvest tables and columns through the catalog seam. Two
        // requests replace two hand-written Postgres queries; the FK edges come
        // from a third and are applied after embedding, as before.
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
            "SchemaIndexer: {} tables, {} columns from the catalog",
            tables.len(),
            table_columns.values().map(Vec::len).sum::<usize>()
        );

        // Step 3: fetch sample values — collapsed into ONE combined query across all
        // tables instead of one query per table (which caused a ~23x regression:
        // 214ms → 4.88s on a 12-table / 77-column schema because N round trips
        // between the app and database add up fast over a single connection).
        //
        // Each column gets a parenthesized subquery with an inner bounded-scan (LIMIT 1000)
        // to avoid full-table hash aggregates on huge tables, then GROUP BY to deduplicate
        // and capped at 20 values per column.
        let mut sample_values: HashMap<(usize, String), Vec<String>> = HashMap::new();
        if include_sample_values {
            let builder = crate::sql_builder::for_driver(capabilities);
            if let Some(sql) =
                build_sampling_sql(&tables, &table_columns, builder.as_ref(), &capabilities.id)
            {
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
                    Ok(res) => {
                        for row in &res.rows {
                            if let (Some(tid_f), Some(col_str), Some(val_str)) =
                                (row[0].as_i64(), row[1].as_str(), row[2].as_str())
                            {
                                let truncated = truncate_sample_value(val_str);
                                sample_values
                                    .entry((tid_f as usize, col_str.to_string()))
                                    .or_default()
                                    .push(truncated);
                            }
                        }
                        log::info!(
                            "SchemaIndexer: sampled values for {} (table,column) pairs",
                            sample_values.len()
                        );
                    }
                    Err(e) => {
                        log::warn!(
                            "Value sampling failed for the whole batch, continuing with name+type only: {e}"
                        );
                    }
                }
            }
        }

        // Step 4: build columns + FK edges
        let mut columns: Vec<ColumnEntry> = Vec::new();
        let mut columns_by_table: HashMap<usize, Vec<usize>> = HashMap::new();
        let mut col_lookup: HashMap<(String, String, String), usize> = HashMap::new();

        for (tid, col_infos) in &table_columns {
            let ids = columns_by_table.entry(*tid).or_default();
            for (name, data_type, schema, table, is_pk) in col_infos {
                let cid = columns.len();
                let vals = sample_values
                    .get(&(*tid, name.clone()))
                    .cloned()
                    .unwrap_or_default();
                let doc_text = doc_text_for(schema, table, name, data_type, *is_pk, &vals);
                columns.push(ColumnEntry {
                    id: cid,
                    table_id: *tid,
                    schema: schema.clone(),
                    table: table.clone(),
                    name: name.clone(),
                    data_type: data_type.clone(),
                    is_primary_key: *is_pk,
                    sample_values: vals,
                    fk_ref: None,
                    embedding: vec![],
                    doc_text,
                });
                ids.push(cid);
                col_lookup.insert((schema.clone(), table.clone(), name.clone()), cid);
            }
        }

        // Step 5: fetch FK constraints through the catalog seam — the driver
        // answers with normalized ForeignKey { from: ColumnPath, to: ColumnPath }.
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
            "SchemaIndexer: {} FK edges, {} table adjacencies",
            fk_edges.len(),
            table_adjacency.len()
        );

        // Step 6: embed all column documents
        let doc_texts: Vec<&str> = columns.iter().map(|c| c.doc_text.as_str()).collect();
        let embeddings = match embedder.embed(&doc_texts).await {
            Ok(embs) => embs,
            Err(e) => {
                log::warn!(
                    "Batch embed failed ({e}), retrying column-by-column to isolate bad input"
                );
                let mut embs = Vec::with_capacity(columns.len());
                for (i, c) in columns.iter().enumerate() {
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

        for (c, emb) in columns.iter_mut().zip(embeddings) {
            c.embedding = emb;
        }

        let elapsed = start.elapsed();
        log::info!(
            "SchemaGraph built in {elapsed:?}: {} tables, {} columns",
            tables.len(),
            columns.len()
        );

        Ok(SchemaGraph {
            tables,
            columns,
            columns_by_table,
            fk_edges,
            table_adjacency,
            built_at: std::time::Instant::now(),
        })
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
            built_at: std::time::Instant::now(),
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
        let col = doc_text_for("public", "routes", "validity", "tstzrange", false, &[]);
        assert!(
            col.contains("time-versioned") || col.contains("may fan out"),
            "range-typed columns must warn that a naive join on the table's other \
             columns can multiply rows across historical periods, got: {col}"
        );
    }

    #[test]
    fn plain_typed_column_doc_text_has_no_hazard_hint() {
        let col = doc_text_for("public", "users", "email", "text", false, &[]);
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
}
