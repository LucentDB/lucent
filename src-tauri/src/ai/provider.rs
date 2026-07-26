use async_trait::async_trait;

/// A single incremental chunk emitted by the model during a `complete()` call,
/// forwarded live so the caller can stream it to the frontend as it arrives.
pub enum AgentDelta {
    Thinking(String),
    Text(String),
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn build_agent(
        &self,
        model: &str,
        preamble: String,
        max_tokens: u32,
        tools: Vec<crate::ai::tools::LucentToolEnum>,
    ) -> Box<dyn LucentAgent>;
}

#[async_trait]
pub trait LucentAgent: Send {
    async fn complete(
        &self,
        prompt: crate::ai::agent::Message,
        history: Vec<crate::ai::agent::Message>,
        on_delta: &(dyn Fn(AgentDelta) + Send + Sync),
    ) -> Result<crate::ai::agent::LlmResponse, crate::ai::agent::LlmError>;
}
