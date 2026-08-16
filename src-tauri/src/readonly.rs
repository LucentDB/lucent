//! Capability-driven read-only enforcement.
//!
//! Layer 1 is `ai::guard::validate_readonly`, which always runs. Layer 2 is
//! whatever the engine can be made to enforce, which is what this module
//! negotiates. When there is no layer 2, `ReadOnlyMode::disclosure()` says so
//! — to the user, to the model, and to the log.

use lucent_protocol::{ConnectionId, DriverCapabilities, ReadOnlyMode, TimeoutSupport};

use crate::client::ConnectorClient;

/// The statements that open a read-only scope, in order. Empty means "this
/// engine offers nothing to open".
pub(crate) fn setup_statements(
    readonly: ReadOnlyMode,
    timeout: TimeoutSupport,
    timeout_ms: u64,
) -> Vec<String> {
    let mut out = Vec::new();
    match readonly {
        ReadOnlyMode::TransactionScoped => {
            out.push("BEGIN".to_string());
            out.push("SET TRANSACTION READ ONLY".to_string());
        }
        // Script-scoped transactions are opened by the driver around the whole
        // script, not by us; a session flag is set at connect time. Neither has
        // a per-query statement to issue.
        ReadOnlyMode::ScriptScoped | ReadOnlyMode::SessionFlag => {}
        // No engine enforcement exists. Emitting BEGIN here would open a real
        // read-WRITE transaction and leave it open — strictly worse than
        // emitting nothing.
        ReadOnlyMode::GuardOnly => return Vec::new(),
        _ => return Vec::new(),
    }

    // SET LOCAL is only valid inside an explicit transaction, and only means
    // anything for engines with a server-side statement timeout.
    if timeout == TimeoutSupport::Statement && timeout_ms > 0 && !out.is_empty() {
        out.push(format!("SET LOCAL statement_timeout = {timeout_ms}"));
    }
    out
}

/// The statements that close the scope. Always a rollback where a transaction
/// was opened — a read-only transaction has nothing to commit.
pub(crate) fn teardown_statements(readonly: ReadOnlyMode) -> Vec<String> {
    match readonly {
        ReadOnlyMode::TransactionScoped => vec!["ROLLBACK".to_string()],
        _ => Vec::new(),
    }
}

/// RAII handle for a read-only scope. Dropping it runs the teardown, on every
/// exit path including `?` and panic unwind.
pub struct ReadOnlySession {
    client: ConnectorClient,
    conn_id: ConnectionId,
    teardown: Vec<String>,
}

impl ReadOnlySession {
    /// Open the strongest read-only scope this connection supports.
    ///
    /// Returns `Ok` even when the engine offers nothing — the caller has
    /// already run the AST guard, and `capabilities.readonly.disclosure()` is
    /// how the weakened guarantee reaches the user and the model.
    pub async fn begin(
        client: &ConnectorClient,
        conn_id: ConnectionId,
        capabilities: &DriverCapabilities,
        timeout_ms: u64,
    ) -> Result<Self, String> {
        let setup = setup_statements(
            capabilities.readonly,
            capabilities.statement_timeout,
            timeout_ms,
        );

        let mut opened: Vec<String> = Vec::new();
        for (i, stmt) in setup.iter().enumerate() {
            if let Err(e) = client.execute(conn_id, stmt).await {
                // A failed BEGIN means no transaction to roll back. A failure
                // after BEGIN means there is one, and it must not be leaked.
                if i > 0 {
                    for stmt in teardown_statements(capabilities.readonly) {
                        let _ = client.execute(conn_id, &stmt).await;
                    }
                }
                return Err(format!("failed to open a read-only scope ({stmt}): {e}"));
            }
            opened.push(stmt.clone());
        }

        Ok(Self {
            client: client.clone(),
            conn_id,
            teardown: if opened.is_empty() {
                Vec::new()
            } else {
                teardown_statements(capabilities.readonly)
            },
        })
    }

    /// Run the teardown synchronously (awaited). Prefer this over drop:
    /// drop spawns the teardown, which can race the caller's next
    /// transaction on the same connection — if the caller's next BEGIN wins
    /// the writer lock, the spawned ROLLBACK aborts the NEW transaction
    /// (C7). Drop remains as a best-effort fallback for error paths.
    pub async fn close(mut self) {
        if self.teardown.is_empty() {
            return;
        }
        let statements = std::mem::take(&mut self.teardown);
        for stmt in statements {
            let _ = self.client.execute(self.conn_id, &stmt).await;
        }
    }
}

