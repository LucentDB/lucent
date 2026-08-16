//! Driver-agnostic conformance suite.
//!
//! Every driver must pass this against a database seeded with the schema in
//! [`SEED_SQL`]. The point is that "done" for a new driver is a command to run,
//! not a document to read.
//!
//! Deliberately does NOT test SQL dialect specifics — those belong to each
//! driver's own tests. What it tests is the *contract* the app relies on:
//! batching, typed values, catalog shapes, cancellation, and error behaviour.

use lucent_protocol::{
    CatalogRequest, CatalogResult, ConnectionId, ObjectKind, QueryId, ResultShape, Value,
};
use lucent_worker_host::{Connector, ExecutionEvent};
use uuid::Uuid;

/// The schema every driver's conformance test must create before calling
/// [`run_all`]. Written in the intersection of PostgreSQL and DuckDB syntax.
pub const SEED_SQL: &str = "
    CREATE TABLE conformance_parent (id BIGINT PRIMARY KEY, label VARCHAR NOT NULL);
    CREATE TABLE conformance_child (
        id BIGINT PRIMARY KEY,
        parent_id BIGINT REFERENCES conformance_parent(id)
    );
    CREATE VIEW conformance_view AS SELECT id FROM conformance_parent;
    INSERT INTO conformance_parent VALUES (1, 'a'), (2, 'b');
";

#[derive(Debug)]
pub struct ConformanceFailure {
    pub check: &'static str,
    pub detail: String,
}

fn fail(check: &'static str, detail: impl Into<String>) -> ConformanceFailure {
    ConformanceFailure {
        check,
        detail: detail.into(),
    }
}

async fn collect<C: Connector>(
    connector: &C,
    cid: ConnectionId,
    sql: &str,
) -> Result<(Vec<Vec<Value>>, usize, bool), String> {
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);
    let exec = connector.execute(cid, QueryId(Uuid::new_v4()), sql.to_string(), tx);
    let drain = async {
        let mut rows = Vec::new();
        let mut batches = 0;
        let mut saw_final = false;
        let mut error = None;
        while let Some(event) = rx.recv().await {
            match event {
                ExecutionEvent::Batch(ResultShape::Tabular { rows: batch, .. }, is_final) => {
                    batches += 1;
                    saw_final |= is_final;
                    rows.extend(batch);
                }
                ExecutionEvent::Batch(..) => {}
                ExecutionEvent::Failed(e) => error = Some(e.to_string()),
            }
        }
        (rows, batches, saw_final, error)
    };
    let (_, (rows, batches, saw_final, error)) = tokio::join!(exec, drain);
    match error {
        Some(e) => Err(e),
        None => Ok((rows, batches, saw_final)),
    }
}

