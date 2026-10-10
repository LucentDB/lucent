//! Real-world integration tests (feature-gated: `integration-tests`).
//!
//! Tests validate tool-generated SQL against a real Postgres database.
//!
//! Usage:
//!   cargo test --package lucent --features integration-tests -- --nocapture
//!   OPENAI_API_KEY=sk-... \
//!     cargo test --package lucent --features integration-tests -- --nocapture --ignored

#![cfg(feature = "integration-tests")]

use crate::ai::agent::CachedQuery;
#[allow(unused_imports)]
use crate::ai::provider::LlmProvider;
use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use tokio_postgres::NoTls;

pub(crate) fn seed_sql() -> &'static str {
    "CREATE TABLE IF NOT EXISTS public.organizations (
        id SERIAL PRIMARY KEY, name TEXT, slug TEXT UNIQUE, is_active BOOLEAN DEFAULT true,
        plan TEXT DEFAULT 'free', created_at TIMESTAMP DEFAULT NOW(), settings JSONB DEFAULT '{}'
    );
    INSERT INTO public.organizations (name, slug, is_active, plan) VALUES
        ('Acme Corporation', 'acme-corp', true, 'enterprise'),
        ('Globex Inc.', 'globex', true, 'business'),
        ('Initech', 'initech', false, 'free')
    ON CONFLICT (slug) DO NOTHING;
    CREATE TABLE IF NOT EXISTS public.users (
        id SERIAL PRIMARY KEY, org_id INTEGER REFERENCES organizations(id),
        name TEXT, email TEXT UNIQUE, role TEXT DEFAULT 'member'
    );
    INSERT INTO public.users (org_id, name, email, role) VALUES
        (1, 'Wile E. Coyote', 'coyote@acme.com', 'admin'),
        (1, 'Road Runner', 'roadrunner@acme.com', 'member'),
        (2, 'Hank Scorpio', 'hank@globex.com', 'admin')
    ON CONFLICT (email) DO NOTHING;
    CREATE TABLE IF NOT EXISTS public.line_items (
        org_id INTEGER REFERENCES organizations(id),
        user_id INTEGER REFERENCES users(id),
        quantity INTEGER DEFAULT 1,
        PRIMARY KEY (org_id, user_id)
    );
    INSERT INTO public.line_items (org_id, user_id, quantity) VALUES
        (1, 1, 3), (1, 2, 5), (2, 3, 1)
    ON CONFLICT (org_id, user_id) DO NOTHING;"
}

async fn setup(name: &str) -> (impl Drop, tokio_postgres::Client) {
    let (_port, container, client) = setup_with_port(name).await;
    (container, client)
}

/// As `setup`, but also returns the container's host port so tests can drive
/// the worker seam (`pg_config(port)` + `ConnectorClient`).
pub(crate) async fn setup_with_port(name: &str) -> (u16, impl Drop, tokio_postgres::Client) {
    eprintln!("[{name}] Starting Postgres...");
    let c = testcontainers_modules::postgres::Postgres::default()
        .start()
        .await
        .expect("Postgres container");
    let port = c.get_host_port_ipv4(5432).await.unwrap();
    let conn_str =
        format!("host=127.0.0.1 port={port} user=postgres password=postgres dbname=postgres");
    eprintln!("[{name}] Ready on port {port}");

    let mut last_err = None;
    for i in 0..10 {
        match tokio_postgres::connect(&conn_str, NoTls).await {
            Ok((client, conn)) => {
                tokio::spawn(async move {
                    conn.await.ok();
                });
                return (port, c, client);
            }
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(500 * (i + 1))).await;
            }
        }
    }
    panic!("[{name}] Failed to connect: {}", last_err.unwrap());
}

/// Spawn the worker, connect an app-side `ConnectorClient`, and return it with
/// the connection id and the supervisor (caller must hold the supervisor and
/// shut it down). This is the exact seam the AI tools and the schema index now
/// consume — catalog questions go over this socket, not raw SQL.
pub(crate) async fn worker_client(
    port: u16,
) -> (
    crate::client::ConnectorClient,
    lucent_protocol::ConnectionId,
    crate::supervisor::Supervisor,
) {
    let mut supervisor = crate::supervisor::Supervisor::new();
    supervisor
        .ensure_running()
        .await
        .expect("supervisor running");
    let socket_path = supervisor.endpoint().to_string();
    let token = supervisor.handshake_token().to_owned();
    let (client, conn_id) =
        crate::client::ConnectorClient::connect(&socket_path, &token, pg_config(port))
            .await
            .expect("connect client");
    (client, conn_id, supervisor)
}

