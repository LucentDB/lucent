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
use std::future::Future;

pub struct RigProvider {
    kind: AiProvider,
    api_key: String,
    endpoint: Option<String>,
    /// Stable per-conversation opaque id, sent as `x-opencode-session` on the
    /// OpenCode Go path so the gateway can do routing/prompt-cache affinity.
    session_id: Option<String>,
}

impl RigProvider {
    pub fn new(
        kind: AiProvider,
        api_key: String,
        endpoint: Option<String>,
        session_id: Option<String>,
    ) -> Self {
        Self {
            kind,
            api_key,
            endpoint,
            session_id,
        }
    }

    /// Headers identifying Lucent to the OpenCode Go gateway (docs bullets
    /// 2+3): a stable session id plus Lucent's own User-Agent. Empty when no
    /// session id was supplied — other providers are untouched.
    fn opencode_headers(&self) -> Result<reqwest::header::HeaderMap, LlmError> {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue, USER_AGENT};
        let mut headers = HeaderMap::new();
        if let Some(sid) = self.session_id.as_deref().map(str::trim) {
            if !sid.is_empty() {
                headers.insert(
                    HeaderName::from_static("x-opencode-session"),
                    HeaderValue::from_str(sid)
                        .map_err(|_| LlmError::NotConfigured("invalid session id".into()))?,
                );
                headers.insert(
                    USER_AGENT,
                    HeaderValue::from_str(&format!("lucent/{}", env!("CARGO_PKG_VERSION")))
                        .map_err(|_| LlmError::NotConfigured("invalid user agent".into()))?,
                );
            }
        }
        Ok(headers)
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
                Ok(client) => {
                    // Automatic prompt caching: rig-core owns breakpoint
                    // placement and advances the moving breakpoint as the
                    // conversation grows — otherwise even a byte-stable
                    // prefix is never marked cacheable.
                    let model = client.completion_model(model).with_automatic_caching();
                    wrap_agent(
                        rig_core::agent::AgentBuilder::new(model)
                            .preamble(&preamble)
                            .max_tokens(max_tokens as u64)
                            .tools(rig_tools)
                            .build(),
                    )
                }
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
                    .http_headers(match self.opencode_headers() {
                        Ok(h) => h,
                        Err(e) => {
                            eprintln!("{e}");
                            return Box::new(StubAgent);
                        }
                    })
                    // Normalise gateway reasoning dialects (`reasoning` /
                    // `reasoning_details[]`) onto rig-core's `reasoning_content`
                    // before parsing — see ReasoningRewritingClient. NOTE: the
                    // inner client MUST be rig-core's own re-exported reqwest
                    // (0.13): Lucent's workspace reqwest is 0.12, a different
                    // `Client` type that does not implement HttpClientExt.
                    .http_client(ReasoningRewritingClient {
                        inner: rig_core::http_client::ReqwestClient::new(),
                    })
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

// ── Reasoning-field normalisation (OpenCode Go gateway) ───────────────────
// rig-core 0.39.0's OpenAI-compatible stream parses exactly one reasoning key:
// `delta.reasoning_content` (`completion/streaming.rs:53,192`). The OpenCode Go
// gateway instead streams `delta.reasoning` (string) mirrored in
// `delta.reasoning_details[]` (`{type:"reasoning.text", text}` entries) —
// verified by live capture (see docs/superpowers/findings/opencode-thinking.md).
// Unknown keys are dropped silently by serde, so without translation Lucent
// never receives Thinking deltas. This wrapper rewrites each SSE `data:` line
// onto the one key rig-core parses. The rewrite is stateful and line-aligned
// (`rewrite_sse_stream` buffers bytes across HTTP chunks until complete
// `\n`-terminated lines are available) because chunk boundaries never align
// with SSE lines. Scoped to the OpenAI-compatible arm only;
// the Anthropic/Mistral arms already speak working dialects.

/// Mirrors gateway reasoning dialects onto the one key rig-core parses.
/// When `reasoning_content` is absent it is filled from `reasoning` (string),
/// else from `reasoning_details[]` (`text` of `reasoning.text` entries only —
/// encrypted/redacted entries carry no plain text and are skipped).
fn normalise_reasoning_field(mut delta: serde_json::Value) -> serde_json::Value {
    // An explicit `null` (Go's stop chunk sends `"reasoning": null`) counts as
    // absent — only a present non-empty string wins.
    if delta
        .get("reasoning_content")
        .and_then(|v| v.as_str())
        .is_none()
    {
        let text = delta
            .get("reasoning")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .or_else(|| {
                // MiniMax-style array of parts; only `reasoning.text` entries
                // carry plain text — encrypted/redacted entries are skipped.
                delta
                    .get("reasoning_details")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter(|e| {
                                e.get("type").and_then(|t| t.as_str()) == Some("reasoning.text")
                            })
                            .filter_map(|e| e.get("text").and_then(|t| t.as_str()))
                            .collect::<Vec<_>>()
                            .join("")
                    })
            });

