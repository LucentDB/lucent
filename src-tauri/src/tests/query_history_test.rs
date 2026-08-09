use super::super::query_history::*;

/// Helper: sets test config dir and returns TempDir
fn with_temp_dir() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::TempDir::new().unwrap();
    let config_path = dir.path().join("lucent");
    std::fs::create_dir_all(&config_path).unwrap();
    crate::connections::TEST_CONFIG_DIR
        .with(|cell| *cell.borrow_mut() = Some(dir.path().to_path_buf()));
    (dir, config_path)
}

#[test]
fn test_append_and_read() {
    let (_dir, _) = with_temp_dir();
    let entry = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test DB".into(),
        "postgres".into(),
        "SELECT 1".into(),
        12,
        Some(1),
        "success".into(),
        None,
    );
    append_entry(entry).unwrap();

    let entries = read_all_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].sql, "SELECT 1");
    assert_eq!(entries[0].status, "success");
}

#[test]
fn test_dedup_consecutive_identical() {
    let (_dir, _) = with_temp_dir();

    let entry1 = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test".into(),
        "postgres".into(),
        "SELECT 1".into(),
        10,
        Some(1),
        "success".into(),
        None,
    );
    append_entry(entry1).unwrap();

    let entry2 = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test".into(),
        "postgres".into(),
        "SELECT 1".into(),
        20,
        Some(1),
        "success".into(),
        None,
    );
    append_entry(entry2).unwrap();

    let entries = read_all_entries();
    assert_eq!(
        entries.len(),
        1,
        "should deduplicate consecutive identical queries"
    );
    assert_eq!(entries[0].duration_ms, 20, "should update duration");
}

#[test]
fn test_different_queries_not_deduplicated() {
    let (_dir, _) = with_temp_dir();

    let e1 = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test".into(),
        "postgres".into(),
        "SELECT 1".into(),
        10,
        Some(1),
        "success".into(),
        None,
    );
    let e2 = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test".into(),
        "postgres".into(),
        "SELECT 2".into(),
        10,
        Some(1),
        "success".into(),
        None,
    );
    append_entry(e1).unwrap();
    append_entry(e2).unwrap();

    let entries = read_all_entries();
    assert_eq!(entries.len(), 2);
}

#[test]
fn test_rolling_capacity() {
    let (_dir, _) = with_temp_dir();

    // Add MAX_ENTRIES + 10 entries
    for i in 0..MAX_ENTRIES + 10 {
        let e = QueryHistoryEntry::new(
            "conn-1".into(),
            "Test".into(),
            "postgres".into(),
            format!("SELECT {i}"),
            1,
            Some(1),
            "success".into(),
            None,
        );
        append_entry(e).unwrap();
    }

    let entries = read_all_entries();
    assert_eq!(entries.len(), MAX_ENTRIES);
    // Oldest entries should be pruned
    assert_eq!(entries.last().unwrap().sql, "SELECT 10");
}

#[test]
fn test_search() {
    let (_dir, _) = with_temp_dir();

    let e1 = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test".into(),
        "postgres".into(),
        "SELECT * FROM users".into(),
        10,
        Some(5),
        "success".into(),
        None,
    );
    let e2 = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test".into(),
        "postgres".into(),
        "UPDATE orders SET status = 'shipped'".into(),
        200,
        None,
        "error".into(),
        Some("timeout".into()),
    );
    append_entry(e1).unwrap();
    append_entry(e2).unwrap();

    // Search by SQL substring
    let results = search_entries(Some("users"), None, false);
    assert_eq!(results.len(), 1);
    assert!(results[0].sql.contains("users"));

    // Search by error status
    let results = search_entries(Some("UPDATE"), None, false);
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].status, "error");

    // Filter by connection
    let results = search_entries(None, Some("conn-1"), false);
    assert_eq!(results.len(), 2);

    // No match
    let results = search_entries(Some("nonexistent"), None, false);
    assert!(results.is_empty());
}

#[test]
fn test_favorite() {
    let (_dir, _) = with_temp_dir();

    let e = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test".into(),
        "postgres".into(),
        "SELECT 1".into(),
        5,
        Some(1),
        "success".into(),
        None,
    );
    let id = e.id.clone();
    append_entry(e).unwrap();

    // Toggle favorite
    toggle_favorite(&id).unwrap();
    let results = search_entries(None, None, true);
    assert_eq!(results.len(), 1);

    // Toggle back
    toggle_favorite(&id).unwrap();
    let results = search_entries(None, None, true);
    assert!(results.is_empty());
}

