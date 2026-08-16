use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    OpenAI,
    Anthropic,
    Gemini,
    OpenRouter,
    Mistral,
    DeepSeek,
    Groq,
    XAI,
    Ollama,
    Custom,
    OpenCode,
    Acp,
}

impl std::fmt::Display for AiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiProvider::OpenAI => write!(f, "OpenAI"),
            AiProvider::Anthropic => write!(f, "Anthropic"),
            AiProvider::Gemini => write!(f, "Gemini"),
            AiProvider::OpenRouter => write!(f, "OpenRouter"),
            AiProvider::Mistral => write!(f, "Mistral"),
            AiProvider::DeepSeek => write!(f, "DeepSeek"),
            AiProvider::Groq => write!(f, "Groq"),
            AiProvider::XAI => write!(f, "xAI"),
            AiProvider::Ollama => write!(f, "Ollama"),
            AiProvider::Custom => write!(f, "Custom"),
            AiProvider::OpenCode => write!(f, "OpenCode"),
            AiProvider::Acp => write!(f, "ACP Agent"),
        }
    }
}

/// ACP provider selection: an installed registry agent driven over the
/// Agent Client Protocol (spec §4.7/D8). The agent owns model choice, so
/// `model` / `max_tokens` / `endpoint` are ignored on this path.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpAgentConfig {
    /// Registry id, e.g. "opencode" | "claude-acp".
    pub agent_id: String,
    /// Power-user override (full command line). Split on whitespace — no
    /// quoting in v1; the Settings UI warns about it.
    #[serde(default)]
    pub command: Option<String>,
    /// Extra env vars, layered over the manifest's env.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// When true, agent permission requests are auto-rejected without a dialog.
    #[serde(default)]
    pub auto_deny_permissions: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConfig {
    pub provider: AiProvider,
    pub endpoint: Option<String>,
    pub model: String,
    pub max_tokens: u32,
    pub max_turns: u32,
    pub row_limit: u32,
    pub enable_blast_radius_check: bool,
    #[serde(default = "default_schema_cache_ttl")]
    pub schema_cache_ttl_secs: u64,
    #[serde(default = "default_semantic_index")]
    pub enable_semantic_index: bool,
    /// Background value sampling for semantic hints. Reads at most 1,000 rows
    /// per column and stores sample values in the local cache.
    #[serde(default = "default_sample_column_values")]
    pub sample_column_values: bool,
    /// Per-query statement_timeout for AI read-only queries, seconds. 0 disables.
    #[serde(default = "default_ai_query_timeout_secs")]
    pub ai_query_timeout_secs: u64,
    /// Per-provider "last model picked in Settings" — a pure UX convenience
    /// for re-populating the model picker when the provider dropdown changes.
    /// NOT consulted at agent-run time; `provider` + `model` above remain the
    /// sole source of truth for what the agent actually calls.
    #[serde(default)]
    pub provider_models: HashMap<AiProvider, String>,
    /// ACP provider selection. `None` keeps the rig/provider-key path.
    #[serde(default)]
    pub acp: Option<AcpAgentConfig>,
}

fn default_schema_cache_ttl() -> u64 {
    3600
}
fn default_semantic_index() -> bool {
    true
}
fn default_sample_column_values() -> bool {
    true
}
fn default_ai_query_timeout_secs() -> u64 {
    60
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            provider: AiProvider::OpenAI,
            endpoint: None,
            model: "gpt-4o".into(),
            max_tokens: 4096,
            max_turns: 50,
            row_limit: 500,
            enable_blast_radius_check: true,
            schema_cache_ttl_secs: 3600,
            enable_semantic_index: true,
            sample_column_values: true,
            ai_query_timeout_secs: 60,
            provider_models: HashMap::new(),
            acp: None,
        }
    }
}

pub const KEYCHAIN_SERVICE: &str = "lucent-ai";

pub fn keychain_account(provider: &AiProvider) -> &'static str {
    match provider {
        AiProvider::OpenAI => "openai-api-key",
        AiProvider::Anthropic => "anthropic-api-key",
        AiProvider::Gemini => "gemini-api-key",
        AiProvider::OpenRouter => "openrouter-api-key",
        AiProvider::Mistral => "mistral-api-key",
        AiProvider::DeepSeek => "deepseek-api-key",
        AiProvider::Groq => "groq-api-key",
        AiProvider::XAI => "xai-api-key",
        AiProvider::Ollama => "ollama-api-key",
        AiProvider::Custom => "custom-api-key",
        AiProvider::OpenCode => "opencode-api-key",
        // Placeholder — the loader is never called on the ACP path (no key
        // exists to fetch; the agent owns its auth).
        AiProvider::Acp => "acp-agent",
    }
}

/// Persist AiConfig to a JSON file at ~/.lucent/ai-config.json
pub fn save_config_to_disk(config: &AiConfig) -> Result<(), String> {
    let path = config_file_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create config dir: {e}"))?;
    }
    let json =
        serde_json::to_string_pretty(config).map_err(|e| format!("serialize config: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("write config: {e}"))?;
    log::info!("AI config saved to {path:?}");
    Ok(())
}

/// Load AiConfig from ~/.lucent/ai-config.json, or return default if missing.
pub fn load_config_from_disk() -> AiConfig {
    let path = match config_file_path() {
        Ok(p) => p,
        Err(_) => return AiConfig::default(),
    };
    if !path.exists() {
        log::debug!("No AI config file at {path:?}, using defaults");
        return AiConfig::default();
    }
    let json = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Failed to read AI config at {path:?}: {e}, using defaults");
            return AiConfig::default();
        }
    };
    let config = serde_json::from_str(&json).unwrap_or_else(|e| {
        log::warn!("Failed to parse AI config: {e}, using defaults");
        AiConfig::default()
    });
    log::debug!(
        "AI config loaded from {path:?}: {}",
        serde_json::to_string(&config).unwrap_or_default()
    );
    config
}

