use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionClass {
    Always,
    Retrieved,
}

impl InjectionClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Retrieved => "retrieved",
        }
    }
    pub fn from_str(s: &str) -> Self {
        if s.eq_ignore_ascii_case("always") {
            Self::Always
        } else {
            Self::Retrieved
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Owner,
    Agent,
    Untrusted,
    System,
}

impl Origin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Agent => "agent",
            Self::Untrusted => "untrusted",
            Self::System => "system",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "owner" => Self::Owner,
            "untrusted" => Self::Untrusted,
            "system" => Self::System,
            _ => Self::Agent,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub connection_key: String,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
    pub kind: String,
    pub origin: Origin,
    pub signal: String,
    pub signal_strength: f32,
    pub occurrence_count: i64,
    pub dedup_key: String,
    pub payload_json: String,
    pub status: String,
    pub derived_memory_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Observation {
    pub fn new(
        connection_key: String,
        conversation_id: Option<String>,
        turn_id: Option<String>,
        kind: String,
        origin: Origin,
        signal: String,
        signal_strength: f32,
        payload_json: String,
    ) -> Self {
        let now = chrono::Utc::now().timestamp();
        let dedup_key = {
            let mut hasher = blake3::Hasher::new();
            hasher.update(connection_key.as_bytes());
            hasher.update(kind.as_bytes());
            hasher.update(signal.as_bytes());
            hasher.update(payload_json.trim().as_bytes());
            hasher.finalize().to_hex().to_string()
        };

        Self {
            id: uuid::Uuid::new_v4().to_string(),
            connection_key,
            conversation_id,
            turn_id,
            kind,
            origin,
            signal,
            signal_strength,
            occurrence_count: 1,
            dedup_key,
            payload_json,
            status: "open".into(),
            derived_memory_id: None,
            created_at: now,
            updated_at: now,
        }
    }
}

pub enum ObservationOutcome {
    Inserted { id: String },
    RolledUp { id: String, occurrence_count: i64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_observation_dedup_hashing_and_outcome() {
        let obs = Observation::new(
            "conn-1".into(),
            Some("conv-1".into()),
            Some("turn-1".into()),
            "correction".into(),
            Origin::Agent,
            "user_correction".into(),
            0.85,
            r#"{"corrected_text":"deleted_at IS NULL"}"#.into(),
        );

        assert_eq!(obs.occurrence_count, 1);
        assert!(!obs.dedup_key.is_empty());
        assert_eq!(obs.status, "open");
    }
}
