//! Agent process registry: one `AgentProcess` record per agent id, with the
//! resolved launch spec, a bounded stderr tail (for error surfacing) and the
//! spawn-timestamp log that feeds the crash-restart budget (C5).
//!
//! Phase C owns no live children — the connection task (C2) spawns them via
//! the `agent-client-protocol` crate's own transport, which installs a
//! process-group guard that kills the whole tree (`npx` wrappers included)
//! when the connection ends. This module is the bookkeeping half.

use crate::ai::acp::install::{read_installed, LaunchSpec};
use crate::ai::config::AcpAgentConfig;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// One agent's process record. `spawns` is the crash-restart budget log
/// (pruned/consulted by `AcpManager::allow_spawn`, C5); `stderr_tail` is
/// appended by the connection task's debug callback and surfaced in errors.
pub struct AgentProcess {
    pub agent_id: String,
    pub launch: LaunchSpec,
    pub stderr_tail: Arc<Mutex<String>>,
    pub spawns: Arc<Mutex<Vec<std::time::Instant>>>,
}

impl std::fmt::Debug for AgentProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentProcess")
            .field("agent_id", &self.agent_id)
            .field("launch", &self.launch)
            .finish_non_exhaustive()
    }
}

impl AgentProcess {
    /// Appends `line` to the bounded tail, keeping at most `cap` bytes.
    /// Drains from the front so the newest stderr lines survive.
    pub fn tail_push(tail: Arc<Mutex<String>>, line: &str, cap: usize) -> usize {
        let mut t = tail.lock().unwrap();
        t.push_str(line);
        if t.len() > cap {
            let cut = t.len() - cap;
            t.drain(..cut);
        }
        t.len()
    }

    /// Last ~2 KB of agent stderr, for user-facing error messages.
    pub fn stderr_snippet(&self) -> String {
        let t = self.stderr_tail.lock().unwrap();
        let start = t.len().saturating_sub(2000);
        t[start..].to_string()
    }
}

/// Process registry keyed by agent id. The map itself never holds a live
/// child in phase C; it is the single source of truth for launch specs so
/// every session on an agent shares one resolved configuration.
pub struct AcpManager {
    pub processes: Arc<Mutex<HashMap<String, Arc<AgentProcess>>>>,
}

impl AcpManager {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Resolves the launch spec: `config.command` override wins (split on
    /// whitespace — no quoting in v1; the Settings UI warns), else the
    /// installed agent's spec. Errors when the agent isn't installed — no
    /// fallback to `npx` on the fly: the registry install flow is the only
    /// way in.
    pub async fn ensure_process(
        &self,
        agent_id: &str,
        acp: &AcpAgentConfig,
    ) -> Result<Arc<AgentProcess>, String> {
        if let Some(p) = self.processes.lock().unwrap().get(agent_id).cloned() {
            return Ok(p);
        }
        let launch = match &acp.command {
            Some(cmd) => {
                let mut parts = cmd.split_whitespace();
                let c = parts
                    .next()
                    .ok_or("command override is empty")?
                    .to_string();
                let args: Vec<String> = parts.map(|s| s.to_string()).collect();
                LaunchSpec {
                    cmd: c,
                    args,
                    env: acp.env.clone(),
                }
            }
            None => read_installed(agent_id)?
                .ok_or_else(|| {
                    format!(
                        "Agent '{agent_id}' is not installed. Install it from Settings → Agents first."
                    )
                })?
                .launch,
        };
        // Merge config env over the manifest env (user overrides win).
        let mut env = launch.env.clone();
        env.extend(acp.env.clone());
        let proc = Arc::new(AgentProcess {
            agent_id: agent_id.to_string(),
            launch: LaunchSpec {
                cmd: launch.cmd,
                args: launch.args,
                env,
            },
            stderr_tail: Arc::new(Mutex::new(String::new())),
            spawns: Arc::new(Mutex::new(Vec::new())),
        });
        self.processes
            .lock()
            .unwrap()
            .insert(agent_id.to_string(), proc.clone());
        Ok(proc)
    }

    /// Kills every live agent process. Phase C has no live children (C2's
    /// connection task owns them and dies with its command channel); this is
    /// the shutdown hook phase D wires to app exit.
    pub async fn kill_all(&self) {
        // No-op in phase C — the connection tasks own the processes and are
        // dropped when their command senders go away.
    }

    /// Records a spawn for the process against the crash-restart budget:
    /// at most 2 spawns per 10 minutes per agent, then require user action.
    /// The window is sliding — entries older than 10 minutes are pruned on
    /// each call, so a healthy agent that crashes twice in an hour is never
    /// blocked. Phase D calls this before respawning a crashed process;
    /// phase C pins the primitive.
    pub fn allow_spawn(&self, process: &Arc<AgentProcess>) -> bool {
        const BUDGET_WINDOW: std::time::Duration = std::time::Duration::from_secs(10 * 60);
        const MAX_SPAWNS: usize = 2;
        let now = std::time::Instant::now();
        let mut spawns = process.spawns.lock().unwrap();
        spawns.retain(|t| now.duration_since(*t) < BUDGET_WINDOW);
        if spawns.len() >= MAX_SPAWNS {
            return false;
        }
        spawns.push(now);
        true
    }

    /// Charges a detected crash against the restart budget (spec §4.3: 2
    /// restarts per 10 min per agent, then require user action). `Ok(())`
    /// means a restart is allowed; `Err` carries the budget message with the
    /// agent's stderr tail so the user sees the block reason AND the crash
    /// evidence in one error. Called by the crash-recovery path (F2) when a
    /// finished connection task is reaped.
    pub fn record_crash(&self, process: &Arc<AgentProcess>) -> Result<(), String> {
        if self.allow_spawn(process) {
            Ok(())
        } else {
            Err(format!(
                "Agent '{}' crashed too many times — wait a few minutes or restart Lucent before trying again. Last lines of its stderr: {}",
                process.agent_id,
                process.stderr_snippet()
            ))
        }
    }
}

impl Default for AcpManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::acp::install::{install, uninstall};
    use crate::ai::acp::registry::AgentManifest;
    use std::collections::HashMap;

