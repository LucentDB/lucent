pub mod bounded;
pub mod execute;
pub mod export;
pub mod fetch_rows;
pub mod inspect_connection;
pub mod memory;
pub mod objects;
pub mod preflight;
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
    /// Result-handle scope: the id this surface's results are fenced to, plus
    /// the process store handle. Assigned by each surface (§6A.3).
    pub result_scope: crate::ai::result_store::ResultScope,
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
            result_scope: self.result_scope.clone(),
        }
    }
}

/// An isolated, DDL-initialized in-memory memory DB for tests that need a
/// valid `AiToolContext` without touching the user's real `memory.db`.
#[cfg(test)]
pub fn test_memory_manager() -> Arc<crate::ai::memory::MemoryManager> {
    Arc::new(crate::ai::memory::MemoryManager::open_in_memory().expect("in-memory memory db opens"))
}

/// A neutral tool context for schema/description queries (no connection).
/// Production-safe: `name()`/`description()`/`parameters()` never touch the
/// in-memory memory db this carries.
pub fn neutral_tool_ctx() -> AiToolContext {
    AiToolContext {
        db: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        connection_id: None,
        memory_connection_key: None,
        capabilities: None,
        config: crate::ai::config::AiConfig::default(),
        schema_graph: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        embedder: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        reranker: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        memory_manager: std::sync::Arc::new(
            crate::ai::memory::MemoryManager::open_in_memory()
                .expect("in-memory memory db for the neutral tool context"),
        ),
        result_scope: crate::ai::result_store::ResultScope::isolated("neutral"),
    }
}

/// `{"query": string, "mode"?: "semantic"|"keyword"|"hybrid"}` — the argument
/// shape at a glance (`?` = optional), derived from the JSON schema.
fn compact_args(schema: &serde_json::Value) -> String {
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    let Some(props) = schema["properties"].as_object() else {
        return "{}".into();
    };
    let fields: Vec<String> = props
        .iter()
        .map(|(name, spec)| {
            let ty = match spec["enum"].as_array() {
                Some(values) => values
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(|v| format!("\"{v}\""))
                    .collect::<Vec<_>>()
                    .join("|"),
                None => spec["type"].as_str().unwrap_or("value").to_string(),
            };
            let optional = if required.contains(&name.as_str()) {
                ""
            } else {
                "?"
            };
            format!("\"{name}\"{optional}: {ty}")
        })
        .collect();
    format!("{{{}}}", fields.join(", "))
}

/// The first sentence of a tool description — the prompt's map entry.
fn first_sentence(text: &str) -> &str {
    text.find(". ").map_or(text, |i| &text[..=i])
}

/// The prompt's tool section, generated from `LucentToolEnum` — the single
/// source of truth (P1, F12). Compact on purpose (see the size test): full
/// descriptions and schemas already reach the model natively. Cached: the
/// schemas are static, and rebuilding per prompt would rebuild the neutral
/// context on every turn.
pub fn tool_docs_section() -> &'static str {
    static DOCS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    DOCS.get_or_init(|| {
        let tools = all_tools(neutral_tool_ctx());
        let mut out = String::from("AVAILABLE TOOLS:\n");
        for (i, t) in tools.iter().enumerate() {
            let description = t.description().replace('\n', " ");
            out.push_str(&format!(
                "\n{}. {} — {}\n   Args: {}\n",
                i + 1,
                t.name(),
                first_sentence(&description),
                compact_args(&t.parameters())
            ));
        }
        out
    })
}

/// A tool context bound to a live worker connection — the one literal the
/// integration/capstone tests share.
#[cfg(test)]
pub fn live_tool_ctx(client: ConnectorClient, conn_id: ConnectionId) -> AiToolContext {
    let capabilities = client.server_info.as_ref().map(|s| s.capabilities.clone());
    AiToolContext {
        db: Arc::new(Mutex::new(Some(client))),
        connection_id: Some(conn_id),
        memory_connection_key: None,
        capabilities,
        config: crate::ai::config::AiConfig::default(),
        schema_graph: Arc::new(Mutex::new(None)),
        embedder: Arc::new(Mutex::new(None)),
        reranker: Arc::new(Mutex::new(None)),
        memory_manager: test_memory_manager(),
        result_scope: crate::ai::result_store::ResultScope::isolated("test"),
    }
}

