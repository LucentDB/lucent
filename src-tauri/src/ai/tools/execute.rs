use serde_json::json;
use std::time::Instant;

use super::{AiToolContext, ToolError, ToolOutput};
use crate::ai::guard;

/// Extract the existing LIMIT value from a SQL statement, if present and trailing.
/// Returns `Some(n)` if a trailing `LIMIT n` (optionally followed by `OFFSET m`)
/// is the last clause, and there are no additional clauses after it.
fn extract_existing_limit(sql: &str) -> Option<usize> {
    let s = sql.trim();
    let upper = s.to_uppercase();
    // Find the LAST occurrence of "LIMIT"
    let pos = upper.rfind("LIMIT")?;
    let after = s[pos + 6..].trim();
    let digits_end = after
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(after.len());
    if digits_end == 0 {
        return None;
    }
    let limit_val: usize = after[..digits_end].parse().ok()?;
    let mut rest = after[digits_end..].trim();
    // Allow optional OFFSET after LIMIT
    if rest.to_uppercase().starts_with("OFFSET") {
        let ofs_digits = rest[6..].trim();
        let ofs_end = ofs_digits
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(ofs_digits.len());
        if ofs_end > 0 {
            rest = ofs_digits[ofs_end..].trim();
        }
    }
    // Must be end of statement or just a semicolon (with optional trailing whitespace)
    if rest.is_empty() || rest.trim_end().is_empty() || rest.trim_end() == ";" {
        Some(limit_val)
    } else {
        None
    }
}

/// Apply a row cap to a SQL statement.
/// - If the statement already has a trailing LIMIT ≤ `cap`, keep it unchanged.
/// - If it has a LIMIT > `cap`, replace the limit value with `cap`.
/// - If it has no LIMIT, append `LIMIT {cap}`.
fn apply_limit(sql: &str, cap: usize) -> String {
    let trimmed = sql.trim().trim_end_matches(';').trim();
    match extract_existing_limit(trimmed) {
        Some(n) if n <= cap => trimmed.to_string(),
        Some(_) => {
            // Replace the existing LIMIT value with cap
            let upper = trimmed.to_uppercase();
            let pos = upper.rfind("LIMIT").unwrap();
            let before = trimmed[..pos].trim_end();
            let after = &trimmed[pos + 6..];
            let digits_end = after
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(after.len());
            let rest = &after[digits_end..];
            format!("{before} LIMIT {cap}{rest}")
        }
        None => {
            // Wrap in a subquery to avoid edge cases: trailing semicolons,
            // ORDER BY + LIMIT interaction, WITH/CTE incompatibility, etc.
            format!("SELECT * FROM (\n{trimmed}\n) _limit LIMIT {cap}")
        }
    }
}

