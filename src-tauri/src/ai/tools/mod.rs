pub mod execute;
pub mod memory;
pub mod objects;
pub mod search_schema;

use lucent_protocol::ConnectionId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::client::ConnectorClient;

/// Shared context passed to all tools at call time.
pub struct AiToolContext {
    pub db: Arc<Mutex<Option<ConnectorClient>>>,
    pub connection_id: Option<ConnectionId>,
    /// The frontend memory connection key (profile id or `host:port/database`)
    /// captured on `AppState.memory_connection_key` at connect time. Memory is
    /// stored and retrieved under THIS key, so tools that write memory must key
    /// on it — the worker `ConnectionId` above is a per-process UUID that no
    /// other memory surface knows. `None` in tests and before a connection is
    /// captured; tools then fall back to the worker id.
    pub memory_connection_key: Option<String>,
    /// Capabilities of the connection these tools run against. `None` when
    /// disconnected; every tool already errors with `NotConnected` first.
    pub capabilities: Option<lucent_protocol::DriverCapabilities>,
    pub config: crate::ai::config::AiConfig,
    pub schema_graph: Arc<Mutex<Option<crate::ai::schema_graph::SchemaGraph>>>,
    pub embedder: Arc<Mutex<Option<crate::ai::embed::Embedder>>>,
    pub reranker: Arc<Mutex<Option<crate::ai::rerank::Reranker>>>,
    /// The app's single `MemoryManager`. Tools must write/read memory through
    /// this handle — opening a fresh connection per call re-runs the DDL batch
    /// and bypasses the app's in-memory fallback (B-C2).
    pub memory_manager: Arc<crate::ai::memory::MemoryManager>,
}

impl Clone for AiToolContext {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            connection_id: self.connection_id,
            memory_connection_key: self.memory_connection_key.clone(),
            capabilities: self.capabilities.clone(),
            config: self.config.clone(),
            schema_graph: Arc::clone(&self.schema_graph),
            embedder: Arc::clone(&self.embedder),
            reranker: Arc::clone(&self.reranker),
            memory_manager: Arc::clone(&self.memory_manager),
        }
    }
}

/// An isolated, DDL-initialized in-memory memory DB for tests that need a
/// valid `AiToolContext` without touching the user's real `memory.db`.
#[cfg(test)]
pub fn test_memory_manager() -> Arc<crate::ai::memory::MemoryManager> {
    Arc::new(crate::ai::memory::MemoryManager::open_in_memory().expect("in-memory memory db opens"))
}

#[derive(Clone)]
pub enum LucentToolEnum {
    GetObjectsInfo(objects::GetObjectsInfo),
    SearchSchema(search_schema::SearchSchema),
    RunReadonlyQuery(execute::RunReadonlyQuery),
    PreviewDml(execute::PreviewDml),
    SaveMemory(memory::SaveMemory),
    SearchQueryHistory(memory::SearchQueryHistory),
}

impl LucentToolEnum {
    pub fn name(&self) -> &str {
        match self {
            LucentToolEnum::GetObjectsInfo(_) => "get_objects_info",
            LucentToolEnum::SearchSchema(_) => "search_schema",
            LucentToolEnum::RunReadonlyQuery(_) => "run_readonly_query",
            LucentToolEnum::PreviewDml(_) => "preview_dml",
            LucentToolEnum::SaveMemory(_) => "save_memory",
            LucentToolEnum::SearchQueryHistory(_) => "search_query_history",
        }
    }

    pub fn description(&self) -> String {
        match self {
            LucentToolEnum::GetObjectsInfo(t) => t.description(),
            LucentToolEnum::SearchSchema(t) => t.description(),
            LucentToolEnum::RunReadonlyQuery(t) => t.description(),
            LucentToolEnum::PreviewDml(t) => t.description(),
            LucentToolEnum::SaveMemory(t) => t.description(),
            LucentToolEnum::SearchQueryHistory(t) => t.description(),
        }
    }

    pub fn parameters(&self) -> serde_json::Value {
        match self {
            LucentToolEnum::GetObjectsInfo(t) => t.parameters(),
            LucentToolEnum::SearchSchema(t) => t.parameters(),
            LucentToolEnum::RunReadonlyQuery(t) => t.parameters(),
            LucentToolEnum::PreviewDml(t) => t.parameters(),
            LucentToolEnum::SaveMemory(t) => t.parameters(),
            LucentToolEnum::SearchQueryHistory(t) => t.parameters(),
        }
    }

    pub async fn call(
        &self,
        args: serde_json::Value,
        ctx: &AiToolContext,
    ) -> Result<ToolOutput, ToolError> {
        match self {
            LucentToolEnum::GetObjectsInfo(t) => t.call(args, ctx).await,
            LucentToolEnum::SearchSchema(t) => t.call(args, ctx).await,
            LucentToolEnum::RunReadonlyQuery(t) => t.call(args, ctx).await,
            LucentToolEnum::PreviewDml(t) => t.call(args, ctx).await,
            LucentToolEnum::SaveMemory(t) => t.call(args, ctx).await,
            LucentToolEnum::SearchQueryHistory(t) => t.call(args, ctx).await,
        }
    }
}

#[derive(Error, Debug)]
pub enum ToolError {
    #[error("Execution error: {0}")]
    Execution(String),
    #[error("Invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("SQL validation failed: {0}")]
    SqlValidation(String),
    #[error("Database error: {0}")]
    Database(String),
    #[error("Database not connected — the AI agent has no database connection")]
    NotConnected,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ToolOutput {
    Text {
        content: String,
    },
    QueryResult {
        text_summary: String,
        columns: Vec<crate::ai::events::ColumnMeta>,
        rows: Vec<Vec<serde_json::Value>>,
        row_count: usize,
        sql: String,
        execution_time_ms: u64,
        truncated: bool,
    },
    DmlPreview {
        sql: String,
        statement_type: String,
        tables_affected: Vec<String>,
        description: String,
        estimated_rows_affected: Option<u64>,
    },
}

pub fn all_tools(ctx: AiToolContext) -> Vec<LucentToolEnum> {
    vec![
        LucentToolEnum::SearchSchema(search_schema::SearchSchema::new(ctx.clone())),
        LucentToolEnum::GetObjectsInfo(objects::GetObjectsInfo::new(ctx.clone())),
        LucentToolEnum::RunReadonlyQuery(execute::RunReadonlyQuery::new(ctx.clone())),
        LucentToolEnum::PreviewDml(execute::PreviewDml::new(ctx.clone())),
        LucentToolEnum::SaveMemory(memory::SaveMemory::new(ctx.clone())),
        LucentToolEnum::SearchQueryHistory(memory::SearchQueryHistory::new(ctx)),
    ]
}
