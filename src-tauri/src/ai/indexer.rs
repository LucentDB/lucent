//! Background schema indexer. One task per connection; aborted on disconnect
//! or app exit. Tier-1 graphs are built inline in connect(); this module
//! enriches them (sampling + embeddings) in the background and swaps the
//! Tier-2 graph into the shared slot.

use std::collections::HashMap;
use std::sync::Arc;

use lucent_protocol::{ConnectionId, DriverCapabilities};
use tokio::sync::Mutex;

use crate::ai::cache_store::PersistentVectorCache;
use crate::ai::embed::Embedder;
use crate::ai::events::IndexingProgressPayload;
use crate::ai::memory::drift::DriftAlert;
use crate::ai::memory::MemoryManager;
use crate::ai::schema_graph::{
    compute_schema_hash, snapshot_from_graph, CatalogSnapshot, SchemaGraph, SchemaIndexer,
};
use crate::ai::single_flight::SingleFlightEmbedder;
use crate::ai::AiConfig;
use crate::client::ConnectorClient;

pub trait IndexingEventSink: Send + Sync {
    fn emit_progress(&self, payload: IndexingProgressPayload);
    fn emit_error(&self, connection_id: &str, message: &str);
    /// Gate 1 schema drift: the indexer's catalog diff invalidated one or more
    /// dependent memories (spec §7.2). The indexer never calls this with an
    /// empty slice; it is the `memory:drift_detected` surface.
    fn emit_drift(&self, alerts: &[DriftAlert]);
}

#[derive(Clone)]
pub struct IndexingManager {
    /// Active indexer tasks keyed by connection id. Each entry carries the
    /// abort handle (for stop/stop_all) and the task id so a finishing task
    /// can remove ONLY its own entry (a reconnect could have replaced it).
    tasks: Arc<Mutex<HashMap<String, (tokio::task::AbortHandle, tokio::task::Id)>>>,
    cache: PersistentVectorCache,
    sink: Arc<dyn IndexingEventSink>,
}

