use lucent_protocol::SqlDialect;
use serde::Deserialize;

use crate::query_paging::{
    is_wrappable_query, normalize_sql_body, wrap_for_count, wrap_for_page, FilterSpec, SortSpec,
};
use crate::sql_builder::SqlBuilder;

/// Default rows fetched per notebook cell page. Deliberately small: a cell is an
/// inline preview, not a full grid.
pub const DEFAULT_CELL_PAGE_SIZE: i64 = 10;

#[derive(Debug, Clone, Deserialize)]
pub struct PageRequest {
    pub limit: i64,
    pub offset: i64,
    pub sort: Option<SortSpec>,
    pub filters: Vec<FilterSpec>,
}

/// Wraps a composed cell query for one page. Non-wrappable bodies (DML, DDL,
/// multi-statement) run unchanged and unpaginated — the same rule `execute_query`
/// already applies, rather than a second notebook-specific one.
pub fn build_page_sql(
    composed: &str,
    req: &PageRequest,
    dialect: SqlDialect,
    builder: &dyn SqlBuilder,
) -> String {
    let body = normalize_sql_body(composed);
    if !is_wrappable_query(body, dialect) {
        return composed.to_string();
    }
    wrap_for_page(
        body,
        &req.sort,
        &req.filters,
        req.limit,
        req.offset,
        builder,
    )
}

pub fn build_count_sql(
    composed: &str,
    filters: &[FilterSpec],
    dialect: SqlDialect,
    builder: &dyn SqlBuilder,
) -> Option<String> {
    let body = normalize_sql_body(composed);
    if !is_wrappable_query(body, dialect) {
        return None;
    }
    Some(wrap_for_count(body, filters, builder))
}

pub fn is_pageable(composed: &str, dialect: SqlDialect) -> bool {
    is_wrappable_query(normalize_sql_body(composed), dialect)
}

use tauri::State;

use crate::commands::{AppState, CommandError};
use crate::notebook::rewrite;
use crate::notebook::types::{CellModel, TableOutput};
use lucent_protocol::QueryId;
use uuid::Uuid;

#[tauri::command]
// The IPC surface of a tauri command is its contract with the frontend —
// wrapping these into a struct would just move the 8-tuple elsewhere.
#[allow(clippy::too_many_arguments)]
pub async fn notebook_fetch_page(
    session_key: String,
    cell_id: String,
    cells: Vec<CellModel>,
    limit: i64,
    offset: i64,
    sort: Option<SortSpec>,
    filters: Vec<FilterSpec>,
    state: State<'_, AppState>,
) -> Result<TableOutput, CommandError> {
    let conn_id = state
        .notebook_sessions
        .get(&session_key)
        .map(|s| s.connection_id)
        .ok_or_else(|| CommandError::new("not_found", "notebook session not found"))?;

    let capabilities = state
        .capabilities()
        .await
        .ok_or_else(|| CommandError::new("not_connected", "no active connection"))?;
    let dialect = capabilities.sql_dialect;
    let builder = crate::sql_builder::for_driver(&capabilities);

    let composed = rewrite::rewrite_sql(&cell_id, &cells, dialect).map_err(|e| {
        CommandError::new(
            "rewrite_failed",
            serde_json::to_string(&e).unwrap_or_default(),
        )
    })?;

    let pageable = is_pageable(&composed, dialect);
    let req = PageRequest {
        limit,
        offset,
        sort,
        filters,
    };
    let page_sql = build_page_sql(&composed, &req, dialect, builder.as_ref());

    let query_id = QueryId(Uuid::new_v4());
    if let Some(mut s) = state.notebook_sessions.get_mut(&session_key) {
        s.active_query_id = Some(query_id);
    }

    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("not_connected", "no active connection"))?;
    // Execute under the registered query_id (so notebook cancel reaches the
    // real query) with the hard row cap: non-wrappable bodies (multi-statement
    // etc.) run unpaginated, so the cap is what bounds them.
    let result = client
        .execute_with_id(
            query_id,
            conn_id,
            &page_sql,
            Some(crate::client::HARD_ROW_CAP),
        )
        .await;

    if let Some(mut s) = state.notebook_sessions.get_mut(&session_key) {
        s.active_query_id = None;
    }

    let result = result.map_err(|e| CommandError::new("query_failed", e))?;
    let (result, _query_id) = result;

    Ok(TableOutput {
        columns: result.columns,
        rows: result.rows,
        total_count: None,
        is_truncated: result.truncated,
        page_size: limit,
        is_wrappable: pageable,
        rows_affected: None,
    })
}

