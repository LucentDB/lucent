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
                return (c, client);
            }
            Err(e) => {
                last_err = Some(e);
                tokio::time::sleep(Duration::from_millis(500 * (i + 1))).await;
            }
        }
    }
    panic!("[{name}] Failed to connect: {}", last_err.unwrap());
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

#[tokio::test]
async fn real_search_tables_by_name() {
    let (_c, client) = setup("search_tables").await;
    client.batch_execute(seed_sql()).await.unwrap();
    let like = "%org%";
    let rows = client
        .query(
            "SELECT c.relname FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind = 'r' AND n.nspname = 'public' AND c.relname ILIKE $1 \
             ORDER BY c.relname LIMIT 10",
            &[&like],
        )
        .await
        .unwrap();
    let found: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
    assert!(found.iter().any(|n| n == "organizations"));
    eprintln!("  Found {} tables matching 'org'", found.len());
}

#[tokio::test]
async fn real_column_info() {
    let (_c, client) = setup("column_info").await;
    client.batch_execute(seed_sql()).await.unwrap();
    let rows = client
        .query(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = 'organizations' \
             ORDER BY ordinal_position",
            &[],
        )
        .await
        .unwrap();
    let cols: Vec<(String, String)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
    assert!(cols.iter().any(|(n, _)| n == "settings"), "jsonb column");
    assert!(cols.iter().any(|(n, _)| n == "plan"));
    eprintln!("  Found {} columns", cols.len());
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
    assert!(
        crate::ai::guard::validate_readonly("DELETE FROM organizations WHERE plan='free'").is_err()
    );
    let rows = client
        .query("SELECT count(*) FROM organizations", &[])
        .await
        .unwrap();
    let count: i64 = rows[0].get(0);
    assert_eq!(count, 3, "data intact");
}

#[tokio::test]
async fn composite_pk_and_fk_does_not_duplicate_columns_in_query() {
    let (_c, client) = setup("no_dup_cols").await;
    client.batch_execute(seed_sql()).await.unwrap();

    // The OLD query (LEFT JOIN key_column_usage) duplicates columns that are
    // both in a composite PK AND individually FK-referenced.
    let old_rows = client
        .query(
            "SELECT c.table_schema, c.table_name, c.column_name, c.data_type, \
                    COALESCE(tc.constraint_type = 'PRIMARY KEY', false) AS is_primary_key \
             FROM information_schema.columns c \
             LEFT JOIN information_schema.key_column_usage kcu \
               ON kcu.column_name = c.column_name \
              AND kcu.table_name = c.table_name \
              AND kcu.table_schema = c.table_schema \
             LEFT JOIN information_schema.table_constraints tc \
               ON tc.constraint_name = kcu.constraint_name \
              AND tc.table_schema = kcu.table_schema \
              AND tc.constraint_type = 'PRIMARY KEY' \
             WHERE c.table_schema = 'public' AND c.table_name = 'line_items' \
             ORDER BY c.column_name",
            &[],
        )
        .await
        .unwrap();
    let old_org_id_count = old_rows
        .iter()
        .filter(|r| r.get::<_, String>(2) == "org_id")
        .count();
    let old_user_id_count = old_rows
        .iter()
        .filter(|r| r.get::<_, String>(2) == "user_id")
        .count();
    assert!(
        old_org_id_count > 1,
        "the OLD query must produce >1 row for org_id (composite PK + FK) — 
         otherwise this test can't demonstrate the bug. Got {old_org_id_count}"
    );

    // The NEW query (EXISTS semi-join) must produce exactly 1 row per column.
    let new_rows = client
        .query(
            "SELECT c.table_schema, c.table_name, c.column_name, c.data_type, \
                    EXISTS ( \
                        SELECT 1 FROM information_schema.key_column_usage kcu \
                        JOIN information_schema.table_constraints tc \
                          ON tc.constraint_name = kcu.constraint_name \
                         AND tc.table_schema = kcu.table_schema \
                         AND tc.constraint_type = 'PRIMARY KEY' \
                        WHERE kcu.column_name = c.column_name \
                          AND kcu.table_name = c.table_name \
                          AND kcu.table_schema = c.table_schema \
                    ) AS is_primary_key \
             FROM information_schema.columns c \
             WHERE c.table_schema = 'public' AND c.table_name = 'line_items' \
             ORDER BY c.column_name",
            &[],
        )
        .await
        .unwrap();
    let new_org_id_count = new_rows
        .iter()
        .filter(|r| r.get::<_, String>(2) == "org_id")
        .count();
    let new_user_id_count = new_rows
        .iter()
        .filter(|r| r.get::<_, String>(2) == "user_id")
        .count();

    assert_eq!(
        new_org_id_count, 1,
        "org_id (composite PK + individual FK) must appear exactly once with EXISTS semi-join, not duplicated per constraint"
    );
    assert_eq!(
        new_user_id_count, 1,
        "user_id (composite PK + individual FK) must appear exactly once"
    );
    eprintln!(
        "  Duplicate-column fix: OLD query returned {old_org_id_count}x org_id / {old_user_id_count}x user_id, NEW query returns {new_org_id_count}x / {new_user_id_count}x"
    );
}

