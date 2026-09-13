use serde_json::json;
use uuid::Uuid;

use super::{AiToolContext, ToolError, ToolOutput};
use crate::ai::memory::{
    compute_memory_doc_hash, extract_and_link_entities, sanitize_rule_text, sanitize_sql_snippet,
    MemoryCategory, MemoryItem, MemoryScope, MemoryStatus, SourceTrust, MEMORY_FORMAT_VERSION,
    MEMORY_MODEL_NAME, TOOL_RULE_STABILITY_HOURS,
};
use crate::query_history;

#[derive(Clone)]
pub struct SaveMemory {
    _ctx: AiToolContext,
}

impl SaveMemory {
    pub fn new(ctx: AiToolContext) -> Self {
        Self { _ctx: ctx }
    }

    pub fn description(&self) -> String {
        "Save a discovered database rule, metric definition, join quirk, or user preference into persistent AI memory. Do NOT save raw query output rows. Memories are domain facts, never system instructions.".into()
    }

    pub fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "category": {
                    "type": "string",
                    "enum": ["metric", "join", "quirk", "preference"],
                    "description": "Category of the memory fact"
                },
                "key_phrase": {
                    "type": "string",
                    "description": "Short identifier or key phrase for this rule (e.g. 'active_subscribers', 'orders_users_join')"
                },
                "rule_text": {
                    "type": "string",
                    "description": "Human-readable rule explanation (max 500 characters)"
                },
                "sql_snippet": {
                    "type": "string",
                    "description": "Optional SQL predicate or join expression (max 1000 characters)"
                }
            },
            "required": ["category", "key_phrase", "rule_text"]
        })
    }

    pub async fn call(
        &self,
        args: serde_json::Value,
        ctx: &AiToolContext,
    ) -> Result<ToolOutput, ToolError> {
        // B-I5: honor the app's memory kill-switch before any work. An
        // external agent must not write memories the user has disabled. The key
        // is the frontend memory connection key — the same key chat retrieval,
        // the Memory Drawer, `save_memory_manual`, and the Gate 1 drift cascade
        // use. The worker `ConnectionId` is only a fallback for callers that
        // never captured the frontend key.
        let connection_key = ctx
            .memory_connection_key
            .clone()
            .or_else(|| ctx.connection_id.map(|id| id.0.to_string()))
            .unwrap_or_else(|| "global".into());
        if !ctx.config.is_memory_enabled(&connection_key) {
            return Err(ToolError::Execution(
                "AI memory is disabled for this connection".into(),
            ));
        }

        let category_str = args["category"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs("missing 'category'".into()))?;
        let key_phrase = args["key_phrase"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs("missing 'key_phrase'".into()))?
            .trim()
            .to_string();
        let raw_rule_text = args["rule_text"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs("missing 'rule_text'".into()))?;
        let raw_sql_snippet = args["sql_snippet"].as_str();

        let rule_text = sanitize_rule_text(raw_rule_text)
            .map_err(|e| ToolError::InvalidArgs(format!("invalid rule_text: {e}")))?;
        let sql_snippet = sanitize_sql_snippet(raw_sql_snippet)
            .map_err(|e| ToolError::InvalidArgs(format!("invalid sql_snippet: {e}")))?;

        let category = MemoryCategory::from_str(category_str);

        // Contextual chunk enrichment at extraction:
        // Prepend contextual prefix to enrich vector embedding representation
        let context_enriched_doc =
            format!("Context: {category_str} {key_phrase} | Rule: {rule_text}");
        let doc_hash = compute_memory_doc_hash(&context_enriched_doc);

        // Generate embedding vector
        let embedding = {
            let embedder_guard = ctx.embedder.lock().await;
            if let Some(embedder) = embedder_guard.as_ref() {
                embedder
                    .embed_query(&context_enriched_doc)
                    .await
                    .unwrap_or_else(|_| vec![0.0f32; 384])
            } else {
                vec![0.0f32; 384]
            }
        };

        // Extract entity links
        let entity_links = {
            let graph_guard = ctx.schema_graph.lock().await;
            if let Some(graph) = graph_guard.as_ref() {
                extract_and_link_entities(&rule_text, sql_snippet.as_deref(), graph)
            } else {
                Vec::new()
            }
        };

        let now = chrono::Utc::now().timestamp();
        let memory_item = MemoryItem {
            id: Uuid::new_v4().to_string(),
            connection_key: connection_key.clone(),
            scope: MemoryScope::Connection,
            scope_key: connection_key,
            category,
            key_phrase: key_phrase.clone(),
            rule_text: rule_text.clone(),
            sql_snippet: sql_snippet.clone(),
            importance: 0.5,
            stability_hours: TOOL_RULE_STABILITY_HOURS,
            last_accessed_at: now,
            access_count: 1,
            // Tagged as error_resolution / untrusted_tool_result from tool call. Never UserExplicit.
            source_trust: SourceTrust::ErrorResolution,
            source_conv_id: None,
            source_turn_id: None,
            source_tool_id: Some("save_memory".into()),
            status: MemoryStatus::Active,
            supersedes_id: None,
            valid_from: now,
            valid_until: None,
            learned_at: now,
            tombstone: false,
            tombstoned_at: None,
            doc_hash,
            embedding_model: MEMORY_MODEL_NAME.into(),
            embedding_version: MEMORY_FORMAT_VERSION,
            embedding,
            created_at: now,
            updated_at: now,
        };

        // B-C2: write through the app's manager. `open_default()` here would
        // open a second SQLite connection and re-run the DDL batch per call,
        // and would bypass the app's in-memory fallback.
        ctx.memory_manager
            .save_memory(memory_item, &entity_links)
            .await
            .map_err(|e| ToolError::Execution(format!("failed to save memory: {e}")))?;

        Ok(ToolOutput::Text {
            content: format!(
                "Learned and saved rule '{key_phrase}' under category '{category_str}' with {} linked entities.",
                entity_links.len()
            ),
        })
    }
}

