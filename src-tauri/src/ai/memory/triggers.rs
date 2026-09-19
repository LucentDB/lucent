use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use tauri::Manager;

/// Idle window before the sleep-time compute daemon may run: 15 minutes.
pub const IDLE_THRESHOLD_SECS: i64 = 900;

/// Tracks the last time the user (or app) was active, so the sleep-time compute
/// daemon can run only after the configured idle window. Cheap to clone via the
/// shared `Arc`, and lock-free so activity on the hot path never contends with
/// the daemon's periodic check.
pub struct IdleTracker {
    last_activity: Arc<AtomicI64>,
}

impl IdleTracker {
    pub fn new() -> Self {
        Self {
            last_activity: Arc::new(AtomicI64::new(chrono::Utc::now().timestamp())),
        }
    }

    /// Records an explicit activity timestamp. Exposed for tests; production
    /// callers should use [`IdleTracker::touch`].
    pub fn set_activity(&self, timestamp: i64) {
        self.last_activity.store(timestamp, Ordering::Release);
    }

    /// Marks activity as happening now.
    pub fn touch(&self) {
        self.last_activity
            .store(chrono::Utc::now().timestamp(), Ordering::Release);
    }

    /// True when at least `threshold_secs` have elapsed since the last activity.
    pub fn is_idle(&self, current_time: i64, threshold_secs: i64) -> bool {
        let last = self.last_activity.load(Ordering::Acquire);
        (current_time - last) >= threshold_secs
    }
}

impl Default for IdleTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Polls the shared [`IdleTracker`] every 60 seconds and runs the sleep-cycle
/// consolidation once per idle period.
///
/// The "fired" flag is reset as soon as activity resumes, so a fresh idle
/// window triggers a fresh cycle while a continuously-idle app never re-runs
/// the same cycle on every tick.
pub fn start_idle_daemon<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    tracker: std::sync::Arc<IdleTracker>,
) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        let mut fired_for_idle_period = false;

        loop {
            interval.tick().await;
            let now = chrono::Utc::now().timestamp();

            if tracker.is_idle(now, IDLE_THRESHOLD_SECS) {
                if !fired_for_idle_period {
                    let state = app.state::<crate::AppState>();
                    if let Err(e) =
                        crate::ai::memory::consolidation::run_sleep_cycle(state.inner()).await
                    {
                        log::debug!("idle sleep-cycle consolidation failed: {e}");
                    }
                    fired_for_idle_period = true;
                }
            } else {
                fired_for_idle_period = false;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_idle_trigger_evaluates_activity_threshold() {
        let now = 10_000i64;
        let tracker = IdleTracker::new();
        tracker.set_activity(now);

        // 100 seconds later: not idle (< 900s)
        assert!(!tracker.is_idle(now + 100, 900));

        // 950 seconds later: idle (> 900s)
        assert!(tracker.is_idle(now + 950, 900));
    }
}