/// Blocks until the Postgres port accepts connections (containers can take
/// a few seconds past `start()` returning).
async fn wait_for_postgres(port: u16) {
    let conn_str =
        format!("host=127.0.0.1 port={port} user=postgres password=postgres dbname=postgres");
    let mut last_err = None;
    for i in 0..10 {
        match tokio_postgres::connect(&conn_str, NoTls).await {
            Ok((_client, conn)) => {
                tokio::spawn(async move {
                    conn.await.ok();
                });
                return;
            }
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(500 * (i + 1))).await;
            }
        }
    }
    panic!("Postgres never became ready: {last_err:?}");
}

#[tokio::test]
async fn real_query_acme() {
    let (_c, client) = setup("query_acme").await;
    client.batch_execute(seed_sql()).await.unwrap();
    let rows = client
        .query(
            "SELECT name, slug, plan FROM public.organizations \
             WHERE name ILIKE '%acme%' LIMIT 500",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, String>(0), "Acme Corporation");
    assert_eq!(rows[0].get::<_, String>(1), "acme-corp");
    assert_eq!(rows[0].get::<_, String>(2), "enterprise");
    eprintln!("  Found Acme Corporation");
}

/// Name search through the catalog seam — the same RPC `search_schema` will
/// use. The old raw query shape lived in `keyword_search_objects`, which moved
/// below the seam; this test now asserts the production path.
#[tokio::test]
async fn real_search_tables_by_name() {
    let (_port, _c, client) = setup_with_port("search_tables").await;
    client.batch_execute(seed_sql()).await.unwrap();

    let (mut worker, conn_id, mut supervisor) = worker_client(_port).await;
    let hits = worker
        .search_objects(conn_id, "org", vec![], None, 10)
        .await
        .expect("search objects");
    assert!(
        hits.iter()
            .any(|h| h.reference.name == "organizations" && h.column.is_none()),
        "search for 'org' must hit the organizations table: {hits:?}"
    );
    eprintln!("  Found {} hits matching 'org'", hits.len());

    let _ = worker.shutdown().await;
    let _ = supervisor.shutdown().await;
}

/// Column info through the catalog seam — `get_objects_info` now reads columns
/// from `describe_objects`, not an ad-hoc query.
#[tokio::test]
async fn real_column_info() {
    use lucent_protocol::{ObjectKind, ObjectRef};

    let (_port, _c, client) = setup_with_port("column_info").await;
    client.batch_execute(seed_sql()).await.unwrap();

    let (mut worker, conn_id, mut supervisor) = worker_client(_port).await;
    let details = worker
        .describe_objects(
            conn_id,
            vec![ObjectRef {
                namespace: vec!["public".into()],
                name: "organizations".into(),
                kind: ObjectKind::Table,
            }],
        )
        .await
        .expect("describe objects");
    assert_eq!(details.len(), 1, "one detail per requested object");
    let names: Vec<&str> = details[0].columns.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"settings"), "jsonb column: {names:?}");
    assert!(names.contains(&"plan"), "plan column: {names:?}");
    eprintln!("  Found {} columns", names.len());

    let _ = worker.shutdown().await;
    let _ = supervisor.shutdown().await;
}

#[tokio::test]
async fn real_blast_radius() {
    let (_c, client) = setup("blast_radius").await;
    client.batch_execute(seed_sql()).await.unwrap();
    let rows = client
        .query(
            "SELECT count(*) FROM public.organizations WHERE plan = 'free'",
            &[],
        )
        .await
        .unwrap();
    let count: i64 = rows[0].get(0);
    assert_eq!(count, 1);
    eprintln!("  Free plan: {count} organizations");
}

#[tokio::test]
async fn real_guard_rejects_dml() {
    let (_c, client) = setup("guard_dml").await;
    client.batch_execute(seed_sql()).await.unwrap();
    assert!(crate::ai::guard::validate_readonly(
        "DELETE FROM organizations WHERE plan='free'",
        lucent_protocol::SqlDialect::PostgreSql,
    )
    .is_err());
    let rows = client
        .query("SELECT count(*) FROM organizations", &[])
        .await
        .unwrap();
    let count: i64 = rows[0].get(0);
    assert_eq!(count, 3, "data intact");
}

