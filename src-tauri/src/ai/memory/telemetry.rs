use crate::ai::memory::observations::Origin;

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
