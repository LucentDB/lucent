//! Content-addressable vector cache + persisted schema graph.
//!
//! Keys are BLAKE3 hashes of a namespaced doc text:
//!   blake3("lucent-doc-v{N}:bge-small-en-v1.5:" || doc_text)
//! Sample values are deliberately NOT part of doc_text — they are
//! non-deterministic across harvests and would silently invalidate the cache
//! on data churn. Connection-level state (schema fingerprint + serialized
//! Tier-2 graph) lives in connection_schema_cache.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use lucent_protocol::ConnectionConfig;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

pub const DOC_TEXT_FORMAT_VERSION: u32 = 1;
pub const MODEL_NAME: &str = "bge-small-en-v1.5";
const DB_FILE_NAME: &str = "embeddings_v1.db";

#[derive(Debug, Clone)]
pub struct EmbeddingRow {
    pub doc_hash: String,
    pub model_name: String,
    pub doc_text: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct ConnectionCacheEntry {
    pub schema_hash: String,
    pub graph_blob: Vec<u8>,
    pub built_at_unix: i64,
}

#[derive(Clone)]
pub struct PersistentVectorCache {
    conn: Arc<Mutex<Connection>>,
}

impl PersistentVectorCache {
    pub fn open_default() -> Result<Self, String> {
        let mut dir = dirs::cache_dir().ok_or("no cache directory found")?;
        dir.push("lucent");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Self::open_at(dir.join(DB_FILE_NAME))
    }

    pub fn open_at(db_path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(&db_path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS global_doc_embeddings (
                 doc_hash       TEXT PRIMARY KEY,
                 model_name     TEXT NOT NULL,
                 doc_text       TEXT NOT NULL,
                 embedding_blob BLOB NOT NULL,
                 created_at     INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS connection_schema_cache (
                 connection_key TEXT PRIMARY KEY,
                 schema_hash    TEXT NOT NULL,
                 graph_blob     BLOB NOT NULL,
                 built_at_unix  INTEGER NOT NULL
             );",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// BLAKE3 of the namespaced doc text. The format version and model name in
    /// the prefix mean a template change or model swap invalidates old keys
    /// deliberately instead of silently reusing stale vectors.
    pub fn compute_doc_hash(doc_text: &str) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(format!("lucent-doc-v{DOC_TEXT_FORMAT_VERSION}:{MODEL_NAME}:").as_bytes());
        hasher.update(doc_text.as_bytes());
        hasher.finalize().to_hex().to_string()
    }

    /// One bulk IN-query, not N+1 lookups.
    pub async fn get_embeddings(
        &self,
        hashes: &[String],
    ) -> Result<HashMap<String, Vec<f32>>, String> {
        if hashes.is_empty() {
            return Ok(HashMap::new());
        }
        let placeholders: Vec<String> = (0..hashes.len()).map(|_| "?".to_string()).collect();
        let sql = format!(
            "SELECT doc_hash, embedding_blob FROM global_doc_embeddings WHERE doc_hash IN ({})",
            placeholders.join(",")
        );
        let hashes = hashes.to_vec();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            let mut stmt = guard.prepare(&sql).map_err(|e| e.to_string())?;
            let mut rows = stmt
                .query(rusqlite::params_from_iter(hashes.iter()))
                .map_err(|e| e.to_string())?;
            let mut out = HashMap::new();
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let hash: String = row.get(0).map_err(|e| e.to_string())?;
                let blob: Vec<u8> = row.get(1).map_err(|e| e.to_string())?;
                let floats: Vec<f32> = blob
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                out.insert(hash, floats);
            }
            Ok(out)
        })
        .await
        .map_err(|e| format!("cache read task panicked: {e}"))?
    }

