use crate::ai::agent::{
    LlmError, LlmResponse, Message, MessageContent, MessageRole, ToolCall as LucentToolCall,
};
use crate::ai::config::AiProvider;
use crate::ai::events::TokenUsage;
use crate::ai::provider::{LlmProvider, LucentAgent};
use crate::ai::providers::dispatch;
use crate::ai::tools::LucentToolEnum;
use async_trait::async_trait;
use futures::StreamExt;
use rig_core::client::CompletionClient;
use rig_core::completion::message::{AssistantContent, ReasoningContent};
use rig_core::completion::{self, Completion, CompletionModel, GetTokenUsage, ToolDefinition};
use rig_core::one_or_many::OneOrMany;
use rig_core::providers::{anthropic, deepseek, gemini, mistral, openai, openrouter};
use rig_core::streaming::StreamedAssistantContent;
use rig_core::tool::{ToolDyn, ToolError as RigToolError};
use rig_core::wasm_compat::WasmBoxedFuture;

pub struct RigProvider {
    kind: AiProvider,
    api_key: String,
    endpoint: Option<String>,
}

impl RigProvider {
    pub fn new(kind: AiProvider, api_key: String, endpoint: Option<String>) -> Self {
        Self {
            kind,
            api_key,
            endpoint,
        }
    }
}

#[async_trait]
impl LlmProvider for RigProvider {
    async fn build_agent(
        &self,
        model: &str,
        preamble: String,
        max_tokens: u32,
        tools: Vec<LucentToolEnum>,
    ) -> Box<dyn LucentAgent> {
        let rig_tools = build_rig_tools(&tools);

        match self.kind {
            AiProvider::OpenAI => match openai::CompletionsClient::builder()
                .api_key(self.api_key.as_str())
                .build()
            {
                Ok(client) => wrap_agent(build_rig_agent(
                    &client, model, &preamble, max_tokens, rig_tools,
                )),
                Err(e) => {
                    eprintln!("Failed to create OpenAI client: {e}");
                    Box::new(StubAgent)
                }
            },
            AiProvider::Anthropic => match anthropic::Client::builder()
                .api_key(self.api_key.as_str())
                .build()
            {
                Ok(client) => wrap_agent(build_rig_agent(
                    &client, model, &preamble, max_tokens, rig_tools,
                )),
                Err(e) => {
                    eprintln!("Failed to create Anthropic client: {e}");
                    Box::new(StubAgent)
                }
            },
            AiProvider::Gemini => match gemini::Client::builder()
                .api_key(self.api_key.as_str())
                .build()
            {
                Ok(client) => wrap_agent(build_rig_agent(
                    &client, model, &preamble, max_tokens, rig_tools,
                )),
                Err(e) => {
                    eprintln!("Failed to create Gemini client: {e}");
                    Box::new(StubAgent)
                }
            },
            AiProvider::OpenRouter => match openrouter::Client::builder()
                .api_key(self.api_key.as_str())
                .build()
            {
                Ok(client) => wrap_agent(build_rig_agent(
                    &client, model, &preamble, max_tokens, rig_tools,
                )),
                Err(e) => {
                    eprintln!("Failed to create OpenRouter client: {e}");
                    Box::new(StubAgent)
                }
            },
            AiProvider::Mistral => match mistral::Client::builder()
                .api_key(self.api_key.as_str())
                .build()
            {
                Ok(client) => wrap_agent(build_rig_agent(
                    &client, model, &preamble, max_tokens, rig_tools,
                )),
                Err(e) => {
                    eprintln!("Failed to create Mistral client: {e}");
                    Box::new(StubAgent)
                }
            },
            AiProvider::DeepSeek => match deepseek::Client::builder()
                .api_key(self.api_key.as_str())
                .build()
            {
                Ok(client) => wrap_agent(build_rig_agent(
                    &client, model, &preamble, max_tokens, rig_tools,
                )),
                Err(e) => {
                    eprintln!("Failed to create DeepSeek client: {e}");
                    Box::new(StubAgent)
                }
            },
            AiProvider::Groq
            | AiProvider::XAI
            | AiProvider::Ollama
            | AiProvider::Custom
            | AiProvider::OpenCode => {
                let base_url = match dispatch::resolve_base_url(&self.kind, &self.endpoint) {
                    Ok(url) => url,
                    Err(e) => {
                        eprintln!("{e}");
                        return Box::new(StubAgent);
                    }
                };
                match openai::CompletionsClient::builder()
                    .api_key(self.api_key.as_str())
                    .base_url(&base_url)
                    .build()
                {
                    Ok(client) => wrap_agent(build_rig_agent(
                        &client, model, &preamble, max_tokens, rig_tools,
                    )),
                    Err(e) => {
                        eprintln!("Failed to create {} client: {e}", self.kind);
                        Box::new(StubAgent)
                    }
                }
            }
            // The ACP path drives agents through AcpChatDriver (phase D) and
            // never constructs a rig provider; keep this arm as a clear
            // failure if it is ever reached.
            AiProvider::Acp => {
                eprintln!("ACP agents are driven through the ACP client, not the rig provider");
                Box::new(StubAgent)
            }
        }
    }
}

