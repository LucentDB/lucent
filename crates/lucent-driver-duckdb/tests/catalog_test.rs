use lucent_driver_duckdb::connector::DuckDbConnector;
use lucent_protocol::{
    CatalogRequest, CatalogResult, ConnectionConfig, ConnectionId, ObjectKind, ObjectRef, QueryId,
};
use lucent_worker_host::{Connector, ExecutionEvent};
use uuid::Uuid;

const SEED: &str = "
    CREATE SCHEMA app;
    CREATE TABLE app.users (id BIGINT PRIMARY KEY, email VARCHAR NOT NULL, note VARCHAR);
    CREATE TABLE app.orders (
        id BIGINT PRIMARY KEY,
        user_id BIGINT REFERENCES app.users(id),
        total DECIMAL(12,2)
    );
    CREATE VIEW app.recent AS SELECT id FROM app.users;
    INSERT INTO app.users VALUES (1, 'a@b.c', NULL), (2, 'd@e.f', 'x');
";

async fn seeded() -> (DuckDbConnector, ConnectionId) {
    let connector = DuckDbConnector::default();
    let cid = ConnectionId(Uuid::new_v4());
    connector
        .connect(
            cid,
            ConnectionConfig::new("duckdb").with("path", ":memory:"),
        )
        .await
        .expect("connect");

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let exec = connector.execute(cid, QueryId(Uuid::new_v4()), SEED.into(), tx);
    let drain = async {
        while let Some(e) = rx.recv().await {
            if let ExecutionEvent::Failed(err) = e {
                panic!("seed failed: {err}");
            }
        }
    };
    tokio::join!(exec, drain);
    (connector, cid)
}

#[tokio::test]
async fn lists_namespaces_as_catalog_and_schema_segments() {
    let (connector, cid) = seeded().await;
    let CatalogResult::Namespaces(namespaces) = connector
        .catalog(cid, CatalogRequest::ListNamespaces)
        .await
        .expect("list namespaces")
    else {
        panic!("expected Namespaces");
    };

    // NamespaceModel::CatalogSchema — two segments, unlike Postgres's one.
    assert!(
        namespaces.iter().all(|n| n.path.len() == 2),
        "DuckDB namespaces are catalog.schema: {namespaces:?}"
    );
    let displays: Vec<String> = namespaces.iter().map(|n| n.display()).collect();
    assert!(
        displays.iter().any(|d| d.ends_with(".app")),
        "expected the app schema: {displays:?}"
    );
    assert!(
        !displays
            .iter()
            .any(|d| d.contains("system") || d.contains("temp")),
        "internal catalogs must not be surfaced: {displays:?}"
    );
}

#[tokio::test]
async fn lists_the_default_main_schema_when_only_it_holds_data() {
    // A database seeded purely into the default schema (no explicit CREATE
    // SCHEMA) must still surface `main`: duckdb_schemas() marks it internal,
    // and hiding it would leave the schema browser empty for every DuckDB
    // connection whose tables live in the default schema.
    let connector = DuckDbConnector::default();
    let cid = ConnectionId(Uuid::new_v4());
    connector
        .connect(
            cid,
            ConnectionConfig::new("duckdb").with("path", ":memory:"),
        )
        .await
        .expect("connect");

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let exec = connector.execute(
        cid,
        QueryId(Uuid::new_v4()),
        "CREATE TABLE t (x int)".into(),
        tx,
    );
    let drain = async {
        while let Some(e) = rx.recv().await {
            if let ExecutionEvent::Failed(err) = e {
                panic!("seed failed: {err}");
            }
        }
    };
    tokio::join!(exec, drain);

    let CatalogResult::Namespaces(namespaces) = connector
        .catalog(cid, CatalogRequest::ListNamespaces)
        .await
        .expect("list namespaces")
    else {
        panic!("expected Namespaces");
    };
    let displays: Vec<String> = namespaces.iter().map(|n| n.display()).collect();
    assert!(
        displays.iter().any(|d| d.ends_with(".main")),
        "the default main schema must be listed: {displays:?}"
    );
    assert!(
        !displays
            .iter()
            .any(|d| d.contains("system") || d.contains("temp")),
        "internal catalogs must not be surfaced: {displays:?}"
    );
}

