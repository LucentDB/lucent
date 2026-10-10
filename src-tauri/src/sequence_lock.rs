//! Per-connection ordering for AI statement sequences (BEGIN … ROLLBACK,
//! BEGIN … COMMIT). Postgres keeps transaction state per backend and DuckDB
//! tracks one in-flight query per connection, so two sequences interleaved on
//! one connection corrupt each other's scope, timeout and cancel. The lock is
//! deliberately NOT re-entrant: no AI path nests sequences.

use std::collections::HashMap;
use std::sync::Arc;

use lucent_protocol::ConnectionId;

pub type SequenceGuard = tokio::sync::OwnedMutexGuard<()>;

#[derive(Default)]
pub struct SequenceLocks {
    locks: std::sync::Mutex<HashMap<ConnectionId, Arc<tokio::sync::Mutex<()>>>>,
}

impl SequenceLocks {
    /// Wait up to `wait_ms` for the connection's sequence lock. A wait that
    /// long means another sequence is stuck; report it instead of hanging the
    /// agent turn. `wait_ms == 0` waits indefinitely.
    pub async fn acquire(
        &self,
        conn_id: ConnectionId,
        wait_ms: u64,
    ) -> Result<SequenceGuard, String> {
        let lock = {
            let mut map = self.locks.lock().unwrap_or_else(|p| p.into_inner());
            Arc::clone(map.entry(conn_id).or_default())
        };
        if wait_ms == 0 {
            return Ok(lock.lock_owned().await);
        }
        tokio::time::timeout(std::time::Duration::from_millis(wait_ms), lock.lock_owned())
            .await
            .map_err(|_| {
                format!("the AI database connection is busy (waited {wait_ms}ms for the previous query to finish)")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucent_protocol::ConnectionId;
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn a_second_sequence_on_the_same_connection_waits_for_the_first() {
        let locks = SequenceLocks::default();
        let c = ConnectionId(uuid::Uuid::new_v4());
        let first = locks.acquire(c, 1_000).await.unwrap();
        let second =
            tokio::time::timeout(Duration::from_millis(100), locks.acquire(c, 1_000)).await;
        assert!(
            second.is_err(),
            "the second sequence must wait while the first holds the guard"
        );
        drop(first);
        assert!(
            locks.acquire(c, 1_000).await.is_ok(),
            "released → acquirable"
        );
    }

    #[tokio::test]
    async fn different_connections_do_not_contend() {
        let locks = SequenceLocks::default();
        let _a = locks
            .acquire(ConnectionId(uuid::Uuid::new_v4()), 1_000)
            .await
            .unwrap();
        assert!(locks
            .acquire(ConnectionId(uuid::Uuid::new_v4()), 50)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn a_held_lock_times_out_with_a_busy_error_instead_of_hanging() {
        let locks = SequenceLocks::default();
        let c = ConnectionId(uuid::Uuid::new_v4());
        let _held = locks.acquire(c, 1_000).await.unwrap();
        let e = locks.acquire(c, 50).await.unwrap_err();
        assert!(e.contains("busy"), "{e}");
    }

    #[tokio::test]
    async fn a_zero_wait_blocks_until_release_instead_of_timing_out() {
        // `wait_ms == 0` waits indefinitely (documented on `acquire`). No
        // production caller passes 0 today — this pins the contract, the
        // inverse of the timeout test above.
        let locks = Arc::new(SequenceLocks::default());
        let c = ConnectionId(uuid::Uuid::new_v4());
        let first = locks.acquire(c, 1_000).await.unwrap();
        let pending = {
            let locks = Arc::clone(&locks);
            tokio::spawn(async move { locks.acquire(c, 0).await })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            !pending.is_finished(),
            "a zero wait must block while the guard is held, not time out"
        );
        drop(first);
        let guard = tokio::time::timeout(Duration::from_secs(5), pending)
            .await
            .expect("the zero wait completes after release")
            .expect("task join")
            .expect("a zero wait acquires, never errors with busy");
        drop(guard);
    }
}