/// Composite PK + individual FK columns must each appear exactly once in the
/// catalog's column listing, with the correct PK flag. The old columns query
/// (LEFT JOIN on key_column_usage) duplicated such columns — that query is gone;
/// `describe_objects` now answers from the driver's catalog RPCs, and this
/// test pins the same contract through the app's catalog seam.
#[tokio::test]
async fn composite_pk_and_fk_does_not_duplicate_columns_in_query() {
    use lucent_protocol::{ObjectKind, ObjectRef};

    let (_port, _c, client) = setup_with_port("no_dup_cols").await;
    client.batch_execute(seed_sql()).await.unwrap();

    let (mut worker, conn_id, mut supervisor) = worker_client(_port).await;
    let details = worker
        .describe_objects(
            conn_id,
            vec![ObjectRef {
                namespace: vec!["public".into()],
                name: "line_items".into(),
                kind: ObjectKind::Table,
            }],
        )
        .await
        .expect("describe objects");
    assert_eq!(details.len(), 1, "one detail for line_items");

    let cols = &details[0].columns;
    for col in ["org_id", "user_id"] {
        let occurrences = cols.iter().filter(|c| c.name == col).count();
        assert_eq!(
            occurrences, 1,
            "{col} (composite PK + individual FK) must appear exactly once, not duplicated per constraint: {cols:?}"
        );
        let c = cols.iter().find(|c| c.name == col).unwrap();
        assert!(c.is_primary_key, "{col} is part of the composite PK");
    }
    let quantity = cols.iter().find(|c| c.name == "quantity").unwrap();
    assert!(!quantity.is_primary_key, "quantity is not a PK column");
    eprintln!(
        "  describe_objects returns {} columns for line_items",
        cols.len()
    );

    let _ = worker.shutdown().await;
    let _ = supervisor.shutdown().await;
}

/// The PK flag must be deterministic through the catalog seam: composite-PK
/// columns report is_primary_key=true exactly once each, no ambiguity from a
/// naive dedup pass.
#[tokio::test]
async fn get_objects_info_query_reports_pk_consistently_for_composite_pk_and_fk_columns() {
    use lucent_protocol::{ObjectKind, ObjectRef};

    let (_port, _c, client) = setup_with_port("get_objects_info_pk_consistency").await;
    client.batch_execute(seed_sql()).await.unwrap();

    let (mut worker, conn_id, mut supervisor) = worker_client(_port).await;
    let details = worker
        .describe_objects(
            conn_id,
            vec![ObjectRef {
                namespace: vec!["public".into()],
                name: "line_items".into(),
                kind: ObjectKind::Table,
            }],
        )
        .await
        .expect("describe objects");
    assert_eq!(details.len(), 1, "one detail for line_items");

    let cols = &details[0].columns;
    for expected_pk_col in ["org_id", "user_id"] {
        let matches: Vec<bool> = cols
            .iter()
            .filter(|c| c.name == expected_pk_col)
            .map(|c| c.is_primary_key)
            .collect();
        assert_eq!(
            matches,
            vec![true],
            "{expected_pk_col} is part of the composite PK and must be reported as is_pk=true exactly once, deterministically: {cols:?}"
        );
    }

    let quantity_matches: Vec<bool> = cols
        .iter()
        .filter(|c| c.name == "quantity")
        .map(|c| c.is_primary_key)
        .collect();
    assert_eq!(quantity_matches, vec![false], "quantity is not a PK column");

    let _ = worker.shutdown().await;
    let _ = supervisor.shutdown().await;
}

#[tokio::test]
#[ignore = "requires OPENAI_API_KEY env var"]
async fn e2e_llm_tool_awareness() {
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY");
    let model = std::env::var("LUCENT_AI_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    let endpoint =
        std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| "https://opencode.ai/zen/go/v1".into());

    let provider = std::sync::Arc::new(crate::ai::providers::rig::RigProvider::new(
        crate::ai::config::AiProvider::OpenAI,
        api_key,
        Some(endpoint),
    ));

    let ctx = crate::ai::tools::AiToolContext {
        db: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        connection_id: None,
        memory_connection_key: None,
        capabilities: None,
        config: crate::ai::config::AiConfig::default(),
        schema_graph: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        embedder: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        reranker: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        memory_manager: crate::ai::tools::test_memory_manager(),
        result_scope: crate::ai::result_store::ResultScope::isolated("test"),
    };
    let tools = crate::ai::tools::all_tools(ctx);

    let agent = provider
        .build_agent(
            &model,
            "You are a database assistant. Be brief.".into(),
            2048,
            tools,
        )
        .await;

    let r = agent
        .complete(
            crate::ai::agent::Message::user("What tools do you have? List them briefly."),
            vec![],
            &|_| {},
        )
        .await
        .expect("LLM");

    eprintln!("  Text: {:?}", r.text.as_deref().unwrap_or("(none)"));
    eprintln!("  Tool calls: {}", r.tool_calls.len());
    eprintln!(
        "  Tokens: {} in / {} out",
        r.usage.prompt_tokens, r.usage.completion_tokens
    );
}