impl IndexingManager {
    pub fn new(cache: PersistentVectorCache, sink: Arc<dyn IndexingEventSink>) -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            cache,
            sink,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        &self,
        connection_id: ConnectionId,
        client: Option<ConnectorClient>,
        sampling_connection_id: Option<ConnectionId>,
        graph_slot: Arc<Mutex<Option<SchemaGraph>>>,
        // The shared "which connection is live" slot. The indexer only swaps
        // its Tier-2 graph when this still names the connection it serves —
        // otherwise a reconnect would let a stale task clobber the new
        // connection's graph.
        current_connection_id: Arc<Mutex<Option<ConnectionId>>>,
        tier1: (SchemaGraph, CatalogSnapshot),
        embedder_slot: Arc<Mutex<Option<Embedder>>>,
        embedder_override: Option<Arc<dyn crate::ai::single_flight::Embed>>,
        config: Arc<AiConfig>,
        connection_key: String,
        memory_connection_key: String,
        capabilities: DriverCapabilities,
        memory_manager: Arc<MemoryManager>,
    ) {
        let conn_id_str = connection_id.0.to_string();
        let conn_id_for_task = conn_id_str.clone();
        let cache = self.cache.clone();
        let sink = self.sink.clone();
        let tasks = self.tasks.clone();
        let handle = tokio::spawn(async move {
            let start = std::time::Instant::now();
            let (graph, snapshot) = tier1;
            let total = graph.tables.len();

            // Model readiness: the ONNX model may still be downloading on the
            // very first run. Failure degrades to Tier-1, never blocks connect.
            // Test injection: embedder_override bypasses the real model.
            let embed: Option<Arc<dyn crate::ai::single_flight::Embed>> = match &embedder_override {
                Some(e) => Some(e.clone()),
                None if !config.enable_semantic_index => None,
                None => {
                    let existing = embedder_slot.lock().await.clone();
                    match existing {
                        Some(model) => {
                            Some(Arc::new(model) as Arc<dyn crate::ai::single_flight::Embed>)
                        }
                        None => {
                            sink.emit_progress(IndexingProgressPayload {
                                connection_id: conn_id_for_task.clone(),
                                stage: "model".into(),
                                processed_tables: 0,
                                total_tables: total,
                                added_tables_count: 0,
                                modified_tables_count: 0,
                                deleted_tables_count: 0,
                                unchanged_tables_count: total,
                                cache_hits: 0,
                                embeddings_computed: 0,
                                is_complete: false,
                                elapsed_ms: start.elapsed().as_millis() as u64,
                                detail: Some("Downloading embedding model (first run)…".into()),
                            });
                            match tokio::task::spawn_blocking(Embedder::new).await {
                                Ok(Ok(model)) => {
                                    *embedder_slot.lock().await = Some(model.clone());
                                    Some(Arc::new(model) as Arc<dyn crate::ai::single_flight::Embed>)
                                }
                                Ok(Err(e)) => {
                                    sink.emit_error(
                                        &conn_id_for_task,
                                        &format!("embedding model init failed: {e}"),
                                    );
                                    None
                                }
                                Err(e) => {
                                    sink.emit_error(
                                        &conn_id_for_task,
                                        &format!("embedding model init panicked: {e}"),
                                    );
                                    None
                                }
                            }
                        }
                    }
                }
            };

            // Gate 1 (spec §7.2): run the DDL cascade independently of the
            // embedder. Drift detection needs no embeddings, so a cold/offline
            // model (or `enable_semantic_index = false`) must still invalidate
            // stale memories. The persisted fingerprint is read BEFORE `enrich`
            // writes the new one, and the predecessor graph is the fallback when
            // the cache is cold. Scoped by the memory connection key (profile id
            // or the frontend's host:port/database key), not the vector-cache
            // hash, so it matches the keys `memories.connection_key` holds.
            let prior_graph = graph_slot.lock().await.clone();
            let still_current_before_enrich =
                *current_connection_id.lock().await == Some(connection_id);
            run_schema_drift_cascade(
                &cache,
                &connection_key,
                &memory_connection_key,
                &snapshot,
                prior_graph.as_ref(),
                &sink,
                &memory_manager,
                &conn_id_for_task,
                still_current_before_enrich,
            )
            .await;

            let Some(embed) = embed else {
                sink.emit_progress(IndexingProgressPayload {
                    connection_id: conn_id_for_task.clone(),
                    stage: "complete".into(),
                    processed_tables: total,
                    total_tables: total,
                    added_tables_count: 0,
                    modified_tables_count: 0,
                    deleted_tables_count: 0,
                    unchanged_tables_count: total,
                    cache_hits: 0,
                    embeddings_computed: 0,
                    is_complete: true,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    detail: Some("Semantic indexing unavailable".into()),
                });
                return;
            };

            let single_flight = SingleFlightEmbedder::new(embed);
            let sink_clone = sink.clone();
            let on_progress = move |payload: IndexingProgressPayload| {
                sink_clone.emit_progress(payload);
            };

            let result = SchemaIndexer::enrich(
                connection_id,
                &snapshot,
                &graph,
                prior_graph.as_ref(),
                client.as_ref(),
                sampling_connection_id,
                &single_flight,
                &cache,
                &connection_key,
                config.sample_column_values,
                &capabilities,
                &on_progress,
            )
            .await;

            // Defense-in-depth swap guard: a reconnect may have started a new
            // connection (and a new indexer) while this task was enriching.
            // Only swap when this connection still owns the slot — otherwise
            // the stale task would clobber the new connection's graph with the
            // old database's schema. The Gate 1 drift cascade already ran before
            // `enrich`, so this guard only protects the graph swap.
            let still_current = *current_connection_id.lock().await == Some(connection_id);
            match result {
                Ok(tier2) => {
                    if still_current {
                        *graph_slot.lock().await = Some(tier2);
                    } else {
                        log::debug!(
                            "indexer for connection {conn_id_for_task} finished after a reconnect; skipping graph swap"
                        );
                    }
                }
                Err(e) => {
                    sink.emit_error(&conn_id_for_task, &e);
                }
            }

            // Self-remove from the task map so a completed run does not leave a
            // stale AbortHandle behind. Only removes the entry if it still names
            // THIS task (a reconnect could have replaced it in the meantime).
            if let Some(current_id) = tokio::task::try_id() {
                let mut map = tasks.lock().await;
                if let Some((_, stored_id)) = map.get(&conn_id_for_task) {
                    if *stored_id == current_id {
                        map.remove(&conn_id_for_task);
                    }
                }
            }
        });
        let task_id = handle.id();
        self.tasks
            .lock()
            .await
            .insert(conn_id_str, (handle.abort_handle(), task_id));
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn sync_delta(
        &self,
        connection_id: ConnectionId,
        client: ConnectorClient,
        sampling_connection_id: Option<ConnectionId>,
        graph_slot: Arc<Mutex<Option<SchemaGraph>>>,
        current_connection_id: Arc<Mutex<Option<ConnectionId>>>,
        embedder_slot: Arc<Mutex<Option<Embedder>>>,
        config: Arc<AiConfig>,
        connection_key: String,
        memory_connection_key: String,
        capabilities: DriverCapabilities,
        force_rebuild: bool,
        memory_manager: Arc<MemoryManager>,
    ) -> Result<(), String> {
        let (graph, snapshot) =
            SchemaGraph::from_catalog(connection_id, &client, &capabilities).await?;
        if force_rebuild {
            let _ = self.cache.delete_connection_cache(&connection_key).await;
        }
        self.start(
            connection_id,
            Some(client),
            sampling_connection_id,
            graph_slot,
            current_connection_id,
            (graph, snapshot),
            embedder_slot,
            None,
            config,
            connection_key,
            memory_connection_key,
            capabilities,
            memory_manager,
        )
        .await;
        Ok(())
    }

    pub async fn stop(&self, connection_id: ConnectionId) {
        if let Some((handle, _)) = self.tasks.lock().await.remove(&connection_id.0.to_string()) {
            handle.abort();
        }
    }

    pub async fn stop_all(&self) {
        for (_, (handle, _)) in self.tasks.lock().await.drain() {
            handle.abort();
        }
    }
}

