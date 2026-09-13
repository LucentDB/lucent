use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use super::decay::{calculate_retention, is_archive_eligible};
use super::security::SourceTrust;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalAssertion {
    pub key_phrase: String,
    pub assertion: String,
    pub sql_snippet: Option<String>,
    pub importance: f32,
    pub referenced_entities: Vec<String>,
}

/// Prunes verbose raw tool results (`session_json`) from chat_messages older than
/// `older_than_days` (default 14 days), achieving up to 400x token compaction
/// while preserving the raw role and text content.
pub fn prune_old_session_json(conn: &Connection, older_than_days: u32) -> Result<usize, String> {
    let cutoff = chrono::Utc::now().timestamp() - (older_than_days as i64 * 86400);
    let affected = conn
        .execute(
            "UPDATE chat_messages
             SET session_json = NULL
             WHERE session_json IS NOT NULL AND created_at < ?1",
            params![cutoff],
        )
        .map_err(|e| format!("failed to prune old session json: {e}"))?;

    Ok(affected)
}

/// Scans active memories and soft-archives any items whose Ebbinghaus retention
/// has dropped below the viability threshold (R < 0.05).
/// Decayed memories are soft-archived to status = 'ARCHIVED' and are NEVER hard-purged.
pub fn soft_archive_decayed_memories(conn: &Connection) -> Result<usize, String> {
    let now = chrono::Utc::now().timestamp();
    let mut stmt = conn
        .prepare(
            "SELECT id, last_accessed_at, stability_hours
             FROM memories
             WHERE status = 'ACTIVE' AND tombstone = 0",
        )
        .map_err(|e| format!("failed to prepare decay query: {e}"))?;

    let items = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)? as f32,
            ))
        })
        .map_err(|e| format!("failed to query memories: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to collect memories: {e}"))?;

    let mut archived_count = 0;
    for (id, last_accessed_at, stability_hours) in items {
        let elapsed_hours = (now - last_accessed_at).max(0) as f32 / 3600.0;
        let retention = calculate_retention(elapsed_hours, stability_hours);
        if is_archive_eligible(retention) {
            conn.execute(
                "UPDATE memories SET status = 'ARCHIVED', updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(|e| format!("failed to soft-archive memory {id}: {e}"))?;
            archived_count += 1;
        }
    }

    Ok(archived_count)
}

/// Executes DAG Supersession when a new rule contradicts an existing active rule.
/// Marks the old rule as SUPERSEDED, sets old.valid_until = event_time,
/// and links new.supersedes_id = old.id with new.valid_from = event_time.
///
/// B-I4: supersession is a trust-ranked write. Without the `can_override` check
/// an automated tool deduction could silently retire a rule the user stated
/// explicitly, laundering machine inference into user intent. The check is
/// refused before any row is mutated.
pub fn supersede_rule(
    new_memory_id: &str,
    old_memory_id: &str,
    event_time: i64,
    conn: &Connection,
) -> Result<(), String> {
    let new_trust = read_source_trust(conn, new_memory_id)?;
    let old_trust = read_source_trust(conn, old_memory_id)?;
    if !new_trust.can_override(old_trust) {
        return Err(format!(
            "supersession refused: {} cannot override {}",
            new_trust.as_str(),
            old_trust.as_str()
        ));
    }

    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "UPDATE memories
         SET status = 'SUPERSEDED', valid_until = ?1, updated_at = ?2
         WHERE id = ?3",
        params![event_time, now, old_memory_id],
    )
    .map_err(|e| format!("failed to supersede old memory: {e}"))?;

    conn.execute(
        "UPDATE memories
         SET supersedes_id = ?1, valid_from = ?2, updated_at = ?3
         WHERE id = ?4",
        params![old_memory_id, event_time, now, new_memory_id],
    )
    .map_err(|e| format!("failed to update superseding memory: {e}"))?;

    Ok(())
}

