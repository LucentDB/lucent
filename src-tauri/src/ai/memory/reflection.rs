use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CurationAction {
    Add,
    Update,
    Supersede,
    Delete,
    Coexist,
    Noop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurationProposal {
    pub action: CurationAction,
    pub target_memory_id: Option<String>,
    pub category: String,
    pub injection: String,
    pub scope: String,
    pub preference_key: Option<String>,
    pub key_phrase: String,
    pub rule_text: String,
    pub sql_snippet: Option<String>,
    pub steps: Option<serde_json::Value>,
    pub referenced_entities: Vec<String>,
    pub confidence: f32,
    pub reasoning: String,
    pub observation: String,
    pub event: Option<String>,
}

pub fn parse_proposal(json_str: &str) -> Result<CurationProposal, String> {
    serde_json::from_str(json_str).map_err(|e| format!("invalid curation proposal JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_curation_proposal_json() {
        let json_text = r#"{
            "action": "ADD",
            "target_memory_id": null,
            "category": "quirk",
            "injection": "retrieved",
            "scope": "connection",
            "preference_key": null,
            "key_phrase": "orders_soft_delete",
            "rule_text": "Orders uses soft deletes; filter deleted_at IS NULL",
            "sql_snippet": "deleted_at IS NULL",
            "steps": null,
            "referenced_entities": ["public.orders"],
            "confidence": 0.92,
            "reasoning": "User clarified soft-delete policy",
            "observation": "raw text",
            "event": "soft_delete_rule"
        }"#;

        let proposal = parse_proposal(json_text).unwrap();
        assert_eq!(proposal.action, CurationAction::Add);
        assert_eq!(proposal.key_phrase, "orders_soft_delete");
        assert_eq!(proposal.confidence, 0.92);
    }
}