        if let Some(text) = text.filter(|s| !s.is_empty()) {
            if let Some(obj) = delta.as_object_mut() {
                obj.insert("reasoning_content".into(), serde_json::Value::String(text));
            }
        }
    }
    delta
}

/// Rewrites only the complete (`\n`-terminated) lines in `buf`.
///
/// Byte-identical for everything that needs no rewrite: lines without a
/// `data:` prefix, lines that fail JSON parse, non-UTF-8 lines, and lines
/// whose delta gains no new `reasoning_content` key are emitted as the exact
/// input bytes. Only lines where `normalise_reasoning_field` newly mirrors a
/// reasoning key are re-serialized (with a `data: ` prefix, SSE-equivalent).
/// Callers must only pass bytes ending at a `\n` line boundary — a `\n` byte
/// can never split a UTF-8 multi-byte character, so per-line `str::from_utf8`
/// is safe and partial characters are never lossy-decoded.
fn rewrite_complete_lines(buf: &[u8]) -> bytes::Bytes {
    // Cheap fast path: no `data:` lines, nothing to rewrite.
    if !buf.windows(5).any(|w| w == b"data:") {
        return bytes::Bytes::copy_from_slice(buf);
    }
    let mut out: Vec<u8> = Vec::with_capacity(buf.len() + 64);
    let mut changed = false;
    for line in buf.split_inclusive(|&b| b == b'\n') {
        // SSE allows `data:` with an optional single space — accept both so
        // a missing space cannot silently lose reasoning.
        let payload = match line.strip_prefix(b"data:") {
            Some(p) => p.strip_prefix(b" ").unwrap_or(p),
            None => {
                out.extend_from_slice(line);
                continue;
            }
        };
        let text = match std::str::from_utf8(payload) {
            Ok(t) => t.trim_end(),
            Err(_) => {
                out.extend_from_slice(line);
                continue;
            }
        };
        match serde_json::from_str::<serde_json::Value>(text) {
            Ok(mut v) if v.pointer("/choices/0/delta").is_some() => {
                let had = v
                    .pointer("/choices/0/delta/reasoning_content")
                    .and_then(|v| v.as_str())
                    .is_some();
                if let Some(d) = v.pointer_mut("/choices/0/delta").map(|d| d.take()) {
                    *v.pointer_mut("/choices/0/delta").unwrap() = normalise_reasoning_field(d);
                }
                let has = v
                    .pointer("/choices/0/delta/reasoning_content")
                    .and_then(|v| v.as_str())
                    .is_some();
                if !had && has {
                    changed = true;
                    out.extend_from_slice(b"data: ");
                    // Serialisation is infallible for a Value built from parsed JSON.
                    out.extend_from_slice(serde_json::to_string(&v).unwrap_or_default().as_bytes());
                    out.push(b'\n');
                } else {
                    // Nothing to mirror (already normalised, or no reasoning
                    // keys at all): emit the original line byte-identically.
                    out.extend_from_slice(line);
                }
            }
            _ => {
                // `[DONE]`, usage chunks, non-JSON lines: byte-identical.
                out.extend_from_slice(line);
            }
        }
    }
    if changed {
        bytes::Bytes::from(out)
    } else {
        bytes::Bytes::copy_from_slice(buf)
    }
}

