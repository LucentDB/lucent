//! Correlates bridge-emitted tool results (which carry the MCP call id,
//! `acp-{n}`) with the agent's ACP `tool_call_id`s, so structured outputs
//! land on the right UI card (spec D3).
//!
//! Ordering guarantee that makes this sound: the bridge executes a tool call
//! BEFORE the agent reports its completion (`tools/call` round-trip precedes
//! the `tool_call_update`), so the wrapper buffers the structured payload
//! and the driver pops it when `ToolCallUpdate(Completed)` arrives. The
//! bridge serves one connection sequentially, so buffered entries arrive in
//! execution order; name match disambiguates parallel calls.

use crate::ai::agent::AgentSink;
use crate::ai::events::{AgentPermissionPayload, AiEvent, DmlApprovalPayload};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// One buffered bridge result: the tool name and the structured payload.
#[derive(Clone, Debug)]
pub struct BufferedToolResult {
    pub tool: String,
    pub summary: String,
    pub output: serde_json::Value,
}

/// The shared buffer between the bridge's sink wrapper (writer) and the
/// chat driver (reader, on `ToolCallUpdate`).
#[derive(Default)]
pub struct CorrelatorState {
    queue: Mutex<VecDeque<BufferedToolResult>>,
}

impl CorrelatorState {
    pub fn push(&self, entry: BufferedToolResult) {
        self.queue.lock().unwrap().push_back(entry);
    }

    /// Pops the oldest entry whose tool name matches `tool`, else the oldest
    /// entry (the bridge serializes calls, so FIFO is the fallback order).
    pub fn pop_for(&self, tool: &str) -> Option<BufferedToolResult> {
        let mut q = self.queue.lock().unwrap();
        if let Some(idx) = q.iter().position(|e| e.tool == tool) {
            return q.remove(idx);
        }
        q.pop_front()
    }

    pub fn clear(&self) {
        self.queue.lock().unwrap().clear();
    }

    pub fn is_empty(&self) -> bool {
        self.queue.lock().unwrap().is_empty()
    }
}

/// Sink wrapper installed as the bridge's sink: buffers structured
/// `ToolResult`s (the raw `acp-{n}` event never reaches the frontend) and
/// forwards everything else unchanged.
pub struct CorrelatingSink {
    inner: Arc<dyn AgentSink>,
    state: Arc<CorrelatorState>,
}

impl CorrelatingSink {
    pub fn new(inner: Arc<dyn AgentSink>, state: Arc<CorrelatorState>) -> Self {
        Self { inner, state }
    }

    pub fn state(&self) -> Arc<CorrelatorState> {
        self.state.clone()
    }
}

impl AgentSink for CorrelatingSink {
    fn event(&self, event: AiEvent) {
        match event {
            AiEvent::ToolResult {
                id,
                tool,
                summary,
                output: Some(output),
                ..
            } if id.starts_with("acp-") => {
                self.state.push(BufferedToolResult {
                    tool,
                    summary,
                    output,
                });
            }
            other => self.inner.event(other),
        }
    }

    fn dml_approval(&self, payload: DmlApprovalPayload) {
        self.inner.dml_approval(payload);
    }

    fn permission_request(&self, payload: AgentPermissionPayload) {
        self.inner.permission_request(payload);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct RecordingSink {
        events: StdMutex<Vec<AiEvent>>,
    }
    impl RecordingSink {
        fn new() -> Self {
            Self {
                events: StdMutex::new(Vec::new()),
            }
        }
    }
    impl AgentSink for RecordingSink {
        fn event(&self, event: AiEvent) {
            self.events.lock().unwrap().push(event);
        }
        fn dml_approval(&self, _payload: DmlApprovalPayload) {}
    }

    fn query_result_payload() -> serde_json::Value {
        serde_json::json!({"type": "query_result", "columns": [], "rows": [], "row_count": 0, "sql": "select 1", "execution_time_ms": 1, "truncated": false})
    }

    fn acp_tool_result(tool: &str) -> AiEvent {
        AiEvent::ToolResult {
            id: "acp-0".into(),
            tool: tool.into(),
            summary: "1 row".into(),
            output: Some(query_result_payload()),
            status: crate::ai::events::ToolResultStatus::Completed,
        }
    }

    #[test]
    fn structured_acp_results_are_buffered_not_forwarded() {
        let sink = Arc::new(RecordingSink::new());
        let state = Arc::new(CorrelatorState::default());
        let wrapper = CorrelatingSink::new(sink.clone(), state.clone());

        wrapper.event(acp_tool_result("run_readonly_query"));
        wrapper.event(AiEvent::Text {
            content: "hi".into(),
        });

        assert_eq!(
            sink.events.lock().unwrap().len(),
            1,
            "only the Text forwards"
        );
        assert!(matches!(
            sink.events.lock().unwrap()[0],
            AiEvent::Text { .. }
        ));
        assert_eq!(
            state.queue.lock().unwrap().len(),
            1,
            "the result is buffered"
        );
        assert!(!state.is_empty());
    }

    #[test]
    fn pop_for_matches_by_name_then_falls_back_to_fifo() {
        let state = Arc::new(CorrelatorState::default());
        state.push(BufferedToolResult {
            tool: "search_schema".into(),
            summary: "s".into(),
            output: query_result_payload(),
        });
        state.push(BufferedToolResult {
            tool: "run_readonly_query".into(),
            summary: "q".into(),
            output: query_result_payload(),
        });

        // Name match pops the SECOND (the one whose update arrives now).
        let got = state.pop_for("run_readonly_query").unwrap();
        assert_eq!(got.tool, "run_readonly_query");
        // A tool with no buffered entry falls back to the oldest.
        let got = state.pop_for("get_objects_info").unwrap();
        assert_eq!(got.tool, "search_schema");
        assert!(state.is_empty());
    }

    #[test]
    fn clear_resets_the_queue() {
        let state = Arc::new(CorrelatorState::default());
        state.push(BufferedToolResult {
            tool: "t".into(),
            summary: "s".into(),
            output: query_result_payload(),
        });
        state.clear();
        assert!(state.is_empty());
    }

    #[test]
    fn non_acp_events_forward_unchanged() {
        let sink = Arc::new(RecordingSink::new());
        let wrapper = CorrelatingSink::new(sink.clone(), Arc::new(CorrelatorState::default()));
        wrapper.event(AiEvent::ToolResult {
            id: "call_1".into(),
            tool: "t".into(),
            summary: "s".into(),
            output: None,
            status: crate::ai::events::ToolResultStatus::Completed,
        });
        assert_eq!(
            sink.events.lock().unwrap().len(),
            1,
            "agent-id events forward untouched"
        );
    }
}
