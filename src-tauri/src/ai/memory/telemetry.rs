use crate::ai::memory::observations::Origin;
use crate::ai::memory::Observation;

/// Which agent runtime a completed turn ran on. Chat turns use `Rig` or `Acp`
/// (both flow through `run_agent_turn`; `Rig` is the default), notebook AI
/// cells use `Notebook`. Kept so each detection site can record where a signal
/// was observed without threading a stringly-typed tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnRuntime {
    Rig,
    Acp,
    Notebook,
}

/// One SQL statement executed during a turn, with the statement the agent
/// proposed (when one exists) so an editor diff can be detected later.
#[derive(Debug, Clone)]
pub struct ExecutedSql {
    pub sql: String,
    pub proposed_sql: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

/// Compact per-tool-call outcome summary for a completed turn.
#[derive(Debug, Clone)]
pub struct ToolSummary {
    pub name: String,
    pub ok: bool,
}

/// Everything the observer needs about one completed turn. `user_text` and
/// `assistant_text` are optional because not every completion point has both
/// (the chat seam knows the user text, the sink does not).
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub connection_key: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub runtime: TurnRuntime,
    pub user_text: Option<String>,
    pub assistant_text: Option<String>,
    pub executed_sql: Vec<ExecutedSql>,
    pub tool_calls: Vec<ToolSummary>,
}

/// Record the deterministic observations a completed turn produced. Runs the
/// explicit-request detector over the user's text and persists a matching
/// `Observation`; errors are intentionally swallowed so observation capture can
/// never fail a turn. Callers invoke this detached, after the final token.
pub async fn record_turn_observations(state: &crate::AppState, turn: TurnOutcome) {
    if !state
        .ai_config
        .read()
        .await
        .is_memory_enabled(&turn.connection_key)
    {
        return;
    }
    if let Some(ref text) = turn.user_text {
        if let Some(draft) = detect_explicit_request(text) {
            let obs = Observation::new(
                turn.connection_key.clone(),
                Some(turn.conversation_id.clone()),
                Some(turn.turn_id.clone()),
                draft.kind,
                draft.origin,
                draft.signal,
                draft.signal_strength,
                draft.payload_json,
            );
            let _ = state.memory_manager.record_observation(obs).await;
        }
    }
}

/// A candidate observation produced by a deterministic detector, before it is
/// assigned an id / dedup key and persisted by `MemoryManager::record_observation`.
pub struct ObservationDraft {
    pub kind: String,
    pub origin: Origin,
    pub signal: String,
    pub signal_strength: f32,
    pub payload_json: String,
}

pub fn detect_explicit_request(user_text: &str) -> Option<ObservationDraft> {
    let lower = user_text.to_lowercase();
    let triggers = [
        "remember that",
        "always use",
        "note that",
        "from now on",
        "remember:",
    ];
    for t in triggers {
        if lower.contains(t) {
            return Some(ObservationDraft {
                kind: "request".into(),
                origin: Origin::Owner,
                signal: "explicit_request".into(),
                signal_strength: 1.0,
                payload_json: serde_json::json!({
                    "raw_text": user_text,
                    "trigger": t
                })
                .to_string(),
            });
        }
    }
    None
}

pub fn detect_editor_diff(proposed_sql: &str, executed_sql: &str) -> Option<ObservationDraft> {
    if proposed_sql.trim() == executed_sql.trim() {
        return None;
    }
    use sqlparser::dialect::GenericDialect;
    use sqlparser::parser::Parser;
    let dialect = GenericDialect {};
    let ast_p = Parser::parse_sql(&dialect, proposed_sql).ok()?;
    let ast_e = Parser::parse_sql(&dialect, executed_sql).ok()?;

    // If executed query added clauses not present in proposed query
    let p_str = format!("{:?}", ast_p);
    let e_str = format!("{:?}", ast_e);
    if p_str != e_str {
        return Some(ObservationDraft {
            kind: "diff".into(),
            origin: Origin::Agent,
            signal: "editor_diff".into(),
            signal_strength: 0.8,
            payload_json: serde_json::json!({
                "proposed_sql": proposed_sql,
                "executed_sql": executed_sql
            })
            .to_string(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_explicit_request_signal() {
        let msg = "Remember that orders always requires deleted_at IS NULL";
        let signal = detect_explicit_request(msg);
        assert!(signal.is_some());
        let s = signal.unwrap();
        assert_eq!(s.signal, "explicit_request");
        assert_eq!(s.signal_strength, 1.0);
        assert_eq!(s.origin, Origin::Owner);
    }

    /// R28: notebook AI cells run through the same observer as chat turns, so a
    /// notebook turn whose user text is an explicit request must record the
    /// same `explicit_request` / `Owner` observation the rig path does.
    #[tokio::test]
    async fn test_notebook_runtime_records_explicit_request_like_rig() {
        let mut state = crate::AppState::new();
        state.memory_manager =
            std::sync::Arc::new(crate::ai::memory::MemoryManager::open_in_memory().unwrap());

        let turn = TurnOutcome {
            connection_key: "conn-nb".into(),
            conversation_id: "conv-nb".into(),
            turn_id: "turn-nb".into(),
            runtime: TurnRuntime::Notebook,
            user_text: Some("Remember that customers uses uuid string ids".into()),
            assistant_text: None,
            executed_sql: vec![],
            tool_calls: vec![],
        };
        record_turn_observations(&state, turn).await;

        let obs = state
            .memory_manager
            .list_observations("conn-nb", "open")
            .await
            .unwrap();
        assert_eq!(obs.len(), 1);
        assert_eq!(obs[0].signal, "explicit_request");
        assert_eq!(obs[0].origin, Origin::Owner);
    }

    #[test]
    fn test_detect_editor_diff_signal() {
        let proposed = "SELECT * FROM orders WHERE status = 'active'";
        let executed = "SELECT * FROM orders WHERE status = 'active' AND deleted_at IS NULL";
        let diff = detect_editor_diff(proposed, executed);
        assert!(diff.is_some());
        let s = diff.unwrap();
        assert_eq!(s.signal, "editor_diff");
        assert!(s.payload_json.contains("deleted_at IS NULL"));
    }
}
