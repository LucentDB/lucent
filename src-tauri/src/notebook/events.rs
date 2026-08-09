use serde::Serialize;

use crate::notebook::types::{AiCellState, CellError, CellOutput};

#[derive(Clone, Serialize)]
#[serde(tag = "type", content = "payload")]
pub enum NotebookEvent {
    #[serde(rename = "thinking_started")]
    ThinkingStarted { cell_id: String },
    #[serde(rename = "thinking_chunk")]
    ThinkingChunk { cell_id: String, chunk: String },
    #[serde(rename = "thinking_done")]
    ThinkingDone { cell_id: String, duration_ms: u64 },
    #[serde(rename = "tool_call")]
    ToolCall {
        cell_id: String,
        tool: serde_json::Value,
    },
    #[serde(rename = "sql_preview")]
    SqlPreview { cell_id: String, sql: String },
    #[serde(rename = "rows_streamed")]
    RowsStreamed {
        cell_id: String,
        rows: Vec<Vec<serde_json::Value>>,
        is_end: bool,
    },
    #[serde(rename = "cell_done")]
    CellDone {
        cell_id: String,
        output: CellOutput,
        ai_state: Option<AiCellState>,
        execution_order: u32,
        duration_ms: u64,
    },
    #[serde(rename = "cell_error")]
    CellError { cell_id: String, error: CellError },
}
