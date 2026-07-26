use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCallInfo {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ColumnMeta {
    pub name: String,
    pub data_type: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub estimated_cost_usd: Option<f64>,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AiErrorPayload {
    pub conversation_id: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn token_usage_deserializes_without_cached_field_for_backward_compat() {
        let json = r#"{"prompt_tokens": 100, "completion_tokens": 20, "estimated_cost_usd": null}"#;
        let usage: TokenUsage = serde_json::from_str(json).unwrap();
        assert_eq!(usage.cached_prompt_tokens, 0, "missing field defaults to 0");
        assert_eq!(usage.prompt_tokens, 100);
    }
}
