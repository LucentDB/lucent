use super::super::commands::{write_approved_sql_file, AppState};

#[tokio::test]
async fn writes_content_to_an_approved_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("query.sql");
    let state = AppState::new();
    // The path must have been approved by a native dialog first (the
    // choose_save_path command inserts the canonicalized path).
    let approved = super::super::commands::canonicalize_allow_missing(&path).unwrap();
    state
        .approved_save_paths
        .lock()
        .await
        .insert(approved.clone());

    write_approved_sql_file(&state, &path.to_string_lossy(), "select 1;".into())
        .await
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(&approved).unwrap(),
        "select 1;",
        "file content must match exactly what was passed"
    );
}

#[tokio::test]
async fn rejects_a_path_that_was_not_dialog_approved() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("query.sql");
    let state = AppState::new(); // empty approved set

    let err = write_approved_sql_file(&state, &path.to_string_lossy(), "x".into())
        .await
        .unwrap_err();
    assert_eq!(
        err.kind, "PathError",
        "an unapproved path must surface a PathError"
    );
    assert!(
        err.message.contains("native save dialog"),
        "the error must say why the path was rejected, got: {err}"
    );
}
