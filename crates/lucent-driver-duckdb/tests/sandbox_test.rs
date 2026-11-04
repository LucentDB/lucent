use lucent_driver_duckdb::connection::DuckHandle;

#[test]
fn duckdb_sandbox_rejects_external_file_access_when_disabled() {
    let temp_dir = tempfile::tempdir().unwrap();
    let csv_path = temp_dir.path().join("canary.csv");
    std::fs::write(&csv_path, b"id,val\n1,foo\n2,bar\n").unwrap();

    let db_path = temp_dir.path().join("sandbox.duckdb");
    // Create the database file first so it can be opened read-only
    {
        let conn = duckdb::Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE init (x INT);").unwrap();
    }
    let db_str = db_path.to_str().unwrap();

    // 1. Open with external_access = false
    let sandboxed = DuckHandle::open_ext(db_str, true, false).expect("open sandboxed handle");

    let query = format!("SELECT count(*) FROM '{}'", csv_path.display());
    let err = sandboxed
        .with_conn(|conn| {
            conn.query_row(&query, [], |row| row.get::<_, i64>(0))
                .map_err(|e| e.to_string())
        })
        .unwrap_err();

    let err_str = err.to_string();
    assert!(
        err_str.contains("disabled by configuration"),
        "expected error to contain 'disabled by configuration', got: {err_str}"
    );

    // 2. Open with external_access = true (default) in a fresh file/handle
    let regular_db = temp_dir.path().join("regular.duckdb");
    {
        let conn = duckdb::Connection::open(&regular_db).unwrap();
        conn.execute_batch("CREATE TABLE init (x INT);").unwrap();
    }
    let regular = DuckHandle::open_ext(regular_db.to_str().unwrap(), true, true)
        .expect("open regular handle");

    let count = regular
        .with_conn(|conn| {
            conn.query_row(&query, [], |row| row.get::<_, i64>(0))
                .map_err(|e| e.to_string())
        })
        .expect("regular handle should allow reading canary CSV");

    assert_eq!(count, 2);
}

#[test]
fn duckdb_sandbox_rejects_external_file_access_in_memory() {
    let temp_dir = tempfile::tempdir().unwrap();
    let csv_path = temp_dir.path().join("canary.csv");
    std::fs::write(&csv_path, b"id,val\n1,foo\n2,bar\n").unwrap();

    let sandboxed =
        DuckHandle::open_ext(":memory:", false, false).expect("open in-memory sandboxed handle");

    let query = format!("SELECT count(*) FROM '{}'", csv_path.display());
    let err = sandboxed
        .with_conn(|conn| {
            conn.query_row(&query, [], |row| row.get::<_, i64>(0))
                .map_err(|e| e.to_string())
        })
        .unwrap_err();

    let err_str = err.to_string();
    assert!(
        err_str.contains("disabled by configuration"),
        "expected error to contain 'disabled by configuration', got: {err_str}"
    );
}
