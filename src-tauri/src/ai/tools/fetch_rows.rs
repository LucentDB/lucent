use serde_json::json;

use super::execute::{header_labels, render_result_block, CELL_CLIP_CHARS, MAX_CELL_CHARS};
use super::{AiToolContext, ToolError, ToolOutput};
use crate::ai::result_store::{expired_result_msg, Lookup, UNKNOWN_RESULT_MSG};

/// The store never holds more than `row_limit` rows; this bounds one page.
const MAX_PAGE_ROWS: u64 = 500;

#[derive(Clone)]
pub struct FetchRows {
    ctx: AiToolContext,
}

/// Map `columns` entries (label strings, incl. positional `id#2`, or 0-based
/// indexes) to column positions. Errors list every label and index (P5).
fn resolve_columns(labels: &[String], wanted: &[serde_json::Value]) -> Result<Vec<usize>, String> {
    let listing = || {
        format!(
            "valid columns: {} (or indexes 0-{})",
            labels.join(", "),
            labels.len().saturating_sub(1)
        )
    };
    wanted
        .iter()
        .map(|w| match w {
            serde_json::Value::Number(n) => n
                .as_u64()
                .map(|i| i as usize)
                .filter(|i| *i < labels.len())
                .ok_or_else(|| format!("column index {n} is out of range; {}", listing())),
            serde_json::Value::String(s) => labels
                .iter()
                .position(|l| l == s)
                .ok_or_else(|| format!("unknown column '{s}'; {}", listing())),
            other => Err(format!(
                "columns entries must be names or indexes, got {other}; {}",
                listing()
            )),
        })
        .collect()
}

impl FetchRows {
    pub fn new(ctx: AiToolContext) -> Self {
        Self { ctx }
    }

    pub fn description(&self) -> String {
        "Read more rows of a result you already ran, by its result_id. Never re-run a query \
         to see rows you were not shown."
            .into()
    }