/// Run every conformance check. An empty result means the driver conforms.
pub async fn run_all<C: Connector>(connector: &C, cid: ConnectionId) -> Vec<ConformanceFailure> {
    let mut failures = Vec::new();

    // 1. A trivial query returns exactly one row and one final batch.
    match collect(connector, cid, "SELECT 1").await {
        Ok((rows, _, saw_final)) => {
            if rows.len() != 1 {
                failures.push(fail(
                    "trivial_query",
                    format!("expected 1 row, got {}", rows.len()),
                ));
            }
            if !saw_final {
                failures.push(fail(
                    "final_batch_flag",
                    "no batch was flagged final; the app's execute loop would hang forever",
                ));
            }
        }
        Err(e) => failures.push(fail("trivial_query", e)),
    }

    // 2. An empty result still terminates.
    match collect(
        connector,
        cid,
        "SELECT id FROM conformance_parent WHERE id < 0",
    )
    .await
    {
        Ok((rows, _, saw_final)) => {
            if !rows.is_empty() {
                failures.push(fail("empty_result", "expected no rows"));
            }
            if !saw_final {
                failures.push(fail(
                    "empty_result",
                    "an empty result must still send a final batch",
                ));
            }
        }
        Err(e) => failures.push(fail("empty_result", e)),
    }

    // 3. NULL survives as NULL, not as an empty string.
    match collect(connector, cid, "SELECT NULL").await {
        Ok((rows, _, _)) => {
            if !matches!(rows.first().and_then(|r| r.first()), Some(Value::Null)) {
                failures.push(fail("null_value", format!("got {:?}", rows.first())));
            }
        }
        Err(e) => failures.push(fail("null_value", e)),
    }

    // 4. Integers decode as integers, not as text.
    match collect(connector, cid, "SELECT 42").await {
        Ok((rows, _, _)) => {
            if !matches!(rows.first().and_then(|r| r.first()), Some(Value::Int64(42))) {
                failures.push(fail(
                    "typed_integer",
                    format!("expected Int64(42), got {:?}", rows.first()),
                ));
            }
        }
        Err(e) => failures.push(fail("typed_integer", e)),
    }

    // 5. A syntax error is an error, and does not poison the connection.
    if collect(connector, cid, "SELEKT 1").await.is_ok() {
        failures.push(fail("syntax_error", "invalid SQL must report Failed"));
    }
    if collect(connector, cid, "SELECT 1").await.is_err() {
        failures.push(fail(
            "connection_survives_error",
            "the connection must still work after a failed query",
        ));
    }

    // 6. Catalog: namespaces exist and none is empty-pathed.
    match connector.catalog(cid, CatalogRequest::ListNamespaces).await {
        Ok(CatalogResult::Namespaces(namespaces)) => {
            if namespaces.is_empty() {
                failures.push(fail("list_namespaces", "no namespaces returned"));
            }
            if namespaces.iter().any(|n| n.path.is_empty()) {
                failures.push(fail("list_namespaces", "a namespace has an empty path"));
            }
        }
        Ok(other) => failures.push(fail("list_namespaces", format!("wrong variant: {other:?}"))),
        Err(e) => failures.push(fail("list_namespaces", e.to_string())),
    }

    // 7. Catalog: the seeded table and view are both listed, with the right kinds.
    let parent_ref = match connector
        .catalog(cid, CatalogRequest::ListAllObjects { kinds: vec![] })
        .await
    {
        Ok(CatalogResult::Objects(objects)) => {
            let parent = objects
                .iter()
                .find(|o| o.reference.name == "conformance_parent");
            match parent {
                Some(o) if o.reference.kind == ObjectKind::Table => {}
                Some(o) => failures.push(fail(
                    "list_objects",
                    format!("conformance_parent has kind {:?}", o.reference.kind),
                )),
                None => failures.push(fail("list_objects", "conformance_parent not listed")),
            }
            if !objects.iter().any(|o| {
                o.reference.name == "conformance_view" && o.reference.kind == ObjectKind::View
            }) {
                failures.push(fail(
                    "list_objects",
                    "conformance_view not listed as a view",
                ));
            }
            parent.map(|o| o.reference.clone())
        }
        Ok(other) => {
            failures.push(fail("list_objects", format!("wrong variant: {other:?}")));
            None
        }
        Err(e) => {
            failures.push(fail("list_objects", e.to_string()));
            None
        }
    };

    // 8. Catalog: columns, ordinals, nullability, primary keys.
    if let Some(reference) = parent_ref {
        match connector
            .catalog(
                cid,
                CatalogRequest::DescribeObjects {
                    refs: vec![reference],
                },
            )
            .await
        {
            Ok(CatalogResult::ObjectDetails(details)) => {
                let Some(detail) = details.first() else {
                    failures.push(fail("describe_objects", "no detail returned"));
                    return failures;
                };
                let id = detail.columns.iter().find(|c| c.name == "id");
                let label = detail.columns.iter().find(|c| c.name == "label");
                match id {
                    Some(c) if c.is_primary_key => {}
                    Some(_) => {
                        failures.push(fail("describe_objects", "id is not flagged primary key"))
                    }
                    None => failures.push(fail("describe_objects", "id column missing")),
                }
                match label {
                    Some(c) if !c.nullable => {}
                    Some(_) => failures.push(fail("describe_objects", "NOT NULL not reported")),
                    None => failures.push(fail("describe_objects", "label column missing")),
                }
                if detail.columns.iter().any(|c| c.ordinal == 0) {
                    failures.push(fail("describe_objects", "ordinals must be 1-based"));
                }
                let mut names: Vec<&str> = detail.columns.iter().map(|c| c.name.as_str()).collect();
                names.sort_unstable();
                names.dedup();
                if names.len() != detail.columns.len() {
                    failures.push(fail(
                        "describe_objects",
                        "duplicate columns — a constraint join is fanning out",
                    ));
                }
            }
            Ok(other) => failures.push(fail(
                "describe_objects",
                format!("wrong variant: {other:?}"),
            )),
            Err(e) => failures.push(fail("describe_objects", e.to_string())),
        }
    }

    // 9. Catalog: the seeded foreign key is discoverable.
    match connector
        .catalog(cid, CatalogRequest::ListForeignKeys)
        .await
    {
        Ok(CatalogResult::ForeignKeys(fks)) => {
            if !fks.iter().any(|f| {
                f.from.table == "conformance_child"
                    && f.from.column == "parent_id"
                    && f.to.table == "conformance_parent"
                    && f.to.column == "id"
            }) {
                failures.push(fail(
                    "list_foreign_keys",
                    format!("FK not found in {fks:?}"),
                ));
            }
        }
        Ok(other) => failures.push(fail(
            "list_foreign_keys",
            format!("wrong variant: {other:?}"),
        )),
        Err(e) => failures.push(fail("list_foreign_keys", e.to_string())),
    }

    // 10. Cancelling a query that is not running must be a harmless no-op.
    if connector
        .cancel(cid, QueryId(Uuid::new_v4()))
        .await
        .is_err()
    {
        failures.push(fail(
            "stale_cancel",
            "cancelling an unknown query must not error",
        ));
    }

    failures
}
