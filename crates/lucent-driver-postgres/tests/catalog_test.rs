//! Docker-backed conformance for the Postgres catalog implementation.

mod common;
use common::seeded;

use lucent_protocol::{CatalogRequest, CatalogResult, ForeignKey, ObjectKind, QueryId};
// ObjectRef is used fully-qualified (lucent_protocol::ObjectRef) in this file.
use lucent_worker_host::Connector;
use tokio::sync::mpsc;
use uuid::Uuid;

#[tokio::test]
async fn lists_user_namespaces_and_counts_objects_including_matviews() {
    let (_c, connector, cid) = seeded().await;

    let result = connector
        .catalog(cid, CatalogRequest::ListNamespaces)
        .await
        .expect("list namespaces");

    let CatalogResult::Namespaces(namespaces) = result else {
        panic!("expected Namespaces");
    };

    let names: Vec<String> = namespaces.iter().map(|n| n.display()).collect();
    assert!(names.contains(&"app".to_string()), "got {names:?}");
    assert!(names.contains(&"public".to_string()), "got {names:?}");
    assert!(
        !names
            .iter()
            .any(|n| n.starts_with("pg_") || n == "information_schema"),
        "system schemas must never be surfaced: {names:?}"
    );

    let app = namespaces.iter().find(|n| n.display() == "app").unwrap();
    // users, orders, deliveries, events, events_2026, recent, mv_users,
    // counter = 8. The matview is the one today's information_schema-based
    // count misses.
    assert_eq!(
        app.object_count,
        Some(8),
        "object count must include the materialized view"
    );
}

#[tokio::test]
async fn lists_objects_with_kinds_estimates_and_partition_metadata() {
    let (_c, connector, cid) = seeded().await;

    let result = connector
        .catalog(
            cid,
            CatalogRequest::ListObjects {
                namespace: vec!["app".into()],
                kinds: vec![],
            },
        )
        .await
        .expect("list objects");
    let CatalogResult::Objects(objects) = result else {
        panic!("expected Objects");
    };

    let by_name = |n: &str| {
        objects
            .iter()
            .find(|o| o.reference.name == n)
            .unwrap_or_else(|| panic!("missing {n} in {objects:?}"))
    };

    assert_eq!(by_name("users").reference.kind, ObjectKind::Table);
    assert_eq!(by_name("recent").reference.kind, ObjectKind::View);
    assert_eq!(
        by_name("mv_users").reference.kind,
        ObjectKind::MaterializedView
    );
    assert_eq!(by_name("counter").reference.kind, ObjectKind::Sequence);

    assert_eq!(by_name("users").comment.as_deref(), Some("people"));

    // ANALYZEd, so an estimate exists. Never-analyzed `orders` must be None,
    // NOT Some(0) — "unknown" and "empty" are different facts.
    assert!(by_name("users").est_rows.is_some());
    assert_eq!(
        by_name("orders").est_rows,
        None,
        "a never-analyzed table has an unknown row count, not zero"
    );

    // Partitioned parent carries its key; the child is flagged so retrieval
    // indexing can skip it.
    let events = by_name("events");
    let partition = events.partition.as_ref().expect("events is partitioned");
    assert!(
        partition.key.as_deref().unwrap_or("").contains("RANGE"),
        "partition key: {partition:?}"
    );
    assert_eq!(partition.child_count, 1);
    assert!(!events.is_partition_child);
    assert!(by_name("events_2026").is_partition_child);
}

#[tokio::test]
async fn kind_filter_narrows_the_listing() {
    let (_c, connector, cid) = seeded().await;

    let result = connector
        .catalog(
            cid,
            CatalogRequest::ListAllObjects {
                kinds: vec![ObjectKind::View],
            },
        )
        .await
        .expect("list all objects");
    let CatalogResult::Objects(objects) = result else {
        panic!("expected Objects");
    };

    assert!(!objects.is_empty());
    assert!(
        objects.iter().all(|o| o.reference.kind == ObjectKind::View),
        "kind filter leaked non-views: {objects:?}"
    );
}

