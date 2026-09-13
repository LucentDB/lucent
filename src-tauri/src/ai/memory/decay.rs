/// Ebbinghaus decay and spaced repetition model for Lucent AI memory.
/// Ad-hoc noise decays quickly; recurring, verified facts gain stability into permanent rules.

pub const DEFAULT_STABILITY_HOURS: f32 = 720.0; // 30 days
pub const MAX_STABILITY_HOURS: f32 = 50_000.0; // ~5.7 years (permanent fact)
pub const MIN_STABILITY_HOURS: f32 = 1.0;
pub const ARCHIVE_RETENTION_THRESHOLD: f32 = 0.05;
pub const VIABILITY_BETA: f32 = 0.6;

/// Calculate Ebbinghaus memory retention R in [0.0, 1.0].
/// delta_hours is the elapsed time in hours since the memory was last accessed.
/// stability_hours is the memory's S parameter in hours.
pub fn calculate_retention(delta_hours: f32, stability_hours: f32) -> f32 {
    if delta_hours <= 0.0 {
        return 1.0;
    }
    let s = stability_hours.max(MIN_STABILITY_HOURS);
    (-delta_hours / s).exp().clamp(0.0, 1.0)
}

/// Calculate reinforced stability after recall/access.
/// S_{n+1} = S_n * [1 + 2.0 * (1 - R) * I]
pub fn reinforce_stability(current_stability: f32, retention: f32, importance: f32) -> f32 {
    let imp = importance.clamp(0.0, 1.0);
    let factor = 1.0 + 2.0 * (1.0 - retention) * imp;
    (current_stability * factor).clamp(MIN_STABILITY_HOURS, MAX_STABILITY_HOURS)
}

/// Calculate Ebbinghaus viability score combining retention and importance.
/// Viability = beta * R + (1 - beta) * I
pub fn calculate_viability(retention: f32, importance: f32) -> f32 {
    let r = retention.clamp(0.0, 1.0);
    let i = importance.clamp(0.0, 1.0);
    (VIABILITY_BETA * r + (1.0 - VIABILITY_BETA) * i).clamp(0.0, 1.0)
}

/// Check whether a memory is eligible for soft-archiving (retention dropped below 0.05).
/// Decayed memories are never hard-purged without explicit user action.
pub fn is_archive_eligible(retention: f32) -> bool {
    retention < ARCHIVE_RETENTION_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retention_calculation() {
        // At t=0, retention is 1.0
        assert_eq!(calculate_retention(0.0, 720.0), 1.0);

        // At t=720 (one half-life/characteristic time), retention is 1/e ~ 0.3678
        let r = calculate_retention(720.0, 720.0);
        assert!((r - 0.3678).abs() < 0.01);

        // After long time, retention approaches 0
        let r_old = calculate_retention(5000.0, 720.0);
        assert!(r_old < 0.01);
        assert!(is_archive_eligible(r_old));
    }

    #[test]
    fn test_stability_reinforcement() {
        let s0 = 720.0;
        let r = 0.5;
        let importance = 0.8;
        let s1 = reinforce_stability(s0, r, importance);
        // factor = 1 + 2.0 * (1 - 0.5) * 0.8 = 1 + 0.8 = 1.8
        assert!((s1 - 720.0 * 1.8).abs() < 1e-3);
        assert!(s1 > s0);
    }

    #[test]
    fn test_viability_calculation() {
        let v = calculate_viability(1.0, 1.0);
        assert_eq!(v, 1.0);

        let v_zero = calculate_viability(0.0, 0.0);
        assert_eq!(v_zero, 0.0);

        let v_mid = calculate_viability(0.5, 0.5);
        assert!((v_mid - 0.5).abs() < 1e-4);
    }
}
