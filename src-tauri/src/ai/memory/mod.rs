pub mod consolidation;
pub mod curation;
pub mod decay;
pub mod drift;
pub mod entity_linker;
pub mod gate;
pub mod migrations;
pub mod observations;
pub mod profile;
pub mod reflection;
pub mod retrieval;
pub mod rules_parser;
pub mod security;
pub mod telemetry;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

pub use consolidation::*;
pub use curation::*;
pub use decay::*;
pub use drift::*;
pub use entity_linker::*;
pub use gate::*;
pub use observations::*;
pub use profile::*;
pub use reflection::*;
pub use retrieval::*;
pub use rules_parser::*;
pub use security::*;
pub use telemetry::*;

pub const MEMORY_FORMAT_VERSION: u32 = 1;
pub const MEMORY_MODEL_NAME: &str = "bge-small-en-v1.5";
pub const HOT_TIER_TOKEN_BUDGET: usize = 800;
const DB_FILE_NAME: &str = "memory.db";

/// Ebbinghaus stability (S), in hours, for a rule the user stated explicitly
/// (saved by hand or imported from Markdown). B-I7: a user's explicit rule is
/// the strongest signal in the hierarchy and must outlive rules the agent
/// inferred from tool results — never decay faster than them. 2160h = 90 days.
pub const USER_EXPLICIT_STABILITY_HOURS: f32 = 2160.0;
/// Ebbinghaus stability (S), in hours, for a rule the agent inferred from a
/// tool result (lowest authority). 720h = 30 days. B-I7: referenced by both the
/// manual-save path and the tool-save path so the two can never silently
/// invert again.
pub const TOOL_RULE_STABILITY_HOURS: f32 = 720.0;

pub fn compute_memory_doc_hash(doc_text: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(format!("lucent-mem-v{MEMORY_FORMAT_VERSION}:{MEMORY_MODEL_NAME}:").as_bytes());
    hasher.update(doc_text.as_bytes());
    hasher.finalize().to_hex().to_string()
}

pub fn embedding_to_blob(vec: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vec.len() * 4);
    for val in vec {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

pub fn blob_to_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    Global,
    Connection,
    Schema,
}

impl MemoryScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryScope::Global => "global",
            MemoryScope::Connection => "connection",
            MemoryScope::Schema => "schema",
        }
    }

    // Lenient parse: unknown strings fall back to `Global` rather than erroring,
    // so this deliberately stays an inherent method instead of `FromStr`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "connection" => MemoryScope::Connection,
            "schema" => MemoryScope::Schema,
            _ => MemoryScope::Global,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    Metric,
    Join,
    Quirk,
    Preference,
    Playbook,
}

impl MemoryCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryCategory::Metric => "metric",
            MemoryCategory::Join => "join",
            MemoryCategory::Quirk => "quirk",
            MemoryCategory::Preference => "preference",
            MemoryCategory::Playbook => "playbook",
        }
    }

    // Lenient parse: unknown strings fall back to `Quirk` rather than erroring,
    // so this deliberately stays an inherent method instead of `FromStr`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "metric" => MemoryCategory::Metric,
            "join" => MemoryCategory::Join,
            "preference" | "formatting_preference" => MemoryCategory::Preference,
            "playbook" => MemoryCategory::Playbook,
            _ => MemoryCategory::Quirk,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Active,
    Superseded,
    StaleInvalid,
    Archived,
}