#[tokio::test]
async fn lists_objects_with_kinds_and_row_estimates() {
    let (connector, cid) = seeded().await;
    let CatalogResult::Objects(objects) = connector
        .catalog(cid, CatalogRequest::ListAllObjects { kinds: vec![] })
        .await
        .expect("list all objects")
    else {
        panic!("expected Objects");
    };

    let users = objects
        .iter()
        .find(|o| o.reference.name == "users")
        .unwrap_or_else(|| panic!("missing users in {objects:?}"));
    assert_eq!(users.reference.kind, ObjectKind::Table);
    assert_eq!(users.reference.namespace.len(), 2);
    // duckdb_tables.estimated_size — the spec assumed this did not exist.
    assert!(
        users.est_rows.is_some(),
        "DuckDB does supply a row estimate: {users:?}"
    );

    assert!(
        objects
            .iter()
            .any(|o| o.reference.name == "recent" && o.reference.kind == ObjectKind::View),
        "views must be listed: {objects:?}"
    );
    assert!(
        objects.iter().all(|o| !o.is_partition_child),
        "DuckDB has no partition children"
    );
}

#[tokio::test]
async fn describes_columns_with_types_nullability_and_primary_keys() {
    let (connector, cid) = seeded().await;
    let CatalogResult::Objects(objects) = connector
        .catalog(
            cid,
            CatalogRequest::ListAllObjects {
                kinds: vec![ObjectKind::Table],
            },
        )
        .await
        .unwrap()
    else {
        panic!("expected Objects");
    };
    let users_ref = objects
        .iter()
        .find(|o| o.reference.name == "users")
        .unwrap()
        .reference
        .clone();

    let CatalogResult::ObjectDetails(details) = connector
        .catalog(
            cid,
            CatalogRequest::DescribeObjects {
                refs: vec![users_ref],
            },
        )
        .await
        .expect("describe")
    else {
        panic!("expected ObjectDetails");
    };

    let cols = &details[0].columns;
    let col = |n: &str| cols.iter().find(|c| c.name == n).unwrap();

    assert_eq!(col("id").ordinal, 1, "ordinals are 1-based");
    assert_eq!(col("email").ordinal, 2);
    assert!(col("id").is_primary_key);
    assert!(!col("email").is_primary_key);
    assert!(!col("email").nullable, "NOT NULL must be reported");
    assert!(col("note").nullable);
    assert!(
        col("id").type_name.to_uppercase().contains("BIGINT"),
        "{}",
        col("id").type_name
    );
}

#[tokio::test]
async fn lists_foreign_keys_and_annotates_the_referencing_column() {
    let (connector, cid) = seeded().await;

    let CatalogResult::ForeignKeys(fks) = connector
        .catalog(cid, CatalogRequest::ListForeignKeys)
        .await
        .expect("list foreign keys")
    else {
        panic!("expected ForeignKeys");
    };

    let fk = fks
        .iter()
        .find(|f| f.from.table == "orders" && f.from.column == "user_id")
        .unwrap_or_else(|| panic!("orders.user_id FK missing from {fks:?}"));
    assert_eq!(fk.to.table, "users");
    assert_eq!(fk.to.column, "id");
}

#[tokio::test]
async fn searches_objects_and_columns_by_name() {
    let (connector, cid) = seeded().await;
    let CatalogResult::SearchHits(hits) = connector
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
        .expect("search")
    else {
        panic!("expected SearchHits");
    };

    assert!(hits
        .iter()
        .any(|h| h.reference.name == "users" && h.column.is_none()));
    assert!(hits
        .iter()
        .any(|h| h.reference.name == "orders" && h.column.as_deref() == Some("user_id")));
}

