use crate::ai::memory::retrieval::cosine_similarity;
use crate::ai::memory::{MemoryCategory, MemoryItem};

/// Intent-matching retrieval for procedural playbooks.
///
/// Playbooks are multi-step procedures stored as `MemoryCategory::Playbook`
/// entries (with their steps in `steps_json`). Unlike metric/join quirks, a
/// playbook is only useful when the user's request *is* the procedure, so it
/// is selected by direct cosine similarity against the query embedding rather
/// than folded into the RRF hybrid ranking. Tombstoned candidates are ignored.
///
/// Returns the first non-tombstoned playbook whose cosine similarity to
/// `query_embedding` meets or exceeds `threshold`.
pub fn match_playbook<'a>(
    query_embedding: &[f32],
    candidates: &'a [MemoryItem],
    threshold: f32,
) -> Option<&'a MemoryItem> {
    candidates
        .iter()
        .filter(|m| m.category == MemoryCategory::Playbook && !m.tombstone)
        .find(|m| cosine_similarity(query_embedding, &m.embedding) >= threshold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::memory::{
        InjectionClass, MemoryScope, MemoryStatus, Origin, SourceTrust, MEMORY_FORMAT_VERSION,
        MEMORY_MODEL_NAME,
    };

    fn sample_playbook(id: &str, intent: &str) -> MemoryItem {
        MemoryItem {
            id: id.into(),
            connection_key: "conn".into(),
            scope: MemoryScope::Connection,
            scope_key: "conn".into(),
            category: MemoryCategory::Playbook,
            key_phrase: intent.into(),
            rule_text: format!("Playbook: {intent}"),
            sql_snippet: None,
            importance: 0.8,
            stability_hours: 720.0,
            last_accessed_at: 0,
            access_count: 1,
            source_trust: SourceTrust::UserExplicit,
            source_conv_id: None,
            source_turn_id: None,
            source_tool_id: None,
            status: MemoryStatus::Active,
            supersedes_id: None,
            valid_from: 0,
            valid_until: None,
            learned_at: 0,
            tombstone: false,
            tombstoned_at: None,
            doc_hash: String::new(),
            embedding_model: MEMORY_MODEL_NAME.into(),
            embedding_version: MEMORY_FORMAT_VERSION,
            embedding: Vec::new(),
            created_at: 0,
            updated_at: 0,
            injection: InjectionClass::Retrieved,
            preference_key: None,
            origin: Origin::Agent,
            steps_json: Some("[]".into()),
            merge_group_id: None,
            confirmed: false,
            confirmation_conv_id: None,
        }
    }

    #[test]
    fn test_retrieve_matching_playbook() {
        let mut pb = sample_playbook("pb1", "weekly revenue reconciliation");
        pb.embedding = vec![0.5; 384];
        let playbooks = vec![pb];

        let query_embedding = vec![0.5; 384];
        let matched = match_playbook(&query_embedding, &playbooks, 0.85);
        assert!(matched.is_some());
        assert_eq!(matched.unwrap().id, "pb1");
    }
}
