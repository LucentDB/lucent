//! Tracing initialization: global subscriber, `log` bridge, rotating file.
//!
//! The codebase emits diagnostics through the `log` crate macros; a
//! `LogTracer` installed here bridges those records into the `tracing`
//! subscriber, so spans added on key paths (`connect`, `ai_chat`,
//! `worker.execute`) correlate every log line within them without rewriting
//! existing `log::` call sites.

use std::path::PathBuf;
use std::sync::Once;

use tracing_appender::rolling::InitError;
use tracing_subscriber::prelude::*;

#[cfg(test)]
use std::{
    io,
    sync::{Arc, Mutex},
};

/// App log directory, mirroring the resolution in
/// `query_history.rs::history_file_path`: `$LUCENT_CONFIG_DIR/lucent`, else
/// `dirs::config_dir()/lucent`.
fn app_log_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("LUCENT_CONFIG_DIR") {
        let path = PathBuf::from(dir).join("lucent");
        std::fs::create_dir_all(&path).ok();
        return path;
    }
    let base = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lucent");
    std::fs::create_dir_all(&base).ok();
    base
}

/// Builds the daily-rotating file appender in `dir`. `build` opens the log
/// file eagerly, so an unwritable directory fails here as an `Err` instead of
/// panicking at startup — callers degrade to stdout-only logging.
fn build_file_appender(
    dir: &std::path::Path,
) -> Result<tracing_appender::rolling::RollingFileAppender, InitError> {
    tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("lucent.log")
        .build(dir)
}

/// Installs the global tracing subscriber and the `log` → `tracing` bridge.
///
/// Filtering comes from `RUST_LOG` (default `info,lucent=debug` — the global
/// `info` level is brief-mandated; third-party info logs can be quieted with
/// `RUST_LOG=warn,lucent=info`). Output goes to stdout and, when the app
/// config dir is writable, a daily-rotating `lucent.log` there; an unwritable
/// dir degrades to stdout-only. Safe to call more than once: the first call
/// installs the global state, later calls are no-ops.
///
/// Returns the file-writer worker guard on the first call only (bind it in
/// `run()` so the logging thread flushes on exit); `None` on later calls or
/// when file logging is unavailable.
pub fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    static INIT: Once = Once::new();
    let mut guard: Option<tracing_appender::non_blocking::WorkerGuard> = None;
    INIT.call_once(|| {
        guard = install();
    });
    guard
}

/// One-shot install; see [`init_tracing`]. Never panics on bad log dirs or a
/// double-install — every fallible step degrades to a warning.
fn install() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,lucent=debug"));

    let dir = app_log_dir();
    let mut file_layer = None;
    let mut guard: Option<tracing_appender::non_blocking::WorkerGuard> = None;
    let mut file_warn: Option<String> = None;
    match build_file_appender(&dir) {
        Ok(appender) => {
            let (file_writer, g) = tracing_appender::non_blocking(appender);
            file_layer = Some(
                tracing_subscriber::fmt::layer()
                    .with_target(true)
                    .with_file(true)
                    .with_line_number(true)
                    .with_ansi(false)
                    .with_writer(file_writer),
            );
            guard = Some(g);
        }
        Err(e) => file_warn = Some(e.to_string()),
    }

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_file(true)
        .with_line_number(true);

    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer);

    if let Err(e) = subscriber.try_init() {
        log::warn!("tracing: global subscriber already installed ({e}); leaving it as-is");
    }

    // Bridge `log::` records into the subscriber above as tracing events.
    // Per-target level filtering still happens in the EnvFilter layer; the
    // tracer only raises the global `log` max level so records reach it.
    if tracing_log::LogTracer::init().is_err() {
        log::warn!("tracing: log bridge already installed; log records will not be bridged");
    }

    if let Some(err) = file_warn {
        log::warn!("tracing: file logging disabled ({dir:?}): {err}; logging to stdout only");
    } else {
        log::info!("tracing initialized; log file at {dir:?}");
    }

    guard
}

/// Span for the `connect` command lifecycle: host/port/db of the resolved
/// connection config.
pub fn connect_span(host: &str, port: u16, database: &str) -> tracing::Span {
    tracing::info_span!("connect", host = %host, port = %port, db = %database)
}