    fn acp_cfg(agent_id: &str, command: Option<&str>) -> AcpAgentConfig {
        AcpAgentConfig {
            agent_id: agent_id.to_string(),
            command: command.map(|s| s.to_string()),
            env: HashMap::new(),
            auto_deny_permissions: false,
        }
    }

    /// Installs a fake npx agent (no network — the npx path never executes)
    /// so `ensure_process` with `command: None` has an installed.json to read.
    async fn install_test_agent(id: &str) {
        let manifest: AgentManifest = serde_json::from_str(&format!(
            r#"{{
                "id": "{id}",
                "name": "Test Agent",
                "version": "1.2.3",
                "description": "manager test fixture",
                "license": "MIT",
                "distribution": {{
                    "npx": {{ "package": "@test/{id}@1.2.3", "args": ["--acp"], "env": {{ "MANIFEST_VAR": "from-manifest" }} }}
                }}
            }}"#
        ))
        .unwrap();
        let http = reqwest::Client::new();
        install(&manifest, &http).await.expect("test install ok");
    }

    #[tokio::test]
    async fn launch_spec_resolution_prefers_command_override() {
        let mgr = AcpManager::new();

        // command override wins, split on whitespace
        let proc = mgr
            .ensure_process("stub", &acp_cfg("stub", Some("opencode acp --custom")))
            .await
            .expect("override resolves");
        assert_eq!(proc.launch.cmd, "opencode");
        assert_eq!(proc.launch.args, vec!["acp", "--custom"]);

        // a second ensure returns the same record (no re-resolution)
        let again = mgr
            .ensure_process("stub", &acp_cfg("stub", Some("opencode acp --custom")))
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&proc, &again), "process record is cached");
    }

    #[tokio::test]
    async fn launch_spec_resolution_uses_installed_spec_without_override() {
        install_test_agent("stub-manager-test").await;
        let mgr = AcpManager::new();
        let proc = mgr
            .ensure_process("stub-manager-test", &acp_cfg("stub-manager-test", None))
            .await
            .expect("installed agent resolves");
        assert_eq!(proc.launch.cmd, "npx");
        assert!(proc.launch.args.contains(&"--acp".to_string()));
        assert_eq!(
            proc.launch.env.get("MANIFEST_VAR").map(|s| s.as_str()),
            Some("from-manifest"),
            "manifest env is carried into the launch spec"
        );
        uninstall("stub-manager-test").expect("cleanup ok");
    }

    #[tokio::test]
    async fn missing_install_errors_with_actionable_message() {
        let mgr = AcpManager::new();
        let err = mgr
            .ensure_process(
                "never-installed-agent",
                &acp_cfg("never-installed-agent", None),
            )
            .await
            .expect_err("no installed.json -> Err");
        assert!(
            err.contains("not installed") && err.contains("Settings"),
            "error points the user at the registry: {err}"
        );
    }

    #[tokio::test]
    async fn config_env_overrides_manifest_env() {
        install_test_agent("stub-manager-env").await;
        let mgr = AcpManager::new();
        let mut cfg = acp_cfg("stub-manager-env", None);
        cfg.env
            .insert("MANIFEST_VAR".to_string(), "user-wins".to_string());
        cfg.env.insert("USER_VAR".to_string(), "x".to_string());
        let proc = mgr.ensure_process("stub-manager-env", &cfg).await.unwrap();
        assert_eq!(
            proc.launch.env.get("MANIFEST_VAR").map(|s| s.as_str()),
            Some("user-wins"),
            "config env overrides the manifest env"
        );
        assert_eq!(
            proc.launch.env.get("USER_VAR").map(|s| s.as_str()),
            Some("x")
        );
        uninstall("stub-manager-env").expect("cleanup ok");
    }

    #[test]
    fn stderr_tail_is_bounded() {
        let tail = Arc::new(Mutex::new(String::new()));
        let len = AgentProcess::tail_push(tail.clone(), &"x".repeat(100_000), 64 * 1024);
        assert!(len <= 64 * 1024, "tail stays under the cap: {len}");
        assert_eq!(tail.lock().unwrap().len(), 64 * 1024);

        // later lines push older ones out
        let mut t = tail.lock().unwrap();
        assert!(t.starts_with('x'));
        *t = String::new();
        drop(t);
        AgentProcess::tail_push(tail.clone(), "first\n", 8);
        AgentProcess::tail_push(tail.clone(), "second\n", 8);
        let t = tail.lock().unwrap();
        assert!(t.ends_with("second\n"), "newest line survives: {t:?}");
        assert!(!t.starts_with("first"), "oldest line drained: {t:?}");
    }

    #[test]
    fn stderr_snippet_returns_last_2kb() {
        let proc = Arc::new(AgentProcess {
            agent_id: "stub".into(),
            launch: LaunchSpec {
                cmd: "stub".into(),
                args: vec![],
                env: HashMap::new(),
            },
            stderr_tail: Arc::new(Mutex::new("boom".repeat(10_000))),
            spawns: Arc::new(Mutex::new(Vec::new())),
        });
        let snippet = proc.stderr_snippet();
        assert!(snippet.len() <= 2000);
        assert!(snippet.ends_with("boom"), "newest bytes kept");
    }

    // ── C5: crash-restart budget ──

    fn budget_process() -> Arc<AgentProcess> {
        Arc::new(AgentProcess {
            agent_id: "stub".into(),
            launch: LaunchSpec {
                cmd: "stub".into(),
                args: vec![],
                env: HashMap::new(),
            },
            stderr_tail: Arc::new(Mutex::new(String::new())),
            spawns: Arc::new(Mutex::new(Vec::new())),
        })
    }

    #[test]
    fn spawn_budget_blocks_after_two_quick_crashes() {
        let mgr = AcpManager::new();
        let proc = budget_process();
        assert!(mgr.allow_spawn(&proc), "first spawn allowed");
        assert!(mgr.allow_spawn(&proc), "second spawn allowed (budget is 2)");
        assert!(
            !mgr.allow_spawn(&proc),
            "third spawn within 10 minutes is blocked"
        );
        assert!(
            !mgr.allow_spawn(&proc),
            "still blocked while the window is full"
        );
        assert_eq!(proc.spawns.lock().unwrap().len(), 2, "no extra records");
    }

    #[test]
    fn spawn_budget_resets_after_ten_minutes() {
        let mgr = AcpManager::new();
        let proc = budget_process();
        // Two crashes 11 minutes ago (injected directly — the window is a
        // sliding prune on the timestamp log).
        let old = std::time::Instant::now() - std::time::Duration::from_secs(11 * 60);
        proc.spawns.lock().unwrap().extend([old, old]);
        assert!(
            mgr.allow_spawn(&proc),
            "old crashes are pruned, a fresh spawn is allowed"
        );
        assert_eq!(proc.spawns.lock().unwrap().len(), 1, "stale entries pruned");
    }

    #[test]
    fn spawn_budget_is_per_process() {
        let mgr = AcpManager::new();
        let a = budget_process();
        let b = budget_process();
        assert!(mgr.allow_spawn(&a));
        assert!(mgr.allow_spawn(&a));
        assert!(!mgr.allow_spawn(&a));
        assert!(mgr.allow_spawn(&b), "another agent's budget is independent");
    }
}
