//! DuckDB catalog queries, built on the `duckdb_*` metadata functions.
//!
//! Blocking — every entry point is called from inside `spawn_blocking`.
//!
//! DuckDB namespaces have two segments (`catalog`.`schema`), which is the first
//! real exercise of `NamespaceModel::CatalogSchema` and of the protocol's
//! `Vec<String>` path.

use duckdb::Connection;
use lucent_protocol::{
    CatalogRequest, CatalogResult, ColumnDetail, ColumnPath, ForeignKey, ForeignKeyTarget,
    LucentError, LucentErrorKind, Namespace, NamespacePath, ObjectDetail, ObjectKind,
    ObjectProperty, ObjectRef, ObjectSummary, SearchHit,
};

use crate::connection::DuckHandle;

/// Catalogs DuckDB manages for itself.
const SYSTEM_CATALOGS: &str = "('system', 'temp')";

fn err(message: impl Into<String>) -> LucentError {
    LucentError::new(LucentErrorKind::Internal, message)
}

/// DuckDB namespaces are `catalog.schema`. Anything else is a caller bug.
fn split_namespace(path: &NamespacePath) -> Result<(&str, &str), LucentError> {
    match path.as_slice() {
        [catalog, schema] => Ok((catalog.as_str(), schema.as_str())),
        // Tolerate a single segment by treating it as a schema in the current
        // catalog — the AI addresses schemas by bare name and should not have
        // to know about catalogs.
        [schema] => Ok(("", schema.as_str())),
        [] => Ok(("", "")),
        _ => Err(err(format!(
            "duckdb namespaces are catalog.schema, got {path:?}"
        ))),
    }
}

pub fn handle(handle: &DuckHandle, request: CatalogRequest) -> Result<CatalogResult, LucentError> {
    handle
        .with_conn(|conn| dispatch(conn, request).map_err(|e| e.message))
        // `with_conn` labels every closure error QuerySyntaxError; a catalog
        // failure (a metadata query, an engine error) is not a syntax error.
        // The message is preserved; only the kind is corrected.
        .map_err(|e| LucentError::new(LucentErrorKind::Internal, e.message))
}

fn dispatch(conn: &Connection, request: CatalogRequest) -> Result<CatalogResult, LucentError> {
    match request {
        CatalogRequest::ListNamespaces => list_namespaces(conn).map(CatalogResult::Namespaces),
        CatalogRequest::ListObjects { namespace, kinds } => {
            let (_, schema) = split_namespace(&namespace)?;
            list_objects(conn, Some(schema), &kinds).map(CatalogResult::Objects)
        }
        CatalogRequest::ListAllObjects { kinds } => {
            list_objects(conn, None, &kinds).map(CatalogResult::Objects)
        }
        CatalogRequest::DescribeObjects { refs } => {
            describe_objects(conn, &refs).map(CatalogResult::ObjectDetails)
        }
        CatalogRequest::ListForeignKeys => list_foreign_keys(conn).map(CatalogResult::ForeignKeys),
        CatalogRequest::SearchObjects {
            query,
            namespace,
            limit,
            ..
        } => {
            let schema = namespace
                .as_ref()
                .map(split_namespace)
                .transpose()?
                .map(|(_, s)| s.to_string());
            search_objects(conn, &query, schema.as_deref(), limit).map(CatalogResult::SearchHits)
        }
        CatalogRequest::GetObjectDdl { reference } => {
            object_ddl(conn, &reference).map(CatalogResult::Ddl)
        }
        CatalogRequest::GetObjectProperties { reference } => {
            object_properties(conn, &reference).map(CatalogResult::Properties)
        }
        other => Err(err(format!("unsupported catalog request: {other:?}"))),
    }
}

fn list_namespaces(conn: &Connection) -> Result<Vec<Namespace>, LucentError> {
    let sql = format!(
        "SELECT s.database_name, s.schema_name, \
                (SELECT count(*) FROM duckdb_tables() t \
                  WHERE t.database_name = s.database_name AND t.schema_name = s.schema_name) \
              + (SELECT count(*) FROM duckdb_views() v \
                  WHERE v.database_name = s.database_name AND v.schema_name = s.schema_name \
                    AND NOT v.internal) AS object_count \
         FROM duckdb_schemas() s \
         WHERE s.database_name NOT IN {SYSTEM_CATALOGS} \
           AND (NOT s.internal OR s.schema_name = 'main') \
         ORDER BY s.database_name, s.schema_name"
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| err(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Namespace {
                path: vec![row.get::<_, String>(0)?, row.get::<_, String>(1)?],
                object_count: Some(row.get::<_, i64>(2)?.max(0) as u64),
            })
        })
        .map_err(|e| err(e.to_string()))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| err(e.to_string()))
}