impl MemoryStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MemoryStatus::Active => "ACTIVE",
            MemoryStatus::Superseded => "SUPERSEDED",
            MemoryStatus::StaleInvalid => "STALE_INVALID",
            MemoryStatus::Archived => "ARCHIVED",
        }
    }

    // Lenient parse: unknown strings fall back to `Active` rather than erroring,
    // so this deliberately stays an inherent method instead of `FromStr`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        if s.eq_ignore_ascii_case("SUPERSEDED") {
            MemoryStatus::Superseded
        } else if s.eq_ignore_ascii_case("STALE_INVALID") {
            MemoryStatus::StaleInvalid
        } else if s.eq_ignore_ascii_case("ARCHIVED") {
            MemoryStatus::Archived
        } else {
            MemoryStatus::Active
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: String,
    pub connection_key: String,
    pub scope: MemoryScope,
    pub scope_key: String,
    pub category: MemoryCategory,
    pub key_phrase: String,
    pub rule_text: String,
    pub sql_snippet: Option<String>,
    pub importance: f32,
    pub stability_hours: f32,
    pub last_accessed_at: i64,
    pub access_count: i64,
    pub source_trust: SourceTrust,
    pub source_conv_id: Option<String>,
    pub source_turn_id: Option<String>,
    pub source_tool_id: Option<String>,
    pub status: MemoryStatus,
    pub supersedes_id: Option<String>,
    pub valid_from: i64,
    pub valid_until: Option<i64>,
    pub learned_at: i64,
    pub tombstone: bool,
    pub tombstoned_at: Option<i64>,
    pub doc_hash: String,
    pub embedding_model: String,
    pub embedding_version: u32,
    #[serde(skip_serializing)]
    pub embedding: Vec<f32>,
    pub created_at: i64,
    pub updated_at: i64,
    pub injection: InjectionClass,
    pub preference_key: Option<String>,
    pub origin: Origin,
    pub steps_json: Option<String>,
    pub merge_group_id: Option<String>,
    pub confirmed: bool,
    pub confirmation_conv_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenQuery {
    pub id: String,
    pub connection_id: String,
    pub schema_name: String,
    pub natural_prompt: String,
    pub sql_text: String,
    pub tables_used: Vec<String>,
    pub verified: bool,
    pub run_count: u32,
    pub last_run_at: i64,
    pub embedding_model: String,
    pub embedding_version: u32,
    #[serde(skip_serializing)]
    pub embedding: Vec<f32>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatConversation {
    pub id: String,
    pub connection_id: String,
    pub title: String,
    pub archived: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub session_json: Option<String>,
    pub created_at: i64,
}

#[derive(Clone)]
pub struct MemoryManager {
    conn: Arc<Mutex<Connection>>,
}

impl MemoryManager {
    pub fn open_default() -> Result<Self, String> {
        let mut dir = dirs::config_dir().ok_or("no config directory found")?;
        dir.push("lucent");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Self::open_at(dir.join(DB_FILE_NAME))
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        Self::init_tables(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn open_at(db_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
        Self::init_tables(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn init_tables(conn: &Connection) -> Result<(), String> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;

             CREATE TABLE IF NOT EXISTS chat_conversations (
                 id             TEXT PRIMARY KEY,
                 connection_id  TEXT NOT NULL,
                 title          TEXT NOT NULL,
                 archived       INTEGER NOT NULL DEFAULT 0,
                 created_at     INTEGER NOT NULL,
                 updated_at     INTEGER NOT NULL
             );

             CREATE TABLE IF NOT EXISTS chat_messages (
                 id              TEXT PRIMARY KEY,
                 conversation_id TEXT NOT NULL REFERENCES chat_conversations(id) ON DELETE CASCADE,
                 role            TEXT NOT NULL,
                 content         TEXT NOT NULL,
                 session_json    TEXT,
                 created_at      INTEGER NOT NULL
             );

             CREATE INDEX IF NOT EXISTS idx_chat_messages_conv ON chat_messages(conversation_id, created_at);

             CREATE TABLE IF NOT EXISTS memories (
                 id                 TEXT PRIMARY KEY,
                 connection_key     TEXT NOT NULL,
                 scope              TEXT NOT NULL,
                 scope_key          TEXT NOT NULL,
                 category           TEXT NOT NULL,
                 key_phrase         TEXT NOT NULL,
                 rule_text          TEXT NOT NULL,
                 sql_snippet        TEXT,
                 importance         REAL NOT NULL DEFAULT 0.5,
                 stability_hours    REAL NOT NULL DEFAULT 720.0,
                 last_accessed_at   INTEGER NOT NULL,
                 access_count       INTEGER NOT NULL DEFAULT 1,
                 source_trust       TEXT NOT NULL,
                 source_conv_id     TEXT,
                 source_turn_id     TEXT,
                 source_tool_id     TEXT,
                 status             TEXT NOT NULL DEFAULT 'ACTIVE',
                 supersedes_id      TEXT REFERENCES memories(id),
                 valid_from         INTEGER NOT NULL,
                 valid_until        INTEGER,
                 learned_at         INTEGER NOT NULL,
                 tombstone          INTEGER NOT NULL DEFAULT 0,
                 tombstoned_at      INTEGER,
                 doc_hash           TEXT NOT NULL,
                 embedding_model    TEXT NOT NULL,
                 embedding_version  INTEGER NOT NULL DEFAULT 1,
                 embedding_blob     BLOB NOT NULL,
                 created_at         INTEGER NOT NULL,
                 updated_at         INTEGER NOT NULL
             );

             CREATE INDEX IF NOT EXISTS idx_memories_lifecycle 
             ON memories(connection_key, status, tombstone, last_accessed_at);

             CREATE TABLE IF NOT EXISTS memory_entity_links (
                 memory_id          TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
                 schema_name        TEXT NOT NULL,
                 table_name         TEXT NOT NULL,
                 column_name        TEXT NOT NULL DEFAULT '',
                 entity_fingerprint TEXT NOT NULL,
                 PRIMARY KEY (memory_id, schema_name, table_name, column_name)
             );

             CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                 id UNINDEXED,
                 content,
                 tokenize = 'porter unicode61'
             );

             CREATE TABLE IF NOT EXISTS golden_queries (
                 id                TEXT PRIMARY KEY,
                 connection_id     TEXT NOT NULL,
                 schema_name       TEXT NOT NULL,
                 natural_prompt    TEXT NOT NULL,
                 sql_text          TEXT NOT NULL,
                 tables_used       TEXT NOT NULL,
                 verified          INTEGER NOT NULL DEFAULT 1,
                 run_count         INTEGER NOT NULL DEFAULT 1,
                 last_run_at       INTEGER NOT NULL,
                 embedding_model   TEXT NOT NULL,
                 embedding_version INTEGER NOT NULL DEFAULT 1,
                 embedding_blob    BLOB NOT NULL,
                 created_at        INTEGER NOT NULL
             );

             CREATE INDEX IF NOT EXISTS idx_golden_conn ON golden_queries(connection_id, schema_name);",
        )
        .map_err(|e| format!("failed to initialize memory database: {e}"))?;

        Self::migrate_entity_links(conn)?;
        migrations::ensure_v2_schema(conn)?;

        Ok(())
    }

    /// B-I8: `memory_entity_links.column_name` was nullable *and* part of the
    /// composite PRIMARY KEY. SQLite permits NULLs in (non-rowid) PRIMARY KEY
    /// columns, so that key never actually enforced entity-link uniqueness.
    /// The table now stores table-level links as `''` with `NOT NULL DEFAULT ''`.
    ///
    /// `CREATE TABLE IF NOT EXISTS` cannot alter an existing database, so
    /// rebuild the table in place when the legacy shape is detected, preserving
    /// rows and normalizing NULL (table-level link) to `''`. The standard
    /// SQLite table-rebuild procedure disables foreign keys for the duration
    /// and re-enables them afterwards.
    fn migrate_entity_links(conn: &Connection) -> Result<(), String> {
        let legacy_schema = {
            let mut stmt = conn
                .prepare("PRAGMA table_info(memory_entity_links)")
                .map_err(|e| format!("failed to inspect memory_entity_links: {e}"))?;
            let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
            let mut column_notnull: Option<i64> = None;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let name: String = row.get(1).map_err(|e| e.to_string())?;
                if name == "column_name" {
                    column_notnull = Some(row.get(3).map_err(|e| e.to_string())?);
                }
            }
            matches!(column_notnull, Some(0))
        };

        if !legacy_schema {
            return Ok(());
        }

        conn.execute_batch(
            "PRAGMA foreign_keys = OFF;

             BEGIN;
             DROP TABLE IF EXISTS memory_entity_links_new;
             CREATE TABLE memory_entity_links_new (
                 memory_id          TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
                 schema_name        TEXT NOT NULL,
                 table_name         TEXT NOT NULL,
                 column_name        TEXT NOT NULL DEFAULT '',
                 entity_fingerprint TEXT NOT NULL,
                 PRIMARY KEY (memory_id, schema_name, table_name, column_name)
             );
             INSERT OR IGNORE INTO memory_entity_links_new
                 (memory_id, schema_name, table_name, column_name, entity_fingerprint)
             SELECT memory_id, schema_name, table_name, COALESCE(column_name, ''), entity_fingerprint
             FROM memory_entity_links;
             DROP TABLE memory_entity_links;
             ALTER TABLE memory_entity_links_new RENAME TO memory_entity_links;
             COMMIT;

             PRAGMA foreign_keys = ON;",
        )
        .map_err(|e| format!("failed to migrate memory_entity_links: {e}"))?;

        Ok(())
    }

    pub async fn save_memory(
        &self,
        item: MemoryItem,
        entity_links: &[EntityRef],
    ) -> Result<(), String> {
        let mut guard = self.conn.lock().await;
        let emb_blob = embedding_to_blob(&item.embedding);
        let tx = guard
            .transaction()
            .map_err(|e| format!("failed to begin save_memory transaction: {e}"))?;

        tx.execute(
            "INSERT INTO memories (
                id, connection_key, scope, scope_key, category, key_phrase, rule_text,
                sql_snippet, importance, stability_hours, last_accessed_at, access_count,
                source_trust, source_conv_id, source_turn_id, source_tool_id, status,
                supersedes_id, valid_from, valid_until, learned_at, tombstone, tombstoned_at,
                doc_hash, embedding_model, embedding_version, embedding_blob, created_at, updated_at,
                injection, preference_key, origin, steps_json, merge_group_id, confirmed,
                confirmation_conv_id
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29,
                ?30, ?31, ?32, ?33, ?34, ?35, ?36
            ) ON CONFLICT(id) DO UPDATE SET
                rule_text = excluded.rule_text,
                sql_snippet = excluded.sql_snippet,
                importance = excluded.importance,
                stability_hours = excluded.stability_hours,
                last_accessed_at = excluded.last_accessed_at,
                status = excluded.status,
                tombstone = excluded.tombstone,
                updated_at = excluded.updated_at,
                embedding_blob = excluded.embedding_blob,
                injection = excluded.injection,
                preference_key = excluded.preference_key,
                origin = excluded.origin,
                steps_json = excluded.steps_json,
                merge_group_id = excluded.merge_group_id,
                confirmed = excluded.confirmed,
                confirmation_conv_id = excluded.confirmation_conv_id",
            params![
                item.id,
                item.connection_key,
                item.scope.as_str(),
                item.scope_key,
                item.category.as_str(),
                item.key_phrase,
                item.rule_text,
                item.sql_snippet,
                item.importance,
                item.stability_hours,
                item.last_accessed_at,
                item.access_count,
                item.source_trust.as_str(),
                item.source_conv_id,
                item.source_turn_id,
                item.source_tool_id,
                item.status.as_str(),
                item.supersedes_id,
                item.valid_from,
                item.valid_until,
                item.learned_at,
                item.tombstone as i64,
                item.tombstoned_at,
                item.doc_hash,
                item.embedding_model,
                item.embedding_version,
                emb_blob,
                item.created_at,
                item.updated_at,
                item.injection.as_str(),
                item.preference_key,
                item.origin.as_str(),
                item.steps_json,
                item.merge_group_id,
                item.confirmed as i64,
                item.confirmation_conv_id,
            ],
        )
        .map_err(|e| format!("failed to insert memory: {e}"))?;

        // FTS update
        let fts_content = format!(
            "{} {} {}",
            item.key_phrase,
            item.rule_text,
            item.sql_snippet.as_deref().unwrap_or("")
        );
        tx.execute("DELETE FROM memories_fts WHERE id = ?1", params![item.id])
            .map_err(|e| format!("failed to clear fts entry: {e}"))?;
        tx.execute(
            "INSERT INTO memories_fts (id, content) VALUES (?1, ?2)",
            params![item.id, fts_content],
        )
        .map_err(|e| format!("failed to update fts: {e}"))?;

        // Entity links update. `column_name` is NOT NULL DEFAULT '' (B-I8);
        // table-level links are stored as the empty string.
        tx.execute(
            "DELETE FROM memory_entity_links WHERE memory_id = ?1",
            params![item.id],
        )
        .map_err(|e| format!("failed to clear entity links: {e}"))?;
        for link in entity_links {
            tx.execute(
                "INSERT INTO memory_entity_links (memory_id, schema_name, table_name, column_name, entity_fingerprint)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    item.id,
                    link.schema_name,
                    link.table_name,
                    link.column_name.as_deref().unwrap_or(""),
                    link.entity_fingerprint
                ],
            ).map_err(|e| format!("failed to insert entity link: {e}"))?;
        }

        tx.commit()
            .map_err(|e| format!("failed to commit save_memory transaction: {e}"))?;

        Ok(())
    }

    /// Folds one or more `fold_ids` into a surviving `keep_id` memory.
    ///
    /// All participating rows share a freshly minted `merge_group_id`, and each
    /// fold row is marked `SUPERSEDED` with `supersedes_id = keep_id` so the
    /// merge is a traversable DAG edge rather than a destructive overwrite.
    /// The keeper is never superseded even if it appears in `fold_ids`, and a
    /// fold id with no matching row is skipped without error (defensive).
    pub async fn merge_memories(
        &self,
        keep_id: &str,
        fold_ids: &[String],
    ) -> Result<(), String> {
        let mut guard = self.conn.lock().await;
        let now = chrono::Utc::now().timestamp();
        let merge_group_id = uuid::Uuid::new_v4().to_string();

        let tx = guard
            .transaction()
            .map_err(|e| format!("failed to begin merge_memories transaction: {e}"))?;

        // Stamp the shared group on the keeper first. `WHERE id = ?` is a no-op
        // for an unknown keeper; the caller owns the keeper's existence.
        tx.execute(
            "UPDATE memories SET merge_group_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![merge_group_id, now, keep_id],
        )
        .map_err(|e| format!("failed to stamp merge group on keeper: {e}"))?;

        for fold_id in fold_ids {
            // Never let a caller fold the keeper into itself: guard in addition
            // to the UI/curation callers only ever passing distinct ids.
            if fold_id == keep_id {
                continue;
            }
            // An UPDATE matching zero rows is a silent no-op in SQLite, which is
            // exactly the defensive "skip missing fold id" behavior we want.
            tx.execute(
                "UPDATE memories
                 SET status = 'SUPERSEDED', supersedes_id = ?1, merge_group_id = ?2, updated_at = ?3
                 WHERE id = ?4",
                params![keep_id, merge_group_id, now, fold_id],
            )
            .map_err(|e| format!("failed to fold memory {fold_id}: {e}"))?;
        }

        tx.commit()
            .map_err(|e| format!("failed to commit merge_memories transaction: {e}"))?;

        Ok(())
    }

    pub async fn list_memories(
        &self,
        connection_key: &str,
        include_archived: bool,
    ) -> Result<Vec<MemoryItem>, String> {
        let guard = self.conn.lock().await;
        let mut sql = String::from(
            "SELECT id, connection_key, scope, scope_key, category, key_phrase, rule_text,
                    sql_snippet, importance, stability_hours, last_accessed_at, access_count,
                    source_trust, source_conv_id, source_turn_id, source_tool_id, status,
                    supersedes_id, valid_from, valid_until, learned_at, tombstone, tombstoned_at,
                    doc_hash, embedding_model, embedding_version, embedding_blob, created_at, updated_at,
                    injection, preference_key, origin, steps_json, merge_group_id, confirmed,
                    confirmation_conv_id
             FROM memories
             WHERE (connection_key = ?1 OR scope = 'global')",
        );

        if !include_archived {
            sql.push_str(" AND status != 'ARCHIVED' AND tombstone = 0");
        }
        sql.push_str(" ORDER BY last_accessed_at DESC");

        let mut stmt = guard.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![connection_key], |row| {
                let emb_bytes: Vec<u8> = row.get(26)?;
                Ok(MemoryItem {
                    id: row.get(0)?,
                    connection_key: row.get(1)?,
                    scope: MemoryScope::from_str(&row.get::<_, String>(2)?),
                    scope_key: row.get(3)?,
                    category: MemoryCategory::from_str(&row.get::<_, String>(4)?),
                    key_phrase: row.get(5)?,
                    rule_text: row.get(6)?,
                    sql_snippet: row.get(7)?,
                    importance: row.get::<_, f64>(8)? as f32,
                    stability_hours: row.get::<_, f64>(9)? as f32,
                    last_accessed_at: row.get(10)?,
                    access_count: row.get(11)?,
                    source_trust: SourceTrust::from_str(&row.get::<_, String>(12)?)
                        .unwrap_or(SourceTrust::UntrustedToolResult),
                    source_conv_id: row.get(13)?,
                    source_turn_id: row.get(14)?,
                    source_tool_id: row.get(15)?,
                    status: MemoryStatus::from_str(&row.get::<_, String>(16)?),
                    supersedes_id: row.get(17)?,
                    valid_from: row.get(18)?,
                    valid_until: row.get(19)?,
                    learned_at: row.get(20)?,
                    tombstone: row.get::<_, i64>(21)? != 0,
                    tombstoned_at: row.get(22)?,
                    doc_hash: row.get(23)?,
                    embedding_model: row.get(24)?,
                    embedding_version: row.get(25)?,
                    embedding: blob_to_embedding(&emb_bytes),
                    created_at: row.get(27)?,
                    updated_at: row.get(28)?,
                    injection: InjectionClass::from_str(&row.get::<_, String>(29)?),
                    preference_key: row.get(30)?,
                    origin: Origin::from_str(&row.get::<_, String>(31)?),
                    steps_json: row.get(32)?,
                    merge_group_id: row.get(33)?,
                    confirmed: row.get::<_, i64>(34)? != 0,
                    confirmation_conv_id: row.get(35)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(rows)
    }

    pub async fn delete_memory(&self, id: &str) -> Result<bool, String> {
        let mut guard = self.conn.lock().await;
        let tx = guard
            .transaction()
            .map_err(|e| format!("failed to begin delete_memory transaction: {e}"))?;
        tx.execute("DELETE FROM memories_fts WHERE id = ?1", params![id])
            .map_err(|e| format!("failed to delete fts entry: {e}"))?;
        tx.execute(
            "DELETE FROM memory_entity_links WHERE memory_id = ?1",
            params![id],
        )
        .map_err(|e| format!("failed to delete entity links: {e}"))?;
        let affected = tx
            .execute("DELETE FROM memories WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        tx.commit()
            .map_err(|e| format!("failed to commit delete_memory transaction: {e}"))?;
        Ok(affected > 0)
    }

    pub async fn toggle_memory_status(&self, id: &str, status: MemoryStatus) -> Result<(), String> {
        let guard = self.conn.lock().await;
        let now = chrono::Utc::now().timestamp();
        let tombstone = if status == MemoryStatus::StaleInvalid {
            1
        } else {
            0
        };
        guard
            .execute(
                "UPDATE memories SET status = ?1, tombstone = ?2, updated_at = ?3 WHERE id = ?4",
                params![status.as_str(), tombstone, now, id],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn revalidate_drift(
        &self,
        id: &str,
        graph: Option<&crate::ai::schema_graph::SchemaGraph>,
    ) -> Result<(), String> {
        let guard = self.conn.lock().await;
        let mut stmt = guard
            .prepare("SELECT schema_name, table_name, NULLIF(column_name, '') FROM memory_entity_links WHERE memory_id = ?1")
            .map_err(|e| e.to_string())?;

        let links = stmt
            .query_map(params![id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        if let Some(g) = graph {
            for (schema, table, col_opt) in &links {
                let table_entry = g.tables.iter().find(|t| {
                    t.schema.eq_ignore_ascii_case(schema) && t.name.eq_ignore_ascii_case(table)
                });
                let Some(t) = table_entry else {
                    return Err(format!(
                        "Cannot revalidate: table '{schema}.{table}' does not exist in schema"
                    ));
                };

                if let Some(col_name) = col_opt {
                    let cols = g.columns_for_table(t.id);
                    let col_entry = cols.iter().find(|c| c.name.eq_ignore_ascii_case(col_name));
                    let Some(c) = col_entry else {
                        return Err(format!("Cannot revalidate: column '{schema}.{table}.{col_name}' does not exist in schema"));
                    };
                    let new_fp = crate::ai::memory::entity_linker::compute_entity_fingerprint(
                        schema,
                        table,
                        Some(col_name),
                        &c.data_type,
                        c.is_nullable,
                    );
                    guard.execute(
                        "UPDATE memory_entity_links SET entity_fingerprint = ?1 WHERE memory_id = ?2 AND schema_name = ?3 AND table_name = ?4 AND column_name = ?5",
                        params![new_fp, id, schema, table, col_name],
                    ).map_err(|e| e.to_string())?;
                } else {
                    let new_fp = crate::ai::memory::entity_linker::compute_entity_fingerprint(
                        schema, table, None, "", false,
                    );
                    guard.execute(
                        "UPDATE memory_entity_links SET entity_fingerprint = ?1 WHERE memory_id = ?2 AND schema_name = ?3 AND table_name = ?4 AND column_name = ''",
                        params![new_fp, id, schema, table],
                    ).map_err(|e| e.to_string())?;
                }
            }
        }

        let now = chrono::Utc::now().timestamp();
        guard.execute(
            "UPDATE memories SET status = 'ACTIVE', tombstone = 0, tombstoned_at = NULL, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        ).map_err(|e| e.to_string())?;

        Ok(())
    }

    pub async fn reinforce_memory_access(&self, id: &str) -> Result<(), String> {
        let guard = self.conn.lock().await;
        let now = chrono::Utc::now().timestamp();
        let mut stmt = guard
            .prepare(
                "SELECT last_accessed_at, stability_hours, importance FROM memories WHERE id = ?1",
            )
            .map_err(|e| e.to_string())?;

        let res = stmt.query_row(params![id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, f64>(1)? as f32,
                row.get::<_, f64>(2)? as f32,
            ))
        });

        if let Ok((last_accessed_at, stability_hours, importance)) = res {
            let elapsed = (now - last_accessed_at).max(0) as f32 / 3600.0;
            let retention = calculate_retention(elapsed, stability_hours);
            let new_stability = reinforce_stability(stability_hours, retention, importance);
            guard.execute(
                "UPDATE memories
                 SET access_count = access_count + 1, last_accessed_at = ?1, stability_hours = ?2, updated_at = ?1
                 WHERE id = ?3",
                params![now, new_stability, id],
            ).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    // ── Observations ──────────────────────────────────────────────────────────
    pub async fn record_observation(
        &self,
        obs: Observation,
    ) -> Result<ObservationOutcome, String> {
        let guard = self.conn.lock().await;
        let now = chrono::Utc::now().timestamp();

        let mut existing_stmt = guard
            .prepare("SELECT id, occurrence_count FROM memory_observations WHERE dedup_key = ?1")
            .map_err(|e| e.to_string())?;
        let existing: Option<(String, i64)> = existing_stmt
            .query_row(params![obs.dedup_key], |row| Ok((row.get(0)?, row.get(1)?)))
            .optional()
            .map_err(|e| e.to_string())?;

        if let Some((id, count)) = existing {
            let new_count = count + 1;
            guard
                .execute(
                    "UPDATE memory_observations
                     SET occurrence_count = ?1, updated_at = ?2
                     WHERE id = ?3",
                    params![new_count, now, id],
                )
                .map_err(|e| format!("failed to rollup observation: {e}"))?;
            return Ok(ObservationOutcome::RolledUp {
                id,
                occurrence_count: new_count,
            });
        }

        guard
            .execute(
                "INSERT INTO memory_observations (
                    id, connection_key, conversation_id, turn_id, kind, origin, signal,
                    signal_strength, occurrence_count, dedup_key, payload_json, status,
                    derived_memory_id, created_at, updated_at
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    obs.id,
                    obs.connection_key,
                    obs.conversation_id,
                    obs.turn_id,
                    obs.kind,
                    obs.origin.as_str(),
                    obs.signal,
                    obs.signal_strength,
                    obs.occurrence_count,
                    obs.dedup_key,
                    obs.payload_json,
                    obs.status,
                    obs.derived_memory_id,
                    obs.created_at,
                    obs.updated_at,
                ],
            )
            .map_err(|e| format!("failed to insert observation: {e}"))?;

        Ok(ObservationOutcome::Inserted { id: obs.id })
    }

    pub async fn list_observations(
        &self,
        connection_key: &str,
        status: &str,
    ) -> Result<Vec<Observation>, String> {
        let guard = self.conn.lock().await;
        let mut stmt = guard
            .prepare(
                "SELECT id, connection_key, conversation_id, turn_id, kind, origin, signal,
                        signal_strength, occurrence_count, dedup_key, payload_json, status,
                        derived_memory_id, created_at, updated_at
                 FROM memory_observations
                 WHERE connection_key = ?1 AND status = ?2
                 ORDER BY created_at DESC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![connection_key, status], |row| {
                Ok(Observation {
                    id: row.get(0)?,
                    connection_key: row.get(1)?,
                    conversation_id: row.get(2)?,
                    turn_id: row.get(3)?,
                    kind: row.get(4)?,
                    origin: Origin::from_str(&row.get::<_, String>(5)?),
                    signal: row.get(6)?,
                    signal_strength: row.get::<_, f64>(7)? as f32,
                    occurrence_count: row.get(8)?,
                    dedup_key: row.get(9)?,
                    payload_json: row.get(10)?,
                    status: row.get(11)?,
                    derived_memory_id: row.get(12)?,
                    created_at: row.get(13)?,
                    updated_at: row.get(14)?,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    pub async fn retrieve_hybrid_memories(
        &self,
        query: &str,
        connection_key: &str,
        query_embedding: Option<&[f32]>,
        graph: Option<&crate::ai::schema_graph::SchemaGraph>,
        reranker: Option<&crate::ai::rerank::Reranker>,
        active_memories: &[MemoryItem],
    ) -> Vec<MemoryItem> {
        let candidates = {
            let guard = self.conn.lock().await;
            crate::ai::memory::retrieval::score_hybrid_candidates(
                query,
                connection_key,
                query_embedding,
                graph,
                active_memories,
                &guard,
            )
        };
        crate::ai::memory::retrieval::finalize_hybrid_memories(query, reranker, candidates).await
    }

    // ── Golden Queries ────────────────────────────────────────────────────────
    pub async fn save_golden_query(&self, q: GoldenQuery) -> Result<(), String> {
        let guard = self.conn.lock().await;
        let tables_json = serde_json::to_string(&q.tables_used).unwrap_or_else(|_| "[]".into());
        let emb_blob = embedding_to_blob(&q.embedding);

        guard
            .execute(
                "INSERT INTO golden_queries (
                id, connection_id, schema_name, natural_prompt, sql_text, tables_used,
                verified, run_count, last_run_at, embedding_model, embedding_version,
                embedding_blob, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
            ON CONFLICT(id) DO UPDATE SET
                natural_prompt = excluded.natural_prompt,
                sql_text = excluded.sql_text,
                tables_used = excluded.tables_used,
                verified = excluded.verified,
                run_count = excluded.run_count,
                last_run_at = excluded.last_run_at,
                embedding_blob = excluded.embedding_blob",
                params![
                    q.id,
                    q.connection_id,
                    q.schema_name,
                    q.natural_prompt,
                    q.sql_text,
                    tables_json,
                    q.verified as i64,
                    q.run_count,
                    q.last_run_at,
                    q.embedding_model,
                    q.embedding_version,
                    emb_blob,
                    q.created_at,
                ],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_golden_queries(
        &self,
        connection_id: &str,
    ) -> Result<Vec<GoldenQuery>, String> {
        let guard = self.conn.lock().await;
        let mut stmt = guard
            .prepare(
                "SELECT id, connection_id, schema_name, natural_prompt, sql_text, tables_used,
                    verified, run_count, last_run_at, embedding_model, embedding_version,
                    embedding_blob, created_at
             FROM golden_queries
             WHERE connection_id = ?1
             ORDER BY last_run_at DESC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![connection_id], |row| {
                let tables_str: String = row.get(5)?;
                let tables: Vec<String> = serde_json::from_str(&tables_str).unwrap_or_default();
                let emb_bytes: Vec<u8> = row.get(11)?;
                Ok(GoldenQuery {
                    id: row.get(0)?,
                    connection_id: row.get(1)?,
                    schema_name: row.get(2)?,
                    natural_prompt: row.get(3)?,
                    sql_text: row.get(4)?,
                    tables_used: tables,
                    verified: row.get::<_, i64>(6)? != 0,
                    run_count: row.get(7)?,
                    last_run_at: row.get(8)?,
                    embedding_model: row.get(9)?,
                    embedding_version: row.get(10)?,
                    embedding: blob_to_embedding(&emb_bytes),
                    created_at: row.get(12)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(rows)
    }

    pub async fn delete_golden_query(&self, id: &str) -> Result<bool, String> {
        let guard = self.conn.lock().await;
        let affected = guard
            .execute("DELETE FROM golden_queries WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(affected > 0)
    }

    // ── Chat Conversations & Persistence ──────────────────────────────────────
    pub async fn save_conversation(&self, conv: ChatConversation) -> Result<(), String> {
        let guard = self.conn.lock().await;
        guard.execute(
            "INSERT INTO chat_conversations (id, connection_id, title, archived, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                title = excluded.title,
                archived = excluded.archived,
                updated_at = excluded.updated_at",
            params![conv.id, conv.connection_id, conv.title, conv.archived as i64, conv.created_at, conv.updated_at],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_conversations(
        &self,
        connection_id: Option<&str>,
    ) -> Result<Vec<ChatConversation>, String> {
        let guard = self.conn.lock().await;
        let (sql, params_vec): (String, Vec<rusqlite::types::Value>) = match connection_id {
            Some(cid) => (
                "SELECT id, connection_id, title, archived, created_at, updated_at FROM chat_conversations WHERE connection_id = ?1 ORDER BY updated_at DESC".into(),
                vec![cid.to_string().into()],
            ),
            None => (
                "SELECT id, connection_id, title, archived, created_at, updated_at FROM chat_conversations ORDER BY updated_at DESC".into(),
                vec![],
            ),
        };

        let mut stmt = guard.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params_vec.iter()), |row| {
                Ok(ChatConversation {
                    id: row.get(0)?,
                    connection_id: row.get(1)?,
                    title: row.get(2)?,
                    archived: row.get::<_, i64>(3)? != 0,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(rows)
    }

    pub async fn delete_conversation(&self, id: &str) -> Result<bool, String> {
        let guard = self.conn.lock().await;
        let affected = guard
            .execute("DELETE FROM chat_conversations WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(affected > 0)
    }

    pub async fn save_message(&self, msg: ChatMessage) -> Result<(), String> {
        let guard = self.conn.lock().await;
        guard.execute(
            "INSERT INTO chat_messages (id, conversation_id, role, content, session_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                content = excluded.content,
                session_json = excluded.session_json",
            params![msg.id, msg.conversation_id, msg.role, msg.content, msg.session_json, msg.created_at],
        ).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn list_messages(&self, conversation_id: &str) -> Result<Vec<ChatMessage>, String> {
        let guard = self.conn.lock().await;
        let mut stmt = guard
            .prepare(
                "SELECT id, conversation_id, role, content, session_json, created_at
             FROM chat_messages
             WHERE conversation_id = ?1
             ORDER BY created_at ASC",
            )
            .map_err(|e| e.to_string())?;

        let rows = stmt
            .query_map(params![conversation_id], |row| {
                Ok(ChatMessage {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    session_json: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;

        Ok(rows)
    }

    /// Access to inner rusqlite Connection for sync operations (e.g. in tests or retrieval)
    pub async fn with_connection<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        let guard = self.conn.lock().await;
        f(&guard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B-I7: manually saved / imported user rules previously carried a 7-day
    /// stability while low-authority tool deductions carried 30 days, so the
    /// user's own rules decayed ~4x faster. The two values are named constants
    /// now, and this invariant fails if they ever invert again.
    #[test]
    fn user_explicit_rules_must_not_decay_faster_than_tool_rules() {
        const {
            assert!(
                USER_EXPLICIT_STABILITY_HOURS > TOOL_RULE_STABILITY_HOURS,
                "user-explicit rules must outlive tool-derived rules"
            );
        }
        assert_eq!(
            USER_EXPLICIT_STABILITY_HOURS, 2160.0,
            "the audit mandates 90 days of stability for user-explicit rules"
        );
    }

    #[tokio::test]
    async fn test_memory_manager_crud_and_fts() {
        let mgr = MemoryManager::open_in_memory().unwrap();
        let item = MemoryItem {
            id: "mem_1".into(),
            connection_key: "conn_pg".into(),
            scope: MemoryScope::Connection,
            scope_key: "conn_pg".into(),
            category: MemoryCategory::Metric,
            key_phrase: "active_subscribers".into(),
            rule_text: "Users with status = 'active'".into(),
            sql_snippet: Some("status = 'active'".into()),
            importance: 0.8,
            stability_hours: 720.0,
            last_accessed_at: 1000,
            access_count: 1,
            source_trust: SourceTrust::UserExplicit,
            source_conv_id: None,
            source_turn_id: None,
            source_tool_id: None,
            status: MemoryStatus::Active,
            supersedes_id: None,
            valid_from: 1000,
            valid_until: None,
            learned_at: 1000,
            tombstone: false,
            tombstoned_at: None,
            doc_hash: compute_memory_doc_hash("Users with status = 'active'"),
            embedding_model: MEMORY_MODEL_NAME.into(),
            embedding_version: MEMORY_FORMAT_VERSION,
            embedding: vec![0.1; 384],
            created_at: 1000,
            updated_at: 1000,
            injection: InjectionClass::Retrieved,
            preference_key: None,
            origin: Origin::Agent,
            steps_json: None,
            merge_group_id: None,
            confirmed: false,
            confirmation_conv_id: None,
        };

        mgr.save_memory(item, &[]).await.unwrap();

        let list = mgr.list_memories("conn_pg", false).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "mem_1");

        // Reinforce access
        mgr.reinforce_memory_access("mem_1").await.unwrap();
        let updated = mgr.list_memories("conn_pg", false).await.unwrap();
        assert_eq!(updated[0].access_count, 2);

        // Delete memory
        let deleted = mgr.delete_memory("mem_1").await.unwrap();
        assert!(deleted);
        let empty = mgr.list_memories("conn_pg", false).await.unwrap();
        assert!(empty.is_empty());
    }

    fn test_memory_item(id: &str, rule_text: &str) -> MemoryItem {
        MemoryItem {
            id: id.into(),
            connection_key: "conn_pg".into(),
            scope: MemoryScope::Connection,
            scope_key: "conn_pg".into(),
            category: MemoryCategory::Metric,
            key_phrase: "k".into(),
            rule_text: rule_text.into(),
            sql_snippet: None,
            importance: 0.8,
            stability_hours: 720.0,
            last_accessed_at: 1000,
            access_count: 1,
            source_trust: SourceTrust::UserExplicit,
            source_conv_id: None,
            source_turn_id: None,
            source_tool_id: None,
            status: MemoryStatus::Active,
            supersedes_id: None,
            valid_from: 1000,
            valid_until: None,
            learned_at: 1000,
            tombstone: false,
            tombstoned_at: None,
            doc_hash: compute_memory_doc_hash(rule_text),
            embedding_model: MEMORY_MODEL_NAME.into(),
            embedding_version: MEMORY_FORMAT_VERSION,
            embedding: vec![0.0; 384],
            created_at: 1000,
            updated_at: 1000,
            injection: InjectionClass::Retrieved,
            preference_key: None,
            origin: Origin::Agent,
            steps_json: None,
            merge_group_id: None,
            confirmed: false,
            confirmation_conv_id: None,
        }
    }

    /// B-C4/β: a failure in the middle of `save_memory`'s multi-statement
    /// sequence must roll back the earlier `memories` INSERT and the entity
    /// links it wrote. Inject the fault at the FTS index write by swapping the
    /// virtual table for a lookalike whose INSERT rejects the indexed content.
    #[tokio::test]
    async fn test_save_memory_rolls_back_on_fts_failure() {
        let mgr = MemoryManager::open_in_memory().unwrap();

        mgr.with_connection(|conn| {
            conn.execute_batch(
                "DROP TABLE memories_fts;
                 CREATE TABLE memories_fts (
                     id      TEXT,
                     content TEXT NOT NULL CHECK (instr(content, 'boom') = 0)
                 );",
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
        .unwrap();

        let item = test_memory_item("mem_rollback", "boom");
        let links = vec![EntityRef {
            schema_name: "public".into(),
            table_name: "users".into(),
            column_name: None,
            data_type: String::new(),
            is_nullable: false,
            entity_fingerprint: compute_entity_fingerprint("public", "users", None, "", false),
        }];

        let err = mgr.save_memory(item, &links).await;
        assert!(err.is_err(), "FTS insert failure must surface as an error");

        let (memories, entity_links) = mgr
            .with_connection(|conn| {
                let memories: i64 = conn
                    .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
                    .map_err(|e| e.to_string())?;
                let entity_links: i64 = conn
                    .query_row("SELECT COUNT(*) FROM memory_entity_links", [], |r| r.get(0))
                    .map_err(|e| e.to_string())?;
                Ok((memories, entity_links))
            })
            .await
            .unwrap();

        assert_eq!(
            memories, 0,
            "memory row must roll back with the failed FTS insert"
        );
        assert_eq!(
            entity_links, 0,
            "entity links must roll back with the failed FTS insert"
        );
    }

    /// B-I1/ε: the connection must be configured with a non-zero busy timeout
    /// so concurrent writes wait instead of failing with `database is locked`.
    #[tokio::test]
    async fn test_busy_timeout_is_configured() {
        let mgr = MemoryManager::open_in_memory().unwrap();
        let timeout: i64 = mgr
            .with_connection(|conn| {
                conn.query_row("PRAGMA busy_timeout", [], |r| r.get(0))
                    .map_err(|e| e.to_string())
            })
            .await
            .unwrap();
        assert_eq!(
            timeout, 5000,
            "PRAGMA busy_timeout must be set on connection init"
        );
    }

    /// B-I8: a table-level link (`column_name: None`) is persisted as `''`
    /// (NOT NULL) and read back as `None` via the `NULLIF` boundary, so the
    /// drift/revalidation code still distinguishes table- from column-level
    /// links.
    #[tokio::test]
    async fn test_table_level_entity_link_round_trips_as_none() {
        let mgr = MemoryManager::open_in_memory().unwrap();
        let item = test_memory_item("mem_link", "users table rule");
        let links = vec![EntityRef {
            schema_name: "public".into(),
            table_name: "users".into(),
            column_name: None,
            data_type: String::new(),
            is_nullable: false,
            entity_fingerprint: compute_entity_fingerprint("public", "users", None, "", false),
        }];

        mgr.save_memory(item, &links).await.unwrap();

        let (stored, normalized) = mgr
            .with_connection(|conn| {
                let stored: String = conn
                    .query_row(
                        "SELECT column_name FROM memory_entity_links WHERE memory_id = 'mem_link'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                let normalized: Option<String> = conn
                    .query_row(
                        "SELECT NULLIF(column_name, '') FROM memory_entity_links WHERE memory_id = 'mem_link'",
                        [],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                Ok((stored, normalized))
            })
            .await
            .unwrap();

        assert_eq!(stored, "", "table-level links must be stored as ''");
        assert_eq!(
            normalized, None,
            "the read boundary must normalize '' back to None"
        );
    }

    #[test]
    fn test_v2_schema_migration_adds_columns_and_observations_table() {
        let conn = Connection::open_in_memory().unwrap();
        MemoryManager::init_tables(&conn).unwrap();

        // Verify v2 columns on memories table
        let mut stmt = conn.prepare("PRAGMA table_info(memories)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert!(cols.contains(&"injection".to_string()));
        assert!(cols.contains(&"preference_key".to_string()));
        assert!(cols.contains(&"origin".to_string()));
        assert!(cols.contains(&"steps_json".to_string()));
        assert!(cols.contains(&"merge_group_id".to_string()));
        assert!(cols.contains(&"confirmed".to_string()));
        assert!(cols.contains(&"confirmation_conv_id".to_string()));

        // Verify memory_observations table exists
        let mut obs_stmt = conn
            .prepare("PRAGMA table_info(memory_observations)")
            .unwrap();
        let obs_cols: Vec<String> = obs_stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert!(obs_cols.contains(&"dedup_key".to_string()));
        assert!(obs_cols.contains(&"signal_strength".to_string()));
        assert!(obs_cols.contains(&"occurrence_count".to_string()));
    }

    /// B-I8: an existing database has the old nullable `column_name` in the
    /// `memory_entity_links` primary key. `CREATE TABLE IF NOT EXISTS` will not
    /// alter it, so `init_tables` must detect and rebuild it in place,
    /// normalizing NULL (table-level link) to `''`.
    #[tokio::test]
    async fn test_entity_links_schema_migrates_nullable_column_to_not_null_default() {
        let mgr = MemoryManager::open_in_memory().unwrap();

        mgr.with_connection(|conn| {
            conn.execute_batch(
                "PRAGMA foreign_keys = OFF;
                 DROP TABLE memory_entity_links;
                 CREATE TABLE memory_entity_links (
                     memory_id          TEXT NOT NULL REFERENCES memories(id) ON DELETE CASCADE,
                     schema_name        TEXT NOT NULL,
                     table_name         TEXT NOT NULL,
                     column_name        TEXT,
                     entity_fingerprint TEXT NOT NULL,
                     PRIMARY KEY (memory_id, schema_name, table_name, column_name)
                 );
                 INSERT INTO memory_entity_links
                     (memory_id, schema_name, table_name, column_name, entity_fingerprint)
                 VALUES ('mem_old', 'public', 'users', NULL, 'fp_table');",
            )
            .map_err(|e| e.to_string())?;

            MemoryManager::init_tables(conn)?;

            let notnull: i64 = conn
                .query_row(
                    "SELECT \"notnull\" FROM pragma_table_info('memory_entity_links') WHERE name = 'column_name'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            assert_eq!(notnull, 1, "column_name must be NOT NULL after migration");

            let col: String = conn
                .query_row(
                    "SELECT column_name FROM memory_entity_links WHERE memory_id = 'mem_old'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            assert_eq!(col, "", "migrated table-level link must normalize NULL to ''");
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_record_observation_rolls_up_on_duplicate() {
        let mgr = MemoryManager::open_in_memory().unwrap();
        let obs1 = Observation::new(
            "conn-a".into(),
            Some("c1".into()),
            Some("t1".into()),
            "diff".into(),
            Origin::Agent,
            "editor_diff".into(),
            0.8,
            r#"{"predicate":"deleted_at IS NULL"}"#.into(),
        );
        let obs2 = Observation::new(
            "conn-a".into(),
            Some("c2".into()),
            Some("t2".into()),
            "diff".into(),
            Origin::Agent,
            "editor_diff".into(),
            0.8,
            r#"{"predicate":"deleted_at IS NULL"}"#.into(),
        );

        let outcome1 = mgr.record_observation(obs1).await.unwrap();
        match outcome1 {
            ObservationOutcome::Inserted { .. } => {}
            _ => panic!("first observation should be inserted"),
        }

        let outcome2 = mgr.record_observation(obs2).await.unwrap();
        match outcome2 {
            ObservationOutcome::RolledUp { occurrence_count, .. } => {
                assert_eq!(occurrence_count, 2);
            }
            _ => panic!("second identical observation should roll up"),
        }

        let pending = mgr.list_observations("conn-a", "open").await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].occurrence_count, 2);
    }
}
