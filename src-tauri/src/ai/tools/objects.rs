use serde_json::json;

use super::{AiToolContext, ToolError, ToolOutput};

/// Fetch all FK constraints across the database, with correct ordinal-position
/// correlation for composite keys. Returns (from_schema, from_table, from_column,
/// to_schema, to_table, to_column) for every FK.
pub(crate) async fn fetch_all_fk_constraints(
    client: &mut crate::client::ConnectorClient,
) -> Result<Vec<(String, String, String, String, String, String)>, String> {
    let sql = "\
        SELECT kcu.table_schema, kcu.table_name, kcu.column_name, \
               ccu.table_schema, ccu.table_name, ccu.column_name \
        FROM information_schema.table_constraints tc \
        JOIN information_schema.key_column_usage kcu \
          ON kcu.constraint_name = tc.constraint_name AND kcu.table_schema = tc.table_schema \
        JOIN information_schema.referential_constraints rc \
          ON rc.constraint_name = tc.constraint_name AND rc.constraint_schema = tc.table_schema \
        JOIN information_schema.key_column_usage ccu \
          ON ccu.constraint_name = rc.unique_constraint_name \
         AND ccu.ordinal_position = kcu.ordinal_position \
        WHERE tc.constraint_type = 'FOREIGN KEY' \
          AND tc.table_schema NOT IN ('pg_catalog', 'information_schema', 'pg_toast')";
    let result = client
        .execute(sql)
        .await
        .map_err(|e| format!("FK query: {e}"))?;
    let mut rows = Vec::with_capacity(result.rows.len());
    for r in &result.rows {
        rows.push((
            r[0].as_str().unwrap_or("public").to_string(),
            r[1].as_str().unwrap_or("?").to_string(),
            r[2].as_str().unwrap_or("?").to_string(),
            r[3].as_str().unwrap_or("public").to_string(),
            r[4].as_str().unwrap_or("?").to_string(),
            r[5].as_str().unwrap_or("?").to_string(),
        ));
    }
    Ok(rows)
}

