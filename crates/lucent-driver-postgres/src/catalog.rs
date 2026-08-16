//! Postgres catalog queries.
//!
//! Every piece of `information_schema` / `pg_catalog` SQL in Lucent lives here
//! and nowhere else. The app above the `Connector` trait sees only normalized
//! types; `src-tauri/tests/no_provider_sql_test.rs` enforces that.
//!
//! These queries use the **extended** protocol (`client.query`), unlike
//! `connector.rs`'s `simple_query_raw` path, because we author this SQL and can
//! therefore bind user-supplied identifiers as parameters instead of escaping
//! them into a string.

use lucent_protocol::{
    CatalogRequest, CatalogResult, ColumnDetail, ColumnPath, ForeignKey, ForeignKeyTarget,
    LucentError, LucentErrorKind, Namespace, NamespacePath, ObjectDetail, ObjectKind,
    ObjectProperty, ObjectRef, ObjectSummary, PartitionInfo, SearchHit,
};
use tokio_postgres::Client;

/// Schemas that are Postgres's own bookkeeping and must never be surfaced.
const SYSTEM_SCHEMA_FILTER: &str =
    "n.nspname NOT IN ('pg_catalog', 'information_schema', 'pg_toast') \
     AND n.nspname NOT LIKE 'pg\\_temp%' AND n.nspname NOT LIKE 'pg\\_toast\\_temp%'";

fn db_err(context: &str, e: tokio_postgres::Error) -> LucentError {
    let detail = match e.as_db_error() {
        Some(db) => db.to_string(),
        None => e.to_string(),
    };
    LucentError::new(LucentErrorKind::Internal, format!("{context}: {detail}"))
}

/// Postgres has exactly one namespace segment. Anything longer is a caller bug.
fn single_segment(path: &NamespacePath) -> Result<&str, LucentError> {
    match path.as_slice() {
        [one] => Ok(one.as_str()),
        _ => Err(LucentError::new(
            LucentErrorKind::Internal,
            format!("postgres namespaces have exactly one segment, got {path:?}"),
        )),
    }
}

/// Map `pg_class.relkind` onto the normalized kind.
fn kind_from_relkind(relkind: &str) -> ObjectKind {
    match relkind {
        // Partitioned parents are tables as far as the app is concerned.
        "r" | "p" => ObjectKind::Table,
        "v" => ObjectKind::View,
        "m" => ObjectKind::MaterializedView,
        "S" => ObjectKind::Sequence,
        other => ObjectKind::Other(other.to_string()),
    }
}

/// The `relkind` values a kind filter selects. An empty filter means all of
/// them. Values are compile-time constants, so interpolating them is safe.
fn relkinds_for(kinds: &[ObjectKind]) -> Vec<&'static str> {
    if kinds.is_empty() {
        return vec!["r", "p", "v", "m", "S"];
    }
    let mut out = Vec::new();
    for k in kinds {
        match k {
            ObjectKind::Table => out.extend_from_slice(&["r", "p"]),
            ObjectKind::View => out.push("v"),
            ObjectKind::MaterializedView => out.push("m"),
            ObjectKind::Sequence => out.push("S"),
            _ => {}
        }
    }
    out
}

fn wants_functions(kinds: &[ObjectKind]) -> bool {
    kinds.is_empty() || kinds.contains(&ObjectKind::Function)
}