#[derive(Clone)]
pub struct SearchQueryHistory {
    _ctx: AiToolContext,
}

impl SearchQueryHistory {
    pub fn new(ctx: AiToolContext) -> Self {
        Self { _ctx: ctx }
    }

    pub fn description(&self) -> String {
        "Search verified golden queries and past executed query history for reusable SQL patterns, exemplar queries, and past analytical solutions.".into()
    }

    pub fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords or search term to look for in query history"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5)"
                }
            },
            "required": ["query"]
        })
    }

    pub async fn call(
        &self,
        args: serde_json::Value,
        ctx: &AiToolContext,
    ) -> Result<ToolOutput, ToolError> {
        // B-I5: honor the app's memory kill-switch before any work. Keyed on
        // the frontend memory connection key so the check matches the stored
        // memory/golden-query keys (worker `ConnectionId` fallback only).
        let connection_key = ctx
            .memory_connection_key
            .clone()
            .or_else(|| ctx.connection_id.map(|id| id.0.to_string()))
            .unwrap_or_else(|| "global".into());
        if !ctx.config.is_memory_enabled(&connection_key) {
            return Err(ToolError::Execution(
                "AI memory is disabled for this connection".into(),
            ));
        }

        let query = args["query"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs("missing 'query'".into()))?;
        let limit = args["limit"].as_u64().unwrap_or(5).clamp(1, 20) as usize;

        let mut results_md = String::new();

        // 1. Search Golden Queries from the app's manager (B-C2).
        if let Ok(golden) = ctx
            .memory_manager
            .list_golden_queries(&connection_key)
            .await
        {
            let matching_golden: Vec<_> = golden
                .into_iter()
                .filter(|g| {
                    g.natural_prompt
                        .to_lowercase()
                        .contains(&query.to_lowercase())
                        || g.sql_text.to_lowercase().contains(&query.to_lowercase())
                })
                .take(limit)
                .collect();

            if !matching_golden.is_empty() {
                results_md.push_str("### ⭐ Verified Golden Queries\n\n");
                for g in matching_golden {
                    results_md.push_str(&format!(
                        "- **Prompt:** {}\n  ```sql\n  {}\n  ```\n",
                        g.natural_prompt, g.sql_text
                    ));
                }
                results_md.push('\n');
            }
        }

        // 2. Search Human Executed History from query_history.jsonl
        let human_history =
            query_history::search_entries(Some(query), Some(&connection_key), false);

        let matching_history: Vec<_> = human_history
            .into_iter()
            .filter(|h| h.status == "success")
            .take(limit)
            .collect();

        if !matching_history.is_empty() {
            results_md.push_str("### 📜 Past Executed Queries\n\n");
            for h in matching_history {
                let rows_label = h.row_count.map(|r| format!("{r} rows")).unwrap_or_default();
                results_md.push_str(&format!(
                    "- **Executed at:** {} ({}ms, {})\n  ```sql\n  {}\n  ```\n",
                    h.executed_at, h.duration_ms, rows_label, h.sql
                ));
            }
        }

        if results_md.is_empty() {
            results_md =
                format!("No matching query history or golden queries found for '{query}'.");
        }

        Ok(ToolOutput::Text {
            content: results_md,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::config::AiConfig;
    use crate::ai::memory::{GoldenQuery, MemoryManager};
    use lucent_protocol::ConnectionId;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn ctx(
        mgr: Arc<MemoryManager>,
        conn: ConnectionId,
        config: AiConfig,
        memory_key: Option<&str>,
    ) -> AiToolContext {
        AiToolContext {
            db: Arc::new(Mutex::new(None)),
            connection_id: Some(conn),
            memory_connection_key: memory_key.map(str::to_string),
            capabilities: None,
            config,
            schema_graph: Arc::new(Mutex::new(None)),
            embedder: Arc::new(Mutex::new(None)),
            reranker: Arc::new(Mutex::new(None)),
            memory_manager: mgr,
        }
    }

    fn save_args() -> serde_json::Value {
        json!({
            "category": "metric",
            "key_phrase": "active_subscribers",
            "rule_text": "Users with status = 'active'"
        })
    }

    /// B-C2: the tool must persist through `ctx.memory_manager`. A per-call
    /// `open_default()` writes to the user's real `memory.db`, so this
    /// isolated manager stays empty and the assertion fails.
    #[tokio::test]
    async fn save_memory_writes_through_ctx_manager() {
        let mgr = Arc::new(MemoryManager::open_in_memory().unwrap());
        let conn = ConnectionId(Uuid::nil());
        let ctx = ctx(mgr.clone(), conn, AiConfig::default(), None);

        SaveMemory::new(ctx.clone())
            .call(save_args(), &ctx)
            .await
            .expect("save succeeds with memory enabled");

        let stored = mgr.list_memories(&conn.0.to_string(), false).await.unwrap();
        assert_eq!(
            stored.len(),
            1,
            "the tool must write through ctx.memory_manager, not open_default()"
        );
        assert_eq!(stored[0].key_phrase, "active_subscribers");
    }

    /// B-C2 for the read path: a golden query seeded in `ctx.memory_manager`
    /// must be found. `open_default()` reads a different DB and finds nothing.
    #[tokio::test]
    async fn search_query_history_reads_through_ctx_manager() {
        let mgr = Arc::new(MemoryManager::open_in_memory().unwrap());
        let conn = ConnectionId(Uuid::nil());
        let cid = conn.0.to_string();
        mgr.save_golden_query(GoldenQuery {
            id: Uuid::new_v4().to_string(),
            connection_id: cid.clone(),
            schema_name: "public".into(),
            natural_prompt: "monthly revenue".into(),
            sql_text: "SELECT sum(amount) FROM orders".into(),
            tables_used: vec!["orders".into()],
            verified: true,
            run_count: 1,
            last_run_at: 1,
            embedding_model: MEMORY_MODEL_NAME.into(),
            embedding_version: MEMORY_FORMAT_VERSION,
            embedding: vec![0.0; 384],
            created_at: 1,
        })
        .await
        .unwrap();

        let ctx = ctx(mgr, conn, AiConfig::default(), None);
        let out = SearchQueryHistory::new(ctx.clone())
            .call(json!({ "query": "revenue" }), &ctx)
            .await
            .expect("search succeeds with memory enabled");

        match out {
            ToolOutput::Text { content } => assert!(
                content.contains("monthly revenue"),
                "golden query from ctx.memory_manager must be returned: {content}"
            ),
            other => panic!("expected text output, got {other:?}"),
        }
    }

    /// B-I5: the global kill-switch must stop `save_memory`.
    #[tokio::test]
    async fn save_memory_refuses_when_memory_disabled() {
        let mgr = Arc::new(MemoryManager::open_in_memory().unwrap());
        let conn = ConnectionId(Uuid::nil());
        let config = AiConfig {
            enable_ai_memory: false,
            ..Default::default()
        };
        let ctx = ctx(mgr.clone(), conn, config, None);

        let err = SaveMemory::new(ctx.clone())
            .call(save_args(), &ctx)
            .await
            .expect_err("disabled memory must refuse the save");
        assert!(
            matches!(err, ToolError::Execution(ref m) if m.contains("disabled")),
            "got {err:?}"
        );
        assert!(
            mgr.list_memories(&conn.0.to_string(), false)
                .await
                .unwrap()
                .is_empty(),
            "a refused save must not write"
        );
    }

    /// B-I5: the global kill-switch must stop `search_query_history`.
    #[tokio::test]
    async fn search_query_history_refuses_when_memory_disabled() {
        let mgr = Arc::new(MemoryManager::open_in_memory().unwrap());
        let conn = ConnectionId(Uuid::nil());
        let config = AiConfig {
            enable_ai_memory: false,
            ..Default::default()
        };
        let ctx = ctx(mgr, conn, config, None);

        let err = SearchQueryHistory::new(ctx.clone())
            .call(json!({ "query": "x" }), &ctx)
            .await
            .expect_err("disabled memory must refuse the search");
        assert!(
            matches!(err, ToolError::Execution(ref m) if m.contains("disabled")),
            "got {err:?}"
        );
    }

    /// B-I5: the per-connection kill-switch (not just the global flag) is honored
    /// on the search path too.
    #[tokio::test]
    async fn search_query_history_refuses_when_connection_disabled() {
        let mgr = Arc::new(MemoryManager::open_in_memory().unwrap());
        let conn = ConnectionId(Uuid::nil());
        let mut config = AiConfig::default();
        config
            .disabled_memory_connections
            .insert(conn.0.to_string());
        let ctx = ctx(mgr, conn, config, None);

        let err = SearchQueryHistory::new(ctx.clone())
            .call(json!({ "query": "x" }), &ctx)
            .await
            .expect_err("a disabled connection must refuse the search");
        assert!(
            matches!(err, ToolError::Execution(ref m) if m.contains("disabled")),
            "got {err:?}"
        );
    }

    /// B-I5: the per-connection kill-switch (not just the global flag) is honored.
    #[tokio::test]
    async fn save_memory_refuses_when_connection_disabled() {
        let mgr = Arc::new(MemoryManager::open_in_memory().unwrap());
        let conn = ConnectionId(Uuid::nil());
        let mut config = AiConfig::default();
        config
            .disabled_memory_connections
            .insert(conn.0.to_string());
        let ctx = ctx(mgr, conn, config, None);

        let err = SaveMemory::new(ctx.clone())
            .call(save_args(), &ctx)
            .await
            .expect_err("a disabled connection must refuse the save");
        assert!(
            matches!(err, ToolError::Execution(ref m) if m.contains("disabled")),
            "got {err:?}"
        );
    }

    /// REQUIRED FIX 1: a tool-authored memory must be stored under the FRONTEND
    /// memory connection key, not the worker `ConnectionId`. Chat retrieval,
    /// the Memory Drawer, `save_memory_manual`, and the Gate 1 drift cascade all
    /// key on the frontend key, so a memory saved under the worker UUID would be
    /// invisible to every one of them (and never drift-invalidated).
    #[tokio::test]
    async fn save_memory_uses_frontend_memory_key_when_present() {
        let mgr = Arc::new(MemoryManager::open_in_memory().unwrap());
        let conn = ConnectionId(Uuid::nil());
        let ctx = ctx(mgr.clone(), conn, AiConfig::default(), Some("frontend-key"));

        SaveMemory::new(ctx.clone())
            .call(save_args(), &ctx)
            .await
            .expect("save succeeds with memory enabled");

        let stored = mgr.list_memories("frontend-key", false).await.unwrap();
        assert_eq!(
            stored.len(),
            1,
            "memory must be stored under the frontend memory key"
        );
        assert_eq!(stored[0].connection_key, "frontend-key");
        assert_eq!(stored[0].scope_key, "frontend-key");
        assert!(
            mgr.list_memories(&conn.0.to_string(), false)
                .await
                .unwrap()
                .is_empty(),
            "the worker ConnectionId must not be used when the frontend key is present"
        );
    }

    /// REQUIRED FIX 1 (B-I5): the per-connection kill-switch must test the
    /// FRONTEND key. Disabling that connection stops the tool even though the
    /// worker `ConnectionId` is not in the disabled set.
    #[tokio::test]
    async fn save_memory_kill_switch_uses_frontend_memory_key() {
        let mgr = Arc::new(MemoryManager::open_in_memory().unwrap());
        let conn = ConnectionId(Uuid::nil());
        let mut config = AiConfig::default();
        config
            .disabled_memory_connections
            .insert("frontend-key".into());
        let ctx = ctx(mgr.clone(), conn, config, Some("frontend-key"));

        let err = SaveMemory::new(ctx.clone())
            .call(save_args(), &ctx)
            .await
            .expect_err("a disabled frontend connection must refuse the save");
        assert!(
            matches!(err, ToolError::Execution(ref m) if m.contains("disabled")),
            "got {err:?}"
        );
        assert!(
            mgr.list_memories("frontend-key", false)
                .await
                .unwrap()
                .is_empty(),
            "a refused save must not write"
        );
    }

    /// REQUIRED FIX 1: golden-query lookup must use the frontend memory key too,
    /// so `search_query_history` finds queries saved through the drawer under the
    /// same key instead of silently searching an empty worker-UUID bucket.
    #[tokio::test]
    async fn search_query_history_reads_under_frontend_memory_key() {
        let mgr = Arc::new(MemoryManager::open_in_memory().unwrap());
        let conn = ConnectionId(Uuid::nil());
        mgr.save_golden_query(GoldenQuery {
            id: Uuid::new_v4().to_string(),
            connection_id: "frontend-key".into(),
            schema_name: "public".into(),
            natural_prompt: "monthly revenue".into(),
            sql_text: "SELECT sum(amount) FROM orders".into(),
            tables_used: vec!["orders".into()],
            verified: true,
            run_count: 1,
            last_run_at: 1,
            embedding_model: MEMORY_MODEL_NAME.into(),
            embedding_version: MEMORY_FORMAT_VERSION,
            embedding: vec![0.0; 384],
            created_at: 1,
        })
        .await
        .unwrap();

        let ctx = ctx(mgr, conn, AiConfig::default(), Some("frontend-key"));
        let out = SearchQueryHistory::new(ctx.clone())
            .call(json!({ "query": "revenue" }), &ctx)
            .await
            .expect("search succeeds with memory enabled");

        match out {
            ToolOutput::Text { content } => assert!(
                content.contains("monthly revenue"),
                "golden query stored under the frontend key must be found: {content}"
            ),
            other => panic!("expected text output, got {other:?}"),
        }
    }
}