#[test]
fn test_delete_entry() {
    let (_dir, _) = with_temp_dir();

    let e1 = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test".into(),
        "postgres".into(),
        "SELECT 1".into(),
        5,
        Some(1),
        "success".into(),
        None,
    );
    let e2 = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test".into(),
        "postgres".into(),
        "SELECT 2".into(),
        5,
        Some(1),
        "success".into(),
        None,
    );
    let id1 = e1.id.clone();
    append_entry(e1).unwrap();
    append_entry(e2).unwrap();

    delete_entry(&id1).unwrap();
    let entries = read_all_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].sql, "SELECT 2");
}

#[test]
fn test_clear_history() {
    let (_dir, _) = with_temp_dir();

    let e = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test".into(),
        "postgres".into(),
        "SELECT 1".into(),
        5,
        Some(1),
        "success".into(),
        None,
    );
    append_entry(e).unwrap();
    clear_history().unwrap();

    let entries = read_all_entries();
    assert!(entries.is_empty());
}

#[test]
fn concurrent_readers_never_observe_torn_writes() {
    let (dir, _) = with_temp_dir();

    // Seed the file so a reader has a baseline to dip below.
    for i in 0..100 {
        let e = QueryHistoryEntry::new(
            "conn-1".into(),
            "Test".into(),
            "postgres".into(),
            format!("SELECT {i}"),
            1,
            Some(1),
            "success".into(),
            None,
        );
        append_entry(e).unwrap();
    }

    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader_stop = stop.clone();
    let dir_for_reader = dir.path().to_path_buf();
    let reader = std::thread::spawn(move || {
        // TEST_CONFIG_DIR is thread_local — the reader thread must point at
        // the same temp dir or it would read the real user history.
        crate::connections::TEST_CONFIG_DIR
            .with(|cell| *cell.borrow_mut() = Some(dir_for_reader.clone()));
        let mut min_seen = usize::MAX;
        while !reader_stop.load(std::sync::atomic::Ordering::Relaxed) {
            min_seen = min_seen.min(read_all_entries().len());
        }
        min_seen
    });

    // Keep rewriting while the reader spins — pre-fix, an unsynchronized
    // reader can catch the file mid-rewrite (set_len(0) → partial write).
    for i in 100..400 {
        let e = QueryHistoryEntry::new(
            "conn-1".into(),
            "Test".into(),
            "postgres".into(),
            format!("SELECT {i}"),
            1,
            Some(1),
            "success".into(),
            None,
        );
        append_entry(e).unwrap();
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);

    let min_seen = reader.join().unwrap();
    assert!(
        min_seen >= 100,
        "reader must never observe a torn/empty history file (saw {min_seen})"
    );
}

#[test]
fn test_date_group() {
    assert_eq!(date_group(&chrono::Utc::now().to_rfc3339()), "Today");
    // Yesterday
    let yesterday = (chrono::Utc::now() - chrono::TimeDelta::days(1)).to_rfc3339();
    assert_eq!(date_group(&yesterday), "Yesterday");
    // This week
    let three_days = (chrono::Utc::now() - chrono::TimeDelta::days(3)).to_rfc3339();
    assert_eq!(date_group(&three_days), "This Week");
    // Last week
    let ten_days = (chrono::Utc::now() - chrono::TimeDelta::days(10)).to_rfc3339();
    assert_eq!(date_group(&ten_days), "Last Week");
    // Older
    let thirty_days = (chrono::Utc::now() - chrono::TimeDelta::days(30)).to_rfc3339();
    assert_eq!(date_group(&thirty_days), "4 weeks ago");
}

#[test]
fn test_empty_history_file_returns_empty() {
    // Isolate to a temp dir; setting the override to None reads the real user
    // config dir, which may hold real history on a machine that has used the app.
    let (_dir, _path) = with_temp_dir();
    let entries = read_all_entries();
    assert!(entries.is_empty());
}

#[test]
fn test_entry_serialization_camelcase() {
    let e = QueryHistoryEntry::new(
        "conn-1".into(),
        "Test".into(),
        "postgres".into(),
        "SELECT 1".into(),
        5,
        Some(1),
        "success".into(),
        None,
    );
    let json = serde_json::to_value(&e).unwrap();
    assert!(json.get("connectionId").is_some());
    assert!(json.get("connectionName").is_some());
    assert!(json.get("durationMs").is_some());
    assert!(json.get("rowCount").is_some());
    assert!(json.get("executedAt").is_some());
}
