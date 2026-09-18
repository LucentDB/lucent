use crate::ai::memory::observations::{InjectionClass, Observation, Origin};
use crate::ai::memory::reflection::CurationProposal;
use crate::ai::memory::security::{sanitize_rule_text, sanitize_sql_snippet, SourceTrust};
use crate::ai::memory::{
    compute_memory_doc_hash, MemoryCategory, MemoryItem, MemoryScope, MemoryStatus,
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

    // Trust ceiling: LLM output derived from agent evidence is capped at ErrorResolution
    let (source_trust, origin, stability_hours) = match obs.origin {
        Origin::Owner => (
            SourceTrust::UserExplicit,
            Origin::Owner,
            USER_EXPLICIT_STABILITY_HOURS,
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
        assert_eq!(item.source_trust, SourceTrust::ErrorResolution);
        assert_eq!(item.origin, Origin::Agent);
    }
}
