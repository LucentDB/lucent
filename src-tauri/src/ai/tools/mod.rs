pub mod execute;
pub mod objects;
pub mod search_schema;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::client::ConnectorClient;

/// Shared context passed to all tools at call time.
pub struct AiToolContext {
    pub db: Arc<Mutex<Option<ConnectorClient>>>,
    pub config: crate::ai::config::AiConfig,
    pub schema_graph: Arc<Mutex<Option<crate::ai::schema_graph::SchemaGraph>>>,
    pub embedder: Arc<Mutex<Option<crate::ai::embed::Embedder>>>,
    pub reranker: Arc<Mutex<Option<crate::ai::rerank::Reranker>>>,
}

impl Clone for AiToolContext {
    fn clone(&self) -> Self {
        Self {
            db: Arc::clone(&self.db),
            config: self.config.clone(),
            schema_graph: Arc::clone(&self.schema_graph),
            embedder: Arc::clone(&self.embedder),
            reranker: Arc::clone(&self.reranker),
        }
    }
}

#[derive(Clone)]
pub enum LucentToolEnum {
    GetObjectsInfo(objects::GetObjectsInfo),
    SearchSchema(search_schema::SearchSchema),
    RunReadonlyQuery(execute::RunReadonlyQuery),
    PreviewDml(execute::PreviewDml),
}

impl LucentToolEnum {
    pub fn name(&self) -> &str {
        match self {
            LucentToolEnum::GetObjectsInfo(_) => "get_objects_info",
            LucentToolEnum::SearchSchema(_) => "search_schema",
            LucentToolEnum::RunReadonlyQuery(_) => "run_readonly_query",
            LucentToolEnum::PreviewDml(_) => "preview_dml",
        }
    }

    pub fn description(&self) -> String {
        match self {
            LucentToolEnum::GetObjectsInfo(t) => t.description(),
            LucentToolEnum::SearchSchema(t) => t.description(),
            LucentToolEnum::RunReadonlyQuery(t) => t.description(),
            LucentToolEnum::PreviewDml(t) => t.description(),
        }
    }

    pub fn parameters(&self) -> serde_json::Value {
        match self {
            LucentToolEnum::GetObjectsInfo(t) => t.parameters(),
            LucentToolEnum::SearchSchema(t) => t.parameters(),
            LucentToolEnum::RunReadonlyQuery(t) => t.parameters(),
            LucentToolEnum::PreviewDml(t) => t.parameters(),
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
        LucentToolEnum::PreviewDml(execute::PreviewDml::new(ctx)),
    ]
}