/// Rewrites one SSE byte-chunk: complete lines go through
/// `rewrite_complete_lines`; any trailing partial line (no `\n` yet) passes
/// through byte-identically. Only lines starting with `data:` (optional
/// space) that parse as JSON objects with `choices[0].delta` are touched;
/// every other byte ([DONE], usage chunks, non-JSON lines) passes through
/// intact. NOTE: this single-chunk entry cannot normalize a `data:` line
/// split across chunks — production streaming uses `rewrite_sse_stream`,
/// which buffers the partial line across chunk calls instead.
#[cfg(test)]
fn rewrite_sse_reasoning(chunk: bytes::Bytes) -> bytes::Bytes {
    // Cheap fast path: chunks without `data:` lines (keep-alives, comments)
    // never need rewriting — return them untouched, allocation-free.
    if !chunk.windows(5).any(|w| w == b"data:") {
        return chunk;
    }
    let head_end = chunk
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    if head_end == chunk.len() {
        return rewrite_complete_lines(&chunk);
    }
    let rewritten = rewrite_complete_lines(&chunk[..head_end]);
    let mut out = Vec::with_capacity(chunk.len() + 64);
    out.extend_from_slice(&rewritten);
    out.extend_from_slice(&chunk[head_end..]);
    bytes::Bytes::from(out)
}

/// Stateful, line-aligned SSE rewrite wrapper.
///
/// Buffers incoming bytes across chunk calls and rewrites only complete
/// `\n`-terminated lines (via `rewrite_complete_lines`), emitting them as
/// soon as a full line is available — never held back waiting for the
/// blank-line event terminator. The trailing partial line stays buffered
/// until the next chunk completes it: never lossy-decoded, never dropped,
/// never emitted early. When the upstream stream ends, any buffered
/// remainder is flushed byte-identically. Error items pass through
/// unchanged, retaining the buffer.
fn rewrite_sse_stream(
    inner: rig_core::http_client::sse::BoxedStream,
) -> rig_core::http_client::sse::BoxedStream {
    Box::pin(futures::stream::unfold(
        (inner, bytes::BytesMut::new()),
        |(mut inner, mut buf)| async move {
            loop {
                if let Some(nl) = buf.iter().rposition(|&b| b == b'\n') {
                    let complete = buf.split_to(nl + 1);
                    let rewritten = rewrite_complete_lines(&complete);
                    return Some((Ok(rewritten), (inner, buf)));
                }
                match inner.next().await {
                    Some(Ok(chunk)) => {
                        buf.extend_from_slice(&chunk);
                    }
                    Some(Err(e)) => return Some((Err(e), (inner, buf))),
                    None if buf.is_empty() => return None,
                    None => {
                        let tail = buf.split_to(buf.len());
                        return Some((Ok(tail.freeze()), (inner, buf)));
                    }
                }
            }
        },
    ))
}

/// Generic over the inner client so tests can wrap rig-core's
/// `MockStreamingClient` and drive the real pipeline; production wraps
/// `reqwest::Client`.
/// `Debug + Default` are required because rig-core's `GenericCompletionModel`
/// bounds its `H` on both (`completion/mod.rs`).
#[derive(Clone, Debug, Default)]
struct ReasoningRewritingClient<H> {
    inner: H,
}