    pub fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "result_id": { "type": "string", "description": "From a run_readonly_query result in this conversation" },
                "offset": { "type": "integer", "description": "0-based row offset" },
                "limit": { "type": "integer", "description": "Rows wanted; the token budget still applies" },
                "columns": {
                    "type": "array",
                    "items": { "type": ["string", "integer"] },
                    "description": "Column names, positional labels like id#2, or 0-based indexes"
                },
                "cell_chars": { "type": "integer", "description": "Per-cell clip, default 120, max 8000" }
            },
            "required": ["result_id", "offset"]
        })
    }

    pub async fn call(
        &self,
        args: serde_json::Value,
        _ctx: &AiToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let id = args["result_id"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs("missing 'result_id'".into()))?;
        let offset = args["offset"]
            .as_u64()
            .ok_or_else(|| ToolError::InvalidArgs("missing 'offset' (0-based row number)".into()))?
            as usize;
        let limit = args["limit"]
            .as_u64()
            .unwrap_or(MAX_PAGE_ROWS)
            .clamp(1, MAX_PAGE_ROWS) as usize;
        let requested_chars = args["cell_chars"]
            .as_u64()
            .unwrap_or(CELL_CLIP_CHARS as u64);
        let cell_chars = (requested_chars as usize).clamp(1, MAX_CELL_CHARS);

        let scope = &self.ctx.result_scope;
        let stored = match scope.store.lookup(&scope.scope_id, id) {
            Lookup::Live(s) => s,
            // Identical for "never existed" and "another scope's id" (§6A.3).
            Lookup::Unknown => return Err(ToolError::InvalidArgs(UNKNOWN_RESULT_MSG.into())),
            Lookup::Evicted => return Err(ToolError::InvalidArgs(expired_result_msg(id))),
        };

        let labels = header_labels(&stored.columns);
        let projection: Vec<usize> = match args["columns"].as_array() {
            Some(wanted) if !wanted.is_empty() => {
                resolve_columns(&labels, wanted).map_err(ToolError::InvalidArgs)?
            }
            _ => (0..stored.columns.len()).collect(),
        };
        // Projected columns keep their ORIGINAL positional labels, so asking for
        // `id#2` shows `id#2`, not a re-derived `id`.
        let columns: Vec<crate::ai::events::ColumnMeta> = projection
            .iter()
            .map(|&i| crate::ai::events::ColumnMeta {
                name: labels[i].clone(),
                data_type: stored.columns[i].data_type.clone(),
            })
            .collect();

        let total = stored.rows.len();
        let end_text = if stored.complete {
            format!("End of result ({total} rows total).")
        } else {
            format!(
                "End of result (the first {total} rows are stored; the result has more — narrow \
                 the query and re-run to see the rest)."
            )
        };
        if offset >= total {
            return Ok(ToolOutput::Text { content: end_text });
        }

        let end = (offset + limit).min(total);
        let page: Vec<Vec<serde_json::Value>> = stored.rows[offset..end]
            .iter()
            .map(|r| {
                projection
                    .iter()
                    .map(|&i| r.get(i).cloned().unwrap_or(serde_json::Value::Null))
                    .collect()
            })
            .collect();
        let (block, shown) = render_result_block(
            Some(id),
            &columns,
            &page,
            offset,
            stored.complete,
            total,
            self.ctx.config.preview_token_budget as usize,
            cell_chars,
        );
        let mut content = block;
        content.push('\n');
        if offset + shown < total {
            content.push_str(&format!(
                "More rows: fetch_rows(result_id: \"{id}\", offset: {}).",
                offset + shown
            ));
        } else {
            content.push_str(&end_text);
        }
        if requested_chars as usize > MAX_CELL_CHARS {
            content.push_str(&format!("\ncell_chars clamped to {MAX_CELL_CHARS}."));
        }
        Ok(ToolOutput::Text { content })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::events::ColumnMeta;
    use serde_json::json;

    /// A context whose scope holds one stored result: columns (id, id, name)
    /// and five rows; returns (ctx, result_id).
    fn ctx_with_result(complete: bool) -> (AiToolContext, String) {
        let ctx = crate::ai::tools::neutral_tool_ctx();
        let scope = &ctx.result_scope;
        let columns = ["id", "id", "name"]
            .iter()
            .map(|n| ColumnMeta {
                name: n.to_string(),
                data_type: "text".into(),
            })
            .collect();
        let rows = (0..5)
            .map(|i| vec![json!(i), json!(i * 10), json!(format!("n{i}"))])
            .collect();
        let id = scope
            .store
            .store(&scope.scope_id, columns, rows, "SELECT …", complete)
            .unwrap();
        (ctx, id)
    }

    async fn text(ctx: &AiToolContext, args: serde_json::Value) -> Result<String, ToolError> {
        match FetchRows::new(ctx.clone()).call(args, ctx).await? {
            ToolOutput::Text { content } => Ok(content),
            other => panic!("text output, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pages_from_offset_and_reports_the_next_offset() {
        let (ctx, id) = ctx_with_result(true);
        let t = text(&ctx, json!({"result_id": id, "offset": 2, "limit": 2}))
            .await
            .unwrap();
        assert!(t.contains("shown=\"3-4\""), "{t}");
        assert!(
            t.contains(&format!("fetch_rows(result_id: \"{id}\", offset: 4)")),
            "{t}"
        );
    }

    #[tokio::test]
    async fn end_of_result_is_not_an_error() {
        let (ctx, id) = ctx_with_result(true);
        let t = text(&ctx, json!({"result_id": id, "offset": 5}))
            .await
            .unwrap();
        assert!(t.contains("End of result (5 rows total)"), "{t}");
        let t = text(&ctx, json!({"result_id": id, "offset": 3}))
            .await
            .unwrap();
        assert!(t.contains("End of result"), "the last page says so: {t}");
    }

    #[tokio::test]
    async fn end_of_an_incomplete_result_says_more_rows_exist() {
        let (ctx, id) = ctx_with_result(false);
        let t = text(&ctx, json!({"result_id": id, "offset": 5}))
            .await
            .unwrap();
        assert!(t.contains("narrow the query"), "{t}");
    }

    #[tokio::test]
    async fn unknown_and_cross_scope_ids_get_the_same_error() {
        let (ctx, id) = ctx_with_result(true);
        let unknown = text(
            &ctx,
            json!({"result_id": "r_0000000000000000", "offset": 0}),
        )
        .await
        .unwrap_err();
        let mut other = ctx.clone();
        other.result_scope.scope_id = "another-conversation".into();
        let cross = text(&other, json!({"result_id": id, "offset": 0}))
            .await
            .unwrap_err();
        assert_eq!(
            unknown.to_string(),
            cross.to_string(),
            "no oracle for ids in other scopes"
        );
        assert!(unknown.to_string().contains(UNKNOWN_RESULT_MSG));
    }

    #[tokio::test]
    async fn evicted_id_reports_expiry() {
        let ctx = crate::ai::tools::neutral_tool_ctx();
        let mut small = ctx.clone();
        small.result_scope.store = std::sync::Arc::new(
            crate::ai::result_store::ResultStore::with_limits(1, 1 << 20, 1 << 30),
        );
        let scope = &small.result_scope;
        let first = scope
            .store
            .store(&scope.scope_id, vec![], vec![vec![json!(1)]], "q1", true)
            .unwrap();
        scope
            .store
            .store(&scope.scope_id, vec![], vec![vec![json!(2)]], "q2", true)
            .unwrap();
        let e = text(&small, json!({"result_id": first, "offset": 0}))
            .await
            .unwrap_err();
        assert!(e.to_string().contains("expired"), "{e}");
    }

    #[tokio::test]
    async fn column_projection_by_label_position_and_index() {
        let (ctx, id) = ctx_with_result(true);
        let t = text(
            &ctx,
            json!({"result_id": id, "offset": 0, "limit": 1, "columns": ["id#2", 2]}),
        )
        .await
        .unwrap();
        assert!(t.contains("| id#2 | name |"), "{t}");
        assert!(t.contains("| 0 | n0 |"), "{t}");
        let e = text(
            &ctx,
            json!({"result_id": id, "offset": 0, "columns": ["nope"]}),
        )
        .await
        .unwrap_err();
        assert!(
            e.to_string().contains("id, id#2, name"),
            "lists the labels: {e}"
        );
    }

    #[tokio::test]
    async fn cell_chars_is_clamped_to_8000_and_says_so() {
        let (ctx, id) = ctx_with_result(true);
        let t = text(
            &ctx,
            json!({"result_id": id, "offset": 0, "cell_chars": 50_000}),
        )
        .await
        .unwrap();
        assert!(t.contains("cell_chars clamped to 8000"), "{t}");
    }

    #[test]
    fn resolve_columns_rejects_out_of_range_indexes() {
        let labels = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            resolve_columns(&labels, &[json!(1), json!("a")]).unwrap(),
            vec![1, 0]
        );
        assert!(resolve_columns(&labels, &[json!(2)]).is_err());
        assert!(resolve_columns(&labels, &[json!(true)]).is_err());
    }
}