pub async fn handle(
    client: &Client,
    request: CatalogRequest,
) -> Result<CatalogResult, LucentError> {
    match request {
        CatalogRequest::ListNamespaces => {
            list_namespaces(client).await.map(CatalogResult::Namespaces)
        }
        CatalogRequest::ListObjects { namespace, kinds } => {
            let schema = single_segment(&namespace)?.to_string();
            list_objects(client, Some(&schema), &kinds)
                .await
                .map(CatalogResult::Objects)
        }
        CatalogRequest::ListAllObjects { kinds } => list_objects(client, None, &kinds)
            .await
            .map(CatalogResult::Objects),
        CatalogRequest::DescribeObjects { refs } => describe_objects(client, &refs)
            .await
            .map(CatalogResult::ObjectDetails),
        CatalogRequest::ListForeignKeys => list_foreign_keys(client)
            .await
            .map(CatalogResult::ForeignKeys),
        CatalogRequest::SearchObjects {
            query,
            kinds,
            namespace,
            limit,
        } => {
            let schema = namespace.as_ref().map(single_segment).transpose()?;
            search_objects(client, &query, &kinds, schema, limit)
                .await
                .map(CatalogResult::SearchHits)
        }
        CatalogRequest::GetObjectDdl { reference } => {
            object_ddl(client, &reference).await.map(CatalogResult::Ddl)
        }
        CatalogRequest::GetObjectProperties { reference } => object_properties(client, &reference)
            .await
            .map(CatalogResult::Properties),
        // `CatalogRequest` is #[non_exhaustive]; a newer app could ask something
        // this worker does not know. Name it rather than answering wrongly.
        other => Err(LucentError::new(
            LucentErrorKind::Internal,
            format!("unsupported catalog request: {other:?}"),
        )),
    }
}

async fn list_namespaces(client: &Client) -> Result<Vec<Namespace>, LucentError> {
    // One query, one round trip. Counts relations AND functions, which is what
    // the sidebar shows. Materialized views ('m') are included — the
    // information_schema-based count this replaces silently omitted them.
    let sql = format!(
        "SELECT n.nspname, \
                (SELECT count(*) FROM pg_class c \
                  WHERE c.relnamespace = n.oid AND c.relkind IN ('r','p','v','m','S')) \
              + (SELECT count(*) FROM pg_proc p \
                  WHERE p.pronamespace = n.oid AND p.prokind = 'f') AS object_count \
         FROM pg_namespace n \
         WHERE {SYSTEM_SCHEMA_FILTER} \
         ORDER BY n.nspname"
    );

    let rows = client
        .query(sql.as_str(), &[])
        .await
        .map_err(|e| db_err("list namespaces", e))?;

    Ok(rows
        .iter()
        .map(|r| {
            let name: String = r.get(0);
            let count: i64 = r.get(1);
            Namespace {
                path: vec![name],
                object_count: Some(count.max(0) as u64),
            }
        })
        .collect())
}