fn list_objects(
    conn: &Connection,
    schema: Option<&str>,
    kinds: &[ObjectKind],
) -> Result<Vec<ObjectSummary>, LucentError> {
    let want = |k: ObjectKind| kinds.is_empty() || kinds.contains(&k);
    let mut out = Vec::new();

    if want(ObjectKind::Table) {
        // `estimated_size` is DuckDB's own row estimate — the spec assumed
        // this did not exist.
        let sql = format!(
            "SELECT database_name, schema_name, table_name, estimated_size, comment \
             FROM duckdb_tables() \
             WHERE database_name NOT IN {SYSTEM_CATALOGS} AND NOT internal \
               AND ($1 IS NULL OR schema_name = $1) \
             ORDER BY database_name, schema_name, table_name"
        );
        collect_objects(conn, &sql, schema, ObjectKind::Table, true, &mut out)?;
    }

    if want(ObjectKind::View) {
        let sql = format!(
            "SELECT database_name, schema_name, view_name, NULL::BIGINT, comment \
             FROM duckdb_views() \
             WHERE database_name NOT IN {SYSTEM_CATALOGS} AND NOT internal \
               AND ($1 IS NULL OR schema_name = $1) \
             ORDER BY database_name, schema_name, view_name"
        );
        collect_objects(conn, &sql, schema, ObjectKind::View, false, &mut out)?;
    }

    if want(ObjectKind::Sequence) {
        // duckdb_sequences() reports system sequences via `temporary` at the
        // pinned version (no `internal` column exists there).
        let sql = format!(
            "SELECT database_name, schema_name, sequence_name, NULL::BIGINT, comment \
             FROM duckdb_sequences() \
             WHERE database_name NOT IN {SYSTEM_CATALOGS} AND NOT temporary \
               AND ($1 IS NULL OR schema_name = $1) \
             ORDER BY database_name, schema_name, sequence_name"
        );
        collect_objects(conn, &sql, schema, ObjectKind::Sequence, false, &mut out)?;
    }

    Ok(out)
}

fn collect_objects(
    conn: &Connection,
    sql: &str,
    schema: Option<&str>,
    kind: ObjectKind,
    has_estimate: bool,
    out: &mut Vec<ObjectSummary>,
) -> Result<(), LucentError> {
    let mut stmt = conn.prepare(sql).map_err(|e| err(e.to_string()))?;
    let rows = stmt
        .query_map([schema], |row| {
            let est: Option<i64> = if has_estimate { row.get(3)? } else { None };
            Ok(ObjectSummary {
                reference: ObjectRef {
                    namespace: vec![row.get::<_, String>(0)?, row.get::<_, String>(1)?],
                    name: row.get::<_, String>(2)?,
                    kind: kind.clone(),
                },
                est_rows: est.filter(|n| *n >= 0).map(|n| n as u64),
                comment: row.get::<_, Option<String>>(4)?,
                // DuckDB has no partitioned tables in the Postgres sense.
                partition: None,
                is_partition_child: false,
            })
        })
        .map_err(|e| err(e.to_string()))?;
    for row in rows {
        out.push(row.map_err(|e| err(e.to_string()))?);
    }
    Ok(())
}