#[tokio::test]
async fn returns_table_and_view_ddl() {
    let (connector, cid) = seeded().await;
    let CatalogResult::Ddl(ddl) = connector
        .catalog(
            cid,
            CatalogRequest::GetObjectDdl {
                reference: ObjectRef {
                    namespace: vec!["memory".into(), "app".into()],
                    name: "recent".into(),
                    kind: ObjectKind::View,
                },
            },
        )
        .await
        .expect("view ddl")
    else {
        panic!("expected Ddl");
    };
    // duckdb_views.sql hands us the statement directly.
    assert!(ddl.to_uppercase().contains("SELECT"), "{ddl}");
}

#[tokio::test]
async fn a_hostile_search_term_returns_nothing_and_breaks_nothing() {
    let (connector, cid) = seeded().await;
    let CatalogResult::SearchHits(hits) = connector
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
        .expect("hostile term must not error")
    else {
        panic!("expected SearchHits");
    };
    assert!(hits.is_empty());

    let CatalogResult::Objects(objects) = connector
        .catalog(
            cid,
            CatalogRequest::ListAllObjects {
                kinds: vec![ObjectKind::Table],
            },
        )
        .await
        .unwrap()
    else {
        panic!("expected Objects");
    };
    assert!(objects.iter().any(|o| o.reference.name == "users"));
}

#[tokio::test]
async fn catalog_errors_are_not_mislabeled_as_syntax_errors() {
    let (connector, cid) = seeded().await;
    // A malformed namespace is a caller/engine failure, not a syntax error
    // in the user's SQL — the schema browser must not surface it as one.
    let err = connector
        .catalog(
            cid,
            CatalogRequest::ListObjects {
                namespace: vec!["a".into(), "b".into(), "c".into()],
                kinds: vec![],
            },
        )
        .await
        .unwrap_err();
    assert_eq!(
        err.kind,
        lucent_protocol::LucentErrorKind::Internal,
        "{err}"
    );
    assert!(
        err.message.contains("catalog.schema"),
        "the engine message must survive: {err}"
    );
}

#[tokio::test]
async fn test_analytics_file() {
    if !std::path::Path::new("/tmp/analytics.duckdb").exists() {
        return;
    }
    let connector = DuckDbConnector::default();
    let cid = ConnectionId(Uuid::new_v4());
    connector
        .connect(
            cid,
            ConnectionConfig::new("duckdb").with("path", "/tmp/analytics.duckdb"),
        )
        .await
        .expect("connect");

    let ns = connector
        .catalog(cid, CatalogRequest::ListNamespaces)
        .await
        .expect("list namespaces");
    eprintln!("NAMESPACES: {ns:?}");

    let CatalogResult::Namespaces(namespaces) = ns else {
        panic!()
    };
    for n in &namespaces {
        let objs = connector
            .catalog(
                cid,
                CatalogRequest::ListObjects {
                    namespace: n.path.clone(),
                    kinds: vec![],
                },
            )
            .await
            .expect("list objects with path");
        eprintln!("OBJECTS FOR {}: {:?}", n.display(), objs);

        let objs_single = connector
            .catalog(
                cid,
                CatalogRequest::ListObjects {
                    namespace: vec![n.display()],
                    kinds: vec![],
                },
            )
            .await;
        eprintln!("OBJECTS FOR SINGLE [{}]: {:?}", n.display(), objs_single);
    }
}

