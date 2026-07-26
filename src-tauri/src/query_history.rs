use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

const MAX_ENTRIES: usize = 1000;

// ─── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryHistoryEntry {
    pub id: String,
    pub connection_id: String,
    pub connection_name: String,
    pub database: String,
    pub sql: String,
    pub duration_ms: u64,
    pub row_count: Option<u64>,
    pub status: String, // "success" | "error"
    pub error: Option<String>,
    pub executed_at: String,
    pub favorite: bool,
}

impl QueryHistoryEntry {
    #[allow(clippy::too_many_arguments)] // constructor — arg count is inherent
    pub fn new(
        connection_id: String,
        connection_name: String,
        database: String,
        sql: String,
        duration_ms: u64,
        row_count: Option<u64>,
        status: String,
        error: Option<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            connection_id,
            connection_name,
            database,
            sql,
            duration_ms,
            row_count,
            status,
            error,
            executed_at: chrono::Utc::now().to_rfc3339(),
            favorite: false,
        }
    }
}

// ─── File Path ──────────────────────────────────────────────────────────────

fn history_file_path() -> PathBuf {
    #[cfg(test)]
    {
        let override_path = crate::connections::TEST_CONFIG_DIR.with(|cell| cell.borrow().clone());
        if let Some(dir) = override_path {
            let path = dir.join("lucent");
            std::fs::create_dir_all(&path).ok();
            return path.join("query_history.jsonl");
        }
    }
    if let Ok(dir) = std::env::var("LUCENT_CONFIG_DIR") {
        let path = PathBuf::from(dir).join("lucent");
        std::fs::create_dir_all(&path).ok();
        return path.join("query_history.jsonl");
    }
    let base = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lucent");
    std::fs::create_dir_all(&base).ok();
    base.join("query_history.jsonl")
}

// ─── Core Operations ────────────────────────────────────────────────────────

/// Append one entry with exclusive file lock. Deduplicates consecutive
/// identical queries (same connection_id + database + sql).
pub fn append_entry(entry: QueryHistoryEntry) -> Result<(), String> {
    let path = history_file_path();

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        // read-modify-write: file is read fully before being rewritten in place
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("history file open: {e}"))?;

    // Exclusive lock before touching the file
    fs2::FileExt::lock_exclusive(&file).map_err(|e| format!("history file lock: {e}"))?;

    // Read existing entries
    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|e| format!("history file read: {e}"))?;

    let mut entries: Vec<QueryHistoryEntry> = content
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    // Dedup: if newest entry matches, update instead of append
    if let Some(last) = entries.last() {
        if last.connection_id == entry.connection_id
            && last.database == entry.database
            && last.sql == entry.sql
        {
            if let Some(last_mut) = entries.last_mut() {
                last_mut.executed_at = entry.executed_at;
                last_mut.duration_ms = entry.duration_ms;
                last_mut.row_count = entry.row_count;
                last_mut.status = entry.status;
                last_mut.error = entry.error;
            }
        } else {
            entries.push(entry);
        }
    } else {
        entries.push(entry);
    }

    // Prune to capacity
    while entries.len() > MAX_ENTRIES {
        entries.remove(0);
    }

    // Rewrite file
    file.set_len(0)
        .map_err(|e| format!("history file truncate: {e}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("history file seek: {e}"))?;

    let mut buf = String::new();
    for e in &entries {
        buf.push_str(&serde_json::to_string(e).map_err(|e| format!("serialize: {e}"))?);
        buf.push('\n');
    }
    file.write_all(buf.as_bytes())
        .map_err(|e| format!("history file write: {e}"))?;
    file.flush()
        .map_err(|e| format!("history file flush: {e}"))?;

    // Lock released when file is dropped
    Ok(())
}

/// Read all entries, newest first.
pub fn read_all_entries() -> Vec<QueryHistoryEntry> {
    let path = history_file_path();
    if !path.exists() {
        return vec![];
    }
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let mut entries: Vec<QueryHistoryEntry> = content
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    entries.reverse(); // newest first
    entries
}

/// Search with optional filters.
pub fn search_entries(
    search: Option<&str>,
    connection_id: Option<&str>,
    favorite_only: bool,
) -> Vec<QueryHistoryEntry> {
    read_all_entries()
        .into_iter()
        .filter(|e| {
            if favorite_only && !e.favorite {
                return false;
            }
            if let Some(conn_id) = connection_id {
                if e.connection_id != conn_id {
                    return false;
                }
            }
            if let Some(q) = search {
                if !e.sql.to_lowercase().contains(&q.to_lowercase()) {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Toggle the favorite flag on an entry.
pub fn toggle_favorite(id: &str) -> Result<(), String> {
    let entries = read_all_entries();
    let mut entries_reversed = entries;
    entries_reversed.reverse(); // need file order for rewrite

    if let Some(e) = entries_reversed.iter_mut().find(|e| e.id == id) {
        e.favorite = !e.favorite;
    }
    rewrite_all(&entries_reversed)
}

/// Delete a single entry by ID.
pub fn delete_entry(id: &str) -> Result<(), String> {
    let entries = read_all_entries();
    let mut entries_reversed = entries;
    entries_reversed.reverse();
    entries_reversed.retain(|e| e.id != id);
    rewrite_all(&entries_reversed)
}

/// Clear all history.
pub fn clear_history() -> Result<(), String> {
    let path = history_file_path();
    if path.exists() {
        // Open with write and lock for safety
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        fs2::FileExt::lock_exclusive(&file).map_err(|e| e.to_string())?;
        file.set_len(0).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Date-group label for an ISO 8601 timestamp.
pub fn date_group(executed_at: &str) -> String {
    let executed = chrono::DateTime::parse_from_rfc3339(executed_at).ok();
    let now = chrono::Utc::now();
    match executed {
        Some(ts) => {
            let days = (now - ts.with_timezone(&chrono::Utc)).num_days();
            match days {
                0 => "Today".into(),
                1 => "Yesterday".into(),
                2..=6 => "This Week".into(),
                7..=13 => "Last Week".into(),
                _ => format!("{} weeks ago", days / 7),
            }
        }
        None => "Unknown".into(),
    }
}

// ─── Internal Helpers ───────────────────────────────────────────────────────

fn rewrite_all(entries: &[QueryHistoryEntry]) -> Result<(), String> {
    let path = history_file_path();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    fs2::FileExt::lock_exclusive(&file).map_err(|e| e.to_string())?;

    let mut buf = String::new();
    for e in entries {
        buf.push_str(&serde_json::to_string(e).map_err(|e| e.to_string())?);
        buf.push('\n');
    }
    file.write_all(buf.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tests/query_history_test.rs"]
mod query_history_tests;
