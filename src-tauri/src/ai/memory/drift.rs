use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::ai::schema_graph::{CatalogSnapshot, SchemaGraph};
use super::entity_linker::compute_entity_fingerprint;

#[cfg(test)]
thread_local! {
    /// Gate 2 invocations on the current test thread. B-I2 is an
    /// I/O-ordering property — no candidate's *inclusion* changes when the RRF
    /// filter moves ahead of the gate — so a call counter is the only faithful
    /// way to assert that zero-scoring candidates never pay for a per-memory
    /// `memory_entity_links` query. Thread-local keeps parallel tests isolated.
    static LIVE_GATE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub fn live_gate_call_count() -> usize {
    LIVE_GATE_CALLS.with(|c| c.get())
}

#[cfg(test)]
pub fn reset_live_gate_call_count() {
    LIVE_GATE_CALLS.with(|c| c.set(0));
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftAlert {
    /// The frontend memory connection key the invalidated memory belongs to
    /// (profile id or `host:port/database`). Carried so the
    /// `memory:drift_detected` event can be routed to the matching connection's
    /// drawer without the frontend having to guess the active connection.
    pub connection_key: String,
    pub memory_id: String,
    pub rule_text: String,
    pub reason: String,
    pub schema_name: String,
    pub table_name: String,
    pub column_name: Option<String>,
}

/// Gate 1: DDL Event Cascade Invalidation.
/// Compares active memories' entity links against a new CatalogSnapshot.
/// If any referenced table or column was dropped or altered, transitions the
/// dependent memory to status = 'STALE_INVALID' and tombstone = 1.
///
/// Scoped to `connection_key`: Lucent's memory model is per-connection, so a
/// catalog diff for one connection must never invalidate another connection's
/// memories (whose linked tables simply belong to a different database). Both
/// the link scan and the write are filtered.
pub fn cascade_schema_drift(
    connection_key: &str,
    snapshot: &CatalogSnapshot,
    conn: &Connection,
) -> Result<Vec<DriftAlert>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT l.memory_id, l.schema_name, l.table_name, NULLIF(l.column_name, ''), l.entity_fingerprint, m.rule_text
             FROM memory_entity_links l
             JOIN memories m ON l.memory_id = m.id
             WHERE m.status = 'ACTIVE' AND m.tombstone = 0 AND m.connection_key = ?1",
        )
        .map_err(|e| format!("failed to prepare drift query: {e}"))?;

    let links = stmt
        .query_map(params![connection_key], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|e| format!("failed to query links: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("failed to collect links: {e}"))?;

    let now = chrono::Utc::now().timestamp();
    let mut alerts: Vec<DriftAlert> = Vec::new();
    let mut invalidated_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (mem_id, schema, table, col_opt, fp, rule_text) in links {
        let table_exists = snapshot
            .tables
            .iter()
            .any(|t| t.schema.eq_ignore_ascii_case(&schema) && t.name.eq_ignore_ascii_case(&table));

        if !table_exists {
            alerts.push(DriftAlert {
                connection_key: connection_key.to_string(),
                memory_id: mem_id.clone(),
                rule_text: rule_text.clone(),
                reason: format!("Table '{schema}.{table}' was dropped"),
                schema_name: schema.clone(),
                table_name: table.clone(),
                column_name: None,
            });
            invalidated_ids.insert(mem_id);
            continue;
        }

        if let Some(col_name) = &col_opt {
            let col = snapshot.columns.iter().find(|c| {
                c.schema.eq_ignore_ascii_case(&schema)
                    && c.table.eq_ignore_ascii_case(&table)
                    && c.name.eq_ignore_ascii_case(col_name)
            });

            match col {
                None => {
                    alerts.push(DriftAlert {
                        connection_key: connection_key.to_string(),
                        memory_id: mem_id.clone(),
                        rule_text: rule_text.clone(),
                        reason: format!("Column '{schema}.{table}.{col_name}' was dropped"),
                        schema_name: schema.clone(),
                        table_name: table.clone(),
                        column_name: Some(col_name.clone()),
                    });
                    invalidated_ids.insert(mem_id);
                }
                Some(c) => {
                    let current_fp = compute_entity_fingerprint(
                        &schema,
                        &table,
                        Some(col_name),
                        &c.data_type,
                        c.is_nullable,
                    );
                    if current_fp != fp {
                        alerts.push(DriftAlert {
                            connection_key: connection_key.to_string(),
                            memory_id: mem_id.clone(),
                            rule_text: rule_text.clone(),
                            reason: format!(
                                "Column '{schema}.{table}.{col_name}' definition changed (type/nullability)"
                            ),
                            schema_name: schema.clone(),
                            table_name: table.clone(),
                            column_name: Some(col_name.clone()),
                        });
                        invalidated_ids.insert(mem_id);
                    }
                }
            }
        }
    }

    for id in &invalidated_ids {
        conn.execute(
            "UPDATE memories SET status = 'STALE_INVALID', tombstone = 1, tombstoned_at = ?1, updated_at = ?1 WHERE id = ?2 AND connection_key = ?3",
            params![now, id, connection_key],
        )
        .map_err(|e| format!("failed to mark memory stale: {e}"))?;
    }

    Ok(alerts)
}

/// Gate 2: Live Retrieval Re-Validation.
/// Verifies entity dependencies directly against the live in-memory SchemaGraph
/// at query time. If any linked table or column is missing, returns false.
pub fn validate_live_schema_gate(
    memory_id: &str,
    graph: &SchemaGraph,
    conn: &Connection,
) -> bool {
    #[cfg(test)]
    LIVE_GATE_CALLS.with(|c| c.set(c.get() + 1));

    let mut stmt = match conn.prepare(
        "SELECT schema_name, table_name, NULLIF(column_name, '') FROM memory_entity_links WHERE memory_id = ?1",
    ) {
        Ok(s) => s,
        Err(_) => return true,
    };

    let links: Vec<(String, String, Option<String>)> = match stmt
        .query_map(params![memory_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .and_then(|mapped| mapped.collect::<Result<Vec<_>, _>>())
    {
        Ok(l) => l,
        Err(_) => return true,
    };

    // If no entity links are attached, gate passes trivially
    if links.is_empty() {
        return true;
    }

    for (schema, table, col_opt) in links {
        let table_entry = graph
            .tables
            .iter()
            .find(|t| t.schema.eq_ignore_ascii_case(&schema) && t.name.eq_ignore_ascii_case(&table));

        let Some(t) = table_entry else {
            return false;
        };

        if let Some(col_name) = col_opt {
            let cols = graph.columns_for_table(t.id);
            let col_exists = cols.iter().any(|c| c.name.eq_ignore_ascii_case(&col_name));
            if !col_exists {
                return false;
            }
        }
    }

    true
}