#[derive(Clone)]
pub enum LucentToolEnum {
    GetObjectsInfo(objects::GetObjectsInfo),
    SearchSchema(search_schema::SearchSchema),
    RunReadonlyQuery(execute::RunReadonlyQuery),
    PreviewDml(execute::PreviewDml),
    SaveMemory(memory::SaveMemory),
    SearchQueryHistory(memory::SearchQueryHistory),
    GetPreflightContext(preflight::GetPreflightContext),
    ListMemories(memory::ListMemories),
    InspectConnection(inspect_connection::InspectConnection),
    ExportQuery(export::ExportQuery),
    FetchRows(fetch_rows::FetchRows),
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
            LucentToolEnum::GetPreflightContext(_) => "get_preflight_context",
            LucentToolEnum::ListMemories(_) => "list_memories",
            LucentToolEnum::InspectConnection(_) => "inspect_connection",
            LucentToolEnum::ExportQuery(_) => "export_query",
            LucentToolEnum::FetchRows(_) => "fetch_rows",
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
            LucentToolEnum::GetPreflightContext(t) => t.description(),
            LucentToolEnum::ListMemories(t) => t.description(),
            LucentToolEnum::InspectConnection(t) => t.description(),
            LucentToolEnum::ExportQuery(t) => t.description(),
            LucentToolEnum::FetchRows(t) => t.description(),
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
            LucentToolEnum::GetPreflightContext(t) => t.parameters(),
            LucentToolEnum::ListMemories(t) => t.parameters(),
            LucentToolEnum::InspectConnection(t) => t.parameters(),
            LucentToolEnum::ExportQuery(t) => t.parameters(),
            LucentToolEnum::FetchRows(t) => t.parameters(),
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
            LucentToolEnum::GetPreflightContext(t) => t.call(args, ctx).await,
            LucentToolEnum::ListMemories(t) => t.call(args, ctx).await,
            LucentToolEnum::InspectConnection(t) => t.call(args, ctx).await,
            LucentToolEnum::ExportQuery(t) => t.call(args, ctx).await,
            LucentToolEnum::FetchRows(t) => t.call(args, ctx).await,
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
        /// Handle into the surface's result scope (§6A.3); `None` when the
        /// result was too large to retain.
        result_id: Option<String>,
        /// Rows actually rendered into the model-visible block (§5.10).
        rendered_rows: usize,
    },
    DmlPreview {
        sql: String,
        statement_type: String,
        tables_affected: Vec<String>,
        description: String,
        estimated_rows_affected: Option<u64>,
    },
}

/// Tools notebook AI cells never get (ledger D1: cells do not write files).
/// The ONE list both notebook seams read — the rig tool list and the ACP
/// bridge executor — so they cannot drift.
pub const NOTEBOOK_CELL_EXCLUDED_TOOLS: &[&str] = &["export_query"];

/// `all_tools` minus a surface's exclusions.
pub fn tools_excluding(ctx: AiToolContext, excluded: &[&str]) -> Vec<LucentToolEnum> {
    all_tools(ctx)
        .into_iter()
        .filter(|t| !excluded.contains(&t.name()))
        .collect()
}

pub fn all_tools(ctx: AiToolContext) -> Vec<LucentToolEnum> {
    vec![
        LucentToolEnum::SearchSchema(search_schema::SearchSchema::new(ctx.clone())),
        LucentToolEnum::GetObjectsInfo(objects::GetObjectsInfo::new(ctx.clone())),
        LucentToolEnum::RunReadonlyQuery(execute::RunReadonlyQuery::new(ctx.clone())),
        LucentToolEnum::PreviewDml(execute::PreviewDml::new(ctx.clone())),
        LucentToolEnum::SaveMemory(memory::SaveMemory::new(ctx.clone())),
        LucentToolEnum::SearchQueryHistory(memory::SearchQueryHistory::new(ctx.clone())),
        LucentToolEnum::GetPreflightContext(preflight::GetPreflightContext::new(ctx.clone())),
        LucentToolEnum::ListMemories(memory::ListMemories::new(ctx.clone())),
        LucentToolEnum::InspectConnection(inspect_connection::InspectConnection::new(ctx.clone())),
        LucentToolEnum::ExportQuery(export::ExportQuery::new(ctx.clone())),
        LucentToolEnum::FetchRows(fetch_rows::FetchRows::new(ctx)),
    ]
}