#[tokio::test]
async fn describes_columns_with_types_keys_defaults_and_comments() {
    let (_c, connector, cid) = seeded().await;

    let result = connector
        .catalog(
            cid,
            CatalogRequest::DescribeObjects {
                refs: vec![
                    lucent_protocol::ObjectRef {
                        namespace: vec!["app".into()],
                        name: "users".into(),
                        kind: ObjectKind::Table,
                    },
                    lucent_protocol::ObjectRef {
                        namespace: vec!["app".into()],
                        name: "orders".into(),
                        kind: ObjectKind::Table,
                    },
                ],
            },
        )
        .await
        .expect("describe objects");
    let CatalogResult::ObjectDetails(details) = result else {
        panic!("expected ObjectDetails");
    };

    assert_eq!(details.len(), 2, "one detail per requested object");

    let users = details
        .iter()
        .find(|d| d.reference.name == "users")
        .unwrap();
    assert_eq!(users.comment.as_deref(), Some("people"));

    let col = |n: &str| users.columns.iter().find(|c| c.name == n).unwrap();

    // Ordinals are 1-based and in declaration order.
    assert_eq!(col("id").ordinal, 1);
    assert_eq!(col("email").ordinal, 2);
    assert_eq!(col("note").ordinal, 3);

    // format_type() carries the modifier — information_schema.data_type
    // would say bare "character varying" and lose the length.
    assert_eq!(col("email").type_name, "character varying(100)");
    assert_eq!(col("id").type_name, "bigint");

    assert!(col("id").is_primary_key);
    assert!(!col("email").is_primary_key);
    assert!(!col("id").nullable);
    assert!(col("note").nullable);
    assert_eq!(col("email").comment.as_deref(), Some("login"));

    // Composite PK: BOTH columns must be flagged, and each must appear exactly
    // once. A LEFT JOIN on key_column_usage duplicates rows here.
    let orders = details
        .iter()
        .find(|d| d.reference.name == "orders")
        .unwrap();
    assert_eq!(
        orders
            .columns
            .iter()
            .filter(|c| c.name == "user_id")
            .count(),
        1,
        "composite keys must not duplicate columns: {orders:?}"
    );
    assert!(
        orders
            .columns
            .iter()
            .find(|c| c.name == "org_id")
            .unwrap()
            .is_primary_key
    );
    assert!(
        orders
            .columns
            .iter()
            .find(|c| c.name == "user_id")
            .unwrap()
            .is_primary_key
    );
}

#[tokio::test]
async fn describes_materialized_view_columns() {
    // information_schema.columns excludes materialized views entirely, so
    // these columns are invisible to the AI today. pg_attribute includes them.
    let (_c, connector, cid) = seeded().await;

    let result = connector
        .catalog(
            cid,
            CatalogRequest::DescribeObjects {
                refs: vec![lucent_protocol::ObjectRef {
                    namespace: vec!["app".into()],
                    name: "mv_users".into(),
                    kind: ObjectKind::MaterializedView,
                }],
            },
        )
        .await
        .expect("describe matview");
    let CatalogResult::ObjectDetails(details) = result else {
        panic!("expected ObjectDetails");
    };

    let names: Vec<&str> = details[0].columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["id", "email"],
        "matview columns must be visible"
    );
}

#[tokio::test]
async fn lists_foreign_keys_from_pg_constraint() {
    let (_c, connector, cid) = seeded().await;

    let result = connector
        .catalog(cid, CatalogRequest::ListForeignKeys)
        .await
        .expect("list foreign keys");
    let CatalogResult::ForeignKeys(fks) = result else {
        panic!("expected ForeignKeys");
    };

    let fk = fks
        .iter()
        .find(|f| f.from.table == "orders" && f.from.column == "user_id")
        .unwrap_or_else(|| panic!("orders.user_id FK missing from {fks:?}"));

    assert_eq!(fk.from.namespace, vec!["app".to_string()]);
    assert_eq!(fk.to.table, "users");
    assert_eq!(fk.to.column, "id");
    assert_eq!(fk.to.namespace, vec!["app".to_string()]);

    // Composite FK: deliveries(org_id, user_id) -> orders(org_id, user_id).
    // The pairing is positional — from.column at ordinal position k pairs with
    // to.column at position k — which is what `unnest(conkey, confkey) WITH
    // ORDINALITY` guarantees. One FK row per referencing column.
    let deliveries: Vec<&ForeignKey> = fks
        .iter()
        .filter(|f| f.from.table == "deliveries")
        .collect();
    assert_eq!(
        deliveries.len(),
        2,
        "one FK row per referencing column: {fks:?}"
    );

    let org = deliveries
        .iter()
        .find(|f| f.from.column == "org_id")
        .unwrap_or_else(|| panic!("deliveries.org_id FK missing from {fks:?}"));
    assert_eq!(org.from.namespace, vec!["app".to_string()]);
    assert_eq!(org.to.table, "orders");
    assert_eq!(
        org.to.column, "org_id",
        "positional pairing: org_id -> org_id"
    );
    assert_eq!(org.to.namespace, vec!["app".to_string()]);

    let uid = deliveries
        .iter()
        .find(|f| f.from.column == "user_id")
        .unwrap_or_else(|| panic!("deliveries.user_id FK missing from {fks:?}"));
    assert_eq!(uid.to.table, "orders");
    assert_eq!(
        uid.to.column, "user_id",
        "positional pairing: user_id -> user_id"
    );
}