async fn list_objects(
    client: &Client,
    schema: Option<&str>,
    kinds: &[ObjectKind],
) -> Result<Vec<ObjectSummary>, LucentError> {
    let mut out = Vec::new();
    let relkinds = relkinds_for(kinds);

    if !relkinds.is_empty() {
        let kind_list = relkinds
            .iter()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<_>>()
            .join(",");
        let schema_pred = if schema.is_some() {
            "AND n.nspname = $1"
        } else {
            ""
        };

        // `n_live_tup` (stats collector) is preferred over `reltuples` (planner
        // estimate) because it is what the sidebar has always shown. The
        // estimate is gated on the table having been analyzed at least once:
        // `reltuples = -1` marks a never-estimated table only on PG 12+ — PG 11
        // initializes it to 0 at CREATE TABLE, so the timestamp check is the
        // version-independent "has an estimate" signal. A never-analyzed table
        // must read as None (unknown), never as Some(0) (empty).
        let sql = format!(
            "SELECT n.nspname, c.relname, c.relkind::text, \
                    CASE WHEN st.last_analyze IS NULL AND st.last_autoanalyze IS NULL \
                         THEN NULL \
                         ELSE COALESCE(st.n_live_tup, NULLIF(c.reltuples, -1)::bigint) END \
                        AS est_rows, \
                    (inh.inhrelid IS NOT NULL) AS is_partition_child, \
                    (SELECT count(*) FROM pg_inherits pi WHERE pi.inhparent = c.oid) \
                        AS partition_children, \
                    CASE WHEN c.relkind = 'p' THEN pg_get_partkeydef(c.oid) END AS partition_key, \
                    obj_description(c.oid, 'pg_class') AS comment \
             FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             LEFT JOIN pg_inherits inh ON inh.inhrelid = c.oid \
             LEFT JOIN pg_stat_user_tables st ON st.relid = c.oid \
             WHERE c.relkind IN ({kind_list}) AND {SYSTEM_SCHEMA_FILTER} {schema_pred} \
             ORDER BY n.nspname, c.relname"
        );

        let rows = match schema {
            Some(s) => client.query(sql.as_str(), &[&s]).await,
            None => client.query(sql.as_str(), &[]).await,
        }
        .map_err(|e| db_err("list objects", e))?;

        for r in &rows {
            let namespace: String = r.get(0);
            let name: String = r.get(1);
            let relkind: String = r.get(2);
            let est_rows: Option<i64> = r.get(3);
            let is_partition_child: bool = r.get(4);
            let partition_children: i64 = r.get(5);
            let partition_key: Option<String> = r.get(6);
            let comment: Option<String> = r.get(7);

            out.push(ObjectSummary {
                reference: ObjectRef {
                    namespace: vec![namespace],
                    name,
                    kind: kind_from_relkind(&relkind),
                },
                est_rows: est_rows.filter(|n| *n >= 0).map(|n| n as u64),
                comment,
                partition: partition_key.map(|k| PartitionInfo {
                    key: Some(k),
                    child_count: partition_children.max(0) as u64,
                }),
                is_partition_child,
            });
        }
    }

    if wants_functions(kinds) {
        let schema_pred = if schema.is_some() {
            "AND n.nspname = $1"
        } else {
            ""
        };
        let sql = format!(
            "SELECT n.nspname, p.proname, obj_description(p.oid, 'pg_proc') \
             FROM pg_proc p \
             JOIN pg_namespace n ON n.oid = p.pronamespace \
             WHERE p.prokind = 'f' AND {SYSTEM_SCHEMA_FILTER} {schema_pred} \
             ORDER BY n.nspname, p.proname"
        );
        let rows = match schema {
            Some(s) => client.query(sql.as_str(), &[&s]).await,
            None => client.query(sql.as_str(), &[]).await,
        }
        .map_err(|e| db_err("list functions", e))?;

        for r in &rows {
            let namespace: String = r.get(0);
            let name: String = r.get(1);
            let comment: Option<String> = r.get(2);
            out.push(ObjectSummary {
                reference: ObjectRef {
                    namespace: vec![namespace],
                    name,
                    kind: ObjectKind::Function,
                },
                est_rows: None,
                comment,
                partition: None,
                is_partition_child: false,
            });
        }
    }

    Ok(out)
}

/// Every foreign key in the database, one row per referencing column.
///
/// Built from `pg_constraint`, not `information_schema`: the latter joins via
/// `referential_constraints.unique_constraint_name` and therefore finds nothing
/// when an FK references a bare unique *index* rather than a named constraint.
/// `unnest(conkey, confkey) WITH ORDINALITY` pairs composite-key columns by
/// position, which is what makes multi-column FKs correct.
async fn list_foreign_keys(client: &Client) -> Result<Vec<ForeignKey>, LucentError> {
    let sql = format!(
        "SELECT fn.nspname, fc.relname, fa.attname, \
                tn.nspname, tc.relname, ta.attname \
         FROM pg_constraint con \
         JOIN pg_class fc ON fc.oid = con.conrelid \
         JOIN pg_namespace fn ON fn.oid = fc.relnamespace \
         JOIN pg_class tc ON tc.oid = con.confrelid \
         JOIN pg_namespace tn ON tn.oid = tc.relnamespace \
         JOIN LATERAL unnest(con.conkey, con.confkey) \
              WITH ORDINALITY AS u(from_attnum, to_attnum, ord) ON true \
         JOIN pg_attribute fa ON fa.attrelid = con.conrelid AND fa.attnum = u.from_attnum \
         JOIN pg_attribute ta ON ta.attrelid = con.confrelid AND ta.attnum = u.to_attnum \
         WHERE con.contype = 'f' \
           AND {} \
         ORDER BY fn.nspname, fc.relname, u.ord",
        SYSTEM_SCHEMA_FILTER.replace("n.nspname", "fn.nspname")
    );

    let rows = client
        .query(sql.as_str(), &[])
        .await
        .map_err(|e| db_err("list foreign keys", e))?;

    Ok(rows
        .iter()
        .map(|r| ForeignKey {
            from: ColumnPath {
                namespace: vec![r.get::<_, String>(0)],
                table: r.get(1),
                column: r.get(2),
            },
            to: ColumnPath {
                namespace: vec![r.get::<_, String>(3)],
                table: r.get(4),
                column: r.get(5),
            },
        })
        .collect())
}

