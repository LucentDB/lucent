use std::path::PathBuf;

use lucent_protocol::{ConnectionId, QueryId};

pub struct NotebookSession {
    pub file_path: Option<PathBuf>,
    pub session_key: String,
    pub connection_id: ConnectionId,
    pub database: String,
    pub profile_id: Option<String>,
    pub active_query_id: Option<QueryId>,
    /// The AI cell currently running (if any) and its cancellation token.
    /// AI cells cannot register a DB query id — the agent loop owns its own
    /// queries — so the Stop button cancels the loop via this token (E2).
    pub active_ai_cell: Option<(String, tokio_util::sync::CancellationToken)>,
    /// Monotonic execution counter. An execution number records *when* a cell
    /// ran, so it must be owned by the session that owns the timeline — deriving
    /// it from how many cells are currently green cannot be correct.
    exec_counter: u32,
}

impl NotebookSession {
    pub fn new(session_key: String, connection_id: ConnectionId, database: String) -> Self {
        Self {
            file_path: None,
            session_key,
            connection_id,
            database,
            profile_id: None,
            active_query_id: None,
            active_ai_cell: None,
            exec_counter: 0,
        }
    }

    /// Allocates the next execution number. First call returns 1.
    pub fn next_execution_order(&mut self) -> u32 {
        self.exec_counter += 1;
        self.exec_counter
    }

    pub fn reset_execution_counter(&mut self) {
        self.exec_counter = 0;
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use lucent_protocol::ConnectionId;
    use uuid::Uuid;

    fn session() -> NotebookSession {
        NotebookSession::new(
            "key".into(),
            ConnectionId(Uuid::new_v4()),
            "postgres".into(),
        )
    }

    #[test]
    fn counter_starts_at_one_and_increments_monotonically() {
        let mut s = session();
        assert_eq!(s.next_execution_order(), 1);
        assert_eq!(s.next_execution_order(), 2);
        assert_eq!(s.next_execution_order(), 3);
    }

    #[test]
    fn reset_returns_numbering_to_one() {
        let mut s = session();
        s.next_execution_order();
        s.next_execution_order();
        s.reset_execution_counter();
        assert_eq!(s.next_execution_order(), 1);
    }

    /// Distinguishes a real monotonic counter from a "count the green cells"
    /// derivation: the counter must not reset or change based on how many
    /// times it's read, and must keep climbing regardless of what a caller
    /// does with cell state in between calls (which this type doesn't even
    /// have access to — the whole point of moving it onto the session).
    #[test]
    fn counter_is_independent_of_cell_state_and_does_not_reset_on_repeated_calls() {
        let mut s = session();
        // Simulate re-running "the same cell" many times in a row — a
        // green-cell-count derivation would keep returning the same value
        // (or worse, N+1 for an all-green notebook) because it depends on
        // spatial state, not on how many times execution has actually
        // happened. The session counter must climb every single call.
        let sequence: Vec<u32> = (0..5).map(|_| s.next_execution_order()).collect();
        assert_eq!(sequence, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn reset_then_resume_produces_a_fresh_monotonic_run() {
        let mut s = session();
        for _ in 0..10 {
            s.next_execution_order();
        }
        assert_eq!(s.next_execution_order(), 11);
        s.reset_execution_counter();
        // After reset, numbering must restart from 1 and continue
        // monotonically — not just report 1 once and drift afterward.
        assert_eq!(s.next_execution_order(), 1);
        assert_eq!(s.next_execution_order(), 2);
        assert_eq!(s.next_execution_order(), 3);
    }
}
