//! What DuckDB actually supports, established by execution rather than by
//! reading documentation.
//!
//! Spec §5.3 left DuckDB's read-only mode open, to be "resolved empirically".
//! This file is that resolution. If DuckDB gains or loses these features
//! later, these tests fail loudly and the capability declaration gets
//! revisited — which is exactly the behaviour we want.
//!
//! Settled facts on `duckdb = "1.10505.0"` (verified below):
//!
//! - There is **no** `SET TRANSACTION READ ONLY` — transaction-scoped
//!   enforcement does not exist, so read-write connections declare
//!   `ReadOnlyMode::GuardOnly`.
//! - There is **no** server-side statement timeout (`max_execution_time` is
//!   rejected), so the driver declares `TimeoutSupport::Interrupt`.
//! - The file lock *does* forbid a read-only handle alongside a read-write
//!   handle **across processes**. Within **one process** — which is how this
//!   driver actually uses DuckDB, with every connection multiplexed as a task
//!   in a single worker process — a `READ_ONLY` handle **can** coexist with a
//!   `READ_WRITE` handle on the same file, and the read-only handle's
//!   engine-level enforcement still applies. The plan's premise ("the file
//!   lock forbids the pairing") is therefore true cross-process but false
//!   in-process.

use lucent_driver_duckdb::connection::DuckHandle;

fn memory() -> DuckHandle {
    DuckHandle::open(":memory:", false).expect("open in-memory")
}

#[test]
fn duckdb_has_no_read_only_transaction_mode() {
    // This is why read-write DuckDB connections declare GuardOnly rather than
    // TransactionScoped. Postgres accepts this statement; DuckDB does not.
    let handle = memory();
    handle
        .with_conn(|conn| conn.execute_batch("BEGIN").map_err(|e| e.to_string()))
        .expect("BEGIN is supported");

    let result = handle.with_conn(|conn| {
        conn.execute_batch("SET TRANSACTION READ ONLY")
            .map_err(|e| e.to_string())
    });
    assert!(
        result.is_err(),
        "if this ever succeeds, DuckDB gained read-only transactions and \
         capabilities::duckdb() should be upgraded to TransactionScoped"
    );

    let _ = handle.with_conn(|conn| conn.execute_batch("ROLLBACK").map_err(|e| e.to_string()));
}

#[test]
fn duckdb_has_no_server_side_statement_timeout() {
    // This is why the declaration says TimeoutSupport::Interrupt. The spec
    // claimed `max_execution_time` shipped in January 2026; it is not in the
    // configuration reference and is not accepted here.
    let handle = memory();
    let result = handle.with_conn(|conn| {
        conn.execute_batch("SET max_execution_time = 1000")
            .map_err(|e| e.to_string())
    });
    assert!(
        result.is_err(),
        "if this ever succeeds, DuckDB gained a statement timeout and \
         capabilities::duckdb() should be upgraded to TimeoutSupport::Statement"
    );
}

#[test]
fn a_read_only_handle_can_coexist_with_a_read_write_handle_within_one_process() {
    // The plan claimed the file lock forbids a read-only handle alongside a
    // read-write one, period. Empirically that is only true across processes:
    // within one process the pairing succeeds, and the read-only handle's
    // engine-level enforcement is still real. This is the architecture's
    // actual case — one worker process multiplexes every DuckDB connection as
    // a task — so the AI *can* hold an engine-enforced read-only session
    // alongside a read-write editor session in the same process.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("coexist.duckdb");
    let path_str = path.to_string_lossy().to_string();

    {
        let seed = DuckHandle::open(&path_str, false).expect("create");
        seed.with_conn(|conn| {
            conn.execute_batch("CREATE TABLE t (x int); INSERT INTO t VALUES (1)")
                .map_err(|e| e.to_string())
        })
        .expect("seed");
    }

    let writer = DuckHandle::open(&path_str, false).expect("hold read-write");
    let reader = DuckHandle::open(&path_str, true).expect(
        "a read-only handle CAN coexist with a read-write handle within one \
         process on this DuckDB version — if this fails, the file lock has \
         become per-connection and capabilities::duckdb()'s comment should be \
         revisited",
    );

    // The read-only handle must still read...
    let count: i64 = reader
        .with_conn(|conn| {
            conn.query_row("SELECT count(*) FROM t", [], |row| row.get(0))
                .map_err(|e| e.to_string())
        })
        .expect("read through the coexisting read-only handle");
    assert_eq!(count, 1, "a read-only handle must still read");

    // ...must still refuse writes at the engine level...
    let write = reader.with_conn(|conn| {
        conn.execute_batch("INSERT INTO t VALUES (2)")
            .map_err(|e| e.to_string())
    });
    assert!(
        write.is_err(),
        "access_mode = READ_ONLY must make the ENGINE refuse the write even \
         while coexisting with a read-write handle — this is what makes \
         SessionFlag a real guarantee"
    );

    // ...and must not disturb the read-write handle's own write access.
    writer
        .with_conn(|conn| {
            conn.execute_batch("INSERT INTO t VALUES (3)")
                .map_err(|e| e.to_string())
        })
        .expect("the read-write handle stays writable while a read-only handle coexists");
}

