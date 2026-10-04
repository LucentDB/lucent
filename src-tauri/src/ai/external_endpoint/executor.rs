use lucent_protocol::{ConnectionId, DriverCapabilities};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::ai::acp::bridge::ToolExecutor;
use crate::ai::embed::Embedder;
use crate::ai::external_endpoint::mirror::InodeTracker;
use crate::ai::schema_graph::SchemaGraph;
use crate::ai::tools::{AiToolContext, ToolError, ToolOutput};
use crate::client::ConnectorClient;
use crate::connections::ConnectionProfile;

pub struct ExternalEndpointContext {
    pub client: Arc<Mutex<Option<ConnectorClient>>>,
    pub external_ai_connection_id: Arc<Mutex<Option<ConnectionId>>>,
    pub schema_graph: Arc<Mutex<Option<SchemaGraph>>>,
    pub embedder: Arc<Mutex<Option<Embedder>>>,
    pub capabilities: Arc<Mutex<Option<DriverCapabilities>>>,
    pub active_profile: Arc<Mutex<Option<ConnectionProfile>>>,
    pub inode_tracker: Arc<Mutex<Option<InodeTracker>>>,
    pub reopen_lock: Arc<Mutex<()>>,
    pub memory_manager: Arc<crate::ai::memory::MemoryManager>,
}

pub struct DynamicContextToolExecutor {
    context: Arc<ExternalEndpointContext>,
}

impl DynamicContextToolExecutor {
    pub fn new(context: Arc<ExternalEndpointContext>) -> Self {
        Self { context }
    }
}

pub async fn run_connector_canary(
    client: &ConnectorClient,
    conn_id: ConnectionId,
) -> Result<(), String> {
    let temp_canary =
        std::env::temp_dir().join(format!("lucent-canary-{}.csv", std::process::id()));
    std::fs::write(&temp_canary, b"id,val\n1,canary\n")
        .map_err(|e| format!("Failed to create canary file: {e}"))?;

    let sql = format!("SELECT count(*) FROM '{}'", temp_canary.display());
    let exec_res = client.execute(conn_id, &sql).await;

    let _ = std::fs::remove_file(&temp_canary);

    match exec_res {
        Ok(_) => Err("Canary failed: external access is not disabled on DuckDB handle!".into()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("disabled by configuration") {
                Ok(())
            } else {
                Err(format!("Canary failed unexpectedly: {msg}"))
            }
        }
    }
}