#[tokio::test]
#[ignore]
async fn anthropic_provider_completes_a_real_request() {
    let api_key =
        std::env::var("ANTHROPIC_API_KEY").expect("set ANTHROPIC_API_KEY to run this ignored test");
    let provider = crate::ai::providers::rig::RigProvider::new(
        crate::ai::config::AiProvider::Anthropic,
        api_key,
        None,
    );
    let agent = provider
        .build_agent(
            "claude-3-5-haiku-latest",
            "You are a terse test assistant.".to_string(),
            64,
            vec![],
        )
        .await;
    let response = agent
        .complete(
            crate::ai::agent::Message {
                role: crate::ai::agent::MessageRole::User,
                content: crate::ai::agent::MessageContent::Text(
                    "Reply with exactly the word: pong".to_string(),
                ),
                tool_calls: None,
            },
            vec![],
            &|_delta| {},
        )
        .await
        .expect("a real Anthropic call with a valid key should succeed");
    assert!(
        response.text.is_some(),
        "this is the exact regression this task fixes: before Task 7, selecting \
         Anthropic silently called OpenAI's API instead and this assertion would \
         either fail with an OpenAI auth error (using an Anthropic-shaped key) or \
         hang on a network layer mismatch"
    );
}

/// The driver catalog answers partition metadata and flags partition children;
/// `harvest_to_entries` collapses children into their parent. Drive the real
/// seam — worker binary → catalog RPC → graph harvest — and assert the same
/// facts the old build_index Step 0 query used to provide.
#[tokio::test]
async fn partitioned_table_metadata_query_collapses_children() {
    use crate::ai::schema_graph::harvest_to_entries;
    use crate::client::ConnectorClient;
    use crate::supervisor::Supervisor;
    use lucent_protocol::ObjectKind;

    let container = testcontainers_modules::postgres::Postgres::default()
        .start()
        .await
        .expect("postgres container to start");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get postgres port");
    wait_for_postgres(port).await;

    // Seed through a raw session — the worker's own execute() rejects
    // multi-statement DDL.
    let conn_str =
        format!("host=127.0.0.1 port={port} user=postgres password=postgres dbname=postgres");
    let (raw, conn) = tokio_postgres::connect(&conn_str, NoTls).await.unwrap();
    tokio::spawn(async move {
        conn.await.ok();
    });
    raw.batch_execute(
        "CREATE TABLE IF NOT EXISTS public.events (
             id BIGINT, created_at DATE NOT NULL, kind TEXT
         ) PARTITION BY RANGE (created_at);
         CREATE TABLE IF NOT EXISTS public.events_2025
             PARTITION OF public.events FOR VALUES FROM ('2025-01-01') TO ('2026-01-01');
         CREATE TABLE IF NOT EXISTS public.events_2026
             PARTITION OF public.events FOR VALUES FROM ('2026-01-01') TO ('2027-01-01');",
    )
    .await
    .unwrap();

    let mut supervisor = Supervisor::new();
    supervisor
        .ensure_running()
        .await
        .expect("supervisor running");
    let socket_path = supervisor.endpoint().to_string();
    let token = supervisor.handshake_token().to_owned();
    let (mut client, conn_id) = ConnectorClient::connect(&socket_path, &token, pg_config(port))
        .await
        .expect("connect client");

    // The catalog RPC is the exact path build_index's harvest now consumes.
    let objects = client
        .list_all_objects(conn_id, vec![ObjectKind::Table])
        .await
        .expect("list all objects");

    let parent = objects
        .iter()
        .find(|o| o.reference.name == "events")
        .expect("parent present");
    assert!(
        !parent.is_partition_child,
        "parent is not a partition child"
    );
    let partition = parent
        .partition
        .as_ref()
        .expect("parent reports its partition metadata");
    assert_eq!(
        partition.child_count, 2,
        "parent reports its partition count: {partition:?}"
    );
    assert!(
        partition.key.as_deref().unwrap_or("").contains("RANGE"),
        "partition key def: {partition:?}"
    );

    for child in ["events_2025", "events_2026"] {
        let row = objects
            .iter()
            .find(|o| o.reference.name == child)
            .expect("child row");
        assert!(
            row.is_partition_child,
            "{child} must be flagged as a partition child (and excluded from the graph)"
        );
    }

    // The collapse itself: children are dropped, the parent keeps its
    // annotation — the exact behavior the old raw query fed into the graph.
    let (tables, _) = harvest_to_entries(objects, vec![]);
    let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["events"],
        "partition children must be collapsed into the parent: {names:?}"
    );
    assert!(
        tables[0]
            .partition_info
            .as_deref()
            .unwrap_or("")
            .contains("RANGE"),
        "parent keeps its partition annotation: {:?}",
        tables[0].partition_info
    );

    client.shutdown().await.expect("shutdown");
    let _ = supervisor.shutdown().await;
}

