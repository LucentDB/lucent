use crate::ai::memory::observations::{InjectionClass, Observation, Origin};
use crate::ai::memory::reflection::CurationProposal;
use crate::ai::memory::retrieval::cosine_similarity;
use crate::ai::memory::security::{sanitize_rule_text, sanitize_sql_snippet, SourceTrust};
use crate::ai::memory::{
    compute_memory_doc_hash, MemoryCategory, MemoryItem, MemoryManager, MemoryScope, MemoryStatus,
    MEMORY_FORMAT_VERSION, MEMORY_MODEL_NAME, TOOL_RULE_STABILITY_HOURS,
    USER_EXPLICIT_STABILITY_HOURS,
};
use crate::ai::schema_graph::SchemaGraph;

pub fn validate_and_build_memory_item(
    proposal: &CurationProposal,
    obs: &Observation,
    _graph: Option<&SchemaGraph>,
) -> Result<MemoryItem, String> {
    let sanitized_rule = sanitize_rule_text(&proposal.rule_text)?;
    let sanitized_sql = match proposal.sql_snippet.as_deref() {
        Some(s) => Some(sanitize_sql_snippet(Some(s))?.unwrap_or_default()),
        None => None,
    };

    // Trust ceiling follows the observation's origin, not the LLM's claim:
    // an explicit user request is UserExplicit, an untrusted tool result is
    // UntrustedToolResult, and everything the agent inferred (Agent/System) is
    // capped at ErrorResolution. The origin carries through unchanged so the
    // trust plane is auditable downstream.
    let (source_trust, origin, stability_hours) = match obs.origin {
        Origin::Owner => (
            SourceTrust::UserExplicit,
            Origin::Owner,
            USER_EXPLICIT_STABILITY_HOURS,
        ),
        Origin::Untrusted => (
            SourceTrust::UntrustedToolResult,
            Origin::Untrusted,
            TOOL_RULE_STABILITY_HOURS,
        ),
        _ => (
            SourceTrust::ErrorResolution,
            Origin::Agent,
            TOOL_RULE_STABILITY_HOURS,
        ),
    };

    // Plane selection: the LLM cannot force 'always' unless the origin is Owner or the
    // category is Preference. R6: an untrusted observation never earns the always-on
    // plane, even when the proposal claims `preference`.
    let injection = if proposal.injection == "always"
        && obs.origin != Origin::Untrusted
        && (origin == Origin::Owner || proposal.category == "preference")
    {
        InjectionClass::Always
    } else {
        InjectionClass::Retrieved
    };

    let now = chrono::Utc::now().timestamp();
    let doc_hash = compute_memory_doc_hash(&sanitized_rule);

    Ok(MemoryItem {
        id: uuid::Uuid::new_v4().to_string(),
        connection_key: obs.connection_key.clone(),
        scope: MemoryScope::from_str(&proposal.scope),
        scope_key: obs.connection_key.clone(),
        category: MemoryCategory::from_str(&proposal.category),
        key_phrase: proposal.key_phrase.clone(),
        rule_text: sanitized_rule,
        sql_snippet: sanitized_sql,
        importance: proposal.confidence.clamp(0.1, 1.0),
        stability_hours,
        last_accessed_at: now,
        access_count: 1,
        source_trust,
        source_conv_id: obs.conversation_id.clone(),
        source_turn_id: obs.turn_id.clone(),
        source_tool_id: Some(obs.signal.clone()),
        status: MemoryStatus::Active,
        supersedes_id: proposal.target_memory_id.clone(),
        valid_from: now,
        valid_until: None,
        learned_at: now,
        tombstone: false,
        tombstoned_at: None,
        doc_hash,
        embedding_model: MEMORY_MODEL_NAME.into(),
        embedding_version: MEMORY_FORMAT_VERSION,
        embedding: vec![0.0f32; 384],
        created_at: now,
        updated_at: now,
        injection,
        preference_key: proposal.preference_key.clone(),
        origin,
        steps_json: proposal.steps.as_ref().map(|s| s.to_string()),
        merge_group_id: None,
        confirmed: false,
        confirmation_conv_id: None,
    })
}