#[tokio::test]
async fn test_cross_connection_visibility() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_cross.duckdb");
    let path_str = path.to_string_lossy().to_string();

    let connector = DuckDbConnector::default();
    let cid1 = ConnectionId(Uuid::new_v4());
    connector
        .connect(
            cid1,
            ConnectionConfig::new("duckdb").with("path", &path_str),
        )
        .await
        .expect("connect 1");

    // Connection 1 queries before table exists
    let ns1 = connector
        .catalog(cid1, CatalogRequest::ListNamespaces)
        .await
        .unwrap();
    eprintln!("BEFORE CREATE - NS1: {ns1:?}");
    let objs1_before = connector
        .catalog(
            cid1,
            CatalogRequest::ListObjects {
                namespace: vec!["test_cross".into(), "main".into()],
                kinds: vec![],
            },
        )
        .await
        .unwrap();
    eprintln!("BEFORE CREATE - OBJS1: {objs1_before:?}");

    // Connection 2 connects and creates table
    let cid2 = ConnectionId(Uuid::new_v4());
    connector
        .connect(
            cid2,
            ConnectionConfig::new("duckdb").with("path", &path_str),
        )
        .await
        .expect("connect 2");

    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let exec = connector.execute(
        cid2,
        QueryId(Uuid::new_v4()),
        "CREATE TABLE my_table (id INT); INSERT INTO my_table VALUES (42);".into(),
        tx,
    );
    let drain = async {
        while let Some(e) = rx.recv().await {
            if let ExecutionEvent::Failed(err) = e {
                panic!("exec failed: {err}");
            }
        }
    };
    tokio::join!(exec, drain);

    // Check connection 2: does it see my_table?!
    let objs2 = connector
        .catalog(
            cid2,
            CatalogRequest::ListObjects {
                namespace: vec!["test_cross".into(), "main".into()],
                kinds: vec![],
            },
        )
        .await
        .unwrap();
    eprintln!("AFTER CREATE - OBJS2: {objs2:?}");

    // Now check connection 1: does it see my_table?!
    let ns1_after = connector
        .catalog(cid1, CatalogRequest::ListNamespaces)
        .await
        .unwrap();
    eprintln!("AFTER CREATE - NS1: {ns1_after:?}");
    let objs1_after = connector
        .catalog(
            cid1,
            CatalogRequest::ListObjects {
                namespace: vec!["test_cross".into(), "main".into()],
                kinds: vec![],
            },
        )
        .await
        .unwrap();
    eprintln!("AFTER CREATE - OBJS1: {objs1_after:?}");

    let search1 = connector
        .catalog(
            cid1,
            CatalogRequest::SearchObjects {
                query: "my_table".into(),
                kinds: vec![],
                namespace: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    eprintln!("AFTER CREATE - SEARCH1: {search1:?}");

    // Check duckdb_transactions() on cid1
    let (tx_txn, mut rx_txn) = tokio::sync::mpsc::channel(8);
    let exec_txn = connector.execute(
        cid1,
        QueryId(Uuid::new_v4()),
        "SELECT * FROM duckdb_transactions()".into(),
        tx_txn,
    );
    let drain_txn = async {
        while let Some(e) = rx_txn.recv().await {
            if let ExecutionEvent::Batch(shape, _) = e {
                eprintln!("CID1 TRANSACTIONS: {shape:?}");
            }
        }
    };
    tokio::join!(exec_txn, drain_txn);

    // Now connect cid3
    let cid3 = ConnectionId(Uuid::new_v4());
    connector
        .connect(
            cid3,
            ConnectionConfig::new("duckdb").with("path", &path_str),
        )
        .await
        .expect("connect 3");
    let objs3 = connector
        .catalog(
            cid3,
            CatalogRequest::ListObjects {
                namespace: vec!["test_cross".into(), "main".into()],
                kinds: vec![],
            },
        )
        .await
        .unwrap();
    eprintln!("AFTER CREATE - OBJS3: {objs3:?}");
}

#[test]
fn test_cloned_handle_cross_visibility() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test_shared.duckdb");
    let path_str = path.to_string_lossy().to_string();

    let h1 = lucent_driver_duckdb::connection::DuckHandle::open(&path_str, false).unwrap();
    let h2 = h1.try_clone().unwrap();

    // Query on h1 first
    let count1: i64 = h1
        .with_conn(|conn| {
            conn.query_row("SELECT count(*) FROM duckdb_tables()", [], |r| r.get(0))
                .map_err(|e| e.to_string())
        })
        .unwrap();
    assert_eq!(count1, 0);

    // Create table on h2
    h2.with_conn(|conn| {
        conn.execute_batch("CREATE TABLE shared_t (x INT)")
            .map_err(|e| e.to_string())
    })
    .unwrap();

    // Does h1 see it now?!
    let count2: i64 = h1
        .with_conn(|conn| {
            conn.query_row(
                "SELECT count(*) FROM duckdb_tables() WHERE table_name = 'shared_t'",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
    eprintln!("H1 SEES TABLE ON SHARED DB? count = {count2}");
    assert_eq!(count2, 1);
}