fn describe_objects(
    conn: &Connection,
    refs: &[ObjectRef],
) -> Result<Vec<ObjectDetail>, LucentError> {
    if refs.is_empty() {
        return Ok(Vec::new());
    }

    let fks = list_foreign_keys(conn)?;
    let fk_index: std::collections::HashMap<(String, String), &ForeignKey> = fks
        .iter()
        .map(|f| ((f.from.table.clone(), f.from.column.clone()), f))
        .collect();

    // Primary-key columns, from duckdb_constraints.
    let mut pk_columns: std::collections::HashSet<(String, String, String)> = Default::default();
    {
        let mut stmt = conn
            .prepare(
                "SELECT schema_name, table_name, unnest(constraint_column_names) \
                 FROM duckdb_constraints() WHERE constraint_type = 'PRIMARY KEY'",
            )
            .map_err(|e| err(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| err(e.to_string()))?;
        for row in rows {
            pk_columns.insert(row.map_err(|e| err(e.to_string()))?);
        }
    }

    let mut stmt = conn
        .prepare(
            "SELECT column_name, data_type, is_nullable, column_index, column_default, comment \
             FROM duckdb_columns() \
             WHERE ($1 = '' OR schema_name = $1) AND table_name = $2 \
             ORDER BY column_index",
        )
        .map_err(|e| err(e.to_string()))?;

    let mut details = Vec::with_capacity(refs.len());
    for reference in refs {
        let (_, schema) = split_namespace(&reference.namespace)?;
        let rows = stmt
            .query_map([schema, reference.name.as_str()], |row| {
                let name: String = row.get(0)?;
                Ok(ColumnDetail {
                    is_primary_key: pk_columns.contains(&(
                        schema.to_string(),
                        reference.name.clone(),
                        name.clone(),
                    )) || pk_columns.iter().any(|(_, t, c)| t == &reference.name && c == &name),
                    foreign_key: fk_index
                        .get(&(reference.name.clone(), name.clone()))
                        .map(|f| ForeignKeyTarget {
                            namespace: f.to.namespace.clone(),
                            table: f.to.table.clone(),
                            column: f.to.column.clone(),
                        }),
                    name,
                    type_name: row.get(1)?,
                    nullable: row.get(2)?,
                    ordinal: row.get::<_, i64>(3)?.max(0) as u32,
                    default: row.get(4)?,
                    comment: row.get(5)?,
                })
            })
            .map_err(|e| err(e.to_string()))?;

        details.push(ObjectDetail {
            reference: reference.clone(),
            columns: rows
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| err(e.to_string()))?,
            comment: None,
        });
    }

    Ok(details)
}

fn list_foreign_keys(conn: &Connection) -> Result<Vec<ForeignKey>, LucentError> {
    // `constraint_column_names` and `referenced_column_names` are parallel
    // arrays; `unnest(a, b)` pairs them by position, which is what makes
    // composite keys correct.
    let sql = format!(
        "SELECT database_name, schema_name, table_name, \
                unnest(constraint_column_names) AS from_column, \
                referenced_table, \
                unnest(referenced_column_names) AS to_column \
         FROM duckdb_constraints() \
         WHERE constraint_type = 'FOREIGN KEY' \
           AND database_name NOT IN {SYSTEM_CATALOGS}"
    );

    let mut stmt = conn.prepare(&sql).map_err(|e| err(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            let catalog: String = row.get(0)?;
            let schema: String = row.get(1)?;
            Ok(ForeignKey {
                from: ColumnPath {
                    namespace: vec![catalog.clone(), schema.clone()],
                    table: row.get(2)?,
                    column: row.get(3)?,
                },
                to: ColumnPath {
                    // DuckDB reports the referenced table without a schema;
                    // foreign keys cannot cross schemas here, so reuse it.
                    namespace: vec![catalog, schema],
                    table: row.get(4)?,
                    column: row.get(5)?,
                },
            })
        })
        .map_err(|e| err(e.to_string()))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| err(e.to_string()))
}

fn search_objects(
    conn: &Connection,
    query: &str,
    schema: Option<&str>,
    limit: u32,
) -> Result<Vec<SearchHit>, LucentError> {
    let limit = limit.clamp(1, 200) as i64;
    let mut hits = Vec::new();

    // The term is always a bind parameter, never interpolated.
    let object_sql = format!(
        "SELECT database_name, schema_name, table_name, 'table' AS kind FROM duckdb_tables() \
          WHERE NOT internal AND database_name NOT IN {SYSTEM_CATALOGS} \
            AND ($2 IS NULL OR $2 = '' OR schema_name = $2) AND lower(table_name) LIKE '%' || lower($1) || '%' \
         UNION ALL \
         SELECT database_name, schema_name, view_name, 'view' FROM duckdb_views() \
          WHERE NOT internal AND database_name NOT IN {SYSTEM_CATALOGS} \
            AND ($2 IS NULL OR $2 = '' OR schema_name = $2) AND lower(view_name) LIKE '%' || lower($1) || '%' \
         LIMIT $3"
    );
    let mut stmt = conn.prepare(&object_sql).map_err(|e| err(e.to_string()))?;
    let rows = stmt
        .query_map(duckdb::params![query, schema, limit], |row| {
            let kind: String = row.get(3)?;
            Ok(SearchHit {
                reference: ObjectRef {
                    namespace: vec![row.get::<_, String>(0)?, row.get::<_, String>(1)?],
                    name: row.get::<_, String>(2)?,
                    kind: ObjectKind::from_label(&kind),
                },
                column: None,
                score: 0.5,
            })
        })
        .map_err(|e| err(e.to_string()))?;
    for row in rows {
        hits.push(row.map_err(|e| err(e.to_string()))?);
    }

    let column_sql = format!(
        "SELECT database_name, schema_name, table_name, column_name \
         FROM duckdb_columns() \
         WHERE database_name NOT IN {SYSTEM_CATALOGS} AND NOT internal \
           AND ($2 IS NULL OR $2 = '' OR schema_name = $2) \
           AND lower(column_name) LIKE '%' || lower($1) || '%' \
         ORDER BY schema_name, table_name, column_index \
         LIMIT $3"
    );
    let mut stmt = conn.prepare(&column_sql).map_err(|e| err(e.to_string()))?;
    let needle = query.to_lowercase();
    let rows = stmt
        .query_map(duckdb::params![query, schema, limit], |row| {
            let column: String = row.get(3)?;
            Ok(SearchHit {
                reference: ObjectRef {
                    namespace: vec![row.get::<_, String>(0)?, row.get::<_, String>(1)?],
                    name: row.get::<_, String>(2)?,
                    kind: ObjectKind::Table,
                },
                score: if column.to_lowercase() == needle {
                    1.0
                } else {
                    0.7
                },
                column: Some(column),
            })
        })
        .map_err(|e| err(e.to_string()))?;
    for row in rows {
        hits.push(row.map_err(|e| err(e.to_string()))?);
    }

    Ok(hits)
}

