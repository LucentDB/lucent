use lucent_protocol::ColumnMeta;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookMetadata {
    pub connection_id: Option<String>,
    pub connection_name: Option<String>,
    pub connection_host: Option<String>,
    pub database: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub lucent_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellModel {
    pub id: String,
    pub kind: CellKind,
    pub source: String,
    pub alias: Option<String>,
    pub collapsed: bool,
    pub outputs: Option<CellOutput>,
    pub status: CellStatus,
    pub execution_order: Option<u32>,
    pub duration_ms: Option<u64>,
    pub error: Option<CellError>,
    pub stale_since: Option<u64>,
    pub ai_state: Option<AiCellState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CellKind {
    Sql,
    Markdown,
    Ai,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CellStatus {
    Pending,
    Running,
    Ok,
    Error,
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CellOutput {
    Table(TableOutput),
    Text(TextOutput),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableOutput {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<serde_json::Value>>,
    /// Known row count, or None until the user asks for it. A cell must never pay
    /// COUNT(*) over a large table unasked.
    pub total_count: Option<u64>,
    pub is_truncated: bool,
    /// Rows per page for this cell's grid.
    #[serde(default = "default_page_size")]
    pub page_size: i64,
    /// False for DML/DDL/multi-statement cells, which cannot be paged or filtered.
    #[serde(default)]
    pub is_wrappable: bool,
    /// Rows affected for DML cells (INSERT/UPDATE/DELETE); None for row-returning
    /// queries. Old notebooks predating the field deserialize as None.
    #[serde(default)]
    pub rows_affected: Option<u64>,
}

/// JSON producers that predate the paging fields (AI tool results) omit
/// `page_size`; the notebook default is what they mean.
fn default_page_size() -> i64 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextOutput {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiCellState {
    pub conversation_id: String,
    pub final_sql: Option<String>,
    pub response: Option<String>,
    pub messages: Vec<serde_json::Value>,
    pub tool_calls: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CellError {
    CyclicDependency {
        cycle: Vec<String>,
        hint: String,
    },
    NotExecuted {
        cell_id: String,
        hint: String,
    },
    TextNotReferencable {
        cell_id: String,
        message: String,
    },
    NotATable {
        cell_id: String,
        message: String,
    },
    NotExecutable {
        cell_id: String,
        message: String,
    },
    StaleReference {
        cell_id: String,
        hint: String,
    },
    UnresolvedRef {
        cell_id: String,
        ref_name: String,
        hint: String,
    },
    QueryError {
        message: String,
        sql_error: String,
    },
    ConnectionLost {
        message: String,
    },
}
