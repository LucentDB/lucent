use serde::{Deserialize, Serialize};

use crate::notebook::types::{
    AiCellState, CellKind, CellModel, CellOutput, CellStatus, NotebookMetadata,
};

pub const LUCENT_FORMAT_VERSION: u8 = 2;

/// Document state only. Session state — status, error, stale_since, duration_ms —
/// is deliberately absent: it describes a process that has exited, so persisting
/// it stores a lie. A separate struct makes that boundary a compile-time
/// guarantee rather than a convention someone can quietly break.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookFileCell {
    pub id: String,
    pub kind: CellKind,
    pub source: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub execution_order: Option<u32>,
    #[serde(default)]
    pub outputs: Option<CellOutput>,
    #[serde(default)]
    pub ai_state: Option<AiCellState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookFileV2 {
    pub version: u8,
    pub metadata: NotebookMetadata,
    pub cells: Vec<NotebookFileCell>,
}

pub fn to_file_cell(cell: &CellModel) -> NotebookFileCell {
    NotebookFileCell {
        id: cell.id.clone(),
        kind: cell.kind.clone(),
        source: cell.source.clone(),
        alias: cell.alias.clone(),
        collapsed: cell.collapsed,
        execution_order: cell.execution_order,
        outputs: cell.outputs.clone(),
        ai_state: cell.ai_state.clone(),
    }
}

pub fn from_file_cell(fc: NotebookFileCell) -> CellModel {
    // Status is derived, never read from disk: a cell with an output was run, one
    // without was not. A persisted "running" or "error" is meaningless on reload.
    let status = if fc.outputs.is_some() {
        CellStatus::Ok
    } else {
        CellStatus::Pending
    };
    CellModel {
        id: fc.id,
        kind: fc.kind,
        source: fc.source,
        alias: fc.alias,
        collapsed: fc.collapsed,
        outputs: fc.outputs,
        status,
        execution_order: fc.execution_order,
        duration_ms: None,
        error: None,
        stale_since: None,
        ai_state: fc.ai_state,
    }
}

pub fn parse(json: &str) -> Result<NotebookFileV2, String> {
    let probe: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("invalid notebook file: {e}"))?;
    let version = probe.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
    if version != LUCENT_FORMAT_VERSION as u64 {
        return Err(format!(
            "unsupported notebook format version {version} — this build of Lucent reads version {LUCENT_FORMAT_VERSION} only"
        ));
    }
    serde_json::from_str(json).map_err(|e| format!("invalid notebook file: {e}"))
}

pub fn to_json(metadata: &NotebookMetadata, cells: &[CellModel]) -> Result<String, String> {
    let file = NotebookFileV2 {
        version: LUCENT_FORMAT_VERSION,
        metadata: metadata.clone(),
        cells: cells.iter().map(to_file_cell).collect(),
    };
    serde_json::to_string_pretty(&file).map_err(|e| e.to_string())
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::notebook::types::*;

    fn ok_cell_with_output() -> CellModel {
        CellModel {
            id: "a1b2c3d4".into(),
            kind: CellKind::Sql,
            source: "SELECT 1".into(),
            alias: None,
            collapsed: false,
            outputs: Some(CellOutput::Table(TableOutput {
                columns: vec![],
                rows: vec![vec![serde_json::json!(1)]],
                total_count: Some(42),
                is_truncated: false,
                page_size: 10,
                is_wrappable: true,
                rows_affected: None,
            })),
            status: CellStatus::Running,
            execution_order: Some(3),
            duration_ms: Some(820),
            error: Some(CellError::ConnectionLost {
                message: "boom".into(),
            }),
            stale_since: Some(1234567890),
            ai_state: None,
        }
    }

    #[test]
    fn file_cell_omits_all_transient_fields() {
        let json = serde_json::to_string(&to_file_cell(&ok_cell_with_output())).unwrap();
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
        assert!(json.contains("executionOrder"), "got {json}");
        assert!(json.contains("SELECT 1"), "got {json}");
    }

    #[test]
    fn round_trip_preserves_document_fields() {
        let original = ok_cell_with_output();
        let restored = from_file_cell(to_file_cell(&original));
        assert_eq!(restored.id, original.id);
        assert_eq!(restored.source, original.source);
        assert_eq!(restored.execution_order, Some(3));
        assert!(!restored.collapsed);
    }

    #[test]
    fn load_derives_ok_status_when_output_present() {
        let restored = from_file_cell(to_file_cell(&ok_cell_with_output()));
        assert_eq!(restored.status, CellStatus::Ok);
        assert!(restored.error.is_none());
        assert!(restored.stale_since.is_none());
        assert!(restored.duration_ms.is_none());
    }

    #[test]
    fn load_derives_pending_status_when_no_output() {
        let mut cell = ok_cell_with_output();
        cell.outputs = None;
        let restored = from_file_cell(to_file_cell(&cell));
        assert_eq!(restored.status, CellStatus::Pending);
    }

    #[test]
    fn parse_rejects_version_one() {
        let v1 = r#"{"version":1,"metadata":{"connection_id":null,"connection_name":null,
            "connection_host":null,"database":null,"created_at":"","updated_at":"",
            "lucent_version":"0.1.0"},"cells":[]}"#;
        let err = parse(v1).unwrap_err();
        assert!(err.contains("version"), "got {err}");
    }

    #[test]
    fn parse_accepts_version_two() {
        let v2 = r#"{"version":2,"metadata":{"connectionId":null,"connectionName":null,
            "connectionHost":null,"database":null,"createdAt":"","updatedAt":"",
            "lucentVersion":"0.1.0"},"cells":[]}"#;
        let parsed = parse(v2).unwrap();
        assert_eq!(parsed.version, 2);
        assert!(parsed.cells.is_empty());
    }

    /// Discriminating: a version-1 file, or any version that isn't exactly the
    /// current format, must be rejected outright rather than silently coerced
    /// (e.g. by `unwrap_or(2)` or similar). This would fail if `parse` fell back
    /// to accepting whatever version is present.
    #[test]
    fn parse_rejects_arbitrary_non_two_versions() {
        for bogus in [0u64, 1, 3, 999] {
            let json = format!(
                r#"{{"version":{bogus},"metadata":{{"connectionId":null,"connectionName":null,
                "connectionHost":null,"database":null,"createdAt":"","updatedAt":"",
                "lucentVersion":"0.1.0"}},"cells":[]}}"#
            );
            let err = parse(&json).unwrap_err();
            assert!(
                err.contains(&bogus.to_string()),
                "error should mention the rejected version {bogus}: got {err}"
            );
        }
    }
}
