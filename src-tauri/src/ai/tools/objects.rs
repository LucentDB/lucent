use lucent_protocol::{ObjectDetail, ObjectKind, ObjectRef, SearchHit};
use serde_json::json;

use super::{AiToolContext, ToolError, ToolOutput};

/// Normalized details → the JSON shape `get_objects_info` has always returned.
/// Kept as a pure function so the shape is testable without a database.
pub(crate) fn details_to_json(details: &[ObjectDetail]) -> Vec<serde_json::Value> {
    details
        .iter()
        .map(|d| {
            let columns: Vec<serde_json::Value> = d
                .columns
                .iter()
                .map(|c| {
                    let mut col = json!({
                        "name": c.name,
                        "type": c.type_name,
                        "nullable": c.nullable,
                        "pk": c.is_primary_key,
                    });
                    if let Some(fk) = &c.foreign_key {
                        col["fk"] = json!({ "table": fk.table, "column": fk.column });
                    }
                    col
                })
                .collect();
            json!({
                "schema": d.reference.namespace.join("."),
                "name": d.reference.name,
                "columns": columns,
            })
        })
        .collect()
}

/// Normalized search hits → the JSON shape `search_schema` has always returned.
pub(crate) fn hits_to_json(hits: &[SearchHit]) -> Vec<serde_json::Value> {
    hits.iter()
        .map(|h| match &h.column {
            Some(column) => json!({
                "schema": h.reference.namespace.join("."),
                "name": format!("{}.{}", h.reference.name, column),
                "kind": "column",
                "score": h.score,
                "match_type": "column",
            }),
            None => json!({
                "schema": h.reference.namespace.join("."),
                "name": h.reference.name,
                "kind": h.reference.kind.as_str(),
                "score": h.score,
                "match_type": "table",
            }),
        })
        .collect()
}

/// Name search across objects and columns, answered by the driver.
pub(crate) async fn keyword_search_objects(
    client: &crate::client::ConnectorClient,
    connection_id: lucent_protocol::ConnectionId,
    query: &str,
    kind: Option<&str>,
    schema: Option<&str>,
) -> Result<Vec<serde_json::Value>, ToolError> {
    let kinds = kind
        .map(|k| vec![ObjectKind::from_label(k)])
        .unwrap_or_default();
    let namespace = schema.map(|s| vec![s.to_string()]);

    let hits = client
        .search_objects(connection_id, query, kinds, namespace, 10)
        .await
        .map_err(ToolError::Database)?;

    Ok(hits_to_json(&hits))
}

#[derive(Clone)]
pub struct GetObjectsInfo {
    ctx: AiToolContext,
}

