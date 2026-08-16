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
use crate::ai::schema_graph::{CatalogSnapshot, IndexingStage, SchemaGraph, SchemaIndexer};
use crate::ai::single_flight::SingleFlightEmbedder;
use crate::ai::AiConfig;
use crate::client::ConnectorClient;

pub trait IndexingEventSink: Send + Sync {
    fn emit_progress(&self, payload: IndexingProgressPayload);
    fn emit_error(&self, connection_id: &str, message: &str);
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
        capabilities: DriverCapabilities,
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

            let Some(embed) = embed else {
                sink.emit_progress(IndexingProgressPayload {
                    connection_id: conn_id_for_task.clone(),
                    stage: "complete".into(),
                    processed_tables: total,
                    total_tables: total,
                    cache_hits: 0,
                    embeddings_computed: 0,
                    is_complete: true,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    detail: Some("Semantic indexing unavailable".into()),
                });
                return;
            };

            let single_flight = SingleFlightEmbedder::new(embed);
            let on_progress = |stage: IndexingStage,
                               processed: usize,
                               t: usize,
                               cache_hits: usize,
                               embeddings_computed: usize| {
                let stage = match stage {
                    IndexingStage::Sampling => "sampling",
                    IndexingStage::Embedding => "embedding",
                    IndexingStage::Complete => "complete",
                };
                sink.emit_progress(IndexingProgressPayload {
                    connection_id: conn_id_for_task.clone(),
                    stage: stage.into(),
                    processed_tables: processed,
                    total_tables: t,
                    cache_hits,
                    embeddings_computed,
                    is_complete: stage == "complete",
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    detail: None,
                });
            };
            let result = SchemaIndexer::enrich(
                connection_id,
                &snapshot,
                &graph,
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
            match result {
                Ok(tier2) => {
                    // Defense-in-depth swap guard: a reconnect may have started a
                    // new connection (and a new indexer) while this task was
                    // enriching. Only swap when this connection still owns the
                    // slot — otherwise the stale task would clobber the new
                    // connection's graph with the old database's schema.
                    let still_current = *current_connection_id.lock().await == Some(connection_id);
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
    }

    fn test_graph_and_snapshot() -> (SchemaGraph, CatalogSnapshot) {
        // Same 2-column fixture as schema_graph.rs tests (duplicated here so
        // this module's tests stay self-contained): one table "public.users"
        // with columns "id" (int4, PK) and "status" (text), tier MetadataOnly.
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
        let mut config = AiConfig::default();
        config.sample_column_values = false; // no DB work in this test

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
                fake_capabilities(),
            )
            .await;
        // Poll until terminal (bounded).
        for _ in 0..50 {
            if sink.progress.lock().unwrap().iter().any(|p| p.is_complete) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let progress = sink.progress.lock().unwrap();
        assert!(
            progress.iter().any(|p| p.is_complete),
            "terminal event emitted"
        );
        assert!(
            calls.load(Ordering::SeqCst) >= 1,
            "cold cache embedded the columns"
        );
        drop(progress);

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
        let mut config = AiConfig::default();
        config.sample_column_values = false;

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
                fake_capabilities(),
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
}