#[tauri::command]
pub async fn notebook_count_rows(
    session_key: String,
    cell_id: String,
    cells: Vec<CellModel>,
    filters: Vec<FilterSpec>,
    state: State<'_, AppState>,
) -> Result<i64, CommandError> {
    let conn_id = state
        .notebook_sessions
        .get(&session_key)
        .map(|s| s.connection_id)
        .ok_or_else(|| CommandError::new("not_found", "notebook session not found"))?;

    let capabilities = state
        .capabilities()
        .await
        .ok_or_else(|| CommandError::new("not_connected", "no active connection"))?;
    let dialect = capabilities.sql_dialect;
    let builder = crate::sql_builder::for_driver(&capabilities);

    let composed = rewrite::rewrite_sql(&cell_id, &cells, dialect).map_err(|e| {
        CommandError::new(
            "rewrite_failed",
            serde_json::to_string(&e).unwrap_or_default(),
        )
    })?;

    let count_sql =
        build_count_sql(&composed, &filters, dialect, builder.as_ref()).ok_or_else(|| {
            CommandError::new(
                "not_countable",
                "this cell is not a single SELECT and cannot be counted",
            )
        })?;

    let client = state
        .client_handle()
        .await
        .ok_or_else(|| CommandError::new("not_connected", "no active connection"))?;
    let result = client
        .execute(conn_id, &count_sql)
        .await
        .map_err(|e| CommandError::new("query_failed", e))?;

    let count = result
        .rows
        .first()
        .and_then(|r| r.first())
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0);

    Ok(count)
}

#[cfg(test)]
mod unit_tests {
    use crate::sql_builder::PostgresSqlBuilder;

    use lucent_protocol::SqlDialect;

    use super::*;
    use crate::query_paging::{FilterSpec, SortSpec};

    const PG: SqlDialect = SqlDialect::PostgreSql;

    fn pg() -> PostgresSqlBuilder {
        PostgresSqlBuilder
    }

    fn req(limit: i64, offset: i64) -> PageRequest {
        PageRequest {
            limit,
            offset,
            sort: None,
            filters: vec![],
        }
    }

    #[test]
    fn wraps_a_select_with_limit_and_offset() {
        let sql = build_page_sql("SELECT * FROM t", &req(10, 20), PG, &pg());
        assert!(sql.contains("LIMIT 10"), "got {sql}");
        assert!(sql.contains("OFFSET 20"), "got {sql}");
        assert!(sql.contains("_lucent_page"), "got {sql}");
    }

    #[test]
    fn wraps_a_composed_cte_chain() {
        let composed = "WITH _cell_a1b2c3d4 AS (SELECT 1 AS n) SELECT * FROM _cell_a1b2c3d4";
        let sql = build_page_sql(composed, &req(5, 0), PG, &pg());
        assert!(sql.contains("LIMIT 5"), "got {sql}");
        assert!(sql.contains("_cell_a1b2c3d4"), "got {sql}");
    }

    #[test]
    fn filters_and_sort_apply_outside_the_inner_query() {
        let r = PageRequest {
            limit: 10,
            offset: 0,
            sort: Some(SortSpec {
                column: "total".into(),
                direction: "desc".into(),
            }),
            filters: vec![FilterSpec {
                column: "region".into(),
                operator: "eq".into(),
                value: Some("North".into()),
            }],
        };
        let sql = build_page_sql("SELECT region, total FROM sales", &r, PG, &pg());
        let page_at = sql.find("_lucent_page").expect("wrapper alias");
        let where_at = sql.find("WHERE").expect("where clause");
        let order_at = sql.find("ORDER BY").expect("order by");
        // Both must land after the wrapper alias, i.e. outside the inner query.
        assert!(where_at > page_at, "got {sql}");
        assert!(order_at > where_at, "got {sql}");
    }

    #[test]
    fn leaves_non_wrappable_sql_untouched() {
        let dml = "INSERT INTO t VALUES (1)";
        assert_eq!(build_page_sql(dml, &req(10, 0), PG, &pg()), dml);
        let multi = "SELECT 1; SELECT 2";
        assert_eq!(build_page_sql(multi, &req(10, 0), PG, &pg()), multi);
    }

    #[test]
    fn count_sql_is_none_for_non_wrappable() {
        assert!(build_count_sql("INSERT INTO t VALUES (1)", &[], PG, &pg()).is_none());
    }

    #[test]
    fn count_sql_counts_through_filters() {
        let filters = vec![FilterSpec {
            column: "region".into(),
            operator: "eq".into(),
            value: Some("North".into()),
        }];
        let sql = build_count_sql("SELECT * FROM sales", &filters, PG, &pg()).unwrap();
        assert!(sql.contains("COUNT(*)"), "got {sql}");
        assert!(sql.contains("WHERE"), "got {sql}");
    }

    #[test]
    fn strips_trailing_semicolon_before_wrapping() {
        let sql = build_page_sql("SELECT * FROM t;", &req(10, 0), PG, &pg());
        assert!(!sql.contains(";"), "got {sql}");
    }
}