/// Columns, keys, defaults, and comments for a batch of objects.
///
/// Two queries total regardless of object count: one for columns, one for the
/// foreign keys that annotate them. Built on `pg_attribute` rather than
/// `information_schema.columns` because the latter excludes materialized views
/// and drops type modifiers.
async fn describe_objects(
    client: &Client,
    refs: &[ObjectRef],
) -> Result<Vec<ObjectDetail>, LucentError> {
    if refs.is_empty() {
        return Ok(Vec::new());
    }

    // Bind the batch as two parallel arrays and join against them — one
    // statement for N objects, with no user text interpolated into SQL.
    let schemas: Vec<String> = refs
        .iter()
        .map(|r| single_segment(&r.namespace).map(str::to_string))
        .collect::<Result<_, _>>()?;
    let names: Vec<String> = refs.iter().map(|r| r.name.clone()).collect();

    // `attnum` ordering is declaration order. `pk` is a semi-join, so a column
    // in a composite PK contributes exactly one row (a LEFT JOIN on
    // key_column_usage would duplicate it once per referencing constraint).
    let sql = "\
        SELECT n.nspname, c.relname, c.relkind::text, a.attname, \
               format_type(a.atttypid, a.atttypmod) AS type_name, \
               NOT a.attnotnull AS nullable, a.attnum, \
               pg_get_expr(d.adbin, d.adrelid) AS default_expr, \
               col_description(c.oid, a.attnum) AS column_comment, \
               obj_description(c.oid, 'pg_class') AS object_comment, \
               EXISTS ( \
                   SELECT 1 FROM pg_constraint pk \
                   WHERE pk.conrelid = c.oid AND pk.contype = 'p' \
                     AND a.attnum = ANY(pk.conkey) \
               ) AS is_primary_key \
        FROM unnest($1::text[], $2::text[]) AS req(schema_name, object_name) \
        JOIN pg_namespace n ON n.nspname = req.schema_name \
        JOIN pg_class c ON c.relnamespace = n.oid AND c.relname = req.object_name \
        JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum > 0 AND NOT a.attisdropped \
        LEFT JOIN pg_attrdef d ON d.adrelid = c.oid AND d.adnum = a.attnum \
        ORDER BY n.nspname, c.relname, a.attnum";

    let rows = client
        .query(sql, &[&schemas, &names])
        .await
        .map_err(|e| db_err("describe objects", e))?;

    // Annotate referencing columns with their FK target.
    let fks = list_foreign_keys(client).await?;
    let fk_index: std::collections::HashMap<(String, String, String), &ForeignKey> = fks
        .iter()
        .map(|f| {
            (
                (
                    f.from.namespace.join("."),
                    f.from.table.clone(),
                    f.from.column.clone(),
                ),
                f,
            )
        })
        .collect();

    // Preserve the caller's request order — `get_objects_info` renders results
    // in the order the model asked for them.
    let mut details: Vec<ObjectDetail> = refs
        .iter()
        .map(|r| ObjectDetail {
            reference: r.clone(),
            columns: Vec::new(),
            comment: None,
        })
        .collect();
    let mut index: std::collections::HashMap<(String, String), usize> =
        std::collections::HashMap::new();
    for (i, r) in refs.iter().enumerate() {
        index.insert((r.namespace.join("."), r.name.clone()), i);
    }

    for r in &rows {
        let namespace: String = r.get(0);
        let object: String = r.get(1);
        let relkind: String = r.get(2);
        let Some(&slot) = index.get(&(namespace.clone(), object.clone())) else {
            continue;
        };

        let name: String = r.get(3);
        let object_comment: Option<String> = r.get(9);

        // Trust the server's relkind over whatever the caller guessed.
        details[slot].reference.kind = kind_from_relkind(&relkind);
        if details[slot].comment.is_none() {
            details[slot].comment = object_comment;
        }

        let attnum: i16 = r.get(6);
        details[slot].columns.push(ColumnDetail {
            foreign_key: fk_index
                .get(&(namespace.clone(), object.clone(), name.clone()))
                .map(|f| ForeignKeyTarget {
                    namespace: f.to.namespace.clone(),
                    table: f.to.table.clone(),
                    column: f.to.column.clone(),
                }),
            name,
            type_name: r.get(4),
            nullable: r.get(5),
            ordinal: attnum.max(0) as u32,
            default: r.get(7),
            comment: r.get(8),
            is_primary_key: r.get(10),
        });
    }

    Ok(details)
}

