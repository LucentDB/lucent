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
    /// One tool call finished. Streamed so an AI cell's tool card fills in
    /// live — the same four-state model the chat pane uses — instead of
    /// staying a bare "Done" row until `cell_done` lands. `input` carries the
    /// arguments the tool really ran with, which is the only source when the
    /// agent announced a shell command (the ACP CLI path).
    #[serde(rename = "tool_result")]
    ToolResult {
        cell_id: String,
        id: String,
        tool: String,
        summary: String,
        input: Option<serde_json::Value>,
        output: Option<serde_json::Value>,
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
