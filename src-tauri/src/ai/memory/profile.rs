//! The profile plane: rules that are injected on *every* turn for a
//! connection (as opposed to retrieved per-query). `ProfileSnapshot` is built
//! once per conversation and then frozen for the life of that session, so a
//! rule learned mid-conversation cannot silently change the prompt the agent
//! is already operating under.

use crate::ai::memory::{InjectionClass, MemoryItem, MemoryManager, MemoryStatus};

/// Default token budget for the always-on profile block. Mirrors the memory
/// subsystem's small-block ceiling; a dedicated config knob does not exist yet.
pub const PROFILE_CAPACITY_TOKENS: usize = 250;

/// A frozen, capacity-bounded view of a connection's `Always`-injected
/// memories, captured once per conversation.
#[derive(Clone, Debug)]
pub struct ProfileSnapshot {
    pub connection_key: String,
    pub memories: Vec<MemoryItem>,
    pub built_at: i64,
}

impl ProfileSnapshot {
    pub async fn build(
        mgr: &MemoryManager,
        connection_key: &str,
        capacity_tokens: usize,
    ) -> Result<Self, String> {
        let all = mgr.list_memories(connection_key, false).await?;
        let mut profile_items: Vec<MemoryItem> = all
            .into_iter()
            .filter(|m| {
                m.injection == InjectionClass::Always
                    && m.status == MemoryStatus::Active
                    && !m.tombstone
                    && (m.scope_key == connection_key || m.scope_key == "global")
            })
            .collect();

        // Order by importance DESC, updated_at DESC
        profile_items.sort_by(|a, b| {
            b.importance
                .partial_cmp(&a.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });

        // Bounded capacity check (~4 chars per token)
        let mut total_chars = 0;
        let char_ceiling = capacity_tokens * 4;
        let mut bounded = Vec::new();
        for item in profile_items {
            let item_len = item.rule_text.len() + item.key_phrase.len();
            if total_chars + item_len <= char_ceiling {
                total_chars += item_len;
                bounded.push(item);
            }
        }

        Ok(Self {
            connection_key: connection_key.to_string(),
            memories: bounded,
            built_at: chrono::Utc::now().timestamp(),
        })
    }
}

pub fn format_profile_block(memories: &[MemoryItem]) -> Option<String> {
    if memories.is_empty() {
        return None;
    }
    let mut out = String::from("<user_profile>\n");
    for m in memories {
        out.push_str(&format!("  - {}: {}\n", m.key_phrase, m.rule_text));
    }
    out.push_str("</user_profile>\n");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::memory::{
        compute_memory_doc_hash, MemoryCategory, MemoryScope, Origin, SourceTrust,
        MEMORY_FORMAT_VERSION, MEMORY_MODEL_NAME,
    };

    /// Profile-plane fixture: an `Always`-injected, active, non-tombstoned
    /// global-scope rule, so `list_memories` returns it for any connection.
    fn profile_item(id: &str, key: &str, rule: &str, importance: f32) -> MemoryItem {
        let now = 1000;
        MemoryItem {
            id: id.into(),
            connection_key: "global".into(),
            scope: MemoryScope::Global,
            scope_key: "global".into(),
            category: MemoryCategory::Preference,
            key_phrase: key.into(),
            rule_text: rule.into(),
            sql_snippet: None,
            importance,
            stability_hours: 720.0,
            last_accessed_at: now,
            access_count: 1,
            source_trust: SourceTrust::UserExplicit,
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
            doc_hash: compute_memory_doc_hash(rule),
            embedding_model: MEMORY_MODEL_NAME.into(),
            embedding_version: MEMORY_FORMAT_VERSION,
            embedding: vec![0.0; 384],
            created_at: now,
            updated_at: now,
            injection: InjectionClass::Always,
            preference_key: None,
            origin: Origin::Owner,
            steps_json: None,
            merge_group_id: None,
            confirmed: false,
            confirmation_conv_id: None,
        }
    }

    #[tokio::test]
    async fn test_profile_snapshot_frozen_and_capacity_bounded() {
        let mgr = MemoryManager::open_in_memory().unwrap();
        // Add 2 profile items (injection = Always)
        let p1 = profile_item("p1", "sql_style", "Use CTEs", 0.9);
        let p2 = profile_item("p2", "dialect", "Use ::date casting", 0.8);
        mgr.save_memory(p1, &[]).await.unwrap();
        mgr.save_memory(p2, &[]).await.unwrap();

        let snapshot = ProfileSnapshot::build(&mgr, "conn-1", 250).await.unwrap();
        assert_eq!(snapshot.memories.len(), 2);

        // Rendered block contains both
        let block = format_profile_block(&snapshot.memories).unwrap();
        assert!(block.contains("Use CTEs"));
        assert!(block.contains("Use ::date casting"));

        // Add a 3rd item to DB mid-session
        let p3 = profile_item("p3", "tone", "Be concise", 0.7);
        mgr.save_memory(p3, &[]).await.unwrap();

        // Snapshot remains frozen with original 2 memories
        assert_eq!(snapshot.memories.len(), 2);
    }
}