/// Gate 1 DDL cascade (spec §7.2), extracted so it can run whether or not the
/// embedding model is available. Compares the freshly harvested
/// `CatalogSnapshot`'s schema identity against the persisted fingerprint — read
/// from the vector cache BEFORE `enrich` overwrites it, falling back to the
/// in-slot predecessor graph on a cold cache — and cascades any dependent
/// memory to `STALE_INVALID` when it differs.
///
/// Scoped by `memory_connection_key` (profile id or the frontend's
/// `host:port/database` key), not the vector-cache hash, so it matches the keys
/// `memories.connection_key` is stored under. `still_current` guards against a
/// stale indexer finishing after a reconnect invalidating another database's
/// memories; an empty alert list emits nothing.
#[allow(clippy::too_many_arguments)]
async fn run_schema_drift_cascade(
    cache: &PersistentVectorCache,
    connection_key: &str,
    memory_connection_key: &str,
    snapshot: &CatalogSnapshot,
    prior_graph: Option<&SchemaGraph>,
    sink: &Arc<dyn IndexingEventSink>,
    memory_manager: &Arc<MemoryManager>,
    conn_id_for_task: &str,
    still_current: bool,
) {
    if !still_current {
        return;
    }

    let new_schema_hash = compute_schema_hash(snapshot);
    let prior_schema_hash = match cache.get_connection_cache(connection_key).await {
        Ok(Some(entry)) => Some(entry.schema_hash),
        _ => prior_graph.map(|g| compute_schema_hash(&snapshot_from_graph(g))),
    };
    // Only a real catalog identity change cascades; an identical rebuild hashes
    // the same and must not re-invalidate anything.
    if prior_schema_hash.as_deref() == Some(new_schema_hash.as_str()) {
        return;
    }

    match memory_manager
        .with_connection(|conn| {
            crate::ai::memory::drift::cascade_schema_drift(memory_connection_key, snapshot, conn)
        })
        .await
    {
        Ok(alerts) => {
            if !alerts.is_empty() {
                sink.emit_drift(&alerts);
            }
        }
        Err(e) => log::warn!("Gate 1 schema-drift cascade failed for {conn_id_for_task}: {e}"),
    }
}

/// Production event sink used until T2.5 wires the Tauri emitter: logs
/// progress and errors so indexing telemetry is never silently dropped.
#[derive(Clone, Default)]
pub struct LoggingSink;

impl IndexingEventSink for LoggingSink {
    fn emit_progress(&self, payload: IndexingProgressPayload) {
        log::info!(
            "[indexing] conn={} stage={} tables={}/{} complete={} elapsed={}ms{}",
            payload.connection_id,
            payload.stage,
            payload.processed_tables,
            payload.total_tables,
            payload.is_complete,
            payload.elapsed_ms,
            payload
                .detail
                .as_ref()
                .map(|d| format!(" ({d})"))
                .unwrap_or_default()
        );
    }

    fn emit_error(&self, connection_id: &str, message: &str) {
        log::warn!("[indexing] conn={connection_id}: {message}");
    }

