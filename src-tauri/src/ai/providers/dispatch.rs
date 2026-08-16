use crate::ai::config::AiProvider;
use rig_core::client::ModelListingClient;
use rig_core::model::{Model, ModelListingError};
use rig_core::providers::{anthropic, deepseek, gemini, mistral, openai, openrouter};
use serde::Serialize;

/// Base URL for OpenAI-compatible-shape providers at a non-default host.
/// `None` for `Custom` (no default — the user must supply one) and for every
/// native provider (they use their own client's built-in base URL).
pub fn default_base_url(provider: &AiProvider) -> Option<&'static str> {
    match provider {
        AiProvider::Groq => Some("https://api.groq.com/openai/v1"),
        AiProvider::XAI => Some("https://api.x.ai/v1"),
        AiProvider::Ollama => Some("http://localhost:11434/v1"),
        AiProvider::OpenCode => Some("https://opencode.ai/zen/go/v1"),
        // ACP agents own their transport; the provider has no base URL.
        AiProvider::Acp => None,
        _ => None,
    }
}

/// Resolves the base URL to actually use for an OpenAI-compatible-shape
/// provider: an explicit, non-blank `endpoint` always wins; otherwise fall
/// back to `default_base_url`. `Custom` has no default, so a missing
/// endpoint is an error here rather than silently falling through.
pub fn resolve_base_url(
    provider: &AiProvider,
    endpoint: &Option<String>,
) -> Result<String, String> {
    if let Some(url) = endpoint {
        if !url.trim().is_empty() {
            return Ok(url.trim().to_string());
        }
    }
    default_base_url(provider)
        .map(|s| s.to_string())
        .ok_or_else(|| format!("{provider} requires an endpoint URL"))
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelSummary {
    pub id: String,
    pub display_name: String,
}

impl From<Model> for ModelSummary {
    fn from(m: Model) -> Self {
        let display_name = m.name.clone().unwrap_or_else(|| m.id.clone());
        Self {
            id: m.id,
            display_name,
        }
    }
}

/// Truncates a detail message to a bounded single-line string safe to show
/// in the UI. Newlines and whitespace runs are collapsed to single spaces; if
/// the result exceeds the limit it is cut with a trailing ellipsis.
fn bounded_detail(s: &str) -> String {
    const MAX: usize = 120;
    let collapsed: String = s
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    let words: Vec<&str> = collapsed.split(' ').filter(|w| !w.is_empty()).collect();
    let mut out = String::with_capacity(MAX + 4);
    for w in words {
        let needed = if out.is_empty() { w.len() } else { 1 + w.len() };
        if out.len() + needed > MAX {
            out.push('…');
            return out;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

/// Maps a rig-core model-listing failure to a short, specific string safe to
/// show directly in the Settings UI — never the raw Rust `Debug` output.
pub fn describe_listing_error(provider: &AiProvider, err: &ModelListingError) -> String {
    match err {
        ModelListingError::AuthError { .. } => {
            format!("That API key was rejected by {provider}.")
        }
        ModelListingError::ApiError {
            status_code,
            message,
        } => match status_code {
            401 | 403 => format!("That API key was rejected by {provider}."),
            404 => {
                "This endpoint doesn't return a model list — type the model name directly below."
                    .to_string()
            }
            _ => format!(
                "Couldn't read the model list from {provider}: {}",
                bounded_detail(message)
            ),
        },
        ModelListingError::RequestError { .. } => format!("Could not reach {provider}."),
        ModelListingError::RateLimitError { .. } => {
            format!("{provider} rate-limited this request — wait a moment and try again.")
        }
        ModelListingError::ServiceUnavailable { .. } => {
            format!("{provider} is temporarily unavailable — try again shortly.")
        }
        ModelListingError::ParseError { message } | ModelListingError::UnknownError { message } => {
            format!(
                "Couldn't read the model list from {provider}: {}",
                bounded_detail(message)
            )
        }
    }
}

pub async fn list_models_for(
    provider: &AiProvider,
    api_key: &str,
    endpoint: &Option<String>,
) -> Result<Vec<ModelSummary>, String> {
    let models = match provider {
        AiProvider::OpenAI => {
            list_via(openai::Client::builder().api_key(api_key).build(), provider).await?
        }
        AiProvider::Anthropic => {
            list_via(
                anthropic::Client::builder().api_key(api_key).build(),
                provider,
            )
            .await?
        }
        AiProvider::Gemini => {
            list_via(gemini::Client::builder().api_key(api_key).build(), provider).await?
        }
        AiProvider::OpenRouter => {
            list_via(
                openrouter::Client::builder().api_key(api_key).build(),
                provider,
            )
            .await?
        }
        AiProvider::Mistral => {
            list_via(
                mistral::Client::builder().api_key(api_key).build(),
                provider,
            )
            .await?
        }
        AiProvider::DeepSeek => {
            list_via(
                deepseek::Client::builder().api_key(api_key).build(),
                provider,
            )
            .await?
        }
        AiProvider::Groq
        | AiProvider::XAI
        | AiProvider::Ollama
        | AiProvider::Custom
        | AiProvider::OpenCode => {
            let base_url = resolve_base_url(provider, endpoint)?;
            list_via(
                openai::Client::builder()
                    .api_key(api_key)
                    .base_url(&base_url)
                    .build(),
                provider,
            )
            .await?
        }
        // The agent owns model choice; there is no model list to enumerate.
        // (The frontend hides the model picker for ACP anyway.)
        AiProvider::Acp => return Err("ACP agents don't expose model lists".into()),
    };
    Ok(models.into_iter().map(ModelSummary::from).collect())
}

async fn list_via<C>(
    build_result: rig_core::http_client::Result<C>,
    provider: &AiProvider,
) -> Result<Vec<Model>, String>
where
    C: ModelListingClient,
{
    let client =
        build_result.map_err(|e| format!("Couldn't configure the {provider} client: {e}"))?;
    let list = client
        .list_models()
        .await
        .map_err(|e| describe_listing_error(provider, &e))?;
    Ok(list.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::config::AiProvider;
    use rig_core::model::{Model, ModelListingError};

    #[test]
    fn default_base_url_covers_openai_compatible_providers() {
        assert_eq!(
            default_base_url(&AiProvider::Groq),
            Some("https://api.groq.com/openai/v1")
        );
        assert_eq!(
            default_base_url(&AiProvider::XAI),
            Some("https://api.x.ai/v1")
        );
        assert_eq!(
            default_base_url(&AiProvider::Ollama),
            Some("http://localhost:11434/v1")
        );
        assert_eq!(
            default_base_url(&AiProvider::OpenCode),
            Some("https://opencode.ai/zen/go/v1")
        );
    }

    #[test]
    fn default_base_url_is_none_for_custom_and_native_providers() {
        assert_eq!(default_base_url(&AiProvider::Custom), None);
        assert_eq!(default_base_url(&AiProvider::OpenAI), None);
        assert_eq!(default_base_url(&AiProvider::Anthropic), None);
    }

    #[test]
    fn resolve_base_url_prefers_explicit_endpoint() {
        let url = resolve_base_url(
            &AiProvider::Ollama,
            &Some("http://192.168.1.5:11434".into()),
        )
        .unwrap();
        assert_eq!(url, "http://192.168.1.5:11434");
    }

    #[test]
    fn resolve_base_url_falls_back_to_default() {
        let url = resolve_base_url(&AiProvider::Groq, &None).unwrap();
        assert_eq!(url, "https://api.groq.com/openai/v1");
    }

    #[test]
    fn resolve_base_url_errors_for_custom_with_no_endpoint() {
        assert!(resolve_base_url(&AiProvider::Custom, &None).is_err());
        assert!(resolve_base_url(&AiProvider::Custom, &Some("  ".into())).is_err());
    }

    #[test]
    fn model_summary_falls_back_to_id_when_no_name() {
        let m = Model::from_id("gpt-4o");
        let summary: ModelSummary = m.into();
        assert_eq!(summary.id, "gpt-4o");
        assert_eq!(summary.display_name, "gpt-4o");
    }

    #[test]
    fn model_summary_uses_name_when_present() {
        let m = Model::new("claude-sonnet-5", "Claude Sonnet 5");
        let summary: ModelSummary = m.into();
        assert_eq!(summary.id, "claude-sonnet-5");
        assert_eq!(summary.display_name, "Claude Sonnet 5");
    }

    #[test]
    fn describe_listing_error_maps_auth_failures() {
        let err = ModelListingError::AuthError {
            message: "invalid key".into(),
        };
        let msg = describe_listing_error(&AiProvider::Anthropic, &err);
        assert_eq!(msg, "That API key was rejected by Anthropic.");
    }

    #[test]
    fn describe_listing_error_maps_401_and_403_api_errors() {
        for code in [401u16, 403u16] {
            let err = ModelListingError::ApiError {
                status_code: code,
                message: "nope".into(),
            };
            assert_eq!(
                describe_listing_error(&AiProvider::OpenAI, &err),
                "That API key was rejected by OpenAI."
            );
        }
    }

    #[test]
    fn describe_listing_error_maps_404_to_manual_entry_hint() {
        let err = ModelListingError::ApiError {
            status_code: 404,
            message: "not found".into(),
        };
        assert_eq!(
            describe_listing_error(&AiProvider::Custom, &err),
            "This endpoint doesn't return a model list — type the model name directly below."
        );
    }

    #[test]
    fn describe_listing_error_maps_network_failure() {
        let err = ModelListingError::RequestError {
            message: "connection refused".into(),
        };
        assert_eq!(
            describe_listing_error(&AiProvider::Ollama, &err),
            "Could not reach Ollama."
        );
    }

    #[test]
    fn describe_listing_error_maps_rate_limit_and_unavailable() {
        let rate_limited = ModelListingError::RateLimitError {
            message: "slow down".into(),
        };
        assert_eq!(
            describe_listing_error(&AiProvider::Groq, &rate_limited),
            "Groq rate-limited this request — wait a moment and try again."
        );

        let unavailable = ModelListingError::ServiceUnavailable {
            message: "down for maintenance".into(),
        };
        assert_eq!(
            describe_listing_error(&AiProvider::Gemini, &unavailable),
            "Gemini is temporarily unavailable — try again shortly."
        );
    }

    #[test]
    fn describe_listing_error_falls_back_to_detail_for_parse_and_unknown_errors() {
        let parse_err = ModelListingError::ParseError {
            message: "unexpected shape".into(),
        };
        assert_eq!(
            describe_listing_error(&AiProvider::Mistral, &parse_err),
            "Couldn't read the model list from Mistral: unexpected shape"
        );
    }

    #[test]
    fn bounded_detail_truncates_long_multiline_messages() {
        let long = "error\n".repeat(200);
        let result = bounded_detail(&long);
        assert!(
            result.len() <= 123,
            "expected <= 123 chars, got {}: {result:?}",
            result.len()
        );
        assert!(result.ends_with('…'), "expected ellipsis, got {result:?}");
        assert!(
            !result.contains('\n'),
            "expected single-line, got {result:?}"
        );
    }

    #[test]
    fn bounded_detail_leaves_short_messages_untouched() {
        let short = "That API key was rejected by OpenAI.";
        assert_eq!(bounded_detail(short), short);
    }

    #[tokio::test]
    async fn list_models_for_custom_with_no_endpoint_errors_without_a_network_call() {
        let result = list_models_for(&AiProvider::Custom, "some-key", &None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires an endpoint URL"));
    }
}