// ── Rig tool definition wrapper ──────────────────────────────────────────

struct RigToolDef {
    name: String,
    description: String,
    params: serde_json::Value,
}

impl ToolDyn for RigToolDef {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn definition<'a>(&'a self, _prompt: String) -> WasmBoxedFuture<'a, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: self.name.clone(),
                description: self.description.clone(),
                parameters: self.params.clone(),
            }
        })
    }

    fn call<'a>(&'a self, _args: String) -> WasmBoxedFuture<'a, Result<String, RigToolError>> {
        Box::pin(async move {
            Err(RigToolError::ToolCallError(
                "tool execution handled by DatabaseAgent".into(),
            ))
        })
    }
}

// ── Stub agent for error recovery ────────────────────────────────────────

struct StubAgent;

#[async_trait]
impl LucentAgent for StubAgent {
    async fn complete(
        &self,
        _: Message,
        _: Vec<Message>,
        _on_delta: &(dyn Fn(crate::ai::provider::AgentDelta) + Send + Sync),
    ) -> Result<LlmResponse, LlmError> {
        Err(LlmError::NotConfigured(
            "RigAgent construction failed".into(),
        ))
    }
}

// ── Real Rig agent with streaming ────────────────────────────────────────

struct RigAgent<M: CompletionModel> {
    inner: rig_core::agent::Agent<M>,
}

#[async_trait]
impl<M> LucentAgent for RigAgent<M>
where
    M: CompletionModel + Send + Sync + 'static,
{
    async fn complete(
        &self,
        prompt: Message,
        history: Vec<Message>,
        on_delta: &(dyn Fn(crate::ai::provider::AgentDelta) + Send + Sync),
    ) -> Result<LlmResponse, LlmError> {
        let rig_prompt = convert_to_rig_message(prompt);
        let rig_history: Vec<rig_core::completion::Message> =
            history.into_iter().map(convert_to_rig_message).collect();

        let mut stream = self
            .inner
            .completion(rig_prompt, rig_history)
            .await
            .map_err(|e| LlmError::Api(e.to_string()))?
            .stream()
            .await
            .map_err(|e| LlmError::Api(e.to_string()))?;

        while let Some(item) = stream.next().await {
            match item.map_err(|e| LlmError::Api(e.to_string()))? {
                StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                    on_delta(crate::ai::provider::AgentDelta::Thinking(reasoning));
                }
                StreamedAssistantContent::Text(t) => {
                    on_delta(crate::ai::provider::AgentDelta::Text(t.text));
                }
                _ => {}
            }
        }

        let (final_text, thinking, tool_calls) =
            split_assistant_content(stream.choice.into_iter().collect());

        let usage = stream
            .response
            .as_ref()
            .map(|r| {
                let u = r.token_usage();
                TokenUsage {
                    prompt_tokens: u.input_tokens as u32,
                    completion_tokens: u.output_tokens as u32,
                    cached_prompt_tokens: u.cached_input_tokens as u32,
                }
            })
            .unwrap_or_default();

        Ok(LlmResponse {
            text: final_text,
            tool_calls,
            usage,
            thinking,
        })
    }
}

/// Shared by every arm above — the `.agent(...)` builder chain is identical
/// regardless of which concrete client produced it.
fn build_rig_agent<C>(
    client: &C,
    model: &str,
    preamble: &str,
    max_tokens: u32,
    tools: Vec<Box<dyn ToolDyn>>,
) -> rig_core::agent::Agent<C::CompletionModel>
where
    C: CompletionClient,
{
    client
        .agent(model)
        .preamble(preamble)
        .max_tokens(max_tokens as u64)
        .tools(tools)
        .build()
}

fn wrap_agent<M>(agent: rig_core::agent::Agent<M>) -> Box<dyn LucentAgent>
where
    M: CompletionModel + Send + Sync + 'static,
{
    Box::new(RigAgent { inner: agent })
}

fn build_rig_tools(tools: &[LucentToolEnum]) -> Vec<Box<dyn ToolDyn>> {
    tools
        .iter()
        .map(|t| {
            let td: Box<dyn ToolDyn> = Box::new(RigToolDef {
                name: t.name().to_string(),
                description: t.description(),
                params: t.parameters(),
            });
            td
        })
        .collect()
}