fn config_file_path() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    Ok(std::path::PathBuf::from(home)
        .join(".lucent")
        .join("ai-config.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_query_timeout_defaults_to_60s_and_survives_old_configs() {
        assert_eq!(AiConfig::default().ai_query_timeout_secs, 60);
        let old_json = r#"{
            "provider": "openai", "endpoint": null, "model": "gpt-4o",
            "maxTokens": 4096, "maxTurns": 50, "rowLimit": 500,
            "sendResultsToAi": true, "enableBlastRadiusCheck": true
        }"#;
        let cfg: AiConfig = serde_json::from_str(old_json).expect("old config still parses");
        assert_eq!(
            cfg.ai_query_timeout_secs, 60,
            "missing field falls back to default"
        );
    }

    #[test]
    fn schema_cache_ttl_defaults_to_an_hour() {
        assert_eq!(
            AiConfig::default().schema_cache_ttl_secs,
            3600,
            "a 5-minute TTL expired mid-session and degraded the system prompt \
             to 'context not loaded' — schema metadata is stable, cache it long"
        );
    }

    #[test]
    fn keychain_account_covers_every_provider() {
        let expected = [
            (AiProvider::OpenAI, "openai-api-key"),
            (AiProvider::Anthropic, "anthropic-api-key"),
            (AiProvider::Gemini, "gemini-api-key"),
            (AiProvider::OpenRouter, "openrouter-api-key"),
            (AiProvider::Mistral, "mistral-api-key"),
            (AiProvider::DeepSeek, "deepseek-api-key"),
            (AiProvider::Groq, "groq-api-key"),
            (AiProvider::XAI, "xai-api-key"),
            (AiProvider::Ollama, "ollama-api-key"),
            (AiProvider::Custom, "custom-api-key"),
            (AiProvider::OpenCode, "opencode-api-key"),
            (AiProvider::Acp, "acp-agent"),
        ];
        for (provider, account) in expected {
            assert_eq!(keychain_account(&provider), account);
        }
    }

    #[test]
    fn acp_config_survives_old_configs() {
        let old_json = r#"{"provider":"openai","endpoint":null,"model":"gpt-4o","maxTokens":4096,"maxTurns":50,"rowLimit":500,"enableBlastRadiusCheck":true}"#;
        let cfg: AiConfig = serde_json::from_str(old_json).expect("old config still parses");
        assert!(cfg.acp.is_none(), "missing acp field defaults to None");
    }

    #[test]
    fn acp_config_round_trips() {
        let mut cfg = AiConfig::default();
        cfg.provider = AiProvider::Acp;
        cfg.acp = Some(AcpAgentConfig {
            agent_id: "opencode".into(),
            command: None,
            env: Default::default(),
            auto_deny_permissions: false,
        });
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: AiConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.acp.unwrap().agent_id, "opencode");
    }

    #[test]
    fn keychain_account_has_acp_arm() {
        // Loader must never be called for Acp, but the match must compile and return a marker.
        assert_eq!(keychain_account(&AiProvider::Acp), "acp-agent"); // placeholder, never consulted
    }

    #[test]
    fn provider_serializes_lowercase() {
        let cases = [
            (AiProvider::OpenAI, "\"openai\""),
            (AiProvider::XAI, "\"xai\""),
            (AiProvider::OpenRouter, "\"openrouter\""),
            (AiProvider::DeepSeek, "\"deepseek\""),
            (AiProvider::Custom, "\"custom\""),
            (AiProvider::OpenCode, "\"opencode\""),
            (AiProvider::Acp, "\"acp\""),
        ];
        for (provider, expected_json) in cases {
            assert_eq!(serde_json::to_string(&provider).unwrap(), expected_json);
        }
    }

    #[test]
    fn provider_is_hashable_map_key() {
        use std::collections::HashMap;
        let mut map: HashMap<AiProvider, String> = HashMap::new();
        map.insert(AiProvider::Groq, "llama-3.3-70b-versatile".to_string());
        assert_eq!(
            map.get(&AiProvider::Groq),
            Some(&"llama-3.3-70b-versatile".to_string())
        );
    }

    #[test]
    fn provider_models_defaults_to_empty_and_survives_old_configs() {
        assert!(AiConfig::default().provider_models.is_empty());

        // Fixture shaped like a config saved before this field existed.
        let old_json = r#"{
            "provider": "openai", "endpoint": null, "model": "gpt-4o",
            "maxTokens": 4096, "maxTurns": 50, "rowLimit": 500,
            "sendResultsToAi": true, "enableBlastRadiusCheck": true
        }"#;
        let cfg: AiConfig = serde_json::from_str(old_json).expect("old config still parses");
        assert!(
            cfg.provider_models.is_empty(),
            "missing field falls back to an empty map, not an error"
        );
    }

    #[test]
    fn provider_models_round_trips() {
        let mut cfg = AiConfig::default();
        cfg.provider_models
            .insert(AiProvider::Anthropic, "claude-sonnet-5".to_string());
        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: AiConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            parsed.provider_models.get(&AiProvider::Anthropic),
            Some(&"claude-sonnet-5".to_string())
        );
    }
}