/// Name search over objects and columns.
///
/// Uses `pg_trgm` similarity when the extension is installed and falls back to
/// `ILIKE` otherwise. The search term is always a bind parameter — never
/// interpolated — which is the difference from the code this replaces.
async fn search_objects(
    client: &Client,
    query: &str,
    kinds: &[ObjectKind],
    schema: Option<&str>,
    limit: u32,
) -> Result<Vec<SearchHit>, LucentError> {
    let limit = limit.clamp(1, 200) as i64;

    let has_trgm = client
        .query_one(
            "SELECT count(*) > 0 FROM pg_extension WHERE extname = 'pg_trgm'",
            &[],
        )
        .await
        .map(|r| r.get::<_, bool>(0))
        .unwrap_or(false);

    let relkinds = relkinds_for(kinds);
    // Empty `kinds` (tables and views only for search) OR a non-empty filter
    // that maps to no relkinds (Function, Other escape hatches) must fall
    // back — an empty list would emit `c.relkind IN ()`, a syntax error.
    let kind_list = if kinds.is_empty() || relkinds.is_empty() {
        "'r','v'".to_string()
    } else {
        relkinds
            .iter()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let schema_pred = if schema.is_some() {
        "AND n.nspname = $3"
    } else {
        ""
    };

    let object_sql = if has_trgm {
        format!(
            "SELECT n.nspname, c.relname, c.relkind::text, similarity(c.relname, $1) AS score \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ({kind_list}) AND {SYSTEM_SCHEMA_FILTER} {schema_pred} \
               AND similarity(c.relname, $1) > 0.1 \
             ORDER BY score DESC LIMIT $2"
        )
    } else {
        format!(
            "SELECT n.nspname, c.relname, c.relkind::text, 0.5::float4 AS score \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ({kind_list}) AND {SYSTEM_SCHEMA_FILTER} {schema_pred} \
               AND c.relname ILIKE '%' || $1 || '%' \
             ORDER BY c.relname LIMIT $2"
        )
    };

    let object_rows = match schema {
        Some(s) => {
            client
                .query(object_sql.as_str(), &[&query, &limit, &s])
                .await
        }
        None => client.query(object_sql.as_str(), &[&query, &limit]).await,
    }
    .map_err(|e| db_err("search objects", e))?;

    let mut hits: Vec<SearchHit> = object_rows
        .iter()
        .map(|r| {
            let relkind: String = r.get(2);
            SearchHit {
                reference: ObjectRef {
                    namespace: vec![r.get::<_, String>(0)],
                    name: r.get(1),
                    kind: kind_from_relkind(&relkind),
                },
                column: None,
                score: r.get::<_, f32>(3),
            }
        })
        .collect();

    // Column names, always ILIKE — column counts are small enough that trigram
    // indexing buys nothing here.
    let column_sql = format!(
        "SELECT n.nspname, c.relname, c.relkind::text, a.attname \
         FROM pg_attribute a \
         JOIN pg_class c ON c.oid = a.attrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE a.attnum > 0 AND NOT a.attisdropped \
           AND c.relkind IN ({kind_list}) AND {SYSTEM_SCHEMA_FILTER} {schema_pred} \
           AND a.attname ILIKE '%' || $1 || '%' \
         ORDER BY n.nspname, c.relname, a.attnum LIMIT $2"
    );

    let column_rows = match schema {
        Some(s) => {
            client
                .query(column_sql.as_str(), &[&query, &limit, &s])
                .await
        }
        None => client.query(column_sql.as_str(), &[&query, &limit]).await,
    }
    .map_err(|e| db_err("search columns", e))?;

    let needle = query.to_lowercase();
    for r in &column_rows {
        let relkind: String = r.get(2);
        let column: String = r.get(3);
        // An exact column-name match outranks a substring one.
        let score = if column.to_lowercase() == needle {
            1.0
        } else {
            0.7
        };
        hits.push(SearchHit {
            reference: ObjectRef {
                namespace: vec![r.get::<_, String>(0)],
                name: r.get(1),
                kind: kind_from_relkind(&relkind),
            },
            column: Some(column),
            score,
        });
    }

    Ok(hits)
}

/// Reconstructed DDL for one object.
async fn object_ddl(client: &Client, reference: &ObjectRef) -> Result<String, LucentError> {
    let schema = single_segment(&reference.namespace)?;
    let name = &reference.name;

    match reference.kind {
        ObjectKind::Function => {
            let row = client
                .query_opt(
                    "SELECT pg_get_functiondef(p.oid) \
                     FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                     WHERE n.nspname = $1 AND p.proname = $2 \
                     ORDER BY p.oid LIMIT 1",
                    &[&schema, &name],
                )
                .await
                .map_err(|e| db_err("function ddl", e))?;
            Ok(row
                .map(|r| r.get::<_, String>(0))
                .unwrap_or_else(|| "-- no source found".to_string()))
        }
        ObjectKind::View | ObjectKind::MaterializedView => {
            let row = client
                .query_opt(
                    "SELECT pg_get_viewdef(c.oid, true) \
                     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                     WHERE n.nspname = $1 AND c.relname = $2 \
                     LIMIT 1",
                    &[&schema, &name],
                )
                .await
                .map_err(|e| db_err("view ddl", e))?;
            let Some(body) = row.map(|r| r.get::<_, String>(0)) else {
                return Ok("-- no source found".to_string());
            };
            let keyword = if reference.kind == ObjectKind::MaterializedView {
                "CREATE MATERIALIZED VIEW"
            } else {
                "CREATE OR REPLACE VIEW"
            };
            Ok(format!(
                "{keyword} {}.{} AS\n{body}",
                quote_ident(schema),
                quote_ident(name)
            ))
        }
        _ => Ok(format!(
            "-- DDL reconstruction is not supported for {} objects",
            reference.kind.as_str()
        )),
    }
}

/// Double-quote an identifier for DDL output.
fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// Driver-defined key/value properties. Currently sequences; the key order is
/// the sidebar's display order.
async fn object_properties(
    client: &Client,
    reference: &ObjectRef,
) -> Result<Vec<ObjectProperty>, LucentError> {
    let schema = single_segment(&reference.namespace)?;
    let name = &reference.name;

    if reference.kind != ObjectKind::Sequence {
        return Ok(Vec::new());
    }

    let row = client
        .query_opt(
            "SELECT s.data_type::text, s.start_value::text, s.minimum_value::text, \
                    s.maximum_value::text, s.increment::text, s.cycle_option::text \
             FROM information_schema.sequences s \
             WHERE s.sequence_schema = $1 AND s.sequence_name = $2",
            &[&schema, &name],
        )
        .await
        .map_err(|e| db_err("sequence properties", e))?;

    let Some(row) = row else {
        return Ok(Vec::new());
    };

    let get = |i: usize| -> String { row.get::<_, Option<String>>(i).unwrap_or_default() };
    Ok(vec![
        ObjectProperty {
            key: "Data Type".into(),
            value: get(0),
        },
        ObjectProperty {
            key: "Start Value".into(),
            value: get(1),
        },
        ObjectProperty {
            key: "Min Value".into(),
            value: get(2),
        },
        ObjectProperty {
            key: "Max Value".into(),
            value: get(3),
        },
        ObjectProperty {
            key: "Increment".into(),
            value: get(4),
        },
        ObjectProperty {
            key: "Cycles".into(),
            value: get(5),
        },
    ])
}
