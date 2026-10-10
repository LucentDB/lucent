//! The one way an AI tool runs SQL on the worker (F16). On `Statement`
//! engines the bound is the `SET LOCAL statement_timeout` the caller's
//! `ReadOnlySession` already issued; on `Interrupt` engines (DuckDB) there is
//! no server-side timeout, so this function IS the bound. Feature 2 adds
//! in-flight registration here, so every AI query becomes cancellable by Stop.

use lucent_protocol::{ConnectionId, DriverCapabilities, QueryId, TimeoutSupport};

use crate::client::{ConnectorClient, ExecuteResult};

/// `timeout_ms == 0` means the caller chose no bound.
///
/// `in_flight` is the query's registration in its surface's result scope
/// (§6A.6): when present, the statement runs under the registration's query
/// id so Stop's native cancel addresses exactly this statement, and a Stop
/// that landed while the call waited for the sequence lock refuses to send
/// the statement at all.
pub(crate) async fn execute_bounded(
    client: &ConnectorClient,
    conn_id: ConnectionId,
    sql: &str,
    capabilities: &DriverCapabilities,
    timeout_ms: u64,
    max_rows: Option<usize>,
    in_flight: Option<&crate::ai::result_store::InFlight>,
) -> Result<ExecuteResult, String> {
    // Stop may have marked this query while it waited for the sequence lock:
    // then the statement is never sent (§6A.6).
    if let Some(f) = in_flight {
        f.begin_execute()?;
    }
    let query_id = in_flight.map_or_else(|| QueryId(uuid::Uuid::new_v4()), |f| f.query_id);
    if capabilities.statement_timeout == TimeoutSupport::Interrupt && timeout_ms > 0 {
        client
            .execute_with_deadline(query_id, conn_id, sql, max_rows, timeout_ms)
            .await
    } else {
        client
            .execute_with_id(query_id, conn_id, sql, max_rows)
            .await
            .map(|(result, _)| result)
    }
}