#[test]
fn a_read_only_handle_cannot_coexist_with_a_read_write_handle_across_processes() {
    // The file lock IS real across processes: a process holding the file
    // read-write excludes a read-only open from another process. This is why
    // the AI cannot open a read-only session from a *separate* process — but
    // within the single worker process it can (see the in-process test above).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("xlocked.duckdb");
    let path_str = path.to_string_lossy().to_string();

    {
        let seed = DuckHandle::open(&path_str, false).expect("create");
        seed.with_conn(|conn| {
            conn.execute_batch("CREATE TABLE t (x int)")
                .map_err(|e| e.to_string())
        })
        .expect("seed");
    }

    // Spawn this same test binary as a child holding the read-write handle.
    let exe = std::env::current_exe().expect("current test binary");
    let mut child = std::process::Command::new(&exe)
        .args(["hold_rw_child_process", "--exact", "--nocapture"])
        .env("LUCENT_PROBE_FILE", &path_str)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn child holding the read-write handle");

    // Wait for the child's readiness marker.
    let mut reader = std::io::BufReader::new(child.stdout.take().expect("child stdout"));
    let mut line = String::new();
    use std::io::BufRead;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut ready = false;
    while std::time::Instant::now() < deadline {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if line.contains("CHILD_READY") {
            ready = true;
            break;
        }
    }
    assert!(ready, "child process never signalled readiness");

    let reader_handle = DuckHandle::open(&path_str, true);
    assert!(
        reader_handle.is_err(),
        "if this ever succeeds, DuckDB dropped the cross-process file lock and \
         the GuardOnly disclosure comment in capabilities::duckdb() should be \
         revisited"
    );

    // Reap the child so the suite does not wait out its sleep.
    let _ = child.kill();
    let _ = child.wait();
}

/// Child-process helper for the cross-process test: holds a read-write handle
/// on `LUCENT_PROBE_FILE`, signals readiness, then sleeps until killed.
///
/// Only meaningful when spawned by `a_read_only_handle_cannot_coexist_...`;
/// when the normal suite runs it standalone (no env var), it is a no-op pass.
#[test]
fn hold_rw_child_process() {
    let Ok(path) = std::env::var("LUCENT_PROBE_FILE") else {
        return;
    };
    let writer = DuckHandle::open(&path, false).expect("child holds the read-write handle");
    writer
        .with_conn(|conn| {
            conn.execute_batch("CREATE TABLE IF NOT EXISTS t (x int)")
                .map_err(|e| e.to_string())
        })
        .expect("child write");
    println!("CHILD_READY");
    std::io::Write::flush(&mut std::io::stdout()).ok();
    std::thread::sleep(std::time::Duration::from_secs(60));
    drop(writer);
}

#[test]
fn a_read_write_connection_declares_guard_only_and_a_read_only_one_declares_session_flag() {
    use lucent_protocol::ReadOnlyMode;

    assert_eq!(
        lucent_driver_duckdb::capabilities::duckdb(false).readonly,
        ReadOnlyMode::GuardOnly,
        "read-write DuckDB has no engine-level read-only enforcement"
    );
    assert_eq!(
        lucent_driver_duckdb::capabilities::duckdb(true).readonly,
        ReadOnlyMode::SessionFlag,
        "access_mode = READ_ONLY IS engine-enforced, for the whole connection"
    );
    assert!(
        !lucent_driver_duckdb::capabilities::duckdb(false)
            .readonly
            .is_engine_enforced(),
        "the disclosure path depends on this being false"
    );
}
