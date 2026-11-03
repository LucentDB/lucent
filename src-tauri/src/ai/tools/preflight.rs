use serde_json::json;

use super::{AiToolContext, ToolError, ToolOutput};

#[derive(Clone)]
pub struct GetPreflightContext {
    ctx: AiToolContext,
}

impl GetPreflightContext {
    pub fn new(ctx: AiToolContext) -> Self {
        Self { ctx }
    }

    pub fn description(&self) -> String {
        "Ground query planning with preflight schema context, value hints, and sample column matches \
         based on literals in the user prompt."
            .into()
    }

    pub fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The user query or intention to ground with schema preflight hints and literals."
                }
            },
            "required": ["prompt"]
        })
    }

    pub async fn call(
        &self,
        args: serde_json::Value,
        _ctx: &AiToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let prompt = args["prompt"]
            .as_str()
            .or_else(|| args["question"].as_str())
            .ok_or_else(|| ToolError::InvalidArgs("missing 'prompt'".into()))?;

        let graph_guard = self.ctx.schema_graph.lock().await;
        let embedder_guard = self.ctx.embedder.lock().await;

        let tier = crate::ai::mschema::ContextTier::Pull;
        let preflight = crate::ai::preflight::run_preflight(
            self.ctx.connection_id,
            Some(&self.ctx.db),
            graph_guard.as_ref(),
            embedder_guard.as_ref(),
            &tier,
            prompt,
            self.ctx.capabilities.as_ref(),
        )
        .await;

        let content =
            preflight.unwrap_or_else(|| "No preflight hints found for the given prompt.".to_string());

        Ok(ToolOutput::Text { content })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn preflight_tool_returns_text_output() {
        let ctx = AiToolContext {
            db: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            connection_id: None,
            memory_connection_key: None,
            capabilities: None,
            config: crate::ai::config::AiConfig::default(),
            schema_graph: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            embedder: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            reranker: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            memory_manager: crate::ai::tools::test_memory_manager(),
        };

        let tool = GetPreflightContext::new(ctx.clone());
        let res = tool
            .call(json!({"prompt": "find orders for customer Alice"}), &ctx)
            .await
            .unwrap();
        match res {
            ToolOutput::Text { content } => {
                assert!(!content.is_empty());
            }
            _ => panic!("expected Text output"),
        }
    }
}