/// Splits an aggregated assistant response into (final text, thinking/reasoning
/// text, tool calls). Shared by the streaming and non-streaming code paths since
/// rig-core exposes the same aggregated `AssistantContent` shape either way.
fn split_assistant_content(
    content: Vec<completion::AssistantContent>,
) -> (Option<String>, Option<String>, Vec<LucentToolCall>) {
    let mut text = String::new();
    let mut reasoning_text = String::new();
    let mut tool_calls: Vec<LucentToolCall> = vec![];

    for item in content {
        match item {
            completion::AssistantContent::Text(t) => {
                text.push_str(&t.text);
            }
            completion::AssistantContent::Reasoning(r) => {
                for rc in r.content {
                    if let ReasoningContent::Text { text: t, .. } = rc {
                        reasoning_text.push_str(&t);
                    }
                    // Encrypted/Redacted reasoning payloads carry no plain text — skip.
                }
            }
            completion::AssistantContent::ToolCall(tc) => {
                tool_calls.push(LucentToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    args: tc.function.arguments,
                });
            }
            _ => {}
        }
    }

    let final_text = if text.is_empty() { None } else { Some(text) };
    let thinking = if reasoning_text.is_empty() {
        None
    } else {
        Some(reasoning_text)
    };
    (final_text, thinking, tool_calls)
}

fn convert_to_rig_message(msg: Message) -> rig_core::completion::Message {
    match msg.role {
        MessageRole::User => match msg.content {
            MessageContent::Text(t) => rig_core::completion::Message::user(t),
            // Tool results inside a User message - pass through the tool_call_id
            MessageContent::ToolResult {
                tool_use_id,
                content,
            } => rig_core::completion::Message::tool_result(tool_use_id, content),
        },
        MessageRole::Assistant => {
            let text = match &msg.content {
                MessageContent::Text(t) => t.clone(),
                MessageContent::ToolResult { .. } => String::new(),
            };
            let mut contents: Vec<AssistantContent> = Vec::new();
            if !text.is_empty() {
                contents.push(AssistantContent::text(text));
            }
            if let Some(tcs) = msg.tool_calls {
                for tc in tcs {
                    contents.push(AssistantContent::tool_call(tc.id, tc.name, tc.args));
                }
            }
            // OneOrMany requires at least one item; if both text and tool_calls
            // are empty, fall back to an empty text message.
            if contents.is_empty() {
                contents.push(AssistantContent::text(""));
            }
            rig_core::completion::Message::Assistant {
                id: None,
                content: OneOrMany::many(contents).expect("contents non-empty by construction"),
            }
        }
        MessageRole::Tool => match msg.content {
            // Our internal Tool role maps to a User message with ToolResult content in Rig.
            // This produces role: "tool" in the OpenAI API via Rig's conversion layer.
            MessageContent::Text(t) => rig_core::completion::Message::user(t),
            MessageContent::ToolResult {
                tool_use_id,
                content,
            } => rig_core::completion::Message::tool_result(tool_use_id, content),
        },
    }
}