    fn emit_drift(&self, alerts: &[DriftAlert]) {
        log::warn!(
            "[indexing] schema drift: {} dependent memory rule(s) invalidated",
            alerts.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use lucent_protocol::ConnectionId;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use super::{IndexingEventSink, IndexingManager};
    use crate::ai::cache_store::PersistentVectorCache;
    use crate::ai::embed::Embedder;
    use crate::ai::events::IndexingProgressPayload;
    use crate::ai::schema_graph::{
        doc_text_for, CatalogSnapshot, ColumnEntry, IndexingTier, SchemaGraph, SnapshotColumn,
        SnapshotTable, TableEntry,
    };
    use crate::ai::AiConfig;

    #[derive(Default)]
    struct RecordingSink {
        progress: std::sync::Mutex<Vec<IndexingProgressPayload>>,
        errors: std::sync::Mutex<Vec<String>>,
        drift: std::sync::Mutex<Vec<crate::ai::memory::drift::DriftAlert>>,
    }
    impl IndexingEventSink for RecordingSink {
        fn emit_progress(&self, payload: IndexingProgressPayload) {
            self.progress.lock().unwrap().push(payload);
        }
        fn emit_error(&self, connection_id: &str, message: &str) {
            self.errors
                .lock()
                .unwrap()
                .push(format!("{connection_id}: {message}"));
        }
        fn emit_drift(&self, alerts: &[crate::ai::memory::drift::DriftAlert]) {
            self.drift.lock().unwrap().extend_from_slice(alerts);
        }
    }

    fn test_graph_and_snapshot() -> (SchemaGraph, CatalogSnapshot) {
        // Same 2-column fixture as schema_graph.rs tests (duplicated here so
        // this module's tests stay self-contained): one table "public.users"
        // with columns "id" (int4, PK) and "status" (text), tier MetadataOnly.
        let tables = vec![TableEntry {
            id: 0,
            schema: "public".into(),
            name: "users".into(),
            kind: "table".into(),
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
                is_nullable: false,
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
                is_nullable: true,
                sample_values: vec![],
                fk_ref: None,
                embedding: vec![],
                doc_text: doc_text_for("public", "users", "status", "text"),
            },
        ];
        let graph = SchemaGraph {
            tables: tables.clone(),
            columns: columns.clone(),
            columns_by_table: std::collections::HashMap::from([(0usize, vec![0usize, 1usize])]),
            fk_edges: vec![],
            table_adjacency: std::collections::HashMap::new(),
            tier: IndexingTier::MetadataOnly,
            built_at_unix: 0,
        };
        let snapshot = CatalogSnapshot {
            format_version: crate::ai::cache_store::DOC_TEXT_FORMAT_VERSION,
            tables: vec![SnapshotTable {
                schema: "public".into(),
                name: "users".into(),
                kind: "table".into(),
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
                    is_nullable: false,
                },
                SnapshotColumn {
                    schema: "public".into(),
                    table: "users".into(),
                    name: "status".into(),
                    data_type: "text".into(),
                    is_primary_key: false,
                    is_nullable: true,
                },
            ],
            fks: vec![],
        };
        (graph, snapshot)
    }

    /// A Tier-1 catalog with no relations — the indexer's view of a schema whose
    /// previously-indexed tables were all dropped.
    fn empty_graph_and_snapshot() -> (SchemaGraph, CatalogSnapshot) {
        let graph = SchemaGraph {
            tables: vec![],
            columns: vec![],
            columns_by_table: std::collections::HashMap::new(),
            fk_edges: vec![],
            table_adjacency: std::collections::HashMap::new(),
            tier: IndexingTier::MetadataOnly,
            built_at_unix: 0,
        };
        let snapshot = CatalogSnapshot {
            format_version: crate::ai::cache_store::DOC_TEXT_FORMAT_VERSION,
            tables: vec![],
            columns: vec![],
            fks: vec![],
        };
        (graph, snapshot)
    }

    fn drift_memory(id: &str, connection_key: &str) -> crate::ai::memory::MemoryItem {
        use crate::ai::memory::{
            compute_memory_doc_hash, MemoryCategory, MemoryItem, MemoryScope, MemoryStatus,
            SourceTrust, MEMORY_FORMAT_VERSION, MEMORY_MODEL_NAME,
        };
        let rule_text = "Active users are identified by users.id";
        MemoryItem {
            id: id.into(),
            connection_key: connection_key.into(),
            scope: MemoryScope::Connection,
            scope_key: connection_key.into(),
            category: MemoryCategory::Metric,
            key_phrase: "active users".into(),
            rule_text: rule_text.into(),
            sql_snippet: Some("SELECT id FROM users".into()),
            importance: 0.8,
            stability_hours: 168.0,
            last_accessed_at: 1000,
            access_count: 1,
            source_trust: SourceTrust::UserExplicit,
            source_conv_id: None,
            source_turn_id: None,
            source_tool_id: None,
            status: MemoryStatus::Active,
            supersedes_id: None,
            valid_from: 1000,
            valid_until: None,
            learned_at: 1000,
            tombstone: false,
            tombstoned_at: None,
            doc_hash: compute_memory_doc_hash(rule_text),
            embedding_model: MEMORY_MODEL_NAME.into(),
            embedding_version: MEMORY_FORMAT_VERSION,
            embedding: vec![],
            created_at: 1000,
            updated_at: 1000,
        }
    }

    struct IndexingCountingEmbed {
        calls: Arc<AtomicUsize>,
    }
    impl crate::ai::single_flight::Embed for IndexingCountingEmbed {
        fn embed<'a>(
            &'a self,
            texts: &'a [String],
        ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<Vec<f32>>, String>> + Send + 'a>>
        {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn indexer_emits_terminal_event_and_stop_aborts() {
        let dir = std::env::temp_dir().join(format!("lucent-indexer-{}", std::process::id()));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db")).unwrap();
        let sink = Arc::new(RecordingSink::default());
        let manager = IndexingManager::new(cache, sink.clone());
        let (graph, snapshot) = test_graph_and_snapshot(); // 2-column tier-1 fixture
        let calls = Arc::new(AtomicUsize::new(0));
        let embedder_override: Arc<dyn crate::ai::single_flight::Embed> =
            Arc::new(IndexingCountingEmbed {
                calls: calls.clone(),
            });
        let slot: Arc<Mutex<Option<SchemaGraph>>> = Arc::new(Mutex::new(Some(graph.clone())));
        let current_connection_id: Arc<Mutex<Option<ConnectionId>>> = Arc::new(Mutex::new(None));
        let embedder_slot: Arc<Mutex<Option<Embedder>>> = Arc::new(Mutex::new(None));
        let config = AiConfig {
            sample_column_values: false, // no DB work in this test
            ..AiConfig::default()
        };

        let started_id = ConnectionId(Uuid::new_v4());
        *current_connection_id.lock().await = Some(started_id);

        // Cache is cold; the task must run enrich (with the injected embedder)
        // and emit a complete event.
        manager
            .start(
                started_id,
                None, // no ConnectorClient in unit tests
                None,
                slot.clone(),
                current_connection_id.clone(),
                (graph, snapshot),
                embedder_slot,
                Some(embedder_override),
                Arc::new(config),
                "key".into(),
                "localhost:5432/key".into(),
                fake_capabilities(),
                Arc::new(crate::ai::memory::MemoryManager::open_in_memory().unwrap()),
            )
            .await;
        // Poll until terminal (bounded).
        for _ in 0..50 {
            if sink.progress.lock().unwrap().iter().any(|p| p.is_complete) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        {
            let progress = sink.progress.lock().unwrap();
            assert!(
                progress.iter().any(|p| p.is_complete),
                "terminal event emitted"
            );
            assert!(
                calls.load(Ordering::SeqCst) >= 1,
                "cold cache embedded the columns"
            );
        }

        // The completed task removes its own map entry; a finished run leaves
        // no stale AbortHandle behind.
        assert!(
            manager.tasks.lock().await.is_empty(),
            "completed task removed itself from the map"
        );

        // stop on an unknown id is a no-op…
        manager.stop(ConnectionId(Uuid::new_v4())).await;
        // …and stop on the started id remains a no-op once the task self-removed.
        manager.stop(started_id).await;
        assert!(manager.tasks.lock().await.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stale_indexer_does_not_overwrite_a_new_connection_s_graph() {
        // A reconnect starts a new connection (fresh id) before the previous
        // indexer finishes. The swap guard must stop the stale task from
        // clobbering the shared slot with the old database's tier-2 graph.
        let dir = std::env::temp_dir().join(format!("lucent-indexer-stale-{}", std::process::id()));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db")).unwrap();
        let sink = Arc::new(RecordingSink::default());
        let manager = IndexingManager::new(cache, sink.clone());
        let (graph, snapshot) = test_graph_and_snapshot();
        let calls = Arc::new(AtomicUsize::new(0));
        let embedder_override: Arc<dyn crate::ai::single_flight::Embed> =
            Arc::new(IndexingCountingEmbed {
                calls: calls.clone(),
            });
        let slot: Arc<Mutex<Option<SchemaGraph>>> = Arc::new(Mutex::new(Some(graph.clone())));
        let current_connection_id: Arc<Mutex<Option<ConnectionId>>> = Arc::new(Mutex::new(None));
        let embedder_slot: Arc<Mutex<Option<Embedder>>> = Arc::new(Mutex::new(None));
        let config = AiConfig {
            sample_column_values: false,
            ..AiConfig::default()
        };

        let stale_id = ConnectionId(Uuid::new_v4());
        *current_connection_id.lock().await = Some(stale_id);
        manager
            .start(
                stale_id,
                None,
                None,
                slot.clone(),
                current_connection_id.clone(),
                (graph, snapshot),
                embedder_slot,
                Some(embedder_override),
                Arc::new(config),
                "stale-key".into(),
                "localhost:5432/stale".into(),
                fake_capabilities(),
                Arc::new(crate::ai::memory::MemoryManager::open_in_memory().unwrap()),
            )
            .await;

        // The connection switches before the stale task completes: the slot
        // now names a NEW connection id.
        *current_connection_id.lock().await = Some(ConnectionId(Uuid::new_v4()));

        // Let the stale task finish (bounded poll for its terminal event).
        for _ in 0..50 {
            if sink.progress.lock().unwrap().iter().any(|p| p.is_complete) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            calls.load(Ordering::SeqCst) >= 1,
            "stale task did run enrich"
        );

        // The stale task must NOT have overwritten the slot with its graph:
        // the stored graph is still the tier-1 fixture (its tier field stays
        // MetadataOnly because the swap guard rejected the tier-2 swap).
        let stored = slot.lock().await.clone().expect("slot populated");
        assert_eq!(
            stored.tier,
            IndexingTier::MetadataOnly,
            "stale indexer must not swap its graph into the new connection's slot"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// δ) B-C5: a catalog diff detected by the background indexer must run the
    /// Gate 1 DDL cascade (spec §7.2) and surface `memory:drift_detected`. The
    /// prior Tier-2 graph still in the shared slot references `public.users`;
    /// the freshly harvested `CatalogSnapshot` no longer contains it, so the
    /// dependent memory must flip to `STALE_INVALID` and the sink must receive
    /// exactly one drift alert.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_cascade_invalidates_memories_and_emits_event() {
        use crate::ai::memory::entity_linker::{compute_entity_fingerprint, EntityRef};
        use crate::ai::memory::{MemoryManager, MemoryStatus};

        let dir = std::env::temp_dir().join(format!("lucent-indexer-drift-{}", std::process::id()));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db")).unwrap();
        let sink = Arc::new(RecordingSink::default());
        let manager = IndexingManager::new(cache, sink.clone());

        // Production key shapes: the memory key is the frontend's
        // `host:port/database` (App.svelte's `activeConnectionId`); the
        // vector-cache key is a distinct hash. The dependent memory is stored
        // under the memory key, and a decoy is stored under the cache key to
        // prove the cascade resolves the former, not the latter.
        let memory_key = "localhost:5432/analytics";
        let cache_key = "drift-cache-sha256";

        let memory_manager = Arc::new(MemoryManager::open_in_memory().unwrap());
        let link = || EntityRef {
            schema_name: "public".into(),
            table_name: "users".into(),
            column_name: None,
            data_type: "table".into(),
            is_nullable: true,
            entity_fingerprint: compute_entity_fingerprint("public", "users", None, "table", true),
        };
        memory_manager
            .save_memory(drift_memory("mem_drift_1", memory_key), &[link()])
            .await
            .unwrap();
        memory_manager
            .save_memory(drift_memory("mem_cache_key_decoy", cache_key), &[link()])
            .await
            .unwrap();

        // The prior graph (in the shared slot) has public.users; the freshly
        // harvested catalog is empty — i.e. the table was dropped.
        let (prior_graph, _) = test_graph_and_snapshot();
        let (new_graph, new_snapshot) = empty_graph_and_snapshot();

        let calls = Arc::new(AtomicUsize::new(0));
        let embedder_override: Arc<dyn crate::ai::single_flight::Embed> =
            Arc::new(IndexingCountingEmbed { calls });
        let slot: Arc<Mutex<Option<SchemaGraph>>> = Arc::new(Mutex::new(Some(prior_graph)));
        let current_connection_id: Arc<Mutex<Option<ConnectionId>>> = Arc::new(Mutex::new(None));
        let embedder_slot: Arc<Mutex<Option<Embedder>>> = Arc::new(Mutex::new(None));
        let config = AiConfig {
            sample_column_values: false,
            ..AiConfig::default()
        };

        let started_id = ConnectionId(Uuid::new_v4());
        *current_connection_id.lock().await = Some(started_id);

        manager
            .start(
                started_id,
                None,
                None,
                slot.clone(),
                current_connection_id.clone(),
                (new_graph, new_snapshot),
                embedder_slot,
                Some(embedder_override),
                Arc::new(config),
                cache_key.into(),
                memory_key.into(),
                fake_capabilities(),
                memory_manager.clone(),
            )
            .await;

        // The task self-removes from the manager only AFTER the Gate 1
        // cascade, so waiting on that (not the terminal progress event, which
        // enrich emits before the cascade) removes the race.
        for _ in 0..50 {
            if manager.tasks.lock().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            manager.tasks.lock().await.is_empty(),
            "indexer task completed"
        );
        assert!(
            sink.progress.lock().unwrap().iter().any(|p| p.is_complete),
            "indexer reached terminal state"
        );

        // Upward seam: the indexer reported the drift.
        {
            let drift = sink.drift.lock().unwrap();
            assert_eq!(drift.len(), 1, "expected one drift alert, got {drift:?}");
            assert_eq!(drift[0].memory_id, "mem_drift_1");
            assert!(
                drift[0].reason.contains("public.users"),
                "alert names the dropped table: {}",
                drift[0].reason
            );
        }

        // Downward effect: the dependent memory is invalidated and tombstoned.
        let all = memory_manager
            .list_memories(memory_key, true)
            .await
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].status, MemoryStatus::StaleInvalid);
        assert!(all[0].tombstone, "invalidated memory must be tombstoned");

        // The decoy stored under the vector-cache key is a different
        // connection's memory: it must be untouched.
        let decoy = memory_manager.list_memories(cache_key, true).await.unwrap();
        assert_eq!(decoy.len(), 1);
        assert_eq!(
            decoy[0].status,
            MemoryStatus::Active,
            "the cascade must scope by the memory key, not the vector-cache key"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression (review Important): the automated Gate 1 cascade is scoped to
    /// the connection being indexed. Indexing connection "indexed-key" must not
    /// tombstone "other-key"'s memory even though both link `public.users` and
    /// the indexed catalog no longer contains it.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_cascade_does_not_invalidate_another_connections_memories() {
        use crate::ai::memory::entity_linker::{compute_entity_fingerprint, EntityRef};
        use crate::ai::memory::{MemoryManager, MemoryStatus};

        let dir =
            std::env::temp_dir().join(format!("lucent-indexer-drift-scope-{}", std::process::id()));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db")).unwrap();
        let sink = Arc::new(RecordingSink::default());
        let manager = IndexingManager::new(cache, sink.clone());

        let memory_manager = Arc::new(MemoryManager::open_in_memory().unwrap());
        let key_a = "localhost:5432/analytics";
        let key_b = "reporting.internal:5432/warehouse";
        let cache_key = "scope-cache-sha256";
        let link = || EntityRef {
            schema_name: "public".into(),
            table_name: "users".into(),
            column_name: None,
            data_type: "table".into(),
            is_nullable: true,
            entity_fingerprint: compute_entity_fingerprint("public", "users", None, "table", true),
        };
        memory_manager
            .save_memory(drift_memory("mem_a", key_a), &[link()])
            .await
            .unwrap();
        memory_manager
            .save_memory(drift_memory("mem_b", key_b), &[link()])
            .await
            .unwrap();

        let (prior_graph, _) = test_graph_and_snapshot();
        let (new_graph, new_snapshot) = empty_graph_and_snapshot();

        let calls = Arc::new(AtomicUsize::new(0));
        let embedder_override: Arc<dyn crate::ai::single_flight::Embed> =
            Arc::new(IndexingCountingEmbed { calls });
        let slot: Arc<Mutex<Option<SchemaGraph>>> = Arc::new(Mutex::new(Some(prior_graph)));
        let current_connection_id: Arc<Mutex<Option<ConnectionId>>> = Arc::new(Mutex::new(None));
        let embedder_slot: Arc<Mutex<Option<Embedder>>> = Arc::new(Mutex::new(None));
        let config = AiConfig {
            sample_column_values: false,
            ..AiConfig::default()
        };

        let started_id = ConnectionId(Uuid::new_v4());
        *current_connection_id.lock().await = Some(started_id);

        manager
            .start(
                started_id,
                None,
                None,
                slot.clone(),
                current_connection_id.clone(),
                (new_graph, new_snapshot),
                embedder_slot,
                Some(embedder_override),
                Arc::new(config),
                cache_key.into(),
                key_a.into(),
                fake_capabilities(),
                memory_manager.clone(),
            )
            .await;

        for _ in 0..50 {
            if manager.tasks.lock().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            manager.tasks.lock().await.is_empty(),
            "indexer task completed"
        );

        // The indexed connection's memory is invalidated…
        let indexed = memory_manager.list_memories(key_a, true).await.unwrap();
        assert_eq!(indexed.len(), 1);
        assert_eq!(indexed[0].status, MemoryStatus::StaleInvalid);

        // …but the other connection's memory is untouched.
        let other = memory_manager.list_memories(key_b, true).await.unwrap();
        assert_eq!(other.len(), 1, "other connection's memory still present");
        assert_eq!(
            other[0].status,
            MemoryStatus::Active,
            "the cascade must not invalidate another connection's memory"
        );
        assert!(!other[0].tombstone);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// REQUIRED FIX 3: Gate 1 must run even when no embedder is available.
    /// Drift detection needs no embeddings, so a cold/offline model (or
    /// `enable_semantic_index = false`) must still invalidate dependent memories
    /// and emit `memory:drift_detected`. Before this was fixed the embedder-unavailable
    /// early return skipped the schema-hash read and the cascade entirely.
    #[tokio::test(flavor = "multi_thread")]
    async fn drift_cascade_runs_without_an_embedder() {
        use crate::ai::memory::entity_linker::{compute_entity_fingerprint, EntityRef};
        use crate::ai::memory::{MemoryManager, MemoryStatus};

        let dir = std::env::temp_dir().join(format!(
            "lucent-indexer-drift-offline-{}",
            std::process::id()
        ));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db")).unwrap();
        let sink = Arc::new(RecordingSink::default());
        let manager = IndexingManager::new(cache, sink.clone());

        let memory_key = "localhost:5432/offline";
        let cache_key = "offline-cache-sha256";
        let memory_manager = Arc::new(MemoryManager::open_in_memory().unwrap());
        let link = || EntityRef {
            schema_name: "public".into(),
            table_name: "users".into(),
            column_name: None,
            data_type: "table".into(),
            is_nullable: true,
            entity_fingerprint: compute_entity_fingerprint("public", "users", None, "table", true),
        };
        memory_manager
            .save_memory(drift_memory("mem_offline", memory_key), &[link()])
            .await
            .unwrap();

        let (prior_graph, _) = test_graph_and_snapshot();
        let (new_graph, new_snapshot) = empty_graph_and_snapshot();
        let slot: Arc<Mutex<Option<SchemaGraph>>> = Arc::new(Mutex::new(Some(prior_graph)));
        let current_connection_id: Arc<Mutex<Option<ConnectionId>>> = Arc::new(Mutex::new(None));
        let embedder_slot: Arc<Mutex<Option<Embedder>>> = Arc::new(Mutex::new(None));
        // No embedder override and semantic indexing off ⇒ `embed` is `None`,
        // so the task takes its early return. The cascade must run first.
        let config = AiConfig {
            enable_semantic_index: false,
            sample_column_values: false,
            ..AiConfig::default()
        };

        let started_id = ConnectionId(Uuid::new_v4());
        *current_connection_id.lock().await = Some(started_id);

        manager
            .start(
                started_id,
                None,
                None,
                slot.clone(),
                current_connection_id.clone(),
                (new_graph, new_snapshot),
                embedder_slot,
                None,
                Arc::new(config),
                cache_key.into(),
                memory_key.into(),
                fake_capabilities(),
                memory_manager.clone(),
            )
            .await;

        // The embedder-unavailable path returns via its own early exit (it does
        // not reach the end-of-task self-removal), so wait on the terminal
        // progress event the early exit emits AFTER the cascade.
        for _ in 0..50 {
            if sink.progress.lock().unwrap().iter().any(|p| p.is_complete) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            sink.progress.lock().unwrap().iter().any(|p| p.is_complete),
            "the embedder-unavailable path must still reach terminal state"
        );

        {
            let drift = sink.drift.lock().unwrap();
            assert_eq!(
                drift.len(),
                1,
                "drift must be reported even with no embedder: {drift:?}"
            );
            assert_eq!(drift[0].memory_id, "mem_offline");
        }

        let all = memory_manager
            .list_memories(memory_key, true)
            .await
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(
            all[0].status,
            MemoryStatus::StaleInvalid,
            "the cascade must flip the memory even when the embedder is unavailable"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