/// Result of routing a newly built memory item through near-duplicate curation.
///
/// `MergedInto` means an existing memory was similar enough (cosine `>=` the
/// supplied threshold) that the candidate was folded into it rather than kept
/// as an independent rule. `Superseded` is reserved for the explicit DAG
/// supersession path. `SavedAsNew` is the no-near-duplicate fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeOutcome {
    SavedAsNew { id: String },
    MergedInto { keep_id: String },
    Superseded { old_id: String },
}

/// Persists a built memory item, first checking whether it is a near-duplicate
/// of an active memory in the same connection + category.
///
/// When a near-duplicate exists (`cosine_similarity >= threshold`) the candidate
/// is still persisted — R18: folding must not silently drop the candidate's
/// evidence — and then immediately marked `SUPERSEDED` with `supersedes_id`
/// pointing at the keeper. This leaves a queryable DAG edge and a real row for
/// `merge_memories` to fold, instead of a phantom id that was never inserted.
pub async fn consolidate_or_merge_item(
    item: MemoryItem,
    mgr: &MemoryManager,
    threshold: f32,
) -> Result<MergeOutcome, String> {
    let existing = mgr.list_memories(&item.connection_key, false).await?;
    for ex in existing {
        if ex.category == item.category && ex.status == MemoryStatus::Active && !ex.tombstone {
            let sim = cosine_similarity(&item.embedding, &ex.embedding);
            if sim >= threshold {
                // Persist the candidate before folding so its evidence survives
                // and `merge_memories` has a real row to mark SUPERSEDED (R18).
                mgr.save_memory(item.clone(), &[]).await?;
                mgr.merge_memories(&ex.id, std::slice::from_ref(&item.id))
                    .await?;
                return Ok(MergeOutcome::MergedInto { keep_id: ex.id });
            }
        }
    }
    let id = item.id.clone();
    mgr.save_memory(item, &[]).await?;
    Ok(MergeOutcome::SavedAsNew { id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::memory::reflection::CurationAction;

    #[test]
    fn test_trust_ceiling_prevents_llm_claiming_owner() {
        let obs = Observation::new(
            "conn".into(),
            None,
            None,
            "diff".into(),
            Origin::Agent,
            "editor_diff".into(),
            0.8,
            "{}".into(),
        );
        let proposal = CurationProposal {
            action: CurationAction::Add,
            target_memory_id: None,
            category: "quirk".into(),
            injection: "always".into(), // LLM attempted to mark as always
            scope: "connection".into(),
            preference_key: None,
            key_phrase: "kp".into(),
            rule_text: "Clean rule text".into(),
            sql_snippet: None,
            steps: None,
            referenced_entities: vec![],
            confidence: 0.99,
            reasoning: "none".into(),
            observation: "obs".into(),
            event: None,
        };

        let item = validate_and_build_memory_item(&proposal, &obs, None).unwrap();
        // Trust must be capped at ErrorResolution, origin must be Agent, injection must be forced to Retrieved
        assert_eq!(item.source_trust, SourceTrust::ErrorResolution);
        assert_eq!(item.origin, Origin::Agent);
        assert_eq!(item.injection, InjectionClass::Retrieved);
    }

    /// R6: an untrusted observation must never earn the always-on plane, even when the
    /// proposal claims `category = "preference"` and `injection = "always"`.
    #[test]
    fn test_untrusted_preference_stays_retrieved() {
        let obs = Observation::new(
            "conn".into(),
            None,
            None,
            "diff".into(),
            Origin::Untrusted,
            "editor_diff".into(),
            0.8,
            "{}".into(),
        );
        let proposal = CurationProposal {
            action: CurationAction::Add,
            target_memory_id: None,
            category: "preference".into(),
            injection: "always".into(),
            scope: "connection".into(),
            preference_key: Some("formatting".into()),
            key_phrase: "kp".into(),
            rule_text: "Clean rule text".into(),
            sql_snippet: None,
            steps: None,
            referenced_entities: vec![],
            confidence: 0.99,
            reasoning: "none".into(),
            observation: "obs".into(),
            event: None,
        };

        let item = validate_and_build_memory_item(&proposal, &obs, None).unwrap();
        assert_eq!(item.injection, InjectionClass::Retrieved);
        // Finding C: an untrusted observation keeps its own (lowest) trust
        // tier instead of being relabeled as an agent inference.
        assert_eq!(item.source_trust, SourceTrust::UntrustedToolResult);
        assert_eq!(item.origin, Origin::Untrusted);
    }

    fn sample_memory(id: &str, rule_text: &str) -> MemoryItem {
        MemoryItem {
            id: id.into(),
            connection_key: "conn".into(),
            scope: MemoryScope::Connection,
            scope_key: "conn".into(),
            category: MemoryCategory::Quirk,
            key_phrase: id.into(),
            rule_text: rule_text.into(),
            sql_snippet: None,
            importance: 0.5,
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
            steps_json: None,
            merge_group_id: None,
            confirmed: false,
            confirmation_conv_id: None,
        }
    }

    #[tokio::test]
    async fn test_near_duplicate_cosine_merge() {
        let mgr = MemoryManager::open_in_memory().unwrap();
        let mut item1 = sample_memory("m1", "orders uses soft deletes");
        item1.embedding = vec![0.1; 384]; // identical vector -> cosine 1.0
        mgr.save_memory(item1, &[]).await.unwrap();

        let mut item2 = sample_memory("m2", "orders table filters deleted_at IS NULL");
        item2.embedding = vec![0.1; 384]; // cosine 1.0 >= 0.92

        let outcome = consolidate_or_merge_item(item2, &mgr, 0.92).await.unwrap();
        match outcome {
            MergeOutcome::MergedInto { keep_id } => assert_eq!(keep_id, "m1"),
            _ => panic!("expected merge into m1"),
        }

        // R18: the candidate must have been persisted *before* folding, leaving
        // a real SUPERSEDED row (not the phantom-id bug that dropped evidence).
        // Superseded rows are neither ARCHIVED nor tombstoned, so the default
        // list_memories call still returns them.
        let items = mgr.list_memories("conn", false).await.unwrap();
        let m2 = items
            .iter()
            .find(|m| m.id == "m2")
            .expect("R18: candidate row must exist after consolidate_or_merge_item");
        assert_eq!(m2.status, MemoryStatus::Superseded);
        assert_eq!(m2.supersedes_id, Some("m1".to_string()));
    }

    /// Direct coverage for the DAG-supersession half of the merge: folds carry
    /// `status = SUPERSEDED` + `supersedes_id = keep_id` and every participant
    /// shares one `merge_group_id`, while the keeper stays ACTIVE even if a
    /// caller (defensively) lists it among the fold ids. A fold id with no row
    /// is a silent no-op, not an error.
    #[tokio::test]
    async fn test_merge_memories_marks_folds_superseded_and_shares_group() {
        let mgr = MemoryManager::open_in_memory().unwrap();
        mgr.save_memory(sample_memory("m1", "keep me"), &[])
            .await
            .unwrap();
        mgr.save_memory(sample_memory("m2", "fold me"), &[])
            .await
            .unwrap();

        // Keeper also appears in fold_ids plus a nonexistent id: neither may
        // supersede the keeper nor error.
        mgr.merge_memories("m1", &["m2".into(), "m1".into(), "missing".into()])
            .await
            .unwrap();

        let items = mgr.list_memories("conn", false).await.unwrap();
        let get = |id: &str| items.iter().find(|m| m.id == id).unwrap().clone();
        let (keep, fold) = (get("m1"), get("m2"));

        assert_eq!(keep.status, MemoryStatus::Active);
        assert!(keep.merge_group_id.is_some());
        assert_eq!(fold.status, MemoryStatus::Superseded);
        assert_eq!(fold.supersedes_id.as_deref(), Some("m1"));
        assert_eq!(fold.merge_group_id, keep.merge_group_id);
    }
}