#[tokio::test]
async fn describe_reports_foreign_keys_on_the_referencing_column() {
    let (_c, connector, cid) = seeded().await;

    let result = connector
        .catalog(
            cid,
            CatalogRequest::DescribeObjects {
                refs: vec![lucent_protocol::ObjectRef {
                    namespace: vec!["app".into()],
                    name: "orders".into(),
                    kind: ObjectKind::Table,
                }],
            },
        )
        .await
        .expect("describe orders");
    let CatalogResult::ObjectDetails(details) = result else {
        panic!("expected ObjectDetails");
    };

    let user_id = details[0]
        .columns
        .iter()
        .find(|c| c.name == "user_id")
        .unwrap();
    let fk = user_id
        .foreign_key
        .as_ref()
        .expect("user_id references users");
    assert_eq!(fk.table, "users");
    assert_eq!(fk.column, "id");

    assert!(
        details[0]
            .columns
            .iter()
            .find(|c| c.name == "total")
            .unwrap()
            .foreign_key
            .is_none(),
        "non-FK columns must have no foreign_key"
    );
}

#[tokio::test]
async fn searches_objects_and_columns_by_name() {
    let (_c, connector, cid) = seeded().await;

    let result = connector
        .catalog(
            cid,
            CatalogRequest::SearchObjects {
                query: "user".into(),
                kinds: vec![],
                namespace: None,
                limit: 20,
            },
        )
        .await
        .expect("search objects");
    let CatalogResult::SearchHits(hits) = result else {
        panic!("expected SearchHits");
    };

    // The table itself matches by name.
    assert!(
        hits.iter()
            .any(|h| h.reference.name == "users" && h.column.is_none()),
        "expected an object hit for users: {hits:?}"
    );
    // orders.user_id matches by column name.
    assert!(
        hits.iter()
            .any(|h| h.reference.name == "orders" && h.column.as_deref() == Some("user_id")),
        "expected a column hit for orders.user_id: {hits:?}"
    );
    assert!(
        hits.len() <= 40,
        "limit must bound both halves: {}",
        hits.len()
    );
}

#[tokio::test]
async fn search_never_interpolates_user_text_into_sql() {
    // A quote-heavy query must return no results, not a syntax error — proof
    // the term binds as a parameter rather than being escaped into the string.
    let (_c, connector, cid) = seeded().await;

    let result = connector
        .catalog(
            cid,
            CatalogRequest::SearchObjects {
                query: "'; DROP TABLE app.users; --".into(),
                kinds: vec![],
                namespace: None,
                limit: 10,
            },
        )
        .await
        .expect("hostile search term must not error");
    let CatalogResult::SearchHits(hits) = result else {
        panic!("expected SearchHits");
    };
    assert!(hits.is_empty(), "got {hits:?}");

    // And the table is still there.
    let objects = connector
        .catalog(
            cid,
            CatalogRequest::ListObjects {
                namespace: vec!["app".into()],
                kinds: vec![ObjectKind::Table],
            },
        )
        .await
        .unwrap();
    let CatalogResult::Objects(objects) = objects else {
        panic!("expected Objects");
    };
    assert!(objects.iter().any(|o| o.reference.name == "users"));
}

#[tokio::test]
async fn returns_ddl_for_views_and_tables() {
    let (_c, connector, cid) = seeded().await;

    let view_ddl = match connector
        .catalog(
            cid,
            CatalogRequest::GetObjectDdl {
                reference: lucent_protocol::ObjectRef {
                    namespace: vec!["app".into()],
                    name: "recent".into(),
                    kind: ObjectKind::View,
                },
            },
        )
        .await
        .expect("view ddl")
    {
        CatalogResult::Ddl(s) => s,
        other => panic!("expected Ddl, got {other:?}"),
    };
    assert!(view_ddl.contains("CREATE OR REPLACE VIEW"), "{view_ddl}");
    assert!(view_ddl.contains("app"), "{view_ddl}");
    assert!(view_ddl.contains("recent"), "{view_ddl}");
    assert!(view_ddl.contains("SELECT"), "{view_ddl}");
}

#[tokio::test]
async fn returns_sequence_properties_in_display_order() {
    let (_c, connector, cid) = seeded().await;

    let props = match connector
        .catalog(
            cid,
            CatalogRequest::GetObjectProperties {
                reference: lucent_protocol::ObjectRef {
                    namespace: vec!["app".into()],
                    name: "counter".into(),
                    kind: ObjectKind::Sequence,
                },
            },
        )
        .await
        .expect("sequence properties")
    {
        CatalogResult::Properties(p) => p,
        other => panic!("expected Properties, got {other:?}"),
    };

    let keys: Vec<&str> = props.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "Data Type",
            "Start Value",
            "Min Value",
            "Max Value",
            "Increment",
            "Cycles"
        ],
        "key order is the sidebar's display order"
    );

    let value = |k: &str| props.iter().find(|p| p.key == k).unwrap().value.as_str();
    assert_eq!(value("Start Value"), "5");
    assert_eq!(value("Increment"), "2");
    assert_eq!(value("Max Value"), "100");
}

