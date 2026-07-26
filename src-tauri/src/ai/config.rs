use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    OpenAI,
    Anthropic,
    Ollama,
}

impl std::fmt::Display for AiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiProvider::OpenAI => write!(f, "OpenAI"),
            AiProvider::Anthropic => write!(f, "Anthropic"),
            AiProvider::Ollama => write!(f, "Ollama"),
        }
    }
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
    pub send_results_to_ai: bool,
    pub enable_blast_radius_check: bool,
    #[serde(default = "default_schema_cache_ttl")]
    pub schema_cache_ttl_secs: u64,
    #[serde(default = "default_semantic_index")]
    pub enable_semantic_index: bool,
    /// Per-query statement_timeout for AI read-only queries, seconds. 0 disables.
    #[serde(default = "default_ai_query_timeout_secs")]
    pub ai_query_timeout_secs: u64,
}

fn default_schema_cache_ttl() -> u64 {
    3600
}
fn default_semantic_index() -> bool {
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
            send_results_to_ai: true,
            enable_blast_radius_check: true,
            schema_cache_ttl_secs: 3600,
            enable_semantic_index: true,
            ai_query_timeout_secs: 60,
        }
    }
}

pub const KEYCHAIN_SERVICE: &str = "lucent-ai";

pub fn keychain_account(provider: &AiProvider) -> &'static str {
    match provider {
        AiProvider::OpenAI => "openai-api-key",
        AiProvider::Anthropic => "anthropic-api-key",
        AiProvider::Ollama => "ollama-api-key",
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
}
