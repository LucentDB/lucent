//! Regression test for the startup-panic fix: `LUCENT_CONFIG_DIR` pointing at
//! an uncreatable path must not crash the app before the window opens — the
//! subscriber degrades to stdout-only and keeps running.
//!
//! Own test binary: it installs the global subscriber, which must not pollute
//! the other integration tests.
//!
//! The path is made uncreatable by pointing it THROUGH a regular file
//! (ENOTDIR on every OS) — a chmod 0555 dir would be ignored by root and is
//! meaningless on Windows.

use lucent_lib::trace::init_tracing;

#[test]
fn init_tracing_survives_unwritable_log_dir() {
    let dir = tempfile::TempDir::new().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    let uncreatable = blocker.join("sub").join("lucent");

    std::env::set_var("LUCENT_CONFIG_DIR", uncreatable);

    // Must not panic; no file layer exists, so no guard to keep alive.
    let guard = init_tracing();
    assert!(
        guard.is_none(),
        "uncreatable log dir must fall back to stdout-only"
    );

    // The subscriber and bridge still work after the fallback.
    log::info!("trace smoke: stdout-only fallback alive");
}
