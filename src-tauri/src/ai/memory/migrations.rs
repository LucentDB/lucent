use rusqlite::Connection;

pub const MEMORY_SCHEMA_VERSION: u32 = 2;

pub fn ensure_v2_schema(conn: &Connection) -> Result<(), String> {
    // Check existing columns on memories table
    let mut stmt = conn
        .prepare("PRAGMA table_info(memories)")
        .map_err(|e| e.to_string())?;
    let existing_cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .collect();

    let add_column_if_missing = |col_name: &str, ddl: &str| -> Result<(), String> {
        if !existing_cols.contains(&col_name.to_string()) {
            conn.execute(ddl, [])
                .map_err(|e| format!("failed to add column {col_name}: {e}"))?;
        }
        Ok(())
    };

    add_column_if_missing(
        "injection",
        "ALTER TABLE memories ADD COLUMN injection TEXT NOT NULL DEFAULT 'retrieved'",
    )?;
    add_column_if_missing(
        "preference_key",
        "ALTER TABLE memories ADD COLUMN preference_key TEXT",
    )?;
    add_column_if_missing(
        "origin",
        "ALTER TABLE memories ADD COLUMN origin TEXT NOT NULL DEFAULT 'agent'",
    )?;
    add_column_if_missing(
        "steps_json",
        "ALTER TABLE memories ADD COLUMN steps_json TEXT",
    )?;
    add_column_if_missing(
        "merge_group_id",
        "ALTER TABLE memories ADD COLUMN merge_group_id TEXT",
    )?;
    add_column_if_missing(
        "confirmed",
        "ALTER TABLE memories ADD COLUMN confirmed INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        "confirmation_conv_id",
        "ALTER TABLE memories ADD COLUMN confirmation_conv_id TEXT",
    )?;

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_injection
            ON memories(connection_key, injection, status, tombstone, importance);
         CREATE INDEX IF NOT EXISTS idx_memories_preference_key
            ON memories(connection_key, preference_key) WHERE preference_key IS NOT NULL;
         CREATE INDEX IF NOT EXISTS idx_memories_merge_group
            ON memories(merge_group_id) WHERE merge_group_id IS NOT NULL;

         CREATE TABLE IF NOT EXISTS memory_observations (
             id                TEXT PRIMARY KEY,
             connection_key    TEXT NOT NULL,
             conversation_id   TEXT,
             turn_id           TEXT,
             kind              TEXT NOT NULL,
             origin            TEXT NOT NULL,
             signal            TEXT NOT NULL,
             signal_strength   REAL NOT NULL,
             occurrence_count  INTEGER NOT NULL DEFAULT 1,
             dedup_key         TEXT NOT NULL,
             payload_json      TEXT NOT NULL,
             status            TEXT NOT NULL DEFAULT 'open',
             derived_memory_id TEXT,
             created_at        INTEGER NOT NULL,
             updated_at        INTEGER NOT NULL
         );

         CREATE UNIQUE INDEX IF NOT EXISTS idx_observations_dedup
             ON memory_observations(dedup_key);
         CREATE INDEX IF NOT EXISTS idx_observations_pending
             ON memory_observations(connection_key, status, signal_strength, created_at);",
    )
    .map_err(|e| format!("failed to run v2 schema migration: {e}"))?;

    Ok(())
}
