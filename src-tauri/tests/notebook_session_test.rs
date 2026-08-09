use lucent_lib::notebook::file::{
    from_file_cell, parse, to_file_cell, to_json, NotebookFileCell, NotebookFileV2,
};
use lucent_lib::notebook::types::*;

fn metadata() -> NotebookMetadata {
    NotebookMetadata {
        connection_id: Some("uuid-123".into()),
        connection_name: Some("My DB".into()),
        connection_host: Some("localhost:5432".into()),
        database: Some("mydb".into()),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        lucent_version: "0.2.0".into(),
    }
}

fn sql_cell_with_output() -> CellModel {
    CellModel {
        id: "a1b2c3d4".into(),
        kind: CellKind::Sql,
        source: "SELECT 1".into(),
        alias: None,
        collapsed: false,
        outputs: Some(CellOutput::Table(TableOutput {
            columns: vec![],
            rows: vec![vec![serde_json::json!(1)]],
            total_count: Some(1),
            is_truncated: false,
            page_size: 10,
            is_wrappable: true,
            rows_affected: None,
        })),
        status: CellStatus::Running,
        execution_order: Some(1),
        duration_ms: Some(100),
        error: None,
        stale_since: None,
        ai_state: None,
    }
}

#[test]
fn test_v2_file_roundtrip() {
    let json = to_json(&metadata(), &[sql_cell_with_output()]).unwrap();
    let parsed: NotebookFileV2 = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.version, 2);
    assert_eq!(parsed.cells.len(), 1);
    assert_eq!(parsed.cells[0].id, "a1b2c3d4");
    // v2 files carry document fields only — transient session state is absent.
    assert!(!json.contains("status"), "status must not be persisted");
    assert!(
        !json.contains("duration_ms"),
        "duration_ms must not be persisted"
    );
    assert!(json.contains("executionOrder"), "executionOrder expected");
}

#[test]
fn test_v2_load_derives_status() {
    let restored = from_file_cell(to_file_cell(&sql_cell_with_output()));
    // Source status was Running; on load it must be derived from outputs.
    assert_eq!(restored.status, CellStatus::Ok);
    assert_eq!(restored.execution_order, Some(1));
    assert!(restored.duration_ms.is_none());
    assert!(restored.error.is_none());

    let mut no_output = sql_cell_with_output();
    no_output.outputs = None;
    let restored = from_file_cell(to_file_cell(&no_output));
    assert_eq!(restored.status, CellStatus::Pending);
}

#[test]
fn test_v2_rejects_version_one() {
    let v1 = r#"{"version":1,"metadata":{"connectionId":null,"connectionName":null,
        "connectionHost":null,"database":null,"createdAt":"","updatedAt":"",
        "lucentVersion":"0.1.0"},"cells":[]}"#;
    let err = parse(v1).unwrap_err();
    assert!(err.contains("version"), "got {err}");
}

#[test]
fn test_v2_accepts_empty_cells() {
    let v2 = r#"{"version":2,"metadata":{"connectionId":null,"connectionName":null,
        "connectionHost":null,"database":null,"createdAt":"","updatedAt":"",
        "lucentVersion":"0.1.0"},"cells":[]}"#;
    let parsed = parse(v2).unwrap();
    assert_eq!(parsed.version, 2);
    assert!(parsed.cells.is_empty());
}

#[test]
fn test_v2_file_cell_omits_transient_fields() {
    let fc: NotebookFileCell = to_file_cell(&sql_cell_with_output());
    let json = serde_json::to_string(&fc).unwrap();
    for field in [
        "status",
        "error",
        "staleSince",
        "stale_since",
        "durationMs",
        "duration_ms",
    ] {
        assert!(
            !json.contains(field),
            "{field} must not be persisted: {json}"
        );
    }
}

#[test]
fn test_session_create() {
    use lucent_lib::notebook::session::NotebookSession;
    use lucent_protocol::ConnectionId;
    use uuid::Uuid;

    let conn_id = ConnectionId(Uuid::new_v4());
    let session = NotebookSession::new("test-key".into(), conn_id, "testdb".into());
    assert_eq!(session.session_key, "test-key");
    assert_eq!(session.connection_id, conn_id);
    assert_eq!(session.database, "testdb");
    assert!(session.file_path.is_none());
}

#[test]
fn test_session_execution_counter_is_monotonic_and_resettable() {
    use lucent_lib::notebook::session::NotebookSession;
    use lucent_protocol::ConnectionId;
    use uuid::Uuid;

    let conn_id = ConnectionId(Uuid::new_v4());
    let mut session = NotebookSession::new("test-key".into(), conn_id, "testdb".into());
    // First allocation is 1, and it climbs regardless of cell state.
    assert_eq!(session.next_execution_order(), 1);
    assert_eq!(session.next_execution_order(), 2);
    assert_eq!(session.next_execution_order(), 3);
    session.reset_execution_counter();
    assert_eq!(session.next_execution_order(), 1);
}

#[test]
fn test_two_notebooks_separate_sessions() {
    use lucent_lib::notebook::session::NotebookSession;
    use lucent_protocol::ConnectionId;
    use uuid::Uuid;

    let conn_a = ConnectionId(Uuid::new_v4());
    let conn_b = ConnectionId(Uuid::new_v4());
    let session_a = NotebookSession::new("key-a".into(), conn_a, "db_a".into());
    let session_b = NotebookSession::new("key-b".into(), conn_b, "db_b".into());
    assert_ne!(session_a.connection_id, session_b.connection_id);
    assert_ne!(session_a.session_key, session_b.session_key);
    assert_ne!(session_a.database, session_b.database);
}