/// Span for one `ai_chat` turn.
pub fn ai_chat_span(conversation_id: &str) -> tracing::Span {
    tracing::info_span!("ai_chat", conversation_id = %conversation_id)
}

/// Span for one worker execute round-trip on the IPC socket.
pub fn worker_execute_span(
    connection_id: &lucent_protocol::ConnectionId,
    query_id: &lucent_protocol::QueryId,
) -> tracing::Span {
    tracing::info_span!("worker.execute", conn_id = %connection_id.0, query_id = %query_id.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CaptureWriter(Arc<Mutex<Vec<u8>>>);

    impl io::Write for CaptureWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureWriter;

        fn make_writer(&'a self) -> Self::Writer {
            CaptureWriter(self.0.clone())
        }
    }

    fn capture_with_span(body: impl FnOnce()) -> String {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::fmt()
            .with_writer(CaptureWriter(buf.clone()))
            .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NEW)
            .finish();
        tracing::subscriber::with_default(subscriber, body);
        let out = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        out
    }

    #[test]
    fn connect_span_carries_host_port_db_fields() {
        let out = capture_with_span(|| {
            let span = connect_span("localhost", 5432, "postgres");
            let _entered = span.enter();
            tracing::info!("inside connect span");
        });
        assert!(out.contains("connect"), "output: {out}");
        assert!(out.contains("localhost"), "output: {out}");
        assert!(out.contains("5432"), "output: {out}");
        assert!(out.contains("postgres"), "output: {out}");
    }

    #[test]
    fn ai_chat_span_carries_conversation_id() {
        let out = capture_with_span(|| {
            let span = ai_chat_span("conv-42");
            let _entered = span.enter();
            tracing::info!("inside ai_chat span");
        });
        assert!(out.contains("ai_chat"), "output: {out}");
        assert!(out.contains("conv-42"), "output: {out}");
    }

    #[test]
    fn worker_execute_span_carries_conn_and_query_ids() {
        let conn_id = lucent_protocol::ConnectionId(uuid::Uuid::new_v4());
        let query_id = lucent_protocol::QueryId(uuid::Uuid::new_v4());
        let out = capture_with_span(|| {
            let span = worker_execute_span(&conn_id, &query_id);
            let _entered = span.enter();
            tracing::info!("inside worker.execute span");
        });
        assert!(out.contains("worker.execute"), "output: {out}");
        assert!(out.contains(&conn_id.0.to_string()), "output: {out}");
        assert!(out.contains(&query_id.0.to_string()), "output: {out}");
    }

    #[test]
    fn log_records_bridge_into_tracing() {
        // LogTracer needs a global `log` logger, which init_tracing installs —
        // but tests share a process, so instead drive the same dispatch path
        // `LogTracer::log` uses: dispatch_record emits into the current
        // subscriber. This proves bridged records carry the enclosing span.
        let out = capture_with_span(|| {
            let span = ai_chat_span("bridge-check");
            let _entered = span.enter();
            let record = log::Record::builder()
                .args(format_args!("bridged log line"))
                .level(log::Level::Info)
                .target("lucent_lib::commands")
                .build();
            tracing_log::format_trace(&record).unwrap();
        });
        assert!(out.contains("bridge-check"), "output: {out}");
        assert!(out.contains("bridged log line"), "output: {out}");
    }

    #[test]
    fn file_appender_fails_gracefully_on_unwritable_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let writable = dir.path().join("writable");
        std::fs::create_dir(&writable).unwrap();
        assert!(
            build_file_appender(&writable).is_ok(),
            "writable dir should build"
        );

        // build() opens the log file eagerly, so an uncreatable path must
        // surface as Err (which install() turns into a warn + stdout-only
        // fallback) rather than a startup panic. Path THROUGH a regular file
        // fails with ENOTDIR on every OS — unlike a chmod 0555 dir, which
        // root ignores and Windows doesn't support.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let uncreatable = blocker.join("sub").join("lucent.log");
        assert!(
            build_file_appender(&uncreatable).is_err(),
            "uncreatable log path should fail to build"
        );
    }
}
