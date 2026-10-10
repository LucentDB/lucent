//! Result handles (§6A.3): the per-scope LRU of materialized query results
//! that `fetch_rows` pages, plus the per-scope in-flight registry Stop uses
//! (§6A.6). Memory only — never written to disk.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use lucent_protocol::{ConnectionId, QueryId};

use crate::ai::events::ColumnMeta;

pub const MAX_RESULTS_PER_SCOPE: usize = 8;
pub const MAX_BYTES_PER_SCOPE: usize = 32 * 1024 * 1024;
pub const MAX_BYTES_GLOBAL: usize = 128 * 1024 * 1024;
/// Evicted ids remembered per scope, so a late `fetch_rows` hears "expired"
/// rather than "unknown". Older evictions fall back to "unknown".
const EVICTED_MEMORY: usize = 64;

pub const UNKNOWN_RESULT_MSG: &str = "unknown result_id; run the query first";
pub const CANCELLED_BY_USER: &str = "cancelled by user — no rows retained";

pub fn expired_result_msg(id: &str) -> String {
    format!("result {id} expired — re-run the query (narrow it if possible).")
}

/// `r_` + 16 lowercase hex = 64 bits from the OS CSPRNG (uuid v4 draws from
/// `getrandom`). Unguessable ids are the only fence inside the shared external
/// scope (§6A.3). v4 fixes the version nibble (byte 6) and the variant bits
/// (byte 8), so take bytes that are fully random: 0–5 and 10–11.
pub fn new_result_id() -> String {
    let b = uuid::Uuid::new_v4().into_bytes();
    let random = [b[0], b[1], b[2], b[3], b[4], b[5], b[10], b[11]];
    let mut id = String::with_capacity(18);
    id.push_str("r_");
    for byte in random {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

pub struct StoredResult {
    pub id: String,
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub sql: String,
    pub bytes: usize,
    /// False when the `row_limit + 1` probe showed more rows than are stored.
    pub complete: bool,
}

pub enum Lookup {
    Live(Arc<StoredResult>),
    Evicted,
    Unknown,
}

#[derive(Default)]
struct Scope {
    /// id → (result, last-touch tick). Ticks are store-global, so the global
    /// LRU can compare entries across scopes.
    results: HashMap<String, (Arc<StoredResult>, u64)>,
    bytes: usize,
    evicted: VecDeque<String>,
    in_flight: HashMap<QueryId, InFlightState>,
}

struct InFlightState {
    conn_id: ConnectionId,
    executing: bool,
    cancel_requested: bool,
}

#[derive(Default)]
struct Inner {
    scopes: HashMap<String, Scope>,
    tick: u64,
    total_bytes: usize,
}

pub struct ResultStore {
    inner: Mutex<Inner>,
    per_scope_results: usize,
    per_scope_bytes: usize,
    global_bytes: usize,
}

impl Default for ResultStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Cheap size estimate for the caps: names plus each value's text length.
fn approx_bytes(columns: &[ColumnMeta], rows: &[Vec<serde_json::Value>], sql: &str) -> usize {
    let cols: usize = columns
        .iter()
        .map(|c| c.name.len() + c.data_type.len())
        .sum();
    let cells: usize = rows
        .iter()
        .flat_map(|r| r.iter())
        .map(|v| match v {
            serde_json::Value::String(s) => s.len(),
            other => other.to_string().len(),
        })
        .sum();
    cols + cells + sql.len()
}

impl ResultStore {
    pub fn new() -> Self {
        Self::with_limits(MAX_RESULTS_PER_SCOPE, MAX_BYTES_PER_SCOPE, MAX_BYTES_GLOBAL)
    }

    pub fn with_limits(
        per_scope_results: usize,
        per_scope_bytes: usize,
        global_bytes: usize,
    ) -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            per_scope_results,
            per_scope_bytes,
            global_bytes,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Retain `rows` under a fresh id in `scope`. `None` when this single
    /// result exceeds a cap by itself: retaining it would break the bound, so
    /// the caller renders the block without a handle (§6A.3).
    pub fn store(
        &self,
        scope: &str,
        columns: Vec<ColumnMeta>,
        rows: Vec<Vec<serde_json::Value>>,
        sql: &str,
        complete: bool,
    ) -> Option<String> {
        let bytes = approx_bytes(&columns, &rows, sql);
        if bytes > self.per_scope_bytes || bytes > self.global_bytes {
            return None;
        }
        let id = new_result_id();
        let result = Arc::new(StoredResult {
            id: id.clone(),
            columns,
            rows,
            sql: sql.to_string(),
            bytes,
            complete,
        });
        let mut inner = self.lock();
        inner.tick += 1;
        let tick = inner.tick;
        let s = inner.scopes.entry(scope.to_string()).or_default();
        s.results.insert(id.clone(), (result, tick));
        s.bytes += bytes;
        inner.total_bytes += bytes;

        // Per-scope LRU — never the result just stored.
        loop {
            let s = &inner.scopes[scope];
            if s.results.len() <= self.per_scope_results && s.bytes <= self.per_scope_bytes {
                break;
            }
            let Some(victim) = lru_in(s, &id) else { break };
            evict(&mut inner, scope, &victim);
        }
        // Global LRU across scopes.
        while inner.total_bytes > self.global_bytes {
            let Some((victim_scope, victim)) = global_lru(&inner, &id) else {
                break;
            };
            evict(&mut inner, &victim_scope, &victim);
        }
        Some(id)
    }

    pub fn lookup(&self, scope: &str, id: &str) -> Lookup {
        let mut inner = self.lock();
        inner.tick += 1;
        let tick = inner.tick;
        let Some(s) = inner.scopes.get_mut(scope) else {
            return Lookup::Unknown;
        };
        if let Some((result, touched)) = s.results.get_mut(id) {
            *touched = tick;
            return Lookup::Live(result.clone());
        }
        if s.evicted.iter().any(|e| e == id) {
            Lookup::Evicted
        } else {
            Lookup::Unknown
        }
    }

    pub fn contains(&self, scope: &str, id: &str) -> bool {
        self.lock()
            .scopes
            .get(scope)
            .is_some_and(|s| s.results.contains_key(id))
    }

    /// The scope ended (conversation closed, cell run finished).
    pub fn clear_scope(&self, scope: &str) {
        let mut inner = self.lock();
        if let Some(s) = inner.scopes.remove(scope) {
            inner.total_bytes -= s.bytes;
        }
    }

    /// Connection switch / disconnect: drop every retained result. In-flight
    /// registrations stay — those queries are still running and Stop must
    /// still reach them.
    pub fn clear_results(&self) {
        let mut inner = self.lock();
        for s in inner.scopes.values_mut() {
            s.results.clear();
            s.evicted.clear();
            s.bytes = 0;
        }
        inner.total_bytes = 0;
    }

    /// Stop: mark every in-flight query in `scope`; return only the ones
    /// already executing, for the caller's DB-native cancel. Queued ones bail
    /// in `begin_execute` without ever sending their statement.
    pub fn cancel_scope(&self, scope: &str) -> Vec<(QueryId, ConnectionId)> {
        let mut inner = self.lock();
        let Some(s) = inner.scopes.get_mut(scope) else {
            return Vec::new();
        };
        let mut executing: Vec<(QueryId, ConnectionId)> = Vec::new();
        for (query_id, state) in s.in_flight.iter_mut() {
            state.cancel_requested = true;
            if state.executing {
                executing.push((*query_id, state.conn_id));
            }
        }
        executing
    }

    pub fn executing_count(&self, scope: &str) -> usize {
        self.lock()
            .scopes
            .get(scope)
            .map_or(0, |s| s.in_flight.values().filter(|f| f.executing).count())
    }

    fn register(&self, scope: &str, query_id: QueryId, conn_id: ConnectionId) {
        self.lock()
            .scopes
            .entry(scope.to_string())
            .or_default()
            .in_flight
            .insert(
                query_id,
                InFlightState {
                    conn_id,
                    executing: false,
                    cancel_requested: false,
                },
            );
    }

    fn begin_execute(&self, scope: &str, query_id: QueryId) -> Result<(), String> {
        let mut inner = self.lock();
        let state = inner
            .scopes
            .get_mut(scope)
            .and_then(|s| s.in_flight.get_mut(&query_id));
        match state {
            Some(s) if s.cancel_requested => Err(CANCELLED_BY_USER.to_string()),
            Some(s) => {
                s.executing = true;
                Ok(())
            }
            None => Ok(()), // the scope was cleared meanwhile; nothing to fence
        }
    }

    fn finish(&self, scope: &str, query_id: QueryId) -> bool {
        self.lock()
            .scopes
            .get_mut(scope)
            .and_then(|s| s.in_flight.remove(&query_id))
            .is_some_and(|s| s.cancel_requested)
    }
}

fn lru_in(s: &Scope, keep: &str) -> Option<String> {
    s.results
        .iter()
        .filter(|(id, _)| id.as_str() != keep)
        .min_by_key(|(_, (_, tick))| *tick)
        .map(|(id, _)| id.clone())
}

fn global_lru(inner: &Inner, keep: &str) -> Option<(String, String)> {
    inner
        .scopes
        .iter()
        .flat_map(|(scope, s)| {
            s.results
                .iter()
                .map(move |(id, (_, tick))| (scope, id, *tick))
        })
        .filter(|(_, id, _)| id.as_str() != keep)
        .min_by_key(|(_, _, tick)| *tick)
        .map(|(scope, id, _)| (scope.clone(), id.clone()))
}

fn evict(inner: &mut Inner, scope: &str, id: &str) {
    if let Some(s) = inner.scopes.get_mut(scope) {
        if let Some((result, _)) = s.results.remove(id) {
            s.bytes -= result.bytes;
            inner.total_bytes -= result.bytes;
            s.evicted.push_back(id.to_string());
            if s.evicted.len() > EVICTED_MEMORY {
                s.evicted.pop_front();
            }
        }
    }
}

/// A surface's handle on the store: the scope id it is fenced to (§6A.3).
#[derive(Clone)]
pub struct ResultScope {
    pub scope_id: String,
    pub store: Arc<ResultStore>,
}

impl ResultScope {
    /// A fresh private scope — evals and tests.
    pub fn isolated(scope_id: impl Into<String>) -> Self {
        Self {
            scope_id: scope_id.into(),
            store: Arc::new(ResultStore::new()),
        }
    }

    /// Register a tool query BEFORE it waits for the connection's sequence
    /// lock, so a Stop that lands while it waits still reaches it.
    pub fn register(&self, conn_id: ConnectionId) -> InFlight {
        let query_id = QueryId(uuid::Uuid::new_v4());
        self.store.register(&self.scope_id, query_id, conn_id);
        InFlight {
            store: self.store.clone(),
            scope_id: self.scope_id.clone(),
            query_id,
            live: true,
        }
    }
}

/// One registered AI query. Deregisters on drop, so an early `?` cannot leak.
pub struct InFlight {
    store: Arc<ResultStore>,
    scope_id: String,
    pub query_id: QueryId,
    live: bool,
}

impl InFlight {
    /// Call immediately before the statement is sent. `Err(CANCELLED_BY_USER)`
    /// means Stop already asked — do not send it.
    pub fn begin_execute(&self) -> Result<(), String> {
        self.store.begin_execute(&self.scope_id, self.query_id)
    }

    /// Deregister; true when Stop asked to cancel this query.
    pub fn finish(mut self) -> bool {
        self.live = false;
        self.store.finish(&self.scope_id, self.query_id)
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        if self.live {
            self.store.finish(&self.scope_id, self.query_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucent_protocol::ConnectionId;
    use serde_json::json;

    fn row(s: &str) -> Vec<Vec<serde_json::Value>> {
        vec![vec![json!(s)]]
    }

    #[test]
    fn ids_are_r_prefixed_16_lowercase_hex_and_unique() {
        let a = new_result_id();
        let b = new_result_id();
        assert!(a.starts_with("r_") && a.len() == 18, "{a}");
        assert!(
            a[2..]
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{a}"
        );
        assert_ne!(a, b);
    }

    #[test]
    fn lookups_are_scope_fenced() {
        let store = ResultStore::new();
        let id = store
            .store("conv-a", vec![], row("1"), "SELECT 1", true)
            .unwrap();
        assert!(matches!(store.lookup("conv-a", &id), Lookup::Live(_)));
        assert!(
            matches!(store.lookup("conv-b", &id), Lookup::Unknown),
            "cross-scope = unknown"
        );
    }

    #[test]
    fn per_scope_lru_evicts_the_least_recently_used_and_remembers_it() {
        let store = ResultStore::with_limits(2, 1 << 20, 1 << 30);
        let a = store
            .store("s", vec![], row(&"a".repeat(64)), "q", true)
            .unwrap();
        let b = store
            .store("s", vec![], row(&"b".repeat(64)), "q", true)
            .unwrap();
        assert!(
            matches!(store.lookup("s", &a), Lookup::Live(_)),
            "touch a → b is now LRU"
        );
        let _c = store
            .store("s", vec![], row(&"c".repeat(64)), "q", true)
            .unwrap();
        assert!(
            matches!(store.lookup("s", &b), Lookup::Evicted),
            "LRU evicted, eviction remembered"
        );
        assert!(matches!(store.lookup("s", &a), Lookup::Live(_)));
    }

    #[test]
    fn global_cap_evicts_across_scopes() {
        let store = ResultStore::with_limits(8, 1 << 20, 600);
        let a = store
            .store("s1", vec![], row(&"a".repeat(200)), "q", true)
            .unwrap();
        let _b = store
            .store("s2", vec![], row(&"b".repeat(200)), "q", true)
            .unwrap();
        let _c = store
            .store("s3", vec![], row(&"c".repeat(200)), "q", true)
            .unwrap();
        assert!(
            matches!(store.lookup("s1", &a), Lookup::Evicted),
            "global LRU crossed scopes"
        );
    }

    #[test]
    fn a_result_larger_than_a_cap_is_not_retained() {
        let store = ResultStore::with_limits(8, 100, 1 << 20);
        assert!(store
            .store("s", vec![], row(&"x".repeat(500)), "q", true)
            .is_none());
    }

    #[test]
    fn stop_marks_queued_queries_and_returns_only_executing_ones() {
        let store = Arc::new(ResultStore::new());
        let scope = ResultScope {
            scope_id: "s1".into(),
            store: store.clone(),
        };
        let conn = ConnectionId(uuid::Uuid::new_v4());
        let running = scope.register(conn);
        let queued = scope.register(conn);
        running.begin_execute().unwrap();

        let to_cancel = store.cancel_scope("s1");
        assert_eq!(
            to_cancel,
            vec![(running.query_id, conn)],
            "only the executing one gets a native cancel"
        );
        assert_eq!(
            queued.begin_execute().unwrap_err(),
            CANCELLED_BY_USER,
            "a marked query never sends"
        );
        assert!(running.finish(), "finish reports the Stop");
        assert!(queued.finish());
        assert_eq!(store.executing_count("s1"), 0);
    }

    #[test]
    fn other_scopes_are_untouched_and_drop_deregisters() {
        let store = Arc::new(ResultStore::new());
        let a = ResultScope {
            scope_id: "a".into(),
            store: store.clone(),
        };
        let b = ResultScope {
            scope_id: "b".into(),
            store: store.clone(),
        };
        let conn = ConnectionId(uuid::Uuid::new_v4());
        let qa = a.register(conn);
        let qb = b.register(conn);
        qa.begin_execute().unwrap();
        qb.begin_execute().unwrap();
        assert_eq!(store.cancel_scope("a").len(), 1, "only scope a's query");
        assert!(!qb.finish(), "scope b was never asked to stop");
        drop(qa);
        assert!(
            store.cancel_scope("a").is_empty(),
            "a dropped InFlight deregistered"
        );
    }
}