// ── C1: execute_dml / execute_staged_dml against the real worker ────────────

fn pg_config(port: u16) -> lucent_protocol::ConnectionConfig {
    lucent_protocol::ConnectionConfig::new("postgres")
        .with("host", "127.0.0.1")
        .with("port", port.to_string())
        .with("user", "postgres")
        .with("database", "postgres")
        .with("ssl_mode", "prefer")
        .with_secret("postgres")
}

/// Regression test for C1: approving a DML card must actually execute the
/// staged SQL and report the REAL affected count (Task 3.1's rows_affected),
/// not the hardcoded 0 the command used to return. Drives the same core fn
/// `execute_dml` calls, so it fails if that wiring is ever short-circuited.
#[tokio::test]
async fn execute_staged_dml_runs_the_approved_sql_and_reports_real_rows() {
    use crate::ai::agent::{AgentState, ConversationState};
    use crate::client::ConnectorClient;
    use crate::commands::execute_staged_dml;
    use crate::supervisor::Supervisor;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::sync::Mutex;

    let container = testcontainers_modules::postgres::Postgres::default()
        .start()
        .await
        .expect("postgres container to start");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("get postgres port");
    wait_for_postgres(port).await;

    let mut supervisor = Supervisor::new();
    supervisor
        .ensure_running()
        .await
        .expect("supervisor running");
    let socket_path = supervisor.endpoint().to_string();
    let token = supervisor.handshake_token().to_owned();

    let (mut client, conn_id) = ConnectorClient::connect(&socket_path, &token, pg_config(port))
        .await
        .expect("connect client");

    client
        .execute(conn_id, "CREATE TEMP TABLE dml_target (id INT, v TEXT)")
        .await
        .expect("create table");
    client
        .execute(
            conn_id,
            "INSERT INTO dml_target VALUES (1, 'a'), (2, 'b'), (3, 'c')",
        )
        .await
        .expect("insert seed");

    // Stage DML exactly as the agent does when it pauses for approval (C1).
    let conv = Arc::new(Mutex::new(ConversationState::new("conn-1".into())));
    {
        let mut c = conv.lock().await;
        c.state = AgentState::PausedForDml {
            staged_sql: "UPDATE dml_target SET v = 'x' WHERE id >= 2".into(),
            staged_at: Instant::now(),
        };
        c.query_cache.insert(
            "SELECT 1".into(),
            CachedQuery {
                result_id: "r_0000000000000000".into(),
                summary: "stale".into(),
            },
        );
    }

    // Mirror execute_dml: take the staged SQL (PausedForDml → Idle), then run
    // the core fn on session B.
    let (staged, _staged_at) = conv
        .lock()
        .await
        .take_staged_sql()
        .expect("staged sql present");
    let rows_affected = execute_staged_dml(&client, conn_id, &conv, staged)
        .await
        .expect("staged DML executes");

    assert_eq!(
        rows_affected, 2,
        "UPDATE must report the real affected count"
    );
    assert!(
        conv.lock().await.query_cache.is_empty(),
        "query cache must be cleared after DML — cached summaries are stale"
    );
    assert!(
        matches!(conv.lock().await.state, AgentState::Idle),
        "conversation must be out of PausedForDml after approval"
    );

    // The rows really changed on the worker's session.
    let check = client
        .execute(conn_id, "SELECT COUNT(*) FROM dml_target WHERE v = 'x'")
        .await
        .expect("count changed rows");
    assert_eq!(check.rows[0][0], serde_json::json!(2));

    client.shutdown().await.expect("shutdown");
    let _ = supervisor.shutdown().await;
}