#[tokio::test]
async fn empty_kinds_search_is_scoped_to_tables_and_views() {
    // Empty `kinds` means tables and views only for SEARCH (unlike listing,
    // which keeps all kinds). The seed's matview and sequence must not leak.
    let (_c, connector, cid) = seeded().await;

    let result = connector
        .catalog(
            cid,
            CatalogRequest::SearchObjects {
                query: "mv".into(),
                kinds: vec![],
                namespace: None,
                limit: 20,
            },
        )
        .await
        .expect("search objects");
    let CatalogResult::SearchHits(hits) = result else {
        panic!("expected SearchHits");
    };

    assert!(
        hits.iter()
            .all(|h| matches!(h.reference.kind, ObjectKind::Table | ObjectKind::View)),
        "empty kinds must scope search to tables and views, got: {hits:?}"
    );
    assert!(
        !hits.iter().any(|h| h.reference.name == "mv_users"),
        "matview leaked into empty-kinds search: {hits:?}"
    );

    let result = connector
        .catalog(
            cid,
            CatalogRequest::SearchObjects {
                query: "counter".into(),
                kinds: vec![],
                namespace: None,
                limit: 20,
            },
        )
        .await
        .expect("search objects");
    let CatalogResult::SearchHits(hits) = result else {
        panic!("expected SearchHits");
    };
    assert!(
        !hits.iter().any(|h| h.reference.name == "counter"),
        "sequence leaked into empty-kinds search: {hits:?}"
    );
}

#[tokio::test]
async fn kinds_with_no_relkinds_fall_back_instead_of_syntax_error() {
    // A non-empty kinds filter that maps to no pg_class relkinds (Function is
    // a first-class variant that relkinds_for skips) must not produce
    // `c.relkind IN ()` — that is a syntax error. It falls back to the
    // tables-and-views scope and returns SearchHits, never an error.
    let (_c, connector, cid) = seeded().await;

    let result = connector
        .catalog(
            cid,
            CatalogRequest::SearchObjects {
                query: "user".into(),
                kinds: vec![ObjectKind::Function],
                namespace: None,
                limit: 10,
            },
        )
        .await
        .expect("Function kind filter must not error");
    assert!(
        matches!(result, CatalogResult::SearchHits(_)),
        "expected SearchHits, got {result:?}"
    );

    // Same for the Other escape hatch, reachable via ObjectKind::from_label.
    let result = connector
        .catalog(
            cid,
            CatalogRequest::SearchObjects {
                query: "user".into(),
                kinds: vec![ObjectKind::Other("domain".into())],
                namespace: None,
                limit: 10,
            },
        )
        .await
        .expect("Other kind filter must not error");
    assert!(
        matches!(result, CatalogResult::SearchHits(_)),
        "expected SearchHits, got {result:?}"
    );
}

#[tokio::test]
async fn function_ddl_is_deterministic_across_overloads() {
    // G4: pg_get_functiondef with LIMIT 1 and no ORDER BY returned an
    // ARBITRARY overload's source when a name was overloaded. The oldest
    // overload (lowest oid) must win, deterministically.
    let (_c, connector, cid) = seeded().await;

    for sql in [
        "CREATE FUNCTION app.ddl_probe() RETURNS int LANGUAGE sql AS 'SELECT 1'",
        "CREATE FUNCTION app.ddl_probe(int) RETURNS int LANGUAGE sql AS 'SELECT $1 + 100'",
    ] {
        let (tx, _rx) = mpsc::channel(4);
        connector
            .execute(cid, QueryId(Uuid::new_v4()), sql.to_string(), tx)
            .await;
    }

    let ddl = connector
        .catalog(
            cid,
            CatalogRequest::GetObjectDdl {
                reference: lucent_protocol::ObjectRef {
                    namespace: vec!["app".into()],
                    name: "ddl_probe".into(),
                    kind: ObjectKind::Function,
                },
            },
        )
        .await
        .expect("function DDL must resolve");

    match ddl {
        CatalogResult::Ddl(sql) => {
            assert!(
                sql.contains("SELECT 1") && !sql.contains("100"),
                "the OLDEST overload must win, got: {sql}"
            );
        }
        other => panic!("expected Ddl, got {other:?}"),
    }
}