#[async_trait::async_trait]
impl ToolExecutor for DynamicContextToolExecutor {
    async fn call(&self, tool: &str, args: serde_json::Value) -> Result<ToolOutput, ToolError> {
        // 1. Profile Gating
        let profile = {
            let profile_guard = self.context.active_profile.lock().await;
            profile_guard.clone()
        };

        let Some(profile) = profile else {
            return Err(ToolError::Execution(
                "External agent access is disabled: no active database connection profile.".into(),
            ));
        };

        if !profile.enable_external_agents {
            return Err(ToolError::Execution(
                "External agent access is disabled for the active connection profile.".into(),
            ));
        }

        // 2. DML Blocking & Tool Specific Restrictions
        if tool == "preview_dml" {
            return Err(ToolError::Execution(
                "preview_dml is not allowed in external agent mode: external access is strictly read-only".into(),
            ));
        }

        if tool == "search_query_history" && !profile.allow_query_history {
            return Err(ToolError::Execution(
                "Query history search is disabled for this profile.".into(),
            ));
        }

        // 3. Locked Reopen Protocol on Inode / File Swap
        {
            let _lock = self.context.reopen_lock.lock().await;
            let mut tracker_guard = self.context.inode_tracker.lock().await;
            if let Some(ref mut tracker) = *tracker_guard {
                if tracker.check_and_update() {
                    log::info!(
                        "Detected file swap for {:?}, executing locked reopen protocol",
                        tracker.path()
                    );
                    let path = tracker.path().to_path_buf();
                    let mut client_guard = self.context.client.lock().await;
                    if let Some(ref mut client) = *client_guard {
                        let mut conn_guard = self.context.external_ai_connection_id.lock().await;
                        if let Some(old_id) = *conn_guard {
                            let _ = client.disconnect_id(old_id).await;
                        }

                        let new_id = ConnectionId(uuid::Uuid::new_v4());
                        let cfg = lucent_protocol::ConnectionConfig::new("duckdb")
                            .with("path", path.to_string_lossy().to_string())
                            .with("read_only", "true")
                            .with("external_access", "false");

                        if let Ok(server_info) = client.connect_with_id(new_id, cfg).await {
                            *conn_guard = Some(new_id);
                            *self.context.capabilities.lock().await =
                                Some(server_info.capabilities.clone());

                            // Run connector-level canary probe
                            let canary_res = run_connector_canary(client, new_id).await;
                            if let Err(e) = canary_res {
                                log::error!("Connector canary verification failed on reopen: {e}; refusing to serve");
                                let _ = client.disconnect_id(new_id).await;
                                *conn_guard = None;
                                if let Some(ref mut p) = *self.context.active_profile.lock().await {
                                    p.enable_external_agents = false;
                                }
                                return Err(ToolError::Execution(format!(
                                    "Connector canary verification failed on reopen: {e}. External agent access disabled."
                                )));
                            }

                            // Refresh schema graph in background (spec §4.2 step 4)
                            let schema_graph_slot = Arc::clone(&self.context.schema_graph);
                            let bg_client = client.clone();
                            let bg_caps = server_info.capabilities.clone();
                            tokio::spawn(async move {
                                if let Ok((new_graph, _snapshot)) =
                                    SchemaGraph::from_catalog(new_id, &bg_client, &bg_caps).await
                                {
                                    *schema_graph_slot.lock().await = Some(new_graph);
                                }
                            });
                        }
                    }
                }
            }
        }

        // 4. Build Dynamic Context and Dispatch
        let conn_id = *self.context.external_ai_connection_id.lock().await;
        let capabilities = self.context.capabilities.lock().await.clone();
        let memory_connection_key = {
            let host = profile.params.get("host").map(|s| s.as_str()).unwrap_or("");
            let port = profile
                .params
                .get("port")
                .and_then(|s| s.parse::<u16>().ok())
                .unwrap_or(0);
            let database = profile
                .params
                .get("database")
                .map(|s| s.as_str())
                .unwrap_or("");
            Some(format!("{host}:{port}/{database}"))
        };
        let config = crate::ai::config::AiConfig {
            ai_query_timeout_secs: 15,
            ..Default::default()
        };
        let tool_ctx = AiToolContext {
            db: Arc::clone(&self.context.client),
            connection_id: conn_id,
            memory_connection_key,
            capabilities,
            config,
            schema_graph: Arc::clone(&self.context.schema_graph),
            embedder: Arc::clone(&self.context.embedder),
            reranker: Arc::new(Mutex::new(None)),
            memory_manager: Arc::clone(&self.context.memory_manager),
        };

        match tool {
            "run_readonly_query" => {
                crate::ai::tools::execute::RunReadonlyQuery::new(tool_ctx.clone())
                    .call(args, &tool_ctx)
                    .await
            }
            "search_schema" => {
                crate::ai::tools::search_schema::SearchSchema::new(tool_ctx.clone())
                    .call(args, &tool_ctx)
                    .await
            }
            "get_objects_info" => {
                crate::ai::tools::objects::GetObjectsInfo::new(tool_ctx.clone())
                    .call(args, &tool_ctx)
                    .await
            }
            "get_preflight_context" => {
                crate::ai::tools::preflight::GetPreflightContext::new(tool_ctx.clone())
                    .call(args, &tool_ctx)
                    .await
            }
            "search_query_history" => {
                crate::ai::tools::memory::SearchQueryHistory::new(tool_ctx.clone())
                    .call(args, &tool_ctx)
                    .await
            }
            _ => Err(ToolError::Execution(format!(
                "Tool '{tool}' is not available in external agent mode"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::acp::bridge::ToolExecutor;

    pub fn create_mock_external_context(enabled: bool) -> Arc<ExternalEndpointContext> {
        let profile = if enabled {
            Some(ConnectionProfile {
                id: "test_prof".into(),
                name: "Test DuckDB".into(),
                driver: "duckdb".into(),
                alias: None,
                params: Default::default(),
                ssh_tunnel_id: None,
                group: None,
                color: None,
                icon: None,
                last_used: None,
                created_at: String::new(),
                updated_at: String::new(),
                enable_external_agents: true,
                allow_query_history: false,
            })
        } else {
            None
        };

        Arc::new(ExternalEndpointContext {
            client: Arc::new(Mutex::new(None)),
            external_ai_connection_id: Arc::new(Mutex::new(None)),
            schema_graph: Arc::new(Mutex::new(None)),
            embedder: Arc::new(Mutex::new(None)),
            capabilities: Arc::new(Mutex::new(None)),
            active_profile: Arc::new(Mutex::new(profile)),
            inode_tracker: Arc::new(Mutex::new(None)),
            reopen_lock: Arc::new(Mutex::new(())),
            memory_manager: Arc::new(crate::ai::memory::MemoryManager::open_in_memory().unwrap()),
        })
    }

    #[tokio::test]
    async fn executor_rejects_when_profile_disabled_or_dml_requested() {
        let context = create_mock_external_context(false);
        let executor = DynamicContextToolExecutor::new(context);

        let err = executor
            .call("run_readonly_query", serde_json::json!({"sql": "SELECT 1"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("disabled"));

        let context_enabled = create_mock_external_context(true);
        let executor_enabled = DynamicContextToolExecutor::new(context_enabled);

        let err = executor_enabled
            .call("preview_dml", serde_json::json!({"sql": "DELETE FROM t"}))
            .await
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("not allowed in external agent mode"));
    }

    #[tokio::test]
    async fn executor_rejects_query_history_when_disabled_on_profile() {
        let context = create_mock_external_context(true);
        let executor = DynamicContextToolExecutor::new(context);

        let err = executor
            .call(
                "search_query_history",
                serde_json::json!({"query": "SELECT"}),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Query history search is disabled"));
    }

    #[tokio::test]
    async fn executor_searches_real_query_history_and_golden_queries() {
        // Isolate the query-history file from the developer's real config dir:
        // `history_file_path()` consults this thread-local under `cfg(test)`,
        // so without it the `append_entry` below writes a synthetic entry into
        // the user's `query_history.jsonl`. The default `#[tokio::test]` flavor
        // is current_thread, so the override covers the synchronous append and
        // search calls in this test.
        let history_dir = tempfile::tempdir().unwrap();
        crate::connections::TEST_CONFIG_DIR
            .with(|cell| *cell.borrow_mut() = Some(history_dir.path().to_path_buf()));

        let mut params = std::collections::BTreeMap::new();
        params.insert("host".into(), "localhost".into());
        params.insert("port".into(), "5432".into());
        params.insert("database".into(), "analytics".into());

        let profile = ConnectionProfile {
            id: "test_history_prof".into(),
            name: "Test DuckDB".into(),
            driver: "duckdb".into(),
            alias: None,
            params,
            ssh_tunnel_id: None,
            group: None,
            color: None,
            icon: None,
            last_used: None,
            created_at: String::new(),
            updated_at: String::new(),
            enable_external_agents: true,
            allow_query_history: true,
        };

        let mem_mgr = Arc::new(crate::ai::memory::MemoryManager::open_in_memory().unwrap());
        // Seed golden query for the connection key "localhost:5432/analytics"
        mem_mgr
            .save_golden_query(crate::ai::memory::GoldenQuery {
                id: "g1".into(),
                connection_id: "localhost:5432/analytics".into(),
                schema_name: "public".into(),
                natural_prompt: "monthly active users".into(),
                sql_text: "SELECT count(*) FROM monthly_users".into(),
                tables_used: vec!["monthly_users".into()],
                verified: true,
                run_count: 1,
                last_run_at: 0,
                embedding_model: "".into(),
                embedding_version: 0,
                embedding: vec![],
                created_at: 0,
            })
            .await
            .unwrap();

        // Seed human query history entry matching "localhost:5432/analytics"
        let entry = crate::query_history::QueryHistoryEntry::new(
            "localhost:5432/analytics".into(),
            "localhost:5432/analytics".into(),
            "analytics".into(),
            "SELECT count(*) FROM orders_archive".into(),
            42,
            Some(100),
            "success".into(),
            None,
        );
        crate::query_history::append_entry(entry).unwrap();

        let context = Arc::new(ExternalEndpointContext {
            client: Arc::new(Mutex::new(None)),
            external_ai_connection_id: Arc::new(Mutex::new(None)),
            schema_graph: Arc::new(Mutex::new(None)),
            embedder: Arc::new(Mutex::new(None)),
            capabilities: Arc::new(Mutex::new(None)),
            active_profile: Arc::new(Mutex::new(Some(profile))),
            inode_tracker: Arc::new(Mutex::new(None)),
            reopen_lock: Arc::new(Mutex::new(())),
            memory_manager: mem_mgr,
        });

        let executor = DynamicContextToolExecutor::new(context);

        // Search for golden query
        let golden_res = executor
            .call(
                "search_query_history",
                serde_json::json!({"query": "monthly active users"}),
            )
            .await
            .unwrap();
        let golden_text = match golden_res {
            ToolOutput::Text { content } => content,
            _ => panic!("expected Text output"),
        };
        assert!(
            golden_text.contains("monthly_users"),
            "golden query not found: {golden_text}"
        );

        // Search for human history
        let history_res = executor
            .call(
                "search_query_history",
                serde_json::json!({"query": "orders_archive"}),
            )
            .await
            .unwrap();
        let history_text = match history_res {
            ToolOutput::Text { content } => content,
            _ => panic!("expected Text output"),
        };
        assert!(
            history_text.contains("orders_archive"),
            "history entry not found: {history_text}"
        );
    }
}