/// Review Focus #1: parallel tool calls share one backend. Every one of them
/// must run inside its own read-only scope — before Task 3A some landed after a
/// sibling's ROLLBACK and ran in autocommit (`transaction_read_only = off`).
#[tokio::test]
async fn parallel_tool_queries_all_run_read_only() {
    let (port, _c, _raw) = setup_with_port("parallel_readonly").await;
    let (client, conn_id, mut supervisor) = worker_client(port).await;
    let ctx = crate::ai::tools::live_tool_ctx(client.clone(), conn_id);
    let tasks: Vec<_> = (0..16)
        .map(|_| {
            let ctx = ctx.clone();
            tokio::spawn(async move {
                crate::ai::tools::execute::RunReadonlyQuery::new(ctx.clone())
                    .call(
                        serde_json::json!({"sql": "SELECT current_setting('transaction_read_only') AS ro"}),
                        &ctx,
                    )
                    .await
            })
        })
        .collect();
    for t in tasks {
        let out = t.await.unwrap().expect("query runs");
        let crate::ai::tools::ToolOutput::QueryResult { rows, .. } = out else {
            panic!("query result")
        };
        assert_eq!(
            rows[0][0],
            serde_json::json!("on"),
            "a parallel query escaped its read-only scope"
        );
    }
    let _ = supervisor.shutdown().await;
}

#[tokio::test]
async fn worker_ms_is_present_and_bounded_by_total() {
    let (port, _c, _raw) = setup_with_port("worker_ms").await;
    let (client, conn_id, mut supervisor) = worker_client(port).await;
    let start = std::time::Instant::now();
    let res = client
        .execute(conn_id, "SELECT count(*) FROM generate_series(1, 100000)")
        .await
        .unwrap();
    let total = start.elapsed().as_millis() as u64;
    let worker = res.worker_ms.expect("final batch carries worker_ms");
    assert!(worker <= total + 5, "worker {worker} vs total {total}");
    let _ = supervisor.shutdown().await;
}