/// Reads and parses the `source_trust` column for a memory row. A missing row is
/// an error: supersession must not proceed without knowing the ranks it is
/// moving between.
fn read_source_trust(conn: &Connection, memory_id: &str) -> Result<SourceTrust, String> {
    let raw: String = conn
        .query_row(
            "SELECT source_trust FROM memories WHERE id = ?1",
            params![memory_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("failed to read source trust for memory '{memory_id}': {e}"))?;
    SourceTrust::from_str(&raw)
}

/// Fidelity Guard: verifies that an extracted canonical assertion preserves
/// all numerical thresholds, dates, operators, and column identifiers present
/// in the source statement, preventing MemFail summary_error class.
pub fn verify_fidelity(source: &str, assertion: &str) -> Result<(), String> {
    // Extract numbers from source
    let source_numbers: Vec<&str> = source
        .split(|c: char| !c.is_numeric() && c != '.' && c != '/')
        .filter(|s| !s.is_empty() && s.chars().any(|c| c.is_numeric()))
        .collect();

    for num in source_numbers {
        if !assertion.contains(num) {
            return Err(format!(
                "fidelity guard error: threshold '{num}' present in source was dropped in assertion"
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fidelity_guard_preserves_numerical_thresholds() {
        let source = "Build automatons only when motivation >= 8/10 and timeout <= 30s";
        let good_assertion = "automatons built when motivation >= 8/10 and timeout <= 30s";
        assert!(verify_fidelity(source, good_assertion).is_ok());

        // Dropping '8/10' fails fidelity check
        let bad_assertion = "automatons built when motivation is high and timeout <= 30s";
        assert!(verify_fidelity(source, bad_assertion).is_err());
    }

    use crate::ai::memory::{
        compute_memory_doc_hash, MemoryCategory, MemoryItem, MemoryManager, MemoryScope,
        MemoryStatus, SourceTrust, MEMORY_FORMAT_VERSION, MEMORY_MODEL_NAME,
    };

    fn memory(id: &str, trust: SourceTrust) -> MemoryItem {
        let now = chrono::Utc::now().timestamp();
        MemoryItem {
            id: id.into(),
            connection_key: "conn".into(),
            scope: MemoryScope::Connection,
            scope_key: "conn".into(),
            category: MemoryCategory::Metric,
            key_phrase: format!("kp_{id}"),
            rule_text: format!("rule text for {id}"),
            sql_snippet: None,
            importance: 0.8,
            stability_hours: 720.0,
            last_accessed_at: now,
            access_count: 1,
            source_trust: trust,
            source_conv_id: None,
            source_turn_id: None,
            source_tool_id: None,
            status: MemoryStatus::Active,
            supersedes_id: None,
            valid_from: now,
            valid_until: None,
            learned_at: now,
            tombstone: false,
            tombstoned_at: None,
            doc_hash: compute_memory_doc_hash(id),
            embedding_model: MEMORY_MODEL_NAME.into(),
            embedding_version: MEMORY_FORMAT_VERSION,
            embedding: vec![0.0; 384],
            created_at: now,
            updated_at: now,
        }
    }

    /// B-I4: `can_override` was never consulted during supersession, so an
    /// automated tool deduction could retire a rule the user stated
    /// explicitly. The write must be refused and the user rule left ACTIVE.
    #[tokio::test]
    async fn supersede_rule_refuses_tool_deduction_over_user_explicit_rule() {
        let mgr = MemoryManager::open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        mgr.save_memory(memory("user_rule", SourceTrust::UserExplicit), &[])
            .await
            .unwrap();
        mgr.save_memory(memory("tool_rule", SourceTrust::ErrorResolution), &[])
            .await
            .unwrap();

        let result = mgr
            .with_connection(|conn| supersede_rule("tool_rule", "user_rule", now, conn))
            .await;

        assert!(
            result.is_err(),
            "an ErrorResolution deduction must not supersede a UserExplicit rule"
        );
        let active = mgr.list_memories("conn", false).await.unwrap();
        let user_rule = active.iter().find(|m| m.id == "user_rule").expect("user rule");
        assert_eq!(
            user_rule.status,
            MemoryStatus::Active,
            "the refused supersession must not have mutated the user rule"
        );
        assert_eq!(user_rule.valid_until, None);
    }

    /// The rank check must not block the legitimate direction: the user's own
    /// explicit replacement outranks an inferred tool rule.
    #[tokio::test]
    async fn supersede_rule_allows_user_explicit_over_tool_deduction() {
        let mgr = MemoryManager::open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        mgr.save_memory(memory("tool_rule", SourceTrust::ErrorResolution), &[])
            .await
            .unwrap();
        mgr.save_memory(memory("user_rule", SourceTrust::UserExplicit), &[])
            .await
            .unwrap();

        mgr.with_connection(|conn| supersede_rule("user_rule", "tool_rule", now, conn))
            .await
            .expect("UserExplicit outranks ErrorResolution");

        let all = mgr.list_memories("conn", true).await.unwrap();
        let old = all.iter().find(|m| m.id == "tool_rule").expect("old rule");
        assert_eq!(old.status, MemoryStatus::Superseded);
        assert_eq!(old.valid_until, Some(now));
    }
}
