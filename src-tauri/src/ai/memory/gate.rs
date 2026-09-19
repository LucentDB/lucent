use crate::ai::memory::observations::Observation;

pub fn evaluate_signal_gate(obs: &Observation, threshold: f32, occurrences: i64) -> bool {
    if obs.signal == "explicit_request" {
        return true;
    }
    if obs.signal_strength >= threshold {
        return true;
    }
    if obs.occurrence_count >= occurrences {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::memory::observations::Origin;

    #[test]
    fn test_evaluate_signal_gate() {
        let mut obs = Observation::new(
            "c".into(),
            None,
            None,
            "diff".into(),
            Origin::Agent,
            "editor_diff".into(),
            0.8,
            "{}".into(),
        );
        // 0.8 >= default 0.75 threshold -> clears gate
        assert!(evaluate_signal_gate(&obs, 0.75, 3));

        // Below threshold and occurrences < 3 -> rejected
        obs.signal_strength = 0.5;
        obs.occurrence_count = 2;
        assert!(!evaluate_signal_gate(&obs, 0.75, 3));

        // Occurrence count reaches 3 -> clears gate
        obs.occurrence_count = 3;
        assert!(evaluate_signal_gate(&obs, 0.75, 3));

        // Explicit request always clears gate regardless of score
        obs.signal = "explicit_request".into();
        obs.signal_strength = 0.1;
        obs.occurrence_count = 1;
        assert!(evaluate_signal_gate(&obs, 0.75, 3));
    }
}