/// D2.5 on a real Postgres: Stop during two parallel tool calls on one
/// connection. Asserts the SERVER is idle afterwards — the futures alone
/// returning errors proves nothing (see the task note).
#[tokio::test]
async fn stop_cancels_the_running_query_and_the_queued_one_never_runs() {
    let (port, _c, raw) = setup_with_port("cancel_scope").await;
    let (client, conn_id, mut supervisor) = worker_client(port).await;
    let ctx = crate::ai::tools::live_tool_ctx(client.clone(), conn_id);
    let (store, scope_id) = (
        ctx.result_scope.store.clone(),
        ctx.result_scope.scope_id.clone(),
    );
    let run = |ctx: crate::ai::tools::AiToolContext| {
        tokio::spawn(async move {
            crate::ai::tools::execute::RunReadonlyQuery::new(ctx.clone())
                .call(serde_json::json!({"sql": "SELECT pg_sleep(30)"}), &ctx)
                .await
        })
    };
    let first = run(ctx.clone());
    let second = run(ctx.clone());

    // One executes; the other waits on the sequence lock (Phase 0 Task 3A).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while store.executing_count(&scope_id) != 1 {
        assert!(
            std::time::Instant::now() < deadline,
            "no query started executing"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    tokio::time::sleep(std::time::Duration::from_millis(200)).await; // let pg_sleep reach the server

    let start = std::time::Instant::now();
    for (query_id, cid) in store.cancel_scope(&scope_id) {
        client
            .cancel(cid, query_id)
            .await
            .expect("cancel request written");
    }
    for task in [first, second] {
        let out = task
            .await
            .unwrap()
            .expect("a cancelled query is not a tool error");
        let crate::ai::tools::ToolOutput::Text { content } = out else {
            panic!("cancelled → text")
        };
        assert_eq!(content, crate::ai::result_store::CANCELLED_BY_USER);
    }
    assert!(
        start.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );

    // The server is idle: the queued pg_sleep never ran.
    let sleeping: i64 = raw
        .query_one(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE query LIKE '%pg_sleep(30)%' AND state = 'active' AND pid <> pg_backend_pid()",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        sleeping, 0,
        "a cancelled tool call is still running on the server"
    );
    let probe = client
        .execute(conn_id, "SELECT 1")
        .await
        .expect("connection free at once");
    assert_eq!(probe.rows.len(), 1);
    let _ = supervisor.shutdown().await;
}

#[tokio::test]
async fn a_501_row_result_says_more_rows_exist_and_500_does_not() {
    let (port, _c, _raw) = setup_with_port("probe_501").await;
    let (client, conn_id, mut supervisor) = worker_client(port).await;
    let ctx = crate::ai::tools::live_tool_ctx(client, conn_id); // default row_limit = 500
    let summary = |n: u32| {
        let ctx = ctx.clone();
        async move {
            let out = crate::ai::tools::execute::RunReadonlyQuery::new(ctx.clone())
                .call(
                    serde_json::json!({"sql": format!("SELECT g FROM generate_series(1, {n}) g")}),
                    &ctx,
                )
                .await
                .unwrap();
            let crate::ai::tools::ToolOutput::QueryResult {
                text_summary,
                row_count,
                ..
            } = out
            else {
                panic!()
            };
            (text_summary, row_count)
        }
    };
    let (exact, rows) = summary(500).await;
    assert_eq!(rows, 500);
    assert!(exact.contains("rows=\"500\" complete=\"true\""), "{exact}");
    assert!(!exact.contains("More rows exist"), "{exact}");
    let (over, rows) = summary(501).await;
    assert_eq!(rows, 500, "the probe row is never retained");
    assert!(over.contains("rows=\"500+\" complete=\"false\""), "{over}");
    assert!(
        over.contains("More rows exist beyond the 500-row limit"),
        "{over}"
    );
    let _ = supervisor.shutdown().await;
}

#[tokio::test]
async fn inspect_connection_reports_identity_and_extensions() {
    let (port, _c, raw) = setup_with_port("inspect_conn").await;
    raw.batch_execute(seed_sql()).await.unwrap();
    let (client, conn_id, mut supervisor) = worker_client(port).await;
    let ctx = crate::ai::tools::live_tool_ctx(client, conn_id);
    let out = crate::ai::tools::inspect_connection::InspectConnection::new(ctx.clone())
        .call(serde_json::json!({}), &ctx)
        .await
        .unwrap();
    let crate::ai::tools::ToolOutput::Text { content } = out else {
        panic!("text output")
    };
    assert!(content.contains("user=postgres"), "{content}");
    assert!(content.contains("search_path="), "{content}");
    assert!(content.contains("extensions: "), "{content}");
    assert!(
        !content.contains("unavailable"),
        "a superuser can read every section: {content}"
    );
    let _ = supervisor.shutdown().await;
}

#[tokio::test]
async fn export_csv_and_json_round_trip_seeded_rows() {
    use crate::ai::tools::{export::ExportQuery, live_tool_ctx as live_ctx, ToolError, ToolOutput};
    let (port, _c, raw) = setup_with_port("export_csv").await;
    raw.batch_execute(seed_sql()).await.unwrap();
    let (client, conn_id, mut supervisor) = worker_client(port).await;
    let ctx = live_ctx(client.clone(), conn_id);
    let dir = tempfile::tempdir().unwrap();

    let csv_path = dir.path().join("orgs.csv");
    let out = ExportQuery::new(ctx.clone())
        .call(
            serde_json::json!({
                "sql": "SELECT name, slug, plan FROM public.organizations ORDER BY name",
                "path": csv_path.to_str().unwrap()
            }),
            &ctx,
        )
        .await
        .expect("export runs");
    let ToolOutput::Text { content } = out else {
        panic!("text")
    };
    assert!(content.contains("Exported 3 rows"), "{content}");
    let file = std::fs::read_to_string(&csv_path).unwrap();
    let mut lines = file.lines();
    assert_eq!(lines.next().unwrap(), "name,slug,plan");
    assert_eq!(lines.count(), 3);
    assert!(file.contains("Acme Corporation"), "{file}");

    let json_path = dir.path().join("orgs.json");
    ExportQuery::new(ctx.clone())
        .call(
            serde_json::json!({
                "sql": "SELECT name, slug, plan FROM public.organizations ORDER BY name",
                "path": json_path.to_str().unwrap()
            }),
            &ctx,
        )
        .await
        .unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 3);
    let _ = supervisor.shutdown().await;
}