/// AI-facing result summary: SQL echo, row count, Markdown preview of the
/// first rows — and, for empty results, a wrong-literal recovery hint.
fn build_text_summary(
    sql: &str,
    columns: &[crate::ai::events::ColumnMeta],
    rows: &[Vec<serde_json::Value>],
    row_count: usize,
    elapsed_ms: u64,
    truncated: bool,
) -> String {
    let sql_preview = if sql.len() > 120 {
        format!("{}...", &sql[..117])
    } else {
        sql.to_string()
    };
    let mut text_summary =
        format!("Query: {sql_preview}\nResult: {row_count} rows in {elapsed_ms}ms");

    const SLOW_QUERY_NOTE_MS: u64 = 5_000;
    if elapsed_ms >= SLOW_QUERY_NOTE_MS {
        text_summary.push_str(&format!(
            "\n\nNote: this query took {:.1}s. Avoid re-running it or near-duplicates. \
             If you need a variation, reconsider whether COUNT(DISTINCT …) or wide \
             joins are truly required — cheaper aggregates often answer the question.",
            elapsed_ms as f64 / 1000.0
        ));
    }

    if rows.is_empty() {
        text_summary.push_str(
            "\n\n0 rows returned. If you expected data, a filter literal may not match \
             stored values exactly (case, spelling, or code format). Check the sample \
             values in the schema context, or retry with ILIKE / a broader filter \
             before concluding the data doesn't exist.",
        );
        return text_summary;
    }

    let preview_count = usize::min(10, rows.len());
    text_summary.push_str(&format!(
        "\n\n**Preview (first {preview_count} of {row_count} rows):**\n\n"
    ));
    let col_names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    text_summary.push_str(&format!("| {} |\n", col_names.join(" | ")));
    text_summary.push_str(&format!(
        "|{}|\n",
        col_names
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join("|")
    ));
    for row in rows.iter().take(preview_count) {
        let vals: Vec<String> = row
            .iter()
            .map(|v| match v {
                serde_json::Value::Null => "NULL".into(),
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect();
        text_summary.push_str(&format!("| {} |\n", vals.join(" | ")));
    }
    if truncated {
        text_summary.push_str(&format!(
            "\n_(results truncated — showing first {preview_count})_"
        ));
    }
    text_summary
}

#[derive(Clone)]
pub struct RunReadonlyQuery {
    ctx: AiToolContext,
}

impl RunReadonlyQuery {
    pub fn new(ctx: AiToolContext) -> Self {
        Self { ctx }
    }
    pub fn description(&self) -> String {
        "Execute a read-only SQL query (SELECT, WITH, VALUES, EXPLAIN without ANALYZE). \
         Results appear in the data grid. For INSERT/UPDATE/DELETE use preview_dml."
            .into()
    }
    pub fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": { "sql": { "type": "string" } }, "required": ["sql"] })
    }
    pub async fn call(
        &self,
        args: serde_json::Value,
        _ctx: &AiToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let sql = args["sql"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs("missing 'sql'".into()))?;

        log::info!("Tool 'run_readonly_query' called — sql={sql:?}");

        // Layer 1: syntactic guard
        guard::validate_readonly(sql).map_err(|e| {
            log::warn!("Read-only guard rejected: {e}");
            ToolError::SqlValidation(e.to_string())
        })?;

        let row_limit = self.ctx.config.row_limit as usize;

        let mut db = self.ctx.db.lock().await;
        let client = db.as_mut().ok_or(ToolError::NotConnected)?;

        // Layer 2: READ ONLY transaction — guards against stored-function side effects
        client
            .execute("BEGIN")
            .await
            .map_err(ToolError::Execution)?;
        client
            .execute("SET TRANSACTION READ ONLY")
            .await
            .map_err(ToolError::Execution)?;

        // Bound runaway queries. SET LOCAL is valid here — we are inside an
        // explicit transaction — and reverts automatically at ROLLBACK.
        let timeout_ms = self.ctx.config.ai_query_timeout_secs.saturating_mul(1000);
        if timeout_ms > 0 {
            let _ = client
                .execute(&format!("SET LOCAL statement_timeout = {timeout_ms}"))
                .await;
        }

        // Apply row cap: keep LLM's LIMIT if it's ≤ limit, cap it otherwise, or add one.
        let limited_sql = apply_limit(sql.trim_end_matches(';'), row_limit);
        log::debug!("Executing (limited): {limited_sql}");
        let start = Instant::now();
        let query_result = client.execute(&limited_sql).await;

        // Always ROLLBACK — never commit read-only queries
        let _ = client.execute("ROLLBACK").await;

        let result = query_result.map_err(|e| {
            log::error!("Query failed: {e}");
            ToolError::Execution(e)
        })?;
        let elapsed = start.elapsed().as_millis() as u64;
        let row_count = result.rows.len();
        let truncated = row_count >= row_limit;
        log::debug!("Query returned {row_count} rows in {elapsed}ms (truncated={truncated})");

        let columns: Vec<crate::ai::events::ColumnMeta> = result
            .columns
            .iter()
            .map(|c| crate::ai::events::ColumnMeta {
                name: c.name.clone(),
                data_type: c.type_name.clone(),
            })
            .collect();

        // Build AI-facing summary with readable Markdown preview
        let mut text_summary =
            build_text_summary(sql, &columns, &result.rows, row_count, elapsed, truncated);

        // Join lint: warn on non-FK equijoins and time-versioned fan-out —
        // wrong joins return plausible-looking numbers, so the warning matters
        // MOST when the query succeeds. Fail-open (no graph / parse error = silent).
        {
            let graph_guard = self.ctx.schema_graph.lock().await;
            if let Some(graph) = graph_guard.as_ref() {
                for w in crate::ai::sql_lint::lint_sql(graph, sql) {
                    text_summary.push_str("\n\n");
                    text_summary.push_str(&w);
                }
            }
        }

        Ok(ToolOutput::QueryResult {
            text_summary,
            columns,
            rows: result.rows,
            row_count,
            sql: sql.into(),
            execution_time_ms: elapsed,
            truncated,
        })
    }
}

#[derive(Clone)]
pub struct PreviewDml {
    ctx: AiToolContext,
}

impl PreviewDml {
    pub fn new(ctx: AiToolContext) -> Self {
        Self { ctx }
    }
    pub fn description(&self) -> String {
        "Validate a DML statement without executing it. Returns SQL and estimated impact. \
         NEVER executes. Agent pauses after this for user confirmation. \
         One statement per call."
            .into()
    }
    pub fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": { "sql": { "type": "string" } }, "required": ["sql"] })
    }
    pub async fn call(
        &self,
        args: serde_json::Value,
        _ctx: &AiToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let sql = args["sql"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs("missing 'sql'".into()))?;

        log::info!("Tool 'preview_dml' called — sql={sql:?}");

        let sql_up = sql.trim().to_uppercase();
        let stmt_type = if sql_up.starts_with("INSERT") {
            "INSERT"
        } else if sql_up.starts_with("UPDATE") {
            "UPDATE"
        } else if sql_up.starts_with("DELETE") {
            "DELETE"
        } else {
            return Err(ToolError::SqlValidation("Not a DML statement".into()));
        };

        let table = guard::extract_table_name(sql).unwrap_or_else(|| "unknown".into());

        // Blast-radius preflight
        let estimated = if self.ctx.config.enable_blast_radius_check {
            if let Some(where_clause) = guard::extract_where_for_count(sql) {
                let count_sql = format!("SELECT count(*) FROM {} WHERE {}", table, where_clause);
                let mut db = self.ctx.db.lock().await;
                if let Some(client) = db.as_mut() {
                    let _ = client.execute("BEGIN").await;
                    let _ = client.execute("SET TRANSACTION READ ONLY").await;
                    let result = client.execute(&count_sql).await;
                    let _ = client.execute("ROLLBACK").await;
                    result
                        .ok()
                        .and_then(|r| {
                            r.rows
                                .first()
                                .and_then(|row| row.first().and_then(|v| v.as_i64()))
                        })
                        .map(|n| n as u64)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let desc = format!(
            "{stmt_type} on {table}{}",
            estimated
                .map(|n| format!(" — ~{n} rows"))
                .unwrap_or_default()
        );

        Ok(ToolOutput::DmlPreview {
            sql: sql.into(),
            statement_type: stmt_type.into(),
            tables_affected: vec![table],
            description: desc,
            estimated_rows_affected: estimated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_limit_returns_none_for_no_limit() {
        assert_eq!(extract_existing_limit("SELECT * FROM users"), None);
    }

    #[test]
    fn extract_limit_returns_value() {
        assert_eq!(
            extract_existing_limit("SELECT * FROM users LIMIT 10"),
            Some(10)
        );
    }

    #[test]
    fn extract_limit_ignores_limit_in_where() {
        // "LIMIT" in a string value should not be matched
        assert_eq!(
            extract_existing_limit("SELECT * FROM t WHERE name = 'limit'"),
            None
        );
    }

    #[test]
    fn extract_limit_ignores_inner_limit() {
        // LIMIT in a subquery should not be matched as trailing
        assert_eq!(
            extract_existing_limit("SELECT * FROM (SELECT * FROM t LIMIT 5) sub"),
            None,
        );
    }

    #[test]
    fn extract_limit_with_offset() {
        assert_eq!(
            extract_existing_limit("SELECT * FROM users LIMIT 10 OFFSET 5"),
            Some(10),
        );
    }

    #[test]
    fn extract_limit_case_insensitive() {
        assert_eq!(
            extract_existing_limit("select * from users limit 100"),
            Some(100)
        );
    }

    #[test]
    fn extract_limit_with_semicolon() {
        assert_eq!(
            extract_existing_limit("SELECT * FROM users LIMIT 5;"),
            Some(5)
        );
    }

    #[test]
    fn apply_limit_adds_when_none() {
        let result = apply_limit("SELECT * FROM users", 500);
        assert!(
            result.contains("SELECT * FROM users"),
            "must preserve original query"
        );
        assert!(result.contains("LIMIT 500"), "must add LIMIT");
        assert!(
            result.contains("_limit"),
            "must use subquery wrapper: {result}"
        );
    }

    #[test]
    fn apply_limit_keeps_lower_limit() {
        assert_eq!(
            apply_limit("SELECT * FROM users LIMIT 10", 500),
            "SELECT * FROM users LIMIT 10",
        );
    }

    #[test]
    fn apply_limit_caps_higher_limit() {
        assert_eq!(
            apply_limit("SELECT * FROM users LIMIT 1000", 500),
            "SELECT * FROM users LIMIT 500",
        );
    }

    #[test]
    fn apply_limit_caps_with_offset() {
        assert_eq!(
            apply_limit("SELECT * FROM users LIMIT 1000 OFFSET 5", 500),
            "SELECT * FROM users LIMIT 500 OFFSET 5",
        );
    }

    #[test]
    fn apply_limit_keeps_exact_limit() {
        assert_eq!(
            apply_limit("SELECT * FROM users LIMIT 500", 500),
            "SELECT * FROM users LIMIT 500",
        );
    }

    #[test]
    fn apply_limit_ignores_inner_limit() {
        let result = apply_limit("SELECT * FROM (SELECT * FROM t LIMIT 5) sub", 500);
        // Inner LIMIT preserved, outer wraps with subquery
        assert!(
            result.contains("SELECT * FROM t LIMIT 5"),
            "inner LIMIT must be preserved"
        );
        assert!(result.contains("LIMIT 500"), "must add outer LIMIT cap");
        assert!(result.contains("_limit"), "must use subquery wrapper");
    }

    #[test]
    fn apply_limit_with_semicolon() {
        let result = apply_limit("SELECT * FROM users LIMIT 10;", 500);
        // LIMIT 10 preserved, semicolon handled
        assert!(result.contains("LIMIT 10"), "must preserve original LIMIT");
        assert!(
            !result.contains(";"),
            "must strip trailing semicolons: {result}"
        );
    }

    #[test]
    fn apply_limit_semicolon_with_trailing_newline() {
        // This was the root cause of the LIMIT syntax errors
        let result = apply_limit(
            "SELECT a.model, COUNT(*) AS cnt\nFROM flights f\nGROUP BY a.model\nORDER BY cnt DESC;\n",
            500,
        );
        assert!(
            result.contains("ORDER BY cnt DESC"),
            "must preserve ORDER BY"
        );
        assert!(result.contains("LIMIT 500"), "must cap at 500");
        assert!(!result.contains(";"), "must not have trailing semicolons");
    }
}

#[cfg(test)]
mod summary_tests {
    use super::build_text_summary;
    use crate::ai::events::ColumnMeta;

    fn cols() -> Vec<ColumnMeta> {
        vec![ColumnMeta {
            name: "code".into(),
            data_type: "text".into(),
        }]
    }

    #[test]
    fn empty_result_carries_wrong_literal_hint() {
        let s = build_text_summary(
            "SELECT * FROM airports WHERE code = 'can'",
            &cols(),
            &[],
            0,
            3,
            false,
        );
        assert!(s.contains("0 rows"), "{s}");
        assert!(
            s.contains("may not match stored values exactly"),
            "empty results must warn about literal mismatch before the model \
             concludes the data doesn't exist: {s}"
        );
        assert!(s.contains("ILIKE"), "suggests a concrete recovery: {s}");
    }

    #[test]
    fn non_empty_result_has_preview_and_no_hint() {
        let rows = vec![vec![serde_json::json!("CAN")]];
        let s = build_text_summary("SELECT code FROM airports", &cols(), &rows, 1, 3, false);
        assert!(s.contains("| code |"), "markdown preview header: {s}");
        assert!(s.contains("| CAN |"), "markdown preview row: {s}");
        assert!(
            !s.contains("may not match stored values"),
            "no hint when rows exist"
        );
    }

    #[test]
    fn truncated_result_is_flagged() {
        let rows: Vec<Vec<serde_json::Value>> = (0..12)
            .map(|i| vec![serde_json::json!(format!("v{i}"))])
            .collect();
        let s = build_text_summary("SELECT code FROM airports", &cols(), &rows, 500, 9, true);
        assert!(s.contains("truncated"), "{s}");
    }

    #[test]
    fn slow_query_gets_a_do_not_rerun_note() {
        let rows = vec![vec![serde_json::json!("CAN")]];
        let s = build_text_summary("SELECT 1", &cols(), &rows, 1, 17_000, false);
        assert!(s.contains("17.0s"), "{s}");
        assert!(
            s.contains("Avoid re-running"),
            "slow queries must warn the model: {s}"
        );
    }

    #[test]
    fn fast_query_has_no_slow_note() {
        let rows = vec![vec![serde_json::json!("CAN")]];
        let s = build_text_summary("SELECT 1", &cols(), &rows, 1, 40, false);
        assert!(!s.contains("Avoid re-running"), "{s}");
    }
}