impl GetObjectsInfo {
    pub fn new(ctx: AiToolContext) -> Self {
        Self { ctx }
    }
    pub fn description(&self) -> String {
        "Get detailed column, type, and constraint info for specific schema objects. \
         Supports filtering columns by name pattern or data type. \
         Request multiple objects in one call for efficiency."
            .into()
    }
    pub fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "objects": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "schema": { "type": "string" },
                            "kind": { "type": "string", "enum": ["table", "view", "function"] },
                            "name": { "type": "string" },
                            "column_filter": {
                                "type": "object",
                                "properties": {
                                    "name_pattern": { "type": "string" },
                                    "data_types": { "type": "array", "items": { "type": "string" } },
                                    "include_primary_keys": { "type": "boolean" },
                                    "include_foreign_keys": { "type": "boolean" }
                                }
                            }
                        },
                        "required": ["name"]
                    }
                },
                "include_dependencies": { "type": "boolean" },
                "sample_rows": { "type": "integer" }
            },
            "required": ["objects"]
        })
    }
    pub async fn call(
        &self,
        args: serde_json::Value,
        _ctx: &AiToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let objects = args["objects"]
            .as_array()
            .ok_or_else(|| ToolError::InvalidArgs("missing 'objects'".into()))?;
        let sample_rows = args["sample_rows"].as_u64().map(|n| n.min(20) as usize);

        let object_names: Vec<String> = objects
            .iter()
            .map(|o| {
                let s = o["schema"].as_str().unwrap_or("");
                let name = o["name"].as_str().unwrap_or("?");
                if s.is_empty() {
                    name.to_string()
                } else {
                    format!("{s}.{name}")
                }
            })
            .collect();
        log::info!("Tool 'get_objects_info' called — objects: {object_names:?}, sample_rows: {sample_rows:?}");

        let conn_id = self.ctx.connection_id.ok_or(ToolError::NotConnected)?;
        let client = self
            .ctx
            .db
            .lock()
            .await
            .clone()
            .ok_or(ToolError::NotConnected)?;

        let refs: Vec<ObjectRef> = objects
            .iter()
            .map(|o| {
                let schema_str = o["schema"].as_str().unwrap_or("");
                let namespace = if schema_str.is_empty() {
                    Vec::new()
                } else if schema_str.contains('.') {
                    schema_str.split('.').map(String::from).collect()
                } else {
                    vec![schema_str.to_string()]
                };
                ObjectRef {
                    namespace,
                    name: o["name"].as_str().unwrap_or("?").to_string(),
                    kind: o["kind"]
                        .as_str()
                        .map(ObjectKind::from_label)
                        .unwrap_or(ObjectKind::Table),
                }
            })
            .collect();

        // One request for every object, FK annotations included — the schema
        // graph is no longer needed to fill them in.
        let details = client
            .describe_objects(conn_id, refs)
            .await
            .map_err(ToolError::Database)?;

        let mut results = details_to_json(&details);

        if let Some(n) = sample_rows.filter(|n| *n > 0) {
            for (result_obj, detail) in results.iter_mut().zip(details.iter()) {
                // Sample rows are user data, not catalog data — this stays an
                // ordinary query. Plan C moves the quoting behind SqlBuilder.
                let qualified = if detail.reference.namespace.is_empty()
                    || detail.reference.namespace.iter().all(|s| s.is_empty())
                {
                    format!("\"{}\"", detail.reference.name.replace('"', "\"\""))
                } else {
                    let ns = detail
                        .reference
                        .namespace
                        .iter()
                        .filter(|s| !s.is_empty())
                        .map(|s| format!("\"{}\"", s.replace('"', "\"\"")))
                        .collect::<Vec<_>>()
                        .join(".");
                    format!("{ns}.\"{}\"", detail.reference.name.replace('"', "\"\""))
                };
                match client
                    .execute(conn_id, &format!("SELECT * FROM {qualified} LIMIT {n}"))
                    .await
                {
                    Ok(preview) => {
                        let col_names: Vec<String> =
                            preview.columns.iter().map(|c| c.name.clone()).collect();
                        result_obj["sample_rows"] = json!({
                            "columns": col_names,
                            "rows": preview.rows,
                        });
                    }
                    Err(e) => {
                        log::warn!(
                            "Sample rows fetch failed for {}.{}, omitting preview: {e}",
                            detail.reference.namespace.join("."),
                            detail.reference.name
                        );
                    }
                }
            }
        }

        Ok(ToolOutput::Text {
            content: serde_json::json!({ "objects": results }).to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use lucent_protocol::{ColumnDetail, ForeignKeyTarget, ObjectDetail, ObjectKind, ObjectRef};

    use super::{details_to_json, hits_to_json};

    fn detail() -> ObjectDetail {
        ObjectDetail {
            reference: ObjectRef {
                namespace: vec!["public".into()],
                name: "orders".into(),
                kind: ObjectKind::Table,
            },
            comment: None,
            columns: vec![
                ColumnDetail {
                    name: "id".into(),
                    type_name: "bigint".into(),
                    nullable: false,
                    is_primary_key: true,
                    ordinal: 1,
                    default: None,
                    comment: None,
                    foreign_key: None,
                },
                ColumnDetail {
                    name: "user_id".into(),
                    type_name: "bigint".into(),
                    nullable: true,
                    is_primary_key: false,
                    ordinal: 2,
                    default: None,
                    comment: None,
                    foreign_key: Some(ForeignKeyTarget {
                        namespace: vec!["public".into()],
                        table: "users".into(),
                        column: "id".into(),
                    }),
                },
            ],
        }
    }

    #[test]
    fn object_json_keeps_the_shape_the_model_already_knows() {
        let json = details_to_json(&[detail()]);
        let object = &json[0];
        assert_eq!(object["schema"], "public");
        assert_eq!(object["name"], "orders");

        let columns = object["columns"].as_array().unwrap();
        assert_eq!(columns[0]["name"], "id");
        assert_eq!(columns[0]["type"], "bigint");
        assert_eq!(columns[0]["nullable"], false);
        assert_eq!(columns[0]["pk"], true);
        assert!(
            columns[0].get("fk").is_none(),
            "no fk key when there is no FK"
        );

        assert_eq!(columns[1]["fk"]["table"], "users");
        assert_eq!(columns[1]["fk"]["column"], "id");
    }

    #[test]
    fn search_hits_render_columns_as_table_dot_column() {
        use lucent_protocol::SearchHit;
        let json = hits_to_json(&[
            SearchHit {
                reference: ObjectRef {
                    namespace: vec!["public".into()],
                    name: "users".into(),
                    kind: ObjectKind::Table,
                },
                column: None,
                score: 0.9,
            },
            SearchHit {
                reference: ObjectRef {
                    namespace: vec!["public".into()],
                    name: "orders".into(),
                    kind: ObjectKind::Table,
                },
                column: Some("user_id".into()),
                score: 0.7,
            },
        ]);

        assert_eq!(json[0]["name"], "users");
        assert_eq!(json[0]["kind"], "table");
        assert_eq!(json[0]["match_type"], "table");

        // The column form the model has always seen.
        assert_eq!(json[1]["name"], "orders.user_id");
        assert_eq!(json[1]["kind"], "column");
        assert_eq!(json[1]["match_type"], "column");
    }

    #[test]
    fn sample_rows_arg_parses_and_caps_at_20() {
        let args = serde_json::json!({
            "objects": [{"name": "users"}],
            "sample_rows": 500
        });
        let requested = args["sample_rows"].as_u64().map(|n| n.min(20) as usize);
        assert_eq!(
            requested,
            Some(20),
            "sample_rows must be capped to protect query cost"
        );
    }

    #[test]
    fn sample_rows_absent_means_no_preview() {
        let args = serde_json::json!({"objects": [{"name": "users"}]});
        let requested = args["sample_rows"].as_u64().map(|n| n.min(20) as usize);
        assert_eq!(
            requested, None,
            "omitting sample_rows must not fetch any preview rows"
        );
    }
}
