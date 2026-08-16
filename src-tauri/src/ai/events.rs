use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiEvent {
    Thinking {
        content: String,
    }, // NEW — model reasoning
    Text {
        content: String,
    },
    ToolCalls {
        tools: Vec<ToolCallInfo>,
    },
    ToolResult {
        id: String,
        tool: String,
        summary: String,
        output: Option<serde_json::Value>,
    },
    /// A system notice from Lucent itself (not the agent): rendered as a
    /// note segment in the work session. Used e.g. when the ACP agent never
    /// connected the DB-tools bridge, so the user sees why the database
    /// tools are missing.
    Notice {
        content: String,
    },
    QueryResult {
        columns: Vec<ColumnMeta>,
        rows: Vec<Vec<serde_json::Value>>,
        row_count: usize,
        sql: String,
        execution_time_ms: u64,
    },
    Done {
        conversation_id: String,
        final_message: String,
        usage: TokenUsage,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnMeta {
    pub name: String,
    pub data_type: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// Prompt tokens served from the provider's prefix cache (0 = no cache hit).
    #[serde(default)]
    pub cached_prompt_tokens: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DmlApprovalPayload {
    pub conversation_id: String,
    pub sql: String,
    pub tables_affected: Vec<String>,
    pub description: String,
    pub estimated_rows_affected: Option<u64>,
}

/// The `ai:agent_permission` payload: the agent is asking the user for
/// permission to run one of ITS tools (file/bash/etc) — distinct from DML
/// approval, which is Lucent's own gate on its own tool (spec §3 D6 / §4.5).
/// `options` mirrors the agent's `PermissionOption`s; the frontend renders
/// allow-once / reject from them.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPermissionPayload {
    pub conversation_id: String,
    pub title: String,
    pub description: String,
    pub options: Vec<AgentPermissionOption>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgentPermissionOption {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiErrorPayload {
    pub conversation_id: String,
    pub message: String,
}

/// Payload sink abstraction so tests don't need a Tauri runtime.
pub trait Emit: Send + Sync {
    fn emit_json(&self, event: &str, payload: serde_json::Value);
}

/// Count-based telemetry emitter for background schema indexing. Routes
/// progress/error payloads to the frontend, throttling progress events so a
/// 2,000-table schema cannot flood the IPC channel.
pub struct IndexingEmitter {
    emitter: Arc<dyn Emit>,
    last_emitted: std::sync::Mutex<HashMap<String, (usize, String)>>,
}

impl IndexingEmitter {
    pub fn new(emitter: Arc<dyn Emit>) -> Self {
        Self {
            emitter,
            last_emitted: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Count-based throttle: stage changes, every 5th processed table, and
    /// terminal events always emit. A connection's FIRST event always emits
    /// (a small schema may only produce one non-terminal event, and swallowing
    /// it would leave the indicator dark until completion).
    pub fn progress(&self, payload: &IndexingProgressPayload) {
        let key = payload.connection_id.clone();
        let mut last = self.last_emitted.lock().unwrap_or_else(|e| e.into_inner());
        let prev = last.get(&key).cloned();
        let every_fifth = payload.processed_tables.is_multiple_of(5);
        let (prev_count, prev_stage) = match &prev {
            Some((c, s)) => (*c, s.clone()),
            None => (usize::MAX, String::new()),
        };
        let first_event = prev.is_none();
        let stage_changed = prev_stage != payload.stage;
        let new_count = prev_count != payload.processed_tables;
        if first_event
            || payload.is_complete
            || payload.stage == "model"
            || (every_fifth && new_count)
            || stage_changed
        {
            if payload.is_complete {
                // Prune: a completed connection never emits again, and keeping
                // the entry would make the next (reused-id) event look like a
                // stage change. Bounded map over the session's reconnects.
                last.remove(&key);
            } else {
                last.insert(key, (payload.processed_tables, payload.stage.clone()));
            }
            self.emitter.emit_json(
                "indexing:progress",
                serde_json::to_value(payload).unwrap_or(serde_json::Value::Null),
            );
        }
    }

    pub fn error(&self, payload: &IndexingErrorPayload) {
        self.emitter.emit_json(
            "indexing:error",
            serde_json::to_value(payload).unwrap_or(serde_json::Value::Null),
        );
    }
}

impl crate::ai::indexer::IndexingEventSink for IndexingEmitter {
    fn emit_progress(&self, payload: IndexingProgressPayload) {
        self.progress(&payload);
    }
    fn emit_error(&self, connection_id: &str, message: &str) {
        self.error(&IndexingErrorPayload {
            connection_id: connection_id.into(),
            message: message.into(),
        });
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexingProgressPayload {
    pub connection_id: String,
    pub stage: String, // "model" | "metadata" | "sampling" | "embedding" | "complete"
    pub processed_tables: usize,
    pub total_tables: usize,
    pub cache_hits: usize,
    pub embeddings_computed: usize,
    pub is_complete: bool,
    pub elapsed_ms: u64,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexingErrorPayload {
    pub connection_id: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn thinking_event_serializes_with_type() {
        let event = AiEvent::Thinking {
            content: "Let me analyze the schema...".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "thinking", "frontend matches on type field");
        assert_eq!(json["content"], "Let me analyze the schema...");
    }

    #[test]
    fn tool_call_info_serializes_with_id() {
        let info = ToolCallInfo {
            id: "call_1".into(),
            name: "search_schema".into(),
            args: serde_json::json!({"query": "test"}),
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(
            json["id"], "call_1",
            "frontend needs the id to match results to the right card"
        );
    }

    #[test]
    fn tool_result_event_serializes_with_id() {
        let event = AiEvent::ToolResult {
            id: "call_1".into(),
            tool: "search_schema".into(),
            summary: "ok".into(),
            output: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["id"], "call_1");
    }

    #[test]
    fn notice_event_serializes_with_type() {
        let event = AiEvent::Notice {
            content: "db tools unavailable".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "notice");
        assert_eq!(json["content"], "db tools unavailable");
    }

    #[tokio::test]
    async fn emitter_throttles_by_count_and_always_emits_terminal() {
        struct FakeEmit {
            emitted: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        }
        impl super::Emit for FakeEmit {
            fn emit_json(&self, _event: &str, payload: serde_json::Value) {
                self.emitted.lock().unwrap().push(payload);
            }
        }

        let emitted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let emitter = super::IndexingEmitter::new(Arc::new(FakeEmit {
            emitted: emitted.clone(),
        }));
        let payload = |n: usize, complete: bool| IndexingProgressPayload {
            connection_id: "c1".into(),
            stage: if complete {
                "complete".into()
            } else {
                "sampling".into()
            },
            processed_tables: n,
            total_tables: 200,
            cache_hits: 0,
            embeddings_computed: 0,
            is_complete: complete,
            elapsed_ms: 1,
            detail: None,
        };
        for n in 1..=50 {
            emitter.progress(&payload(n, false));
        }
        emitter.progress(&payload(200, true));
        let count = emitted.lock().unwrap().len();
        assert!(
            count <= 13,
            "50 table events throttle to ~10 + terminal; got {count}"
        );
        assert!(emitted
            .lock()
            .unwrap()
            .iter()
            .any(|v| v["isComplete"] == true));
    }

    #[test]
    fn permission_payload_serializes_camel_case() {
        let p = AgentPermissionPayload {
            conversation_id: "c1".into(),
            title: "Approve?".into(),
            description: "d".into(),
            options: vec![AgentPermissionOption {
                id: "allow-once".into(),
                name: "Allow once".into(),
            }],
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["conversationId"], "c1");
        assert_eq!(v["title"], "Approve?");
        assert_eq!(v["options"][0]["id"], "allow-once");
        assert_eq!(v["options"][0]["name"], "Allow once");
    }

    #[test]
    fn token_usage_deserializes_without_cached_field_for_backward_compat() {
        // E5: estimated_cost_usd was removed (dead — always None). Old JSON
        // carrying it must still parse (serde ignores unknown fields).
        let json = r#"{"prompt_tokens": 100, "completion_tokens": 20, "estimated_cost_usd": null}"#;
        let usage: TokenUsage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.cached_prompt_tokens, 0, "missing field defaults to 0");
        assert_eq!(usage.prompt_tokens, 100);
    }
}
