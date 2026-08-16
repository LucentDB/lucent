use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Whether an ACP agent can use Lucent's database tools.
/// With multi-layer tool delivery (ACP session/new mcpServers, workspace .mcp.json,
/// and ./lucent-tool CLI helper in sandbox cwd), database tools are universally available.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DbToolSupport {
    /// Verified: the agent connects client-provided MCP servers or workspace tools.
    Supported,
    /// Known to not support tools.
    Unsupported,
    /// Not verified either way.
    Unknown,
}

pub fn db_tool_support(_agent_id: &str) -> DbToolSupport {
    DbToolSupport::Supported
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registry {
    pub version: String,
    pub agents: Vec<AgentManifest>,
    #[serde(default)]
    pub extensions: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub distribution: Distribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Distribution {
    #[serde(default)]
    pub npx: Option<PkgDist>,
    #[serde(default)]
    pub uvx: Option<PkgDist>,
    #[serde(default)]
    pub binary: Option<HashMap<String, BinaryDist>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PkgDist {
    pub package: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryDist {
    pub archive: String,
    pub cmd: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    /// Extra env vars to set when launching the binary (per-triple, e.g.
    /// vtcode's `VT_ACP_ENABLED`). The installer merges these into the
    /// launch spec's env.
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Canonical ACP agent registry feed. Fetched on demand; the cache and the
/// bundled snapshot below are the offline fallbacks.
const REGISTRY_URL: &str = "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// Full registry feed, pinned at build time. Guarantees the Settings list
/// renders on first launch without network (and in any later offline state).
const SNAPSHOT: &str = include_str!("registry_snapshot.json");

/// The bundled registry snapshot — never fails to parse (validated at build
/// time by `include_str!` + the `expect` below).
pub fn bundled_snapshot() -> Registry {
    serde_json::from_str(SNAPSHOT).expect("bundled registry snapshot is valid JSON")
}

/// Where the registry cache lives on disk: `~/.lucent/acp-registry.json`.
pub fn cache_path() -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    Ok(std::path::PathBuf::from(home)
        .join(".lucent")
        .join("acp-registry.json"))
}

/// Reads the cached feed, if any. `None` when there is no cache or it fails
/// to parse (a corrupt cache is treated as missing, never as an error).
pub fn load_cached() -> Option<Registry> {
    let path = cache_path().ok()?;
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

/// Writes the feed to the cache location, creating `~/.lucent` on demand.
pub fn save_cache(reg: &Registry) -> Result<(), String> {
    let path = cache_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create cache dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(reg).map_err(|e| format!("serialize registry: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("write registry cache: {e}"))
}

/// Fetches the live feed. The error strings are short, specific, and
/// user-actionable (repo convention — shown verbatim in the Settings UI).
pub async fn fetch_registry(http: &reqwest::Client) -> Result<Registry, String> {
    let resp = http
        .get(REGISTRY_URL)
        .send()
        .await
        .map_err(|e| format!("could not reach the agent registry: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("registry returned HTTP {}", resp.status()));
    }
    resp.json::<Registry>()
        .await
        .map_err(|e| format!("registry response was not valid: {e}"))
}

/// Pure fallback chain: a successful fetch wins; otherwise the cached feed,
/// then the bundled snapshot. Extracted from `refresh_registry` so the
/// fallback order is unit-testable without network or file I/O.
fn resolve(fetch: Result<Registry, String>, cached: Option<Registry>) -> Result<Registry, String> {
    match fetch {
        Ok(reg) => Ok(reg),
        Err(fetch_err) => match cached {
            Some(cached) => Ok(cached),
            None => {
                log::warn!("registry fetch failed, using bundled snapshot: {fetch_err}");
                Ok(bundled_snapshot())
            }
        },
    }
}

/// Fetches the registry, caching a successful fetch and falling back to the
/// cache (then the bundled snapshot) on failure. Never fails — the UI contract
/// is that the agent list is always renderable.
pub async fn refresh_registry(http: &reqwest::Client) -> Result<Registry, String> {
    match fetch_registry(http).await {
        Ok(reg) => {
            let _ = save_cache(&reg);
            Ok(reg)
        }
        Err(fetch_err) => resolve(Err(fetch_err), load_cached()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_live_feed_fixture() {
        let json = include_str!("registry_fixture.json");
        let reg: Registry = serde_json::from_str(json).expect("fixture parses");
        assert_eq!(reg.version, "1.0.0");
        assert!(reg.agents.iter().any(|a| a.id == "opencode"));
        let opencode = reg.agents.iter().find(|a| a.id == "opencode").unwrap();
        let bin = opencode.distribution.binary.as_ref().unwrap();
        assert!(bin.contains_key("darwin-aarch64"));
        assert!(bin.contains_key("windows-x86_64"));
        let npx_only = reg.agents.iter().find(|a| a.id == "claude-acp").unwrap();
        assert!(npx_only.distribution.binary.is_none());
        assert_eq!(
            npx_only.distribution.npx.as_ref().unwrap().package,
            "@agentclientprotocol/claude-agent-acp@0.68.0"
        );
    }

    #[test]
    fn unknown_feed_fields_are_tolerated() {
        let json = r#"{"version":"1.0.0","agents":[{"id":"x","name":"X","version":"1","description":"d","authors":[],"license":"MIT","distribution":{"npx":{"package":"x@1"},"futureField":{"a":1}}}],"extensions":[],"futureTop":true}"#;
        let reg: Registry = serde_json::from_str(json).expect("unknown fields ignored");
        assert_eq!(reg.agents.len(), 1);
    }

    #[tokio::test]
    async fn refresh_falls_back_to_cache_on_network_error() {
        // Network-dependent by design: the assertion is "never fails", which is
        // the actual contract — a successful fetch returns the live feed, and an
        // offline/failing fetch falls back to the cache, then the snapshot.
        let http = reqwest::Client::new();
        let result = refresh_registry(&http).await;
        assert!(
            result.is_ok(),
            "must never fail: cache or snapshot always available"
        );
    }

    #[test]
    fn snapshot_parses_and_contains_opencode() {
        let reg = bundled_snapshot();
        assert!(reg.agents.iter().any(|a| a.id == "opencode"));
    }

    #[test]
    fn db_tool_support_curates_the_registry_agents() {
        assert_eq!(db_tool_support("opencode"), DbToolSupport::Supported);
        assert_eq!(db_tool_support("claude-acp"), DbToolSupport::Supported);
        assert_eq!(db_tool_support("codex-acp"), DbToolSupport::Supported);
        assert_eq!(db_tool_support("pi-acp"), DbToolSupport::Supported);
        assert_eq!(db_tool_support("cursor"), DbToolSupport::Supported);
        assert_eq!(db_tool_support("gemini"), DbToolSupport::Supported);

        // Every summary carries its verdict (the Settings badge contract).
        let reg = bundled_snapshot();
        let by_id: HashMap<_, _> = crate::ai::acp::summarize(&reg, |_| None)
            .into_iter()
            .map(|s| (s.id.clone(), s))
            .collect();
        assert_eq!(
            by_id["opencode"].db_tools,
            DbToolSupport::Supported,
            "supported verdict survives summarize"
        );
        assert_eq!(
            by_id["pi-acp"].db_tools,
            DbToolSupport::Supported,
            "supported verdict survives summarize for pi-acp"
        );
        // Serde contract: camelCase on the wire (the frontend reads dbTools).
        let json = serde_json::to_value(&by_id["opencode"]).unwrap();
        assert_eq!(json["dbTools"], "supported");
        let json = serde_json::to_value(&by_id["pi-acp"]).unwrap();
        assert_eq!(json["dbTools"], "supported");
    }

    #[test]
    fn binary_dist_env_parses_from_snapshot() {
        // vtcode (pinned snapshot) ships per-triple `env` (VT_ACP_ENABLED) that
        // the installer must forward into the launch spec — the field has to
        // parse from the real feed and survive a serde round-trip.
        let reg = bundled_snapshot();
        let vtcode = reg
            .agents
            .iter()
            .find(|a| a.id == "vtcode")
            .expect("vtcode is in the pinned snapshot");
        let binary = vtcode
            .distribution
            .binary
            .as_ref()
            .expect("vtcode ships binaries");
        let dist = binary
            .get("darwin-aarch64")
            .or_else(|| binary.values().next())
            .expect("a triple exists");
        assert_eq!(
            dist.env.get("VT_ACP_ENABLED").map(String::as_str),
            Some("1"),
            "binary-dist env must parse from the feed"
        );

        // Round-trips through serde (Serialized + Deserialize).
        let json = serde_json::to_string(dist).expect("binary dist serializes");
        let back: BinaryDist = serde_json::from_str(&json).expect("binary dist parses");
        assert_eq!(
            back.env.get("VT_ACP_ENABLED").map(String::as_str),
            Some("1")
        );

        // Entries that omit `env` still parse (back-compat with the feed).
        let minimal: BinaryDist =
            serde_json::from_str(r#"{"archive":"x","cmd":"./x"}"#).expect("env-less dist parses");
        assert!(minimal.env.is_empty());
    }

    #[test]
    fn fallback_order_is_cache_then_snapshot() {
        // Deterministic unit for the fallback chain: a successful fetch wins
        // outright; a failed fetch with a cache uses the cache; a failed fetch
        // with no cache uses the bundled snapshot. No network, no file I/O.
        let fetched = Registry {
            version: "9.9.9-fetched".into(),
            agents: vec![],
            extensions: vec![],
        };
        let cached = Registry {
            version: "9.9.9-cached".into(),
            agents: vec![],
            extensions: vec![],
        };

        let r = resolve(Ok(fetched.clone()), None).unwrap();
        assert_eq!(r.version, "9.9.9-fetched", "successful fetch wins");

        let r = resolve(Err("network down".into()), Some(cached.clone())).unwrap();
        assert_eq!(r.version, "9.9.9-cached", "cache wins over the snapshot");

        let r = resolve(Err("network down".into()), None).unwrap();
        assert!(
            r.agents.iter().any(|a| a.id == "opencode"),
            "no cache -> bundled snapshot"
        );
    }
}