#[tokio::test]
async fn export_truncation_boundaries() {
    use crate::ai::tools::{export::ExportQuery, live_tool_ctx as live_ctx, ToolError, ToolOutput};
    let (port, _c, _raw) = setup_with_port("export_truncation").await;
    let (client, conn_id, mut supervisor) = worker_client(port).await;
    let ctx = live_ctx(client.clone(), conn_id);
    let dir = tempfile::tempdir().unwrap();

    // Exactly 10,000 rows: NOT truncated.
    let at_cap = dir.path().join("at_cap.csv");
    let out = ExportQuery::new(ctx.clone())
        .call(
            serde_json::json!({
                "sql": "SELECT g FROM generate_series(1, 10000) g",
                "path": at_cap.to_str().unwrap()
            }),
            &ctx,
        )
        .await
        .unwrap();
    let ToolOutput::Text { content } = out else {
        panic!()
    };
    assert!(!content.contains("first 10,000 rows"), "{content}");
    assert_eq!(
        std::fs::read_to_string(&at_cap).unwrap().lines().count(),
        10_001
    ); // header + 10,000

    // 10,001 rows: truncated to 10,000 + the Q1 copy.
    let over = dir.path().join("over.csv");
    let out = ExportQuery::new(ctx.clone())
        .call(
            serde_json::json!({
                "sql": "SELECT g FROM generate_series(1, 10001) g",
                "path": over.to_str().unwrap()
            }),
            &ctx,
        )
        .await
        .unwrap();
    let ToolOutput::Text { content } = out else {
        panic!()
    };
    assert!(
        content.contains("the file contains the first 10,000 rows"),
        "{content}"
    );
    assert_eq!(
        std::fs::read_to_string(&over).unwrap().lines().count(),
        10_001
    );
    let _ = supervisor.shutdown().await;
}

#[tokio::test]
async fn export_zero_rows_still_writes_the_file() {
    use crate::ai::tools::{export::ExportQuery, live_tool_ctx as live_ctx, ToolError, ToolOutput};
    let (port, _c, raw) = setup_with_port("export_zero").await;
    raw.batch_execute(seed_sql()).await.unwrap(); // a fresh container has no organizations table
    let (client, conn_id, mut supervisor) = worker_client(port).await;
    let ctx = live_ctx(client.clone(), conn_id);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.csv");
    let out = ExportQuery::new(ctx.clone())
        .call(
            serde_json::json!({
                "sql": "SELECT name FROM public.organizations WHERE false",
                "path": path.to_str().unwrap()
            }),
            &ctx,
        )
        .await
        .unwrap();
    let ToolOutput::Text { content } = out else {
        panic!()
    };
    assert!(content.contains("Exported 0 rows"), "{content}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "name\n");
    let _ = supervisor.shutdown().await;
}

#[tokio::test]
async fn export_guard_rejects_dml_and_creates_no_file() {
    use crate::ai::tools::{export::ExportQuery, live_tool_ctx as live_ctx, ToolError, ToolOutput};
    let (port, _c, _raw) = setup_with_port("export_guard").await;
    let (client, conn_id, mut supervisor) = worker_client(port).await;
    let ctx = live_ctx(client.clone(), conn_id);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nope.csv");
    // Spec §6.6: DELETE / UPDATE.
    for sql in [
        "DELETE FROM public.organizations",
        "UPDATE public.organizations SET plan = 'x'",
    ] {
        let err = ExportQuery::new(ctx.clone())
            .call(
                serde_json::json!({ "sql": sql, "path": path.to_str().unwrap() }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::SqlValidation(_)), "{sql}: {err:?}");
        assert!(!path.exists(), "{sql}: no file may be created");
    }
    let _ = supervisor.shutdown().await;
}

#[tokio::test]
async fn export_leaves_no_open_transaction() {
    use crate::ai::tools::{export::ExportQuery, live_tool_ctx as live_ctx, ToolError, ToolOutput};
    let (port, _c, _raw) = setup_with_port("export_txn").await;
    let (client, conn_id, mut supervisor) = worker_client(port).await;
    let ctx = live_ctx(client.clone(), conn_id);
    let dir = tempfile::tempdir().unwrap();
    ExportQuery::new(ctx.clone())
        .call(
            serde_json::json!({
                "sql": "SELECT 1",
                "path": dir.path().join("one.csv").to_str().unwrap()
            }),
            &ctx,
        )
        .await
        .unwrap();
    // The read-only scope was rolled back. `SELECT 1` would also succeed
    // inside a leaked READ ONLY transaction, so ask the session directly.
    let after = client
        .execute(conn_id, "SHOW transaction_read_only")
        .await
        .expect("connection usable after export");
    assert_eq!(
        after.rows[0][0],
        serde_json::json!("off"),
        "a read-only transaction leaked"
    );
    let _ = supervisor.shutdown().await;
}
