//! The blocking-connection model.
//!
//! `duckdb::Connection` is `Send` but **`!Sync`**, and every method on it
//! blocks. `Connector` is async, takes `&self`, and requires `Send + Sync`.
//! Bridging the two is this module's only job:
//!
//! - `Mutex<Connection>` is `Sync` because `Connection` is `Send`, so the
//!   handle satisfies the trait bound.
//! - Every database call runs inside `spawn_blocking`, so a long query never
//!   occupies a Tokio worker thread.
//! - The interrupt handle is captured at open time and stored **outside** the
//!   mutex. A running query holds the lock; a cancel that had to acquire it
//!   would deadlock against the query it is cancelling.

use std::sync::{Arc, Mutex};

use duckdb::{AccessMode, Config, Connection, InterruptHandle};
use lucent_protocol::{LucentError, LucentErrorKind};

/// One open DuckDB database.
pub struct DuckHandle {
    conn: Arc<Mutex<Connection>>,
    /// Captured at open time and deliberately not behind `conn`'s lock.
    interrupt: Arc<InterruptHandle>,
    read_only: bool,
}

impl DuckHandle {
    /// Open a database file, or `:memory:` for an ephemeral one.
    ///
    /// `read_only` maps to DuckDB's `access_mode = READ_ONLY`, which the engine
    /// enforces for the whole connection. DuckDB has no `SET TRANSACTION READ
    /// ONLY`, so engine-level read-only is only available through `access_mode`
    /// at open time.
    ///
    /// File-lock reality, established empirically in Task 3
    /// (`tests/readonly_reality_test.rs`): a read-only handle CAN coexist with
    /// a read-write handle on the same file **within one process** (the
    /// worker's own multiplexing case) and stays engine-enforced. The file
    /// lock only bites **across processes** — a second process cannot open the
    /// same file, regardless of mode.
    pub fn open(path: &str, read_only: bool) -> Result<Self, LucentError> {
        let mut config = Config::default();
        config = config
            .access_mode(if read_only {
                AccessMode::ReadOnly
            } else {
                AccessMode::ReadWrite
            })
            .map_err(|e| err(LucentErrorKind::Internal, format!("access mode: {e}")))?;

        let conn = Connection::open_with_flags(path, config).map_err(|e| {
            // A missing file in read-only mode and a locked file are the two
            // failures users actually hit; both surface as ConnectionRefused so
            // the app's existing error handling applies.
            err(
                LucentErrorKind::ConnectionRefused,
                format!("could not open {path:?}: {e}"),
            )
        })?;

        let interrupt = conn.interrupt_handle();

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            interrupt,
            read_only,
        })
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    /// Run a closure against the connection, holding the lock only for its
    /// duration.
    ///
    /// Callers must invoke this from inside `spawn_blocking` — it blocks. A
    /// poisoned lock (a previous panic inside a closure) is recovered rather
    /// than propagated: DuckDB's own state is intact, and failing every
    /// subsequent query because one decode panicked would be worse.
    pub fn with_conn<F, T>(&self, f: F) -> Result<T, LucentError>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        let guard = match self.conn.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&guard).map_err(|message| err(LucentErrorKind::QuerySyntaxError, message))
    }

    /// Interrupt whatever is running on this connection.
    ///
    /// Connection-scoped, not query-scoped: DuckDB has no notion of cancelling
    /// one specific statement, so the caller must confirm the query it wants to
    /// cancel is the one in flight.
    pub fn interrupt(&self) {
        self.interrupt.interrupt();
    }

    /// Clone the inner Arc for a `spawn_blocking` closure that needs `'static`.
    pub fn conn_arc(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }
}

fn err(kind: LucentErrorKind, message: impl Into<String>) -> LucentError {
    LucentError::new(kind, message)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::DuckHandle;

    #[test]
    fn the_handle_is_send_and_sync_so_it_can_live_behind_the_connector_trait() {
        // `duckdb::Connection` is Send but !Sync, and `Connector` requires
        // Send + Sync with &self. This assertion is the whole reason the
        // connection lives behind a Mutex.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<DuckHandle>();
        assert_send_sync::<Arc<DuckHandle>>();
    }

    #[tokio::test]
    async fn runs_a_query_and_returns_its_result() {
        let handle = DuckHandle::open(":memory:", false).expect("open in-memory");
        let answer: i64 = handle
            .with_conn(|conn| {
                conn.query_row("SELECT 42", [], |row| row.get(0))
                    .map_err(|e| e.to_string())
            })
            .expect("query");
        assert_eq!(answer, 42);
    }

    #[tokio::test]
    async fn a_read_only_handle_refuses_writes_at_the_engine_level() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ro.duckdb");
        let path_str = path.to_string_lossy().to_string();

        // Create the file with a read-write handle, then drop it so the file
        // lock is released.
        {
            let rw = DuckHandle::open(&path_str, false).expect("open read-write");
            rw.with_conn(|conn| {
                conn.execute_batch("CREATE TABLE t (x int); INSERT INTO t VALUES (1)")
                    .map_err(|e| e.to_string())
            })
            .expect("seed");
        }

        let ro = DuckHandle::open(&path_str, true).expect("open read-only");
        let count: i64 = ro
            .with_conn(|conn| {
                conn.query_row("SELECT count(*) FROM t", [], |row| row.get(0))
                    .map_err(|e| e.to_string())
            })
            .expect("read");
        assert_eq!(count, 1, "a read-only handle must still read");

        let write = ro.with_conn(|conn| {
            conn.execute_batch("INSERT INTO t VALUES (2)")
                .map_err(|e| e.to_string())
        });
        assert!(
            write.is_err(),
            "access_mode = READ_ONLY must make the ENGINE refuse the write — \
             this is what makes SessionFlag a real guarantee"
        );
    }

    #[tokio::test]
    async fn the_interrupt_handle_works_while_the_connection_lock_is_held() {
        // The critical property. A long query holds the connection Mutex; a
        // cancel that needed that same lock would block behind the query it is
        // trying to kill. The interrupt handle is captured at open time and
        // lives OUTSIDE the mutex precisely so this works.
        let handle = Arc::new(DuckHandle::open(":memory:", false).expect("open"));

        let runner = handle.clone();
        let query = tokio::task::spawn_blocking(move || {
            runner.with_conn(|conn| {
                // Large enough to still be running when the interrupt lands.
                conn.execute_batch("SELECT count(*) FROM range(1, 200000000) t1, range(1, 100) t2")
                    .map_err(|e| e.to_string())
            })
        });

        tokio::time::sleep(Duration::from_millis(200)).await;
        handle.interrupt();

        let result = tokio::time::timeout(Duration::from_secs(20), query)
            .await
            .expect("the interrupt must land — a hang here means cancel deadlocks")
            .expect("join");
        assert!(result.is_err(), "an interrupted query must report an error");
    }
}
