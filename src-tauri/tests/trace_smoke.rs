//! Smoke test for the tracing subscriber installed at startup: the global
//! subscriber + `log` bridge must install exactly once without panicking, a
//! second call must be a no-op, and a bridged `log` record must land in the
//! daily-rotating file.
//!
//! Runs as its own test binary so the global subscriber/logger state does not
//! leak into the lib unit tests.

use lucent_lib::trace::init_tracing;

#[test]
fn init_tracing_installs_once_and_bridges_log_records_to_file() {
    let dir = tempfile::TempDir::new().unwrap();
    std::env::set_var("LUCENT_CONFIG_DIR", dir.path());

    let first = init_tracing();
    assert!(
        first.is_some(),
        "first call should own the file-writer guard"
    );

    // Second call must be a no-op, not a panic (reviewer: double-init /
    // re-entrant setup, e.g. a test calling run() twice).
    let second = init_tracing();
    assert!(second.is_none(), "second call must be a no-op");

    log::info!("trace smoke: subscriber + bridge alive");
    drop(first); // flush the file-writer worker thread

    // rolling::daily writes `lucent.log.YYYY-MM-DD`, so glob for the prefix.
    let log_dir = dir.path().join("lucent");
    let log_files: Vec<_> = std::fs::read_dir(&log_dir)
        .unwrap_or_else(|e| panic!("log dir missing at {log_dir:?}: {e}"))
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("lucent.log")
        })
        .collect();
    assert!(
        !log_files.is_empty(),
        "no daily log file in {log_dir:?}: {:?}",
        std::fs::read_dir(&log_dir).map(|it| it.count())
    );
    let path = log_files[0].path();
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("rotating log file missing at {path:?}: {e}"));
    assert!(
        contents.contains("trace smoke: subscriber + bridge alive"),
        "bridged log record missing from file:\n{contents}"
    );
    assert!(contents.contains("lucent_lib::trace"), "file:\n{contents}");
}