impl Drop for ReadOnlySession {
    fn drop(&mut self) {
        if self.teardown.is_empty() {
            return;
        }
        // Drop is not async. Spawn the teardown, matching the pattern the old
        // `ProbeTimeoutGuard` / `ReadonlyTxnGuard` in `ai/preflight.rs` and
        // `ai/tools/execute.rs` established.
        let client = self.client.clone();
        let conn_id = self.conn_id;
        let statements = std::mem::take(&mut self.teardown);
        tokio::spawn(async move {
            for stmt in statements {
                let _ = client.execute(conn_id, &stmt).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use lucent_protocol::{
        new_framed, read_message, write_message, ConnectionConfig, ReadOnlyMode, ResultShape,
        ServerInfo, TimeoutSupport, WorkerRequest, WorkerResponse,
    };
    use std::sync::Arc;
    use std::time::Duration;

    use super::{setup_statements, teardown_statements, ReadOnlySession};
    use crate::client::ConnectorClient;

    #[test]
    fn transaction_scoped_opens_a_read_only_transaction_with_a_local_timeout() {
        let stmts = setup_statements(
            ReadOnlyMode::TransactionScoped,
            TimeoutSupport::Statement,
            5000,
        );
        assert_eq!(
            stmts,
            vec![
                "BEGIN".to_string(),
                "SET TRANSACTION READ ONLY".to_string(),
                "SET LOCAL statement_timeout = 5000".to_string(),
            ]
        );
        assert_eq!(
            teardown_statements(ReadOnlyMode::TransactionScoped),
            vec!["ROLLBACK".to_string()],
            "a read-only transaction must always roll back, never commit"
        );
    }

    #[test]
    fn a_zero_timeout_emits_no_timeout_statement() {
        let stmts = setup_statements(
            ReadOnlyMode::TransactionScoped,
            TimeoutSupport::Statement,
            0,
        );
        assert_eq!(
            stmts,
            vec!["BEGIN".to_string(), "SET TRANSACTION READ ONLY".to_string()]
        );
    }

    #[test]
    fn guard_only_issues_nothing_at_all() {
        // The critical case. Sending BEGIN to an engine with no read-only
        // transaction mode would open a real read-WRITE transaction and leave
        // it open — strictly worse than sending nothing.
        assert!(
            setup_statements(ReadOnlyMode::GuardOnly, TimeoutSupport::Interrupt, 5000).is_empty()
        );
        assert!(teardown_statements(ReadOnlyMode::GuardOnly).is_empty());
    }

    #[test]
    fn interrupt_timeouts_are_not_expressed_as_sql() {
        // TimeoutSupport::Interrupt means the client sets a deadline. Emitting
        // `SET LOCAL statement_timeout` for it would be a syntax error.
        let stmts = setup_statements(
            ReadOnlyMode::TransactionScoped,
            TimeoutSupport::Interrupt,
            5000,
        );
        assert!(
            !stmts.iter().any(|s| s.contains("statement_timeout")),
            "got {stmts:?}"
        );
    }

    #[test]
    fn session_flag_needs_no_transaction_wrap() {
        assert!(setup_statements(ReadOnlyMode::SessionFlag, TimeoutSupport::None, 0).is_empty());
        assert!(teardown_statements(ReadOnlyMode::SessionFlag).is_empty());
    }

    /// A `ReadOnlySession` must roll back on drop, on every exit path — the
    /// guarantee the pre-flight probe and the AI tool depend on. This is the
    /// direct unit-level version of the integration regression test
    /// (`connector_concurrent_test::test_preflight_probe_cannot_leak_statement_timeout`),
    /// replacing the `ProbeTimeoutGuard` / `ReadonlyTxnGuard` drop tests the
    /// ladder removes.
    #[tokio::test]
    async fn readonly_session_rolls_back_on_drop() {
        let dir = tempfile::TempDir::new().unwrap();
        let socket_path = dir.path().join("worker.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut framed = new_framed(stream);
            let _version: u32 = read_message(&mut framed).await.unwrap().unwrap();
            let _token: String = read_message(&mut framed).await.unwrap().unwrap();
            // Handshake ack (protocol v6): the client reads this before sending
            // any request.
            write_message(&mut framed, &WorkerResponse::HandshakeAccepted)
                .await
                .unwrap();
            let request: WorkerRequest = read_message(&mut framed).await.unwrap().unwrap();
            let connection_id = match request {
                WorkerRequest::Connect { connection_id, .. } => connection_id,
                other => panic!("expected Connect, got {other:?}"),
            };
            write_message(
                &mut framed,
                &WorkerResponse::Connected {
                    connection_id,
                    server_info: ServerInfo {
                        version: "fake".into(),
                        capabilities: fake_capabilities(),
                    },
                },
            )
            .await
            .unwrap();

            // The three setup statements, then the teardown after drop.
            let mut seen_rollback = false;
            let mut tx = Some(tx);
            for _ in 0..4 {
                let request: WorkerRequest = read_message(&mut framed).await.unwrap().unwrap();
                let (query_id, command) = match request {
                    WorkerRequest::Execute {
                        query_id, command, ..
                    } => (query_id, command),
                    other => panic!("expected Execute, got {other:?}"),
                };
                if command == "ROLLBACK" {
                    seen_rollback = true;
                    if let Some(tx_send) = tx.take() {
                        let _ = tx_send.send(());
                    }
                } else {
                    assert!(
                        matches!(
                            command.as_str(),
                            "BEGIN"
                                | "SET TRANSACTION READ ONLY"
                                | "SET LOCAL statement_timeout = 500"
                        ),
                        "unexpected setup statement: {command}"
                    );
                }
                write_message(
                    &mut framed,
                    &WorkerResponse::ResultBatch {
                        query_id,
                        shape: ResultShape::Tabular {
                            columns: Arc::new(vec![]),
                            rows: vec![],
                        },
                        sequence: 0,
                        is_final: true,
                    },
                )
                .await
                .unwrap();
            }
            assert!(seen_rollback, "fake worker must receive ROLLBACK");
        });

        let (client, conn_id) = ConnectorClient::connect(
            socket_path.to_str().unwrap(),
            "test-token",
            ConnectionConfig::default(),
        )
        .await
        .expect("connect to fake worker");

        let caps = fake_capabilities();
        {
            let _session = ReadOnlySession::begin(&client, conn_id, &caps, 500)
                .await
                .expect("begin must succeed against the fake worker");
            // Dropped here — the spawned teardown must issue ROLLBACK.
        }
        tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("fake worker must receive ROLLBACK after session drop")
            .expect("channel");
        server.await.unwrap();
        let mut client = client;
        let _ = client.shutdown().await;
    }

    /// C7: close() must run the teardown to completion BEFORE returning —
    /// the spawned Drop teardown races the caller's next transaction on the
    /// same connection (if the caller's BEGIN wins the writer lock, the
    /// spawned ROLLBACK aborts the NEW transaction). The fake worker records
    /// when it has RECEIVED the ROLLBACK; close() returning after that
    /// proves the round trip completed inside close().
    #[tokio::test]
    async fn close_awaits_the_rollback_round_trip() {
        let dir = tempfile::TempDir::new().unwrap();
        let socket_path = dir.path().join("worker.sock");
        let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
        let rollback_seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rollback_seen_worker = rollback_seen.clone();

        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut framed = new_framed(stream);
            let _version: u32 = read_message(&mut framed).await.unwrap().unwrap();
            let _token: String = read_message(&mut framed).await.unwrap().unwrap();
            write_message(&mut framed, &WorkerResponse::HandshakeAccepted)
                .await
                .unwrap();
            let request: WorkerRequest = read_message(&mut framed).await.unwrap().unwrap();
            let connection_id = match request {
                WorkerRequest::Connect { connection_id, .. } => connection_id,
                other => panic!("expected Connect, got {other:?}"),
            };
            write_message(
                &mut framed,
                &WorkerResponse::Connected {
                    connection_id,
                    server_info: ServerInfo {
                        version: "fake".into(),
                        capabilities: fake_capabilities(),
                    },
                },
            )
            .await
            .unwrap();

            // Three setup statements, then the ROLLBACK from close().
            let mut seen_rollback = false;
            for _ in 0..4 {
                let request: WorkerRequest = read_message(&mut framed).await.unwrap().unwrap();
                let (query_id, command) = match request {
                    WorkerRequest::Execute {
                        query_id, command, ..
                    } => (query_id, command),
                    other => panic!("expected Execute, got {other:?}"),
                };
                if command == "ROLLBACK" {
                    seen_rollback = true;
                    rollback_seen_worker.store(true, std::sync::atomic::Ordering::SeqCst);
                } else {
                    assert!(
                        matches!(
                            command.as_str(),
                            "BEGIN"
                                | "SET TRANSACTION READ ONLY"
                                | "SET LOCAL statement_timeout = 500"
                        ),
                        "unexpected setup statement: {command}"
                    );
                }
                write_message(
                    &mut framed,
                    &WorkerResponse::ResultBatch {
                        query_id,
                        shape: ResultShape::Tabular {
                            columns: Arc::new(vec![]),
                            rows: vec![],
                        },
                        sequence: 0,
                        is_final: true,
                    },
                )
                .await
                .unwrap();
            }
            assert!(seen_rollback, "fake worker must receive ROLLBACK");
        });

        let (client, conn_id) = ConnectorClient::connect(
            socket_path.to_str().unwrap(),
            "test-token",
            ConnectionConfig::default(),
        )
        .await
        .expect("connect to fake worker");

        let caps = fake_capabilities();
        let session = ReadOnlySession::begin(&client, conn_id, &caps, 500)
            .await
            .expect("begin must succeed against the fake worker");
        session.close().await;

        assert!(
            rollback_seen.load(std::sync::atomic::Ordering::SeqCst),
            "close() must not return before the ROLLBACK round trip completes"
        );
        server.await.unwrap();
        let mut client = client;
        let _ = client.shutdown().await;
    }

    fn fake_capabilities() -> lucent_protocol::DriverCapabilities {
        lucent_protocol::DriverCapabilities {
            id: "fake".into(),
            display_name: "Fake".into(),
            sql_dialect: lucent_protocol::SqlDialect::PostgreSql,
            namespace_model: lucent_protocol::NamespaceModel::DbSchemaObject,
            readonly: ReadOnlyMode::TransactionScoped,
            statement_timeout: TimeoutSupport::Statement,
            cancel: lucent_protocol::CancelMode::Native,
            paging: lucent_protocol::PagingStyle::LimitOffset,
            identifier_quote: '"',
            string_literal: lucent_protocol::StringLiteralStyle::StandardConforming,
            auth: lucent_protocol::AuthModel::UserPassword,
        }
    }
}