/// DuckDB stores each object's creating statement, so DDL needs no
/// reconstruction.
fn object_ddl(conn: &Connection, reference: &ObjectRef) -> Result<String, LucentError> {
    let (_, schema) = split_namespace(&reference.namespace)?;
    let sql = match reference.kind {
        ObjectKind::View => {
            "SELECT sql FROM duckdb_views() WHERE schema_name = $1 AND view_name = $2"
        }
        ObjectKind::Table => {
            "SELECT sql FROM duckdb_tables() WHERE schema_name = $1 AND table_name = $2"
        }
        _ => {
            return Ok(format!(
                "-- DDL is not available for {} objects",
                reference.kind.as_str()
            ))
        }
    };

    let mut stmt = conn.prepare(sql).map_err(|e| err(e.to_string()))?;
    let mut rows = stmt
        .query([schema, reference.name.as_str()])
        .map_err(|e| err(e.to_string()))?;
    match rows.next().map_err(|e| err(e.to_string()))? {
        Some(row) => Ok(row
            .get::<_, Option<String>>(0)
            .map_err(|e| err(e.to_string()))?
            .unwrap_or_else(|| "-- no source found".to_string())),
        None => Ok("-- no source found".to_string()),
    }
}

fn object_properties(
    conn: &Connection,
    reference: &ObjectRef,
) -> Result<Vec<ObjectProperty>, LucentError> {
    let (_, schema) = split_namespace(&reference.namespace)?;
    if reference.kind != ObjectKind::Sequence {
        return Ok(Vec::new());
    }

    let mut stmt = conn
        .prepare(
            "SELECT start_value, min_value, max_value, increment_by, cycle \
             FROM duckdb_sequences() WHERE schema_name = $1 AND sequence_name = $2",
        )
        .map_err(|e| err(e.to_string()))?;
    let mut rows = stmt
        .query([schema, reference.name.as_str()])
        .map_err(|e| err(e.to_string()))?;

    let Some(row) = rows.next().map_err(|e| err(e.to_string()))? else {
        return Ok(Vec::new());
    };

    let text = |i: usize| -> String {
        row.get::<_, Option<i64>>(i)
            .ok()
            .flatten()
            .map(|v| v.to_string())
            .unwrap_or_default()
    };
    Ok(vec![
        ObjectProperty {
            key: "Data Type".into(),
            value: "BIGINT".into(),
        },
        ObjectProperty {
            key: "Start Value".into(),
            value: text(0),
        },
        ObjectProperty {
            key: "Min Value".into(),
            value: text(1),
        },
        ObjectProperty {
            key: "Max Value".into(),
            value: text(2),
        },
        ObjectProperty {
            key: "Increment".into(),
            value: text(3),
        },
        ObjectProperty {
            key: "Cycles".into(),
            value: row
                .get::<_, Option<bool>>(4)
                .ok()
                .flatten()
                .map(|c| if c { "YES" } else { "NO" }.to_string())
                .unwrap_or_default(),
        },
    ])
}