impl<H> rig_core::http_client::HttpClientExt for ReasoningRewritingClient<H>
where
    H: rig_core::http_client::HttpClientExt + Clone + 'static,
{
    fn send<T, U>(
        &self,
        req: rig_core::http_client::Request<T>,
    ) -> impl Future<
        Output = rig_core::http_client::Result<
            rig_core::http_client::Response<rig_core::http_client::LazyBody<U>>,
        >,
    > + rig_core::wasm_compat::WasmCompatSend
           + 'static
    where
        T: Into<bytes::Bytes> + rig_core::wasm_compat::WasmCompatSend,
        U: From<bytes::Bytes> + rig_core::wasm_compat::WasmCompatSend + 'static,
    {
        // Materialise the body before the async block: the returned future is
        // `'static`, so the generic `T` must not be captured by it.
        let inner = self.inner.clone();
        let (parts, body) = req.into_parts();
        let body: bytes::Bytes = body.into();
        let req = rig_core::http_client::Request::from_parts(parts, body);
        async move { inner.send(req).await }
    }

    fn send_multipart<U>(
        &self,
        req: rig_core::http_client::Request<rig_core::http_client::MultipartForm>,
    ) -> impl Future<
        Output = rig_core::http_client::Result<
            rig_core::http_client::Response<rig_core::http_client::LazyBody<U>>,
        >,
    > + rig_core::wasm_compat::WasmCompatSend
           + 'static
    where
        U: From<bytes::Bytes> + rig_core::wasm_compat::WasmCompatSend + 'static,
    {
        let inner = self.inner.clone();
        async move { inner.send_multipart(req).await }
    }

    #[allow(clippy::manual_async_fn)]
    fn send_streaming<T>(
        &self,
        req: rig_core::http_client::Request<T>,
    ) -> impl Future<Output = rig_core::http_client::Result<rig_core::http_client::StreamingResponse>>
           + rig_core::wasm_compat::WasmCompatSend
    where
        T: Into<bytes::Bytes> + rig_core::wasm_compat::WasmCompatSend,
    {
        async move {
            let resp = self.inner.send_streaming(req).await?;
            // Rebuild with the mapped body, preserving status/version/headers
            // (same shape as rig-core's own StreamingResponse construction in
            // `test_utils/http.rs`: builder + status + headers + boxed stream).
            let (parts, body) = resp.into_parts();
            let body: rig_core::http_client::sse::BoxedStream = rewrite_sse_stream(body);
            Ok(rig_core::http_client::Response::from_parts(parts, body))
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

        let mut streamed_text = String::new();
        let mut streamed_thinking = String::new();

        while let Some(item) = stream.next().await {
            match item.map_err(|e| LlmError::Api(e.to_string()))? {
                StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                    streamed_thinking.push_str(&reasoning);
                    on_delta(crate::ai::provider::AgentDelta::Thinking(reasoning));
                }
                // A complete reasoning block (no deltas) — some gateways send
                // one whole block. Forward its plain-text parts live, like the
                // delta arm above. Encrypted/redacted parts carry no text.
                StreamedAssistantContent::Reasoning(r) => {
                    let mut text = String::new();
                    for rc in r.content {
                        if let ReasoningContent::Text { text: t, .. } = rc {
                            text.push_str(&t);
                        }
                    }
                    if !text.is_empty() {
                        streamed_thinking.push_str(&text);
                        on_delta(crate::ai::provider::AgentDelta::Thinking(text));
                    }
                }
                StreamedAssistantContent::Text(t) => {
                    streamed_text.push_str(&t.text);
                    on_delta(crate::ai::provider::AgentDelta::Text(t.text));
                }
                _ => {}
            }
        }

        let (mut final_text, mut thinking, tool_calls) =
            split_assistant_content(stream.choice.into_iter().collect());

        if (final_text.is_none() || final_text.as_ref().map(|t| t.is_empty()).unwrap_or(false))
            && !streamed_text.is_empty()
        {
            final_text = Some(streamed_text);
        }

        if (thinking.is_none() || thinking.as_ref().map(|t| t.is_empty()).unwrap_or(false))
            && !streamed_thinking.is_empty()
        {
            thinking = Some(streamed_thinking);
        }

        let usage = stream
            .response
            .as_ref()
            .map(|r| {
                let u = r.token_usage();
                TokenUsage {
                    prompt_tokens: u.input_tokens as u32,
                    completion_tokens: u.output_tokens as u32,
                    cached_prompt_tokens: u.cached_input_tokens as u32,
                    cached_write_prompt_tokens: u.cache_creation_input_tokens as u32,
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

    #[test]
    fn opencode_headers_carry_session_id_and_lucent_user_agent() {
        let p = RigProvider::new(
            AiProvider::OpenCode,
            "k".into(),
            None,
            Some("conv-abc-123".into()),
        );
        let headers = p.opencode_headers().expect("valid session id");
        assert_eq!(
            headers
                .get("x-opencode-session")
                .expect("session header present")
                .to_str()
                .unwrap(),
            "conv-abc-123"
        );
        let ua = headers
            .get(reqwest::header::USER_AGENT)
            .expect("user agent present")
            .to_str()
            .unwrap();
        assert!(
            ua.starts_with("lucent/"),
            "identifies as Lucent, not a generic SDK: {ua}"
        );
    }

    #[test]
    fn opencode_headers_empty_without_session_id() {
        for session in [None, Some("".to_string()), Some("   ".to_string())] {
            let p = RigProvider::new(AiProvider::OpenCode, "k".into(), None, session);
            let headers = p.opencode_headers().expect("no error on empty");
            assert!(
                headers.get("x-opencode-session").is_none(),
                "other providers and empty sessions stay header-free"
            );
            assert!(headers.get(reqwest::header::USER_AGENT).is_none());
        }
    }

    // ── Reasoning normalisation (OpenCode Go) ────────────────────────────

    fn sse_bytes(lines: &[&str]) -> bytes::Bytes {
        let mut s = String::new();
        for l in lines {
            s.push_str("data: ");
            s.push_str(l);
            s.push_str("\n\n");
        }
        s.push_str("data: [DONE]\n\n");
        bytes::Bytes::from(s)
    }

    /// Drives Go-shaped SSE through the REAL generic wrapper around rig-core's
    /// `MockStreamingClient` into rig-core's real SSE parser. Returns
    /// (reasoning deltas, text deltas).
    async fn drain_via_wrapper(sse: bytes::Bytes) -> (Vec<String>, Vec<String>) {
        use futures::StreamExt;
        let inner = rig_core::test_utils::MockStreamingClient { sse_bytes: sse };
        let client = ReasoningRewritingClient { inner };
        let req = rig_core::http_client::Request::builder()
            .method("POST")
            .uri("http://localhost/v1/chat/completions")
            .body(Vec::new())
            .unwrap();
        let mut stream =
            rig_core::providers::openai::completion::streaming::send_compatible_streaming_request(
                client, req,
            )
            .await
            .expect("mocked SSE parses");
        let (mut think, mut text) = (vec![], vec![]);
        while let Some(item) = stream.next().await {
            match item.expect("stream item ok") {
                StreamedAssistantContent::ReasoningDelta { reasoning, .. } => think.push(reasoning),
                StreamedAssistantContent::Text(t) => text.push(t.text),
                _ => {}
            }
        }
        (think, text)
    }

    #[tokio::test]
    async fn reasoning_content_delta_surfaces_as_reasoning_delta() {
        // Sanity: rig-core's own guarantee, passes with or without the wrapper.
        let (think, text) = drain_via_wrapper(sse_bytes(&[
            r#"{"id":"c","model":"m","choices":[{"delta":{"reasoning_content":"plan…","role":"assistant"},"finish_reason":null}],"usage":null}"#,
            r#"{"id":"c","model":"m","choices":[{"delta":{"content":"done"},"finish_reason":"stop"}]}"#,
        ]))
        .await;
        assert_eq!(think, vec!["plan…".to_string()]);
        assert_eq!(text, vec!["done".to_string()]);
    }

    /// THE regression test. Go-shaped delta (`reasoning` + `reasoning_details[]`,
    /// no `reasoning_content` — exact shape from the live step-5-preview-free
    /// capture) must surface as a ReasoningDelta once normalised.
    #[tokio::test]
    async fn go_reasoning_and_details_delta_surfaces_as_reasoning_delta() {
        let (think, _) = drain_via_wrapper(sse_bytes(&[
            r#"{"id":"gen-1","object":"chat.completion.chunk","created":1,"model":"step-5-preview-free","choices":[{"index":0,"finish_reason":null,"delta":{"role":"assistant","content":"","reasoning":"We","reasoning_details":[{"type":"reasoning.text","text":"We","format":"unknown","index":0}]}}]}"#,
            r#"{"id":"gen-1","object":"chat.completion.chunk","created":1,"model":"step-5-preview-free","choices":[{"index":0,"finish_reason":null,"delta":{"role":"assistant","content":"","reasoning":" need answer","reasoning_details":[{"type":"reasoning.text","text":" need answer","format":"unknown","index":0}]}}]}"#,
        ]))
        .await;
        assert_eq!(think, vec!["We".to_string(), " need answer".to_string()]);
    }

    #[test]
    fn normalise_fills_reasoning_content_from_reasoning_string() {
        let delta = serde_json::json!({"role": "assistant", "reasoning": "plan…"});
        let out = normalise_reasoning_field(delta);
        assert_eq!(
            out.get("reasoning_content").and_then(|v| v.as_str()),
            Some("plan…")
        );
    }

    #[test]
    fn normalise_joins_reasoning_details_text_parts() {
        let delta = serde_json::json!({
            "role": "assistant",
            "reasoning_details": [
                {"type": "reasoning.text", "text": "We", "format": "unknown", "index": 0},
                {"type": "reasoning.text", "text": " think", "format": "unknown", "index": 1}
            ]
        });
        let out = normalise_reasoning_field(delta);
        assert_eq!(
            out.get("reasoning_content").and_then(|v| v.as_str()),
            Some("We think")
        );
    }

    #[test]
    fn normalise_skips_entries_without_plain_text() {
        // Encrypted/redacted-style entries carry no `text` — nothing to mirror.
        let delta = serde_json::json!({
            "role": "assistant",
            "reasoning_details": [
                {"type": "reasoning.encrypted", "format": "unknown", "index": 0},
                {"type": "reasoning.redacted", "format": "unknown", "index": 1}
            ]
        });
        let out = normalise_reasoning_field(delta);
        assert!(
            out.get("reasoning_content").is_none(),
            "no plain text means no reasoning_content"
        );
    }

    #[test]
    fn normalise_prefers_existing_reasoning_content() {
        let delta = serde_json::json!({
            "reasoning": "shadowed",
            "reasoning_content": "original",
            "reasoning_details": [{"type": "reasoning.text", "text": "shadowed"}]
        });
        let out = normalise_reasoning_field(delta);
        assert_eq!(
            out.get("reasoning_content").and_then(|v| v.as_str()),
            Some("original")
        );
    }

    #[test]
    fn normalise_leaves_content_untouched() {
        let delta = serde_json::json!({"content": "done", "reasoning": "plan…"});
        let out = normalise_reasoning_field(delta);
        assert_eq!(out.get("content").and_then(|v| v.as_str()), Some("done"));
    }

    #[test]
    fn rewrite_passes_through_streams_without_data_lines() {
        let raw = bytes::Bytes::from_static(b": ping\n\n: keep-alive\n\n");
        let out = rewrite_sse_reasoning(raw.clone());
        assert_eq!(out, raw, "cheap no-`data:` fast path keeps bytes intact");
    }

    #[test]
    fn rewrite_leaves_done_usage_and_non_json_lines_intact() {
        let raw = "data: [DONE]\n\ndata: not-json{{\n\n: comment\n\n";
        let out = rewrite_sse_reasoning(bytes::Bytes::from(raw));
        assert_eq!(&out[..], raw.as_bytes());
    }

    #[tokio::test]
    async fn full_reasoning_block_is_forwarded_as_thinking() {
        // R2: a complete Reasoning block (not deltas) must reach on_delta live.
        use crate::ai::provider::{AgentDelta, LucentAgent};
        use rig_core::test_utils::{MockCompletionModel, MockStreamEvent};
        use std::sync::Mutex;

        let model = MockCompletionModel::from_stream_turns([vec![MockStreamEvent::reasoning(
            "full block",
        )]]);
        let agent = rig_core::agent::AgentBuilder::new(model)
            .preamble("p")
            .build();
        let rig_agent = RigAgent { inner: agent };
        let thinking = Mutex::new(vec![]);
        let prompt = make_msg(MessageRole::User, "hi", None);
        let resp = rig_agent
            .complete(prompt, vec![], &|d| {
                if let AgentDelta::Thinking(t) = d {
                    thinking.lock().unwrap().push(t);
                }
            })
            .await
            .expect("mocked agent turn completes");
        assert_eq!(*thinking.lock().unwrap(), vec!["full block".to_string()]);
        assert_eq!(resp.thinking, Some("full block".to_string()));
    }

    // ── Chunk-boundary regression tests (thinking-text corruption) ────────

    /// Go-shaped SSE fixture mirroring the live capture: `delta.reasoning`
    /// mirrored 1:1 in `reasoning_details[]`, plus non-ASCII (— U+2014,
    /// − U+2212), backticks and a code fence. Returns (body, expected joined
    /// reasoning).
    fn thinking_fixture() -> (bytes::Bytes, String) {
        let deltas = [
            "Let us check the DISTINCT rows — ",
            "daily−weekly `prev_close` and ```sql",
            "SELECT 1 — done.",
        ];
        let mut s = String::from(": gateway comment\n\n");
        for (i, d) in deltas.iter().enumerate() {
            let v = serde_json::json!({
                "id": "gen-1",
                "object": "chat.completion.chunk",
                "created": 1,
                "model": "step-5-preview-free",
                "choices": [{
                    "index": 0,
                    "finish_reason": null,
                    "delta": {
                        "role": "assistant",
                        "content": "",
                        "reasoning": d,
                        "reasoning_details": [
                            {"type": "reasoning.text", "text": d, "format": "unknown", "index": i}
                        ]
                    }
                }]
            });
            s.push_str("data: ");
            s.push_str(&serde_json::to_string(&v).unwrap());
            s.push_str("\n\n");
        }
        s.push_str("data: {\"id\":\"gen-1\",\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\n");
        s.push_str("data: [DONE]\n\n");
        (bytes::Bytes::from(s), deltas.join(""))
    }

    /// Drives byte chunks through the REAL production path
    /// (`ReasoningRewritingClient` → rig-core's SSE parser, which uses the
    /// real `eventsource_stream::Eventsource`) and returns (reasoning deltas,
    /// text deltas).
    async fn drain_chunked(chunks: Vec<bytes::Bytes>) -> (Vec<String>, Vec<String>) {
        use futures::StreamExt;
        let inner = rig_core::test_utils::SequencedStreamingHttpClient::new(
            chunks
                .into_iter()
                .map(Ok::<bytes::Bytes, rig_core::http_client::Error>)
                .collect(),
        );
        let client = ReasoningRewritingClient { inner };
        let req = rig_core::http_client::Request::builder()
            .method("POST")
            .uri("http://localhost/v1/chat/completions")
            .body(Vec::new())
            .unwrap();
        let mut stream =
            rig_core::providers::openai::completion::streaming::send_compatible_streaming_request(
                client, req,
            )
            .await
            .expect("mocked SSE parses");
        let (mut think, mut text) = (vec![], vec![]);
        while let Some(item) = stream.next().await {
            match item.expect("stream item ok") {
                StreamedAssistantContent::ReasoningDelta { reasoning, .. } => think.push(reasoning),
                StreamedAssistantContent::Text(t) => text.push(t.text),
                _ => {}
            }
        }
        (think, text)
    }

    /// Collects the raw output bytes of the rewrite wrapper for explicit
    /// chunks (byte-level assertions, no SSE parsing).
    async fn collect_rewritten(chunks: Vec<bytes::Bytes>) -> bytes::Bytes {
        use futures::StreamExt;
        let inner: rig_core::http_client::sse::BoxedStream = Box::pin(futures::stream::iter(
            chunks
                .into_iter()
                .map(Ok::<bytes::Bytes, rig_core::http_client::Error>),
        ));
        let mut out = Vec::new();
        let mut stream = rewrite_sse_stream(inner);
        while let Some(item) = stream.next().await {
            out.extend_from_slice(&item.expect("fixture has no error items"));
        }
        bytes::Bytes::from(out)
    }

    #[tokio::test]
    async fn rewrite_survives_every_byte_split() {
        let (body, expected) = thinking_fixture();
        assert!(!expected.is_empty());
        // Baseline sanity: the unsplit run must surface every delta.
        let (think, _) = drain_chunked(vec![body.clone()]).await;
        assert_eq!(
            think.join(""),
            expected,
            "unsplit baseline must be lossless"
        );
        for off in 0..=body.len() {
            let (think, _) = drain_chunked(vec![body.slice(0..off), body.slice(off..)]).await;
            let joined = think.join("");
            assert!(
                !joined.contains('\u{FFFD}'),
                "offset {off}: replacement char in reasoning"
            );
            assert_eq!(
                joined, expected,
                "offset {off}: chunk split must not lose reasoning"
            );
        }
    }

    #[tokio::test]
    async fn rewrite_normalises_a_data_line_split_across_chunks() {
        let (body, expected) = thinking_fixture();
        // Split halfway through the first `data:` JSON line (past the
        // leading comment line, so the boundary truly lands mid-JSON).
        let data_start = body
            .windows(6)
            .position(|w| w == b"data: ")
            .expect("fixture has data lines");
        let first_nl = data_start
            + body[data_start..]
                .iter()
                .position(|&b| b == b'\n')
                .expect("fixture has lines");
        let off = data_start + (first_nl - data_start) / 2;
        let (think, _) = drain_chunked(vec![body.slice(0..off), body.slice(off..)]).await;
        assert_eq!(
            think.join(""),
            expected,
            "a data line completed by the next chunk must still be normalised"
        );
    }

    #[tokio::test]
    async fn rewrite_flushes_trailing_partial_line_on_stream_end() {
        let raw = "data: [DONE]\n\ndata: {\"partial\": tru";
        let mid = 10;
        let out = collect_rewritten(vec![
            bytes::Bytes::copy_from_slice(&raw.as_bytes()[..mid]),
            bytes::Bytes::copy_from_slice(&raw.as_bytes()[mid..]),
        ])
        .await;
        assert_eq!(
            &out[..],
            raw.as_bytes(),
            "buffered tail must flush unchanged at stream end"
        );
    }

    #[tokio::test]
    async fn rewrite_is_byte_identity_without_reasoning() {
        let s = "data: {\"id\":\"c\",\"choices\":[{\"delta\":{\"content\":\"caf—e `x`\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"c\",\"usage\":{\"prompt_tokens\":3}}\n\n: keep-alive\n\ndata: [DONE]\n\n";
        let bytes = bytes::Bytes::copy_from_slice(s.as_bytes());
        // Every two-way split (including inside the multi-byte —) must round-trip.
        for off in 0..=bytes.len() {
            let out = collect_rewritten(vec![bytes.slice(0..off), bytes.slice(off..)]).await;
            assert_eq!(
                &out[..],
                &bytes[..],
                "offset {off}: no-reasoning streams stay byte-identical"
            );
        }
        // Plus a three-way split with odd boundaries.
        let n = bytes.len();
        let out = collect_rewritten(vec![
            bytes.slice(0..3),
            bytes.slice(3..n.saturating_sub(2)),
            bytes.slice(n.saturating_sub(2)..),
        ])
        .await;
        assert_eq!(&out[..], &bytes[..]);
    }

    #[tokio::test]
    async fn rewrite_handles_data_prefix_without_space() {
        let v = serde_json::json!({
            "id": "gen-1",
            "choices": [{
                "index": 0,
                "finish_reason": null,
                "delta": {"role": "assistant", "reasoning": "nospace…"}
            }]
        });
        let body = bytes::Bytes::from(format!(
            "data:{}\n\ndata: [DONE]\n\n",
            serde_json::to_string(&v).unwrap()
        ));
        let (think, _) = drain_chunked(vec![body]).await;
        assert_eq!(think, vec!["nospace…".to_string()]);
    }
}
