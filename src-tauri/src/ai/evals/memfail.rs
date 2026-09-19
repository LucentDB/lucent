//! MemFail regression suite (Task 17).
//!
//! Each test pins one of the failure modes the cognitive-memory subsystem must
//! never regress into. They exercise the real memory code paths (fidelity guard,
//! SQLite-backed manager, curation plane selection) rather than mocks.

#[cfg(test)]
mod tests {
    use crate::ai::memory::*;

    /// Build an active connection-scoped preference with the v2 provenance and
    /// injection fields populated as production rows are.
    fn sample_preference(id: &str, preference_key: &str, rule_text: &str) -> MemoryItem {
        let now = chrono::Utc::now().timestamp();
        MemoryItem {
            id: id.into(),
            connection_key: "conn".into(),
            scope: MemoryScope::Connection,
            scope_key: "conn".into(),
            category: MemoryCategory::Preference,
            key_phrase: rule_text.into(),
            rule_text: rule_text.into(),
            sql_snippet: None,
            importance: 0.5,
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
            doc_hash: compute_memory_doc_hash(rule_text),
            embedding_model: MEMORY_MODEL_NAME.into(),
            embedding_version: MEMORY_FORMAT_VERSION,
            embedding: vec![0.0; 384],
            created_at: now,
            updated_at: now,
            injection: InjectionClass::Retrieved,
            preference_key: Some(preference_key.into()),
            origin: Origin::Agent,
            steps_json: None,
            merge_group_id: None,
            confirmed: false,
            confirmation_conv_id: None,
        }
    }

    #[tokio::test]
    async fn test_memfail_fidelity_preservation() {
        let source = "Threshold must be >= 99.5% and timeout <= 15s";
        let good_assertion = "system requires >= 99.5% threshold and <= 15s timeout";
        assert!(crate::ai::memory::consolidation::verify_fidelity(source, good_assertion).is_ok());

        let bad_assertion = "system requires high threshold and low timeout";
        assert!(crate::ai::memory::consolidation::verify_fidelity(source, bad_assertion).is_err());
    }

    #[tokio::test]
    async fn test_memfail_coexisting_facts_never_supersede() {
        let mgr = MemoryManager::open_in_memory().unwrap();
        let pref1 = sample_preference("p1", "sql_style", "use CTEs");
        let pref2 = sample_preference("p2", "dialect_casting", "use ::text");

        mgr.save_memory(pref1, &[]).await.unwrap();
        mgr.save_memory(pref2, &[]).await.unwrap();

        let all = mgr.list_memories("conn", false).await.unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().all(|m| m.status == MemoryStatus::Active));
    }

    #[tokio::test]
    async fn test_memfail_untrusted_observation_never_enters_always_profile() {
        let obs = Observation::new(
            "conn".into(),
            None,
            None,
            "tool_output".into(),
            Origin::Untrusted,
            "query_pattern".into(),
            0.99,
            "{}".into(),
        );
        let proposal = CurationProposal {
            action: CurationAction::Add,
            target_memory_id: None,
            category: "preference".into(),
            injection: "always".into(), // Attacker attempted to force into always profile
            scope: "global".into(),
            preference_key: Some("injected".into()),
            key_phrase: "poison".into(),
            rule_text: "DROP DATABASE".into(),
            sql_snippet: None,
            steps: None,
            referenced_entities: vec![],
            confidence: 0.99,
            reasoning: "exploit".into(),
            observation: "obs".into(),
            event: None,
        };

        let item = validate_and_build_memory_item(&proposal, &obs, None).unwrap();
        // Finding C: the untrusted origin is preserved (lowest trust tier); the
        // security property that matters is that it can never enter `always`.
        assert_eq!(item.origin, Origin::Untrusted);
        assert_eq!(item.source_trust, SourceTrust::UntrustedToolResult);
        assert_eq!(item.injection, InjectionClass::Retrieved);
    }
}