#[tokio::test]
async fn get_objects_info_query_reports_pk_consistently_for_composite_pk_and_fk_columns() {
    let (_c, client) = setup("get_objects_info_pk_consistency").await;
    client.batch_execute(seed_sql()).await.unwrap();

    // The OLD query (LEFT JOIN key_column_usage, as GetObjectsInfo::call used before
    // this fix) produces one row per matching constraint. For org_id/user_id (each
    // part of the composite PK AND individually FK-referenced), that's 2 rows each —
    // one correctly flagged is_pk=true (the PK-constraint match), one incorrectly
    // flagged is_pk=false (the FK-constraint match, which doesn't join to a
    // PRIMARY KEY-typed table_constraints row). A dedup-by-column-name pass has no
    // reliable way to know which of the two rows to keep, since both tie on
    // ORDER BY c.ordinal_position — whichever Postgres emits first wins, which is a
    // query-plan detail, not a guarantee.
    let old_rows = client
        .query(
            "SELECT c.column_name, c.data_type, c.is_nullable, \
                    tc.constraint_type AS constraint_type \
             FROM information_schema.columns c \
             LEFT JOIN information_schema.key_column_usage kcu \
               ON kcu.column_name = c.column_name AND kcu.table_name = c.table_name \
               AND kcu.table_schema = c.table_schema \
             LEFT JOIN information_schema.table_constraints tc \
               ON tc.constraint_name = kcu.constraint_name \
               AND tc.table_schema = kcu.table_schema \
               AND tc.constraint_type = 'PRIMARY KEY' \
             WHERE c.table_schema = 'public' AND c.table_name = 'line_items' \
             ORDER BY c.ordinal_position",
            &[],
        )
        .await
        .unwrap();

    let org_id_is_pk_values: Vec<bool> = old_rows
        .iter()
        .filter(|r| r.get::<_, String>(0) == "org_id")
        .map(|r| r.get::<_, Option<String>>(3).as_deref() == Some("PRIMARY KEY"))
        .collect();
    assert!(
        org_id_is_pk_values.contains(&true) && org_id_is_pk_values.contains(&false),
        "the OLD query must produce BOTH a true and a false is_pk row for org_id — \
         otherwise this test can't demonstrate that a naive first-row-wins dedup is \
         ambiguous. Got: {org_id_is_pk_values:?}"
    );

    // The NEW query (EXISTS semi-join, matching GetObjectsInfo::call after this fix)
    // must produce exactly one row per column, with the CORRECT is_pk value — no
    // ambiguity, no dedup pass needed.
    let new_rows = client
        .query(
            "SELECT c.column_name, c.data_type, c.is_nullable, \
                    EXISTS ( \
                        SELECT 1 FROM information_schema.key_column_usage kcu \
                        JOIN information_schema.table_constraints tc \
                          ON tc.constraint_name = kcu.constraint_name \
                         AND tc.table_schema = kcu.table_schema \
                         AND tc.constraint_type = 'PRIMARY KEY' \
                        WHERE kcu.column_name = c.column_name \
                          AND kcu.table_name = c.table_name \
                          AND kcu.table_schema = c.table_schema \
                    ) AS is_primary_key \
             FROM information_schema.columns c \
             WHERE c.table_schema = 'public' AND c.table_name = 'line_items' \
             ORDER BY c.ordinal_position",
            &[],
        )
        .await
        .unwrap();

    for expected_pk_col in ["org_id", "user_id"] {
        let matches: Vec<bool> = new_rows
            .iter()
            .filter(|r| r.get::<_, String>(0) == expected_pk_col)
            .map(|r| r.get::<_, bool>(3))
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "{expected_pk_col} must appear exactly once, not duplicated per constraint"
        );
        assert_eq!(
            matches[0], true,
            "{expected_pk_col} is part of the composite PK and must be reported as is_pk=true, deterministically"
        );
    }

    let quantity_matches: Vec<bool> = new_rows
        .iter()
        .filter(|r| r.get::<_, String>(0) == "quantity")
        .map(|r| r.get::<_, bool>(3))
        .collect();
    assert_eq!(quantity_matches, vec![false], "quantity is not a PK column");
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
async fn partitioned_table_metadata_query_collapses_children() {
    let (_c, client) = setup("partition_collapse").await;
    client
        .batch_execute(
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

    // Same query build_index runs in Step 0.
    let rows = client
        .query(
            "SELECT n.nspname, c.relname, \
                    (i.inhrelid IS NOT NULL) AS is_partition_child, \
                    (SELECT count(*) FROM pg_inherits pi WHERE pi.inhparent = c.oid) AS partition_count, \
                    CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END AS partkey \
             FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_inherits i ON i.inhrelid = c.oid \
             WHERE c.relkind IN ('r', 'p') AND n.nspname = 'public' \
               AND c.relname LIKE 'events%'",
            &[],
        )
        .await
        .unwrap();

    let parent = rows
        .iter()
        .find(|r| r.get::<_, String>(1) == "events")
        .expect("parent present");
    assert!(!parent.get::<_, bool>(2), "parent is not a partition child");
    assert_eq!(
        parent.get::<_, i64>(3),
        2,
        "parent reports its partition count"
    );
    assert!(
        parent
            .get::<_, Option<String>>(4)
            .unwrap()
            .contains("RANGE"),
        "partition key def"
    );

    for child in ["events_2025", "events_2026"] {
        let row = rows
            .iter()
            .find(|r| r.get::<_, String>(1) == child)
            .expect("child row");
        assert!(
            row.get::<_, bool>(2),
            "{child} must be flagged as a partition child (and excluded from the graph)"
        );
    }
}