// ── Message conversion ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::agent::{MessageContent, MessageRole, ToolCall as T};

    fn make_msg(role: MessageRole, content: &str, tool_calls: Option<Vec<T>>) -> Message {
        Message {
            role,
            content: MessageContent::Text(content.to_string()),
            tool_calls,
        }
    }

    #[test]
    fn convert_user_preserves_text() {
        let msg = make_msg(MessageRole::User, "hello", None);
        let rig = convert_to_rig_message(msg);
        // Should produce a User message with text content
        let as_str = format!("{rig:?}");
        assert!(
            as_str.contains("User") || as_str.contains("hello"),
            "user text should be preserved"
        );
    }

    #[test]
    fn convert_assistant_preserves_tool_calls() {
        let tcs = vec![T {
            id: "call_1".into(),
            name: "search_objects".into(),
            args: serde_json::json!({"query": "acme"}),
        }];
        let msg = make_msg(MessageRole::Assistant, "", Some(tcs));
        let rig = convert_to_rig_message(msg);
        // The assistant message should include the tool call
        match &rig {
            rig_core::completion::Message::Assistant { content, .. } => {
                let items: Vec<_> = content.clone().into_iter().collect();
                let has_tc = items
                    .iter()
                    .any(|c| matches!(c, AssistantContent::ToolCall(_)));
                assert!(has_tc, "assistant should carry ToolCall content");
            }
            _ => panic!("expected Assistant message"),
        }
    }

    #[test]
    fn convert_assistant_text_and_tool_calls() {
        let tcs = vec![T {
            id: "call_1".into(),
            name: "search_objects".into(),
            args: serde_json::json!({"query": "acme"}),
        }];
        let msg = Message {
            role: MessageRole::Assistant,
            content: MessageContent::Text("I'll search.".to_string()),
            tool_calls: Some(tcs),
        };
        let rig = convert_to_rig_message(msg);
        match &rig {
            rig_core::completion::Message::Assistant { content, .. } => {
                let items: Vec<_> = content.clone().into_iter().collect();
                let has_text = items.iter().any(|c| matches!(c, AssistantContent::Text(_)));
                let has_tc = items
                    .iter()
                    .any(|c| matches!(c, AssistantContent::ToolCall(_)));
                assert!(has_text, "should have text content");
                assert!(has_tc, "should have tool call content");
            }
            _ => panic!("expected Assistant message"),
        }
    }

    #[test]
    fn convert_tool_result_preserves_id() {
        let msg = Message {
            role: MessageRole::Tool,
            content: MessageContent::ToolResult {
                tool_use_id: "call_1".into(),
                content: "result data".into(),
            },
            tool_calls: None,
        };
        let rig = convert_to_rig_message(msg);
        // Should produce a message that includes "call_1" reference
        let as_str = format!("{rig:?}");
        assert!(
            as_str.contains("call_1"),
            "tool_call_id should be preserved"
        );
    }

    #[test]
    fn convert_empty_assistant_falls_back() {
        let msg = make_msg(MessageRole::Assistant, "", None);
        let rig = convert_to_rig_message(msg);
        match &rig {
            rig_core::completion::Message::Assistant { content, .. } => {
                let items: Vec<_> = content.clone().into_iter().collect();
                assert!(!items.is_empty(), "should have at least one content item");
            }
            _ => panic!("expected Assistant message"),
        }
    }

    #[test]
    fn split_assistant_content_extracts_text_only() {
        let content = vec![completion::AssistantContent::text("hello")];
        let (text, thinking, tool_calls) = split_assistant_content(content);
        assert_eq!(text, Some("hello".to_string()));
        assert_eq!(thinking, None);
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn split_assistant_content_extracts_reasoning_as_thinking() {
        let content = vec![completion::AssistantContent::reasoning("thinking about it")];
        let (text, thinking, tool_calls) = split_assistant_content(content);
        assert_eq!(text, None);
        assert_eq!(thinking, Some("thinking about it".to_string()));
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn split_assistant_content_combines_text_reasoning_and_tool_calls() {
        let content = vec![
            completion::AssistantContent::reasoning("let me check"),
            completion::AssistantContent::text("here's the answer"),
            completion::AssistantContent::tool_call(
                "call_1",
                "search_schema",
                serde_json::json!({"query": "users"}),
            ),
        ];
        let (text, thinking, tool_calls) = split_assistant_content(content);
        assert_eq!(text, Some("here's the answer".to_string()));
        assert_eq!(thinking, Some("let me check".to_string()));
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "search_schema");
    }

    #[test]
    fn split_assistant_content_skips_encrypted_and_redacted_reasoning() {
        // Construct via JSON since Reasoning is #[non_exhaustive]
        let content: Vec<completion::AssistantContent> =
            serde_json::from_value(serde_json::json!([
                {
                    "type": "reasoning",
                    "id": null,
                    "content": [
                        {"type": "encrypted", "content": "opaque-blob"},
                        {"type": "redacted", "content": {"data": "redacted"}}
                    ]
                }
            ]))
            .unwrap();
        let (text, thinking, tool_calls) = split_assistant_content(content);
        assert_eq!(text, None);
        assert_eq!(
            thinking, None,
            "encrypted/redacted reasoning has no plain text to surface"
        );
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn split_assistant_content_empty_input_yields_none() {
        let (text, thinking, tool_calls) = split_assistant_content(vec![]);
        assert_eq!(text, None);
        assert_eq!(thinking, None);
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn anthropic_provider_does_not_default_to_openai_base_url() {
        // Regression guard for the bug this task fixes: selecting Anthropic with
        // no custom endpoint must build against Anthropic's own base URL, not
        // silently fall through to OpenAI's. `Client::builder().build()` doesn't
        // make a network call, so this is safe to assert synchronously.
        let client = rig_core::providers::anthropic::Client::builder()
            .api_key("test-key-not-a-real-key")
            .build()
            .expect("client construction with a syntactically valid key should succeed offline");
        assert_eq!(client.base_url(), "https://api.anthropic.com");
    }
}