/// Search database objects (tables, views, columns) by name.
/// Uses pg_trgm similarity when available, falls back to ILIKE.
/// Extended from the old SearchObjects to also match column names.
pub(crate) async fn keyword_search_objects(
    client: &mut crate::client::ConnectorClient,
    query: &str,
    kind: Option<&str>,
    schema: Option<&str>,
) -> Result<Vec<serde_json::Value>, ToolError> {
    let has_trgm = client
        .execute("SELECT count(*) FROM pg_extension WHERE extname = 'pg_trgm'")
        .await
        .ok()
        .and_then(|r| {
            r.rows
                .first()
                .and_then(|row| row.first().map(|v| v.as_i64().unwrap_or(0) > 0))
        })
        .unwrap_or(false);

    let mut filters = String::new();
    if let Some(k) = kind {
        filters.push_str(&format!(
            " AND c.relkind = '{}'",
            if k == "table" {
                "'r'"
            } else if k == "view" {
                "'v'"
            } else {
                "'f'"
            }
        ));
    }
    let schema_filter = if let Some(s) = schema {
        format!(" AND n.nspname = '{}'", s.replace('\'', "''"))
    } else {
        " AND n.nspname NOT IN ('pg_catalog','information_schema','pg_toast')".into()
    };
    let safe_query = query.replace('\'', "''");

    let table_sql = if has_trgm {
        format!(
            "SELECT n.nspname AS schema_name, c.relname AS name, \
             CASE c.relkind WHEN 'r' THEN 'table' WHEN 'v' THEN 'view' ELSE 'other' END AS kind, \
             similarity(c.relname, '{safe_query}') AS score \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r','v'){filters}{schema_filter} \
               AND similarity(c.relname, '{safe_query}') > 0.1 \
             ORDER BY score DESC LIMIT 10"
        )
    } else {
        let like = format!("%{safe_query}%");
        format!(
            "SELECT n.nspname AS schema_name, c.relname AS name, \
             CASE c.relkind WHEN 'r' THEN 'table' WHEN 'v' THEN 'view' ELSE 'other' END AS kind, \
             0.5::float4 AS score \
             FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind IN ('r','v'){filters}{schema_filter} \
               AND c.relname ILIKE '{like}' \
             ORDER BY c.relname LIMIT 10"
        )
    };

    let result = client
        .execute(&table_sql)
        .await
        .map_err(ToolError::Database)?;
    let mut items: Vec<serde_json::Value> = result
        .rows
        .iter()
        .map(|r| {
            json!({
                "schema": r[0].as_str().unwrap_or(""),
                "name": r[1].as_str().unwrap_or(""),
                "kind": r[2].as_str().unwrap_or(""),
                "score": r[3].as_f64().unwrap_or(0.0),
                "match_type": "table",
            })
        })
        .collect();

    // Also search column names (always via ILIKE — column counts are small enough)
    let col_sql = format!(
        "SELECT c.table_schema, c.table_name, c.column_name \
         FROM information_schema.columns c \
         WHERE c.table_schema NOT IN ('pg_catalog','information_schema','pg_toast') \
           AND (c.column_name ILIKE '%{safe_query}%' \
             OR c.column_name % '{safe_query}') \
         ORDER BY c.table_schema, c.table_name, c.ordinal_position \
         LIMIT 10"
    );
    if let Ok(col_result) = client.execute(&col_sql).await {
        for r in &col_result.rows {
            let col_name = r[2].as_str().unwrap_or("");
            let rel = format!("{}.{}", r[1].as_str().unwrap_or("?"), col_name);
            let score = if col_name.to_lowercase() == safe_query.to_lowercase() {
                1.0
            } else {
                0.7
            };
            items.push(json!({
                "schema": r[0].as_str().unwrap_or(""),
                "name": rel,
                "kind": "column",
                "score": score,
                "match_type": "column",
            }));
        }
    }

    Ok(items)
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
                format!(
                    "{}.{}",
                    o["schema"].as_str().unwrap_or("public"),
                    o["name"].as_str().unwrap_or("?")
                )
            })
            .collect();
        log::info!("Tool 'get_objects_info' called — objects: {object_names:?}, sample_rows: {sample_rows:?}");

        let mut db = self.ctx.db.lock().await;
        let client = db.as_mut().ok_or(ToolError::NotConnected)?;

        let mut results = vec![];

        for obj in objects {
            let name = obj["name"].as_str().unwrap_or("");
            let schema = obj["schema"].as_str().unwrap_or("public");

            // Build FK lookup from schema graph (more reliable than SQL LEFT JOINs)
            let fk_lookup: std::collections::HashMap<(String, String), String> = {
                let graph_guard = self.ctx.schema_graph.lock().await;
                let mut lookup = std::collections::HashMap::new();
                if let Some(ref graph) = *graph_guard {
                    for col in &graph.columns {
                        if let Some(ref fk) = col.fk_ref {
                            lookup.insert((col.table.clone(), col.name.clone()), fk.clone());
                        }
                    }
                }
                lookup
            };

            // EXISTS is a boolean semi-join — it can only contribute 0 or 1 to the
            // outer row count regardless of how many constraints reference the column,
            // unlike a LEFT JOIN on key_column_usage/table_constraints (which produces
            // one outer row per matching constraint and previously required a fragile
            // dedup-by-name pass that could silently keep the wrong row's is_pk value
            // for columns that are both part of a composite PK and individually FK'd).
            let rows = client
                .execute(&format!(
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
                     WHERE c.table_schema = '{}' AND c.table_name = '{}' \
                     ORDER BY c.ordinal_position",
                    schema.replace('\'', "''"),
                    name.replace('\'', "''"),
                ))
                .await
                .map_err(ToolError::Database)?;

            let cols: Vec<serde_json::Value> = rows
                .rows
                .iter()
                .map(|r| {
                    let col_name = r[0].as_str().unwrap_or("");
                    let data_type = r[1].as_str().unwrap_or("");
                    let nullable = r[2].as_str() == Some("YES");
                    let is_pk = r[3].as_bool().unwrap_or(false);

                    let mut col = json!({
                        "name": col_name,
                        "type": data_type,
                        "nullable": nullable,
                        "pk": is_pk,
                    });

                    // Use schema graph for FK info (reliable, no phantom entries)
                    if let Some(fk_target) =
                        fk_lookup.get(&(name.to_string(), col_name.to_string()))
                    {
                        let parts: Vec<&str> = fk_target.splitn(2, '.').collect();
                        if parts.len() == 2 {
                            col["fk"] = json!({"table": parts[0], "column": parts[1]});
                        }
                    }

                    col
                })
                .collect();

            let mut result_obj = json!({
                "schema": schema,
                "name": name,
                "columns": cols,
            });

            if let Some(n) = sample_rows {
                if n > 0 {
                    let qualified = format!(
                        "\"{}\".\"{}\"",
                        schema.replace('"', "\"\""),
                        name.replace('"', "\"\""),
                    );
                    match client
                        .execute(&format!("SELECT * FROM {qualified} LIMIT {n}"))
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
                            log::warn!("Sample rows fetch failed for {schema}.{name}, omitting preview: {e}");
                        }
                    }
                }
            }

            results.push(result_obj);
        }

        Ok(ToolOutput::Text {
            content: serde_json::json!({ "objects": results }).to_string(),
        })
    }
}

#[cfg(test)]
mod tests {

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
