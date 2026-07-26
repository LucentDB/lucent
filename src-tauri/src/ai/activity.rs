use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

const MAX_ENTRIES: usize = 500;

// ─── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiActivityEntry {
    pub id: String,
    pub tool_name: String,
    pub sql: Option<String>,
    pub execution_time_ms: u64,
    pub status: String,
    pub error: Option<String>,
    pub created_at: String,
}

impl AiActivityEntry {
    pub fn new(
        tool_name: String,
        sql: Option<String>,
        execution_time_ms: u64,
        status: String,
        error: Option<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            tool_name,
            sql,
            execution_time_ms,
            status,
            error,
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

// ─── File Path ──────────────────────────────────────────────────────────────

fn activity_file_path() -> PathBuf {
    let base = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lucent");
    std::fs::create_dir_all(&base).ok();
    base.join("ai_activity.jsonl")
}

// ─── Operations ─────────────────────────────────────────────────────────────

/// Append an AI activity entry with exclusive file lock.
pub fn append_entry(entry: AiActivityEntry) -> Result<(), String> {
    let path = activity_file_path();

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        // read-modify-write: file is read fully before being rewritten in place
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("activity file open: {e}"))?;

    fs2::FileExt::lock_exclusive(&file).map_err(|e| format!("activity file lock: {e}"))?;

    let mut content = String::new();
    file.read_to_string(&mut content)
        .map_err(|e| format!("activity file read: {e}"))?;

    let mut entries: Vec<AiActivityEntry> = content
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    entries.push(entry);

    // Prune to capacity
    while entries.len() > MAX_ENTRIES {
        entries.remove(0);
    }

    // Rewrite
    file.set_len(0)
        .map_err(|e| format!("activity file truncate: {e}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("activity file seek: {e}"))?;

    let mut buf = String::new();
    for e in &entries {
        buf.push_str(&serde_json::to_string(e).map_err(|e| format!("serialize: {e}"))?);
        buf.push('\n');
    }
    file.write_all(buf.as_bytes())
        .map_err(|e| format!("activity file write: {e}"))?;
    file.flush()
        .map_err(|e| format!("activity file flush: {e}"))?;

    Ok(())
}