    pub async fn put_embeddings(&self, rows: &[EmbeddingRow]) -> Result<(), String> {
        if rows.is_empty() {
            return Ok(());
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let rows = rows.to_vec();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            guard.execute_batch("BEGIN").map_err(|e| e.to_string())?;
            let write = || -> Result<(), String> {
                let mut stmt = guard
                    .prepare_cached(
                        "INSERT OR IGNORE INTO global_doc_embeddings
                         (doc_hash, model_name, doc_text, embedding_blob, created_at)
                         VALUES (?1, ?2, ?3, ?4, ?5)",
                    )
                    .map_err(|e| e.to_string())?;
                for row in &rows {
                    let mut blob = Vec::with_capacity(row.embedding.len() * 4);
                    for f in &row.embedding {
                        blob.extend_from_slice(&f.to_le_bytes());
                    }
                    stmt.execute(params![
                        row.doc_hash,
                        row.model_name,
                        row.doc_text,
                        blob,
                        now
                    ])
                    .map_err(|e| e.to_string())?;
                }
                Ok(())
            };
            let result = write();
            let _ = guard.execute_batch(if result.is_ok() { "COMMIT" } else { "ROLLBACK" });
            result
        })
        .await
        .map_err(|e| format!("cache write task panicked: {e}"))?
    }

    pub async fn get_connection_cache(
        &self,
        connection_key: &str,
    ) -> Result<Option<ConnectionCacheEntry>, String> {
        let key = connection_key.to_string();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            let mut stmt = guard
                .prepare_cached(
                    "SELECT schema_hash, graph_blob, built_at_unix
                     FROM connection_schema_cache WHERE connection_key = ?1",
                )
                .map_err(|e| e.to_string())?;
            let mut rows = stmt.query(params![key]).map_err(|e| e.to_string())?;
            match rows.next().map_err(|e| e.to_string())? {
                Some(row) => Ok(Some(ConnectionCacheEntry {
                    schema_hash: row.get(0).map_err(|e| e.to_string())?,
                    graph_blob: row.get(1).map_err(|e| e.to_string())?,
                    built_at_unix: row.get(2).map_err(|e| e.to_string())?,
                })),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| format!("cache read task panicked: {e}"))?
    }

    pub async fn put_connection_cache(
        &self,
        connection_key: &str,
        schema_hash: &str,
        graph_blob: &[u8],
    ) -> Result<(), String> {
        let key = connection_key.to_string();
        let hash = schema_hash.to_string();
        let blob = graph_blob.to_vec();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            guard
                .execute(
                    "INSERT OR REPLACE INTO connection_schema_cache
                     (connection_key, schema_hash, graph_blob, built_at_unix)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![key, hash, blob, now],
                )
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
        .map_err(|e| format!("cache write task panicked: {e}"))?
    }

    /// Delete one connection's persisted cache row. Used to drop a corrupt or
    /// format-stale graph blob so the next enrich falls through to re-index
    /// instead of failing forever on the same bad bytes. Best-effort: the
    /// caller tolerates a failed delete (a fresh put overwrites anyway).
    pub async fn delete_connection_cache(&self, connection_key: &str) -> Result<(), String> {
        let key = connection_key.to_string();
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn.blocking_lock();
            guard
                .execute(
                    "DELETE FROM connection_schema_cache WHERE connection_key = ?1",
                    params![key],
                )
                .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
        .map_err(|e| format!("cache delete task panicked: {e}"))?
    }
}

/// Canonical connection identity: user@host:port/database. Includes the user —
/// two users with different search_paths must not share a cache key.
pub fn connection_key_for(config: &ConnectionConfig) -> String {
    let host = config
        .params
        .get("host")
        .map(String::as_str)
        .unwrap_or("localhost");
    let port = config
        .params
        .get("port")
        .map(String::as_str)
        .unwrap_or("5432");
    let db = config
        .params
        .get("dbname")
        .or_else(|| config.params.get("database"))
        .map(String::as_str)
        .unwrap_or("");
    let user = config.params.get("user").map(String::as_str).unwrap_or("");
    let canonical = format!("{user}@{host}:{port}/{db}");
    format!("{:x}", Sha256::digest(canonical.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucent_protocol::ConnectionConfig;

    #[tokio::test]
    async fn doc_hash_is_deterministic_and_namespaced() {
        let a = PersistentVectorCache::compute_doc_hash("public.users.status TEXT");
        let b = PersistentVectorCache::compute_doc_hash("public.users.status TEXT");
        let c = PersistentVectorCache::compute_doc_hash("public.users.other TEXT");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64, "blake3 hex");
    }

    #[tokio::test]
    async fn embeddings_roundtrip_and_insert_or_ignore() {
        let dir =
            std::env::temp_dir().join(format!("lucent-cache-test-{}-emb", std::process::id()));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db")).unwrap();
        let hash = PersistentVectorCache::compute_doc_hash("public.users.status TEXT");
        cache
            .put_embeddings(&[EmbeddingRow {
                doc_hash: hash.clone(),
                model_name: MODEL_NAME.into(),
                doc_text: "public.users.status TEXT".into(),
                embedding: vec![1.0, 2.0, 3.0],
            }])
            .await
            .unwrap();
        // Duplicate insert must not clobber the original.
        cache
            .put_embeddings(&[EmbeddingRow {
                doc_hash: hash.clone(),
                model_name: MODEL_NAME.into(),
                doc_text: "public.users.status TEXT".into(),
                embedding: vec![9.0, 9.0, 9.0],
            }])
            .await
            .unwrap();
        let got = cache
            .get_embeddings(std::slice::from_ref(&hash))
            .await
            .unwrap();
        assert_eq!(got[&hash], vec![1.0, 2.0, 3.0]);
        // Missing hash returns nothing.
        let miss = PersistentVectorCache::compute_doc_hash("nope");
        assert!(cache.get_embeddings(&[miss]).await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn connection_cache_roundtrip_and_miss() {
        let dir =
            std::env::temp_dir().join(format!("lucent-cache-test-{}-conn", std::process::id()));
        let cache = PersistentVectorCache::open_at(dir.join("embeddings_v1.db")).unwrap();
        let key = connection_key_for(
            &ConnectionConfig::new("postgres")
                .with("host", "db.internal")
                .with("port", "5432")
                .with("dbname", "app")
                .with("user", "alice"),
        );
        assert!(cache.get_connection_cache(&key).await.unwrap().is_none());
        cache
            .put_connection_cache(&key, "abc123", &[1, 2, 3])
            .await
            .unwrap();
        let entry = cache.get_connection_cache(&key).await.unwrap().unwrap();
        assert_eq!(entry.schema_hash, "abc123");
        assert_eq!(entry.graph_blob, vec![1, 2, 3]);
        // A different user gets a different key.
        let other = connection_key_for(
            &ConnectionConfig::new("postgres")
                .with("host", "db.internal")
                .with("port", "5432")
                .with("dbname", "app")
                .with("user", "bob"),
        );
        assert_ne!(key, other);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
