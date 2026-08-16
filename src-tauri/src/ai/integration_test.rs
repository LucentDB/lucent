//! Real-world integration tests (feature-gated: `integration-tests`).
//!
//! Tests validate tool-generated SQL against a real Postgres database.
//!
//! Usage:
//!   cargo test --package lucent --features integration-tests -- --nocapture
//!   OPENAI_API_KEY=sk-... \
//!     cargo test --package lucent --features integration-tests -- --nocapture --ignored

#![cfg(feature = "integration-tests")]

#[allow(unused_imports)]
use crate::ai::provider::LlmProvider;
use std::time::Duration;
use testcontainers::runners::AsyncRunner;
use tokio_postgres::NoTls;

fn seed_sql() -> &'static str {
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
async fn setup_with_port(name: &str) -> (u16, impl Drop, tokio_postgres::Client) {
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
async fn worker_client(
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
        capabilities: None,
        config: crate::ai::config::AiConfig::default(),
        schema_graph: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        embedder: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        reranker: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
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
        c.query_cache.insert("SELECT 1".into(), "stale".into());
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
