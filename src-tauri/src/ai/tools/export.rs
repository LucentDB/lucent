use serde_json::json;
use std::path::{Path, PathBuf};

use super::execute::append_enforcement_notice;
use super::{AiToolContext, ToolError, ToolOutput};
use crate::ai::guard;
use crate::export::{ExportFormat, ExportOptions};

#[derive(Clone)]
pub struct ExportQuery {
    ctx: AiToolContext,
}

#[derive(Debug)]
struct ExportArgs {
    sql: String,
    path: PathBuf,
    format: ExportFormat,
    overwrite: bool,
    table_name: Option<String>,
}

/// Parse + validate the tool arguments. Pure; no filesystem or DB access.
fn parse_args(args: &serde_json::Value) -> Result<ExportArgs, ToolError> {
    let sql = args["sql"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("missing 'sql'".into()))?;
    let path_str = args["path"]
        .as_str()
        .ok_or_else(|| ToolError::InvalidArgs("missing 'path'".into()))?;
    let path = PathBuf::from(path_str);
    if !path.is_absolute() {
        return Err(ToolError::InvalidArgs(
            "path must be absolute (e.g. /Users/you/exports/out.csv)".into(),
        ));
    }
    // `..` cannot be resolved safely before the directories exist
    // (`~/new/../.ssh/x`: `new` does not exist, so canonicalize fails and the
    // deny-list cannot see where the write lands). An export path never needs it.
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ToolError::InvalidArgs(
            "path must not contain '..' — pass the plain absolute destination".into(),
        ));
    }
    let format = match args["format"].as_str() {
        Some("csv") => ExportFormat::Csv,
        Some("json") => ExportFormat::Json,
        Some("sql_insert") => ExportFormat::SqlInsert,
        Some(other) => {
            return Err(ToolError::InvalidArgs(format!(
                "unknown format '{other}'; use csv, json, or sql_insert"
            )))
        }
        // Inferred from the extension only when `format` is absent; an
        // explicit format always wins (spec §6.2).
        None => format_from_extension(&path),
    };
    let table_name = args["table_name"].as_str().map(str::to_string);
    if format == ExportFormat::SqlInsert && table_name.is_none() {
        return Err(ToolError::InvalidArgs(
            "format 'sql_insert' requires 'table_name' (the INSERT INTO target)".into(),
        ));
    }
    Ok(ExportArgs {
        sql: sql.to_string(),
        path,
        format,
        overwrite: args["overwrite"].as_bool().unwrap_or(false),
        table_name,
    })
}

fn format_from_extension(path: &Path) -> ExportFormat {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("json") => ExportFormat::Json,
        Some("sql") => ExportFormat::SqlInsert,
        _ => ExportFormat::Csv,
    }
}

/// Canonicalize the longest existing ancestor and re-append the rest, so a
/// not-yet-existing destination (`~/exports/new/out.csv`) still resolves
/// through any symlinks that DO exist. Mirrors `canonicalize_allow_missing`
/// but tolerates missing intermediate directories — the deny-list must see
/// `~/.ssh/new/sub/out.csv` as living under `~/.ssh` even when `new/sub`
/// does not exist yet (and must do so BEFORE `create_dir_all` runs).
fn resolve_for_deny_list(path: &Path) -> Option<PathBuf> {
    let mut suffix: Vec<std::ffi::OsString> = Vec::new();
    let mut probe = path;
    loop {
        if let Ok(canon) = probe.canonicalize() {
            let mut full = canon;
            for part in suffix.iter().rev() {
                full.push(part);
            }
            return Some(full);
        }
        let name = probe.file_name()?.to_os_string();
        suffix.push(name);
        probe = probe.parent()?;
    }
}

/// D1 deny-list directories, relative to the home directory.
const PROTECTED_DIRS: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".config/autostart",
    "Library/LaunchAgents",
    "Library/LaunchDaemons",
];

/// Shell startup files in the home directory: the spec's five, plus
/// `.zshenv` (read by EVERY zsh, interactive or not), `.zlogin` and
/// `.bash_login` — the same execution class. SPEC AMENDMENT pending: §6.4 D1
/// lists only the first five; drop the last three if the spec owner objects.
const RC_FILES: &[&str] = &[
    ".zshrc",
    ".zprofile",
    ".bashrc",
    ".bash_profile",
    ".profile",
    ".zshenv",
    ".zlogin",
    ".bash_login",
];

/// A filesystem object's identity: (device, inode). Every spelling of one
/// directory shares it — symlinks, macOS firmlinks (`/System/Volumes/Data/
/// Users/<me>` ≡ `/Users/<me>`; `realpath` does not fold that back), APFS case
/// and Unicode variants (`~/.ſſh` ≡ `~/.ssh`), hard links. String comparison
/// sees none of these (review 2026-10-06, reproduced on macOS). `metadata`
/// follows symlinks, so a link reports its target's identity.
#[cfg(unix)]
fn file_id(p: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| (m.dev(), m.ino()))
}

/// No inode identity in std on Windows (an informational milestone, A4): the
/// folded-name checks below still apply.
#[cfg(not(unix))]
fn file_id(_p: &Path) -> Option<(u64, u64)> {
    None
}

/// Name fold for components that do not exist yet, where identity cannot help.
/// Full Unicode case folding: APFS treats `ſ` (U+017F) as `s`, which
/// `eq_ignore_ascii_case` misses (reproduced).
fn fold(name: &std::ffi::OsStr) -> String {
    name.to_string_lossy().to_uppercase().to_lowercase()
}

/// The folded components of `p` below the home directory, or `None` when `p`
/// is not under home. Home is recognised by identity — any spelling of it —
/// and by folded string only where identity is unavailable.
fn components_below_home(
    p: &Path,
    home: &Path,
    home_id: Option<(u64, u64)>,
) -> Option<Vec<String>> {
    p.ancestors().find_map(|anc| {
        let is_home = match (home_id, file_id(anc)) {
            (Some(h), Some(a)) => h == a,
            _ => fold(anc.as_os_str()) == fold(home.as_os_str()),
        };
        is_home
            .then(|| p.strip_prefix(anc).ok())
            .flatten()
            .map(|rest| rest.components().map(|c| fold(c.as_os_str())).collect())
    })
}

/// D1 deny-list: execution-sensitive directories and shell rc files. Returns
/// the reason when `path` is protected. FAILS CLOSED: a path whose location
/// cannot be resolved is refused, never waved through.
fn protected_location_in(path: &Path, home: &Path) -> Option<String> {
    let Some(resolved) = resolve_for_deny_list(path) else {
        return Some(format!(
            "{} could not be resolved for the protected-location check; choose another path",
            path.display()
        ));
    };
    let refuse = |what: &str| {
        Some(format!(
            "{} is in a protected location ({what}) and cannot be exported to",
            path.display()
        ))
    };
    let home_id = file_id(home);
    let dir_ids: Vec<(&str, (u64, u64))> = PROTECTED_DIRS
        .iter()
        .filter_map(|rel| file_id(&home.join(rel)).map(|id| (*rel, id)))
        .collect();
    let rc_ids: Vec<(u64, u64)> = RC_FILES
        .iter()
        .filter_map(|rc| file_id(&home.join(rc)))
        .collect();

    // Both spellings. The literal one matters for a symlinked rc file (a
    // dotfile manager's `~/.zshrc` → `~/dotfiles/zsh/.zshrc`): `persist`
    // replaces the LINK, so where the link sits is what counts. The resolved
    // one catches symlinked parents anywhere above the destination.
    for candidate in [path, resolved.as_path()] {
        // 1. Identity, for whatever exists: the destination IS an rc file
        //    (through any link chain), or sits inside a protected directory.
        if file_id(candidate).is_some_and(|id| rc_ids.contains(&id)) {
            return refuse("a shell startup file");
        }
        for anc in candidate.ancestors() {
            if let Some(id) = file_id(anc) {
                if let Some((rel, _)) = dir_ids.iter().find(|(_, d)| *d == id) {
                    return refuse(rel);
                }
            }
        }
        // 2. Folded names, for what does not exist yet (no identity to compare).
        if let Some(rest) = components_below_home(candidate, home, home_id) {
            if rest.len() == 1 && RC_FILES.contains(&rest[0].as_str()) {
                return refuse("a shell startup file");
            }
            for rel in PROTECTED_DIRS {
                let parts: Vec<String> = rel.split('/').map(str::to_lowercase).collect();
                if rest.len() >= parts.len() && rest[..parts.len()] == parts[..] {
                    return refuse(rel);
                }
            }
        }
    }
    None
}

fn protected_location(path: &Path) -> Option<String> {
    match dirs::home_dir() {
        Some(home) => protected_location_in(path, &home),
        None => None,
    }
}

/// Atomic write: temp file in the destination directory, then persist.
/// `overwrite:false` uses `persist_noclobber` so a racing creation cannot be
/// silently replaced either. (`call` already refused an existing file with
/// `InvalidArgs` before running the query; the check here only covers a race.)
///
/// Windows note (A4, informational): `std::fs::rename` cannot replace an
/// existing destination on Windows; the overwrite path must switch to a
/// replace-capable rename when the Windows milestone lands.
pub(crate) fn write_export(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), String> {
    const EXISTS: &str = "file already exists; pass overwrite:true or choose another path";
    let parent = path.parent().ok_or("invalid destination path")?;
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    if !overwrite && path.exists() {
        return Err(EXISTS.into());
    }
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|e| format!("cannot create a temp file in {}: {e}", parent.display()))?;
    tmp.write_all(bytes)
        .map_err(|e| format!("write failed: {e}"))?;
    tmp.flush().map_err(|e| format!("write failed: {e}"))?;
    if overwrite {
        tmp.persist(path)
            .map_err(|e| format!("cannot replace {}: {}", path.display(), e.error))?;
        return Ok(());
    }
    match tmp.persist_noclobber(path) {
        Ok(_) => Ok(()),
        // exFAT/FAT and some network volumes support neither an exclusive
        // rename nor hard links, which `persist_noclobber` falls back to. Use
        // a plain rename after a fresh existence check — a narrow race, on
        // volumes that cannot do better. (Inferred from tempfile's source; not
        // tested on such a volume.)
        Err(e)
            if matches!(
                e.error.kind(),
                std::io::ErrorKind::Unsupported | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            if path.exists() {
                return Err(EXISTS.into());
            }
            e.file
                .persist(path)
                .map(|_| ())
                .map_err(|e2| format!("cannot write {}: {}", path.display(), e2.error))
        }
        Err(e) => Err(format!("cannot write {}: {}", path.display(), e.error)),
    }
}

fn with_commas(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        // `is_multiple_of`, not `% 3 == 0`: clippy's `manual_is_multiple_of`
        // fires on the pinned 1.98.0 toolchain (reproduced).
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", (bytes as f64 / 1024.0).round() as u64)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn format_label(f: &ExportFormat) -> &'static str {
    match f {
        ExportFormat::Csv => "CSV",
        ExportFormat::Json => "JSON",
        ExportFormat::SqlInsert => "SQL INSERT",
    }
}

/// The model-visible summary, before the enforcement notice and lint blocks
/// are appended (A1 requires those; they are appended in `call`).
fn export_summary(
    row_count: usize,
    path: &Path,
    format: ExportFormat,
    bytes: u64,
    truncated: bool,
    replaced: bool,
) -> String {
    let mut s = format!(
        "Exported {} rows to {} ({}, {})",
        with_commas(row_count),
        path.display(),
        format_label(&format),
        human_size(bytes)
    );
    if replaced {
        s.push_str("\nreplaced existing file");
    }
    if truncated {
        s.push_str(
            "\nthe file contains the first 10,000 rows; narrow the query (e.g. a WHERE \
             filter) to export the remainder in parts",
        );
    }
    s
}

/// A1/P6: the same weakened-guarantee notice and join-lint warnings
/// `run_readonly_query` appends. Pure, so the spec's A1 unit test needs no DB.
fn finalize_summary(
    base: String,
    readonly: lucent_protocol::ReadOnlyMode,
    lint_warnings: &[String],
) -> String {
    let mut content = append_enforcement_notice(base, readonly);
    for w in lint_warnings {
        content.push_str("\n\n");
        content.push_str(w);
    }
    content
}

/// `format_json` keys each record by column name, so a repeated name would
/// silently drop a column (P4). The formatter stays unchanged (D3); the tool
/// refuses and says how to fix it (P5).
fn duplicate_column_names(columns: &[crate::export::ColumnMeta]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut dups: Vec<String> = columns
        .iter()
        .filter(|c| !seen.insert(c.name.as_str()))
        .map(|c| c.name.clone())
        .collect();
    dups.sort();
    dups.dedup();
    dups
}

impl ExportQuery {
    pub fn new(ctx: AiToolContext) -> Self {
        Self { ctx }
    }

    pub fn description(&self) -> String {
        "Export a read-only query's full result set to a file (CSV / JSON / SQL INSERT) \
         at an absolute path. Returns the path, row count, and format."
            .into()
    }

    pub fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "sql": { "type": "string" },
                "path": { "type": "string", "description": "Absolute destination file path" },
                "format": {
                    "type": "string",
                    "enum": ["csv", "json", "sql_insert"],
                    "description": "Inferred from .csv/.json/.sql when omitted; defaults to csv. Explicit wins."
                },
                "overwrite": { "type": "boolean", "description": "Replace an existing file (default false)" },
                "table_name": { "type": "string", "description": "Required when format=sql_insert: the INSERT INTO target, quoted as ONE identifier — pass an unqualified name (\"orders\", not \"public.orders\")" }
            },
            "required": ["sql", "path"]
        })
    }

    pub async fn call(
        &self,
        args: serde_json::Value,
        _ctx: &AiToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let args = parse_args(&args)?;
        log::info!(
            "Tool 'export_query' called — path={:?}, format={:?}",
            args.path,
            args.format
        );

        // D1: refuse a protected destination before any DB work or directory
        // creation (create_dir_all must never create ~/.ssh).
        if let Some(reason) = protected_location(&args.path) {
            return Err(ToolError::InvalidArgs(reason));
        }
        // §6.5: "file exists, overwrite:false" is InvalidArgs — and it is
        // checked before the query runs, so a refusal never wastes a query.
        // (`write_export` re-checks; that only catches a race.)
        if !args.overwrite && args.path.exists() {
            return Err(ToolError::InvalidArgs(
                "file already exists; pass overwrite:true or choose another path".into(),
            ));
        }

        let capabilities = self
            .ctx
            .capabilities
            .as_ref()
            .ok_or(ToolError::NotConnected)?;
        guard::validate_readonly(&args.sql, capabilities.sql_dialect)
            .map_err(|e| ToolError::SqlValidation(e.to_string()))?;

        let conn_id = self.ctx.connection_id.ok_or(ToolError::NotConnected)?;
        let client = self
            .ctx
            .db
            .lock()
            .await
            .clone()
            .ok_or(ToolError::NotConnected)?;

        let timeout_ms = self.ctx.config.ai_query_timeout_secs.saturating_mul(1000);
        // Reachable by Stop: registered before the session, like
        // run_readonly_query.
        let in_flight = self.ctx.result_scope.register(conn_id);
        let readonly =
            crate::readonly::ReadOnlySession::begin(&client, conn_id, capabilities, timeout_ms)
                .await
                .map_err(ToolError::Execution)?;

        // The export runs with the client cap only — no apply_limit rewrite and
        // no row_limit (spec §6.3 step 3). `execute_bounded` arms the DuckDB
        // deadline (F16); the session above already holds the sequence lock.
        let query_result = super::bounded::execute_bounded(
            &client,
            conn_id,
            args.sql.trim(),
            capabilities,
            timeout_ms,
            Some(crate::client::HARD_ROW_CAP),
            Some(&in_flight),
        )
        .await;

        // Like run_readonly_query: a Stop that landed mid-statement needs one
        // more native cancel before close()'s ROLLBACK, and a marked query
        // that never sent writes nothing.
        let query_id = in_flight.query_id;
        let cancelled = in_flight.finish();
        if cancelled {
            let _ = client.cancel(conn_id, query_id).await;
        }
        // D1/C7: close the read-only scope on both paths, before inspecting.
        readonly.close().await;
        let result = match query_result {
            Err(_) if cancelled => {
                return Err(ToolError::Execution(
                    crate::ai::result_store::CANCELLED_BY_USER.into(),
                ))
            }
            Err(e) => return Err(ToolError::Execution(e)),
            Ok(r) => r,
        };

        let columns: Vec<crate::export::ColumnMeta> = result
            .columns
            .iter()
            .map(|c| crate::export::ColumnMeta {
                name: c.name.clone(),
                type_name: c.type_name.clone(),
            })
            .collect();
        if args.format == ExportFormat::Json {
            let dups = duplicate_column_names(&columns);
            if !dups.is_empty() {
                return Err(ToolError::InvalidArgs(format!(
                    "JSON export needs unique column names, but {} repeat{}; alias them \
                     (e.g. a.id AS a_id) and retry — nothing was written",
                    dups.join(", "),
                    if dups.len() == 1 { "s" } else { "" }
                )));
            }
        }
        let options = ExportOptions {
            format: args.format.clone(),
            table_name: args.table_name.clone(),
            ..Default::default()
        };
        let formatted = match args.format {
            ExportFormat::Csv => crate::export::format_csv(&columns, &result.rows, &options),
            ExportFormat::Json => crate::export::format_json(&columns, &result.rows, &options),
            ExportFormat::SqlInsert => crate::export::format_inserts(
                args.table_name.as_deref().expect("validated by parse_args"),
                &columns,
                &result.rows,
                &options,
            ),
        };

        let replaced = args.path.exists() && args.overwrite;
        let bytes = formatted.len() as u64;
        write_export(&args.path, formatted.as_bytes(), args.overwrite)
            .map_err(ToolError::Execution)?;

        let base = export_summary(
            result.rows.len(),
            &args.path,
            args.format.clone(),
            bytes,
            result.truncated,
            replaced,
        );
        // A1/P6: the model sees weakened guarantees and join warnings here
        // exactly as it does after `run_readonly_query`.
        let lint_warnings: Vec<String> = match self.ctx.schema_graph.lock().await.as_ref() {
            Some(graph) => {
                crate::ai::sql_lint::lint_sql(graph, &args.sql, capabilities.sql_dialect)
            }
            None => Vec::new(),
        };
        let content = finalize_summary(base, capabilities.readonly, &lint_warnings);

        Ok(ToolOutput::Text { content })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(v: serde_json::Value) -> serde_json::Value {
        v
    }

    #[test]
    fn missing_sql_or_path_is_invalid_args() {
        assert!(matches!(
            parse_args(&args(json!({"path": "/tmp/x.csv"}))),
            Err(ToolError::InvalidArgs(_))
        ));
        assert!(matches!(
            parse_args(&args(json!({"sql": "SELECT 1"}))),
            Err(ToolError::InvalidArgs(_))
        ));
    }

    #[test]
    fn relative_path_is_rejected() {
        let e = parse_args(&args(json!({"sql": "SELECT 1", "path": "out.csv"}))).unwrap_err();
        assert!(e.to_string().contains("absolute"), "{e}");
    }

    #[test]
    fn format_is_inferred_from_the_extension() {
        assert_eq!(
            parse_args(&args(json!({"sql": "SELECT 1", "path": "/tmp/x.json"})))
                .unwrap()
                .format,
            ExportFormat::Json
        );
        assert_eq!(
            parse_args(&args(
                json!({"sql": "SELECT 1", "path": "/tmp/x.sql", "table_name": "t"})
            ))
            .unwrap()
            .format,
            ExportFormat::SqlInsert
        );
        assert_eq!(
            parse_args(&args(json!({"sql": "SELECT 1", "path": "/tmp/x.weird"})))
                .unwrap()
                .format,
            ExportFormat::Csv
        );
    }

    #[test]
    fn explicit_format_wins_over_the_extension() {
        let a = parse_args(&args(
            json!({"sql": "SELECT 1", "path": "/tmp/x.csv", "format": "json"}),
        ))
        .unwrap();
        assert_eq!(a.format, ExportFormat::Json);
    }

    #[test]
    fn unknown_explicit_format_is_rejected() {
        let e = parse_args(&args(
            json!({"sql": "SELECT 1", "path": "/tmp/x.csv", "format": "xml"}),
        ))
        .unwrap_err();
        assert!(e.to_string().contains("csv, json, or sql_insert"), "{e}");
    }

    #[test]
    fn sql_insert_without_table_name_is_rejected() {
        let e = parse_args(&args(json!({"sql": "SELECT 1", "path": "/tmp/x.sql"}))).unwrap_err();
        assert!(e.to_string().contains("table_name"), "{e}");
    }

    #[test]
    fn overwrite_semantics_against_a_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.csv");
        write_export(&path, b"first", false).unwrap();
        let e = write_export(&path, b"second", false).unwrap_err();
        assert!(e.contains("overwrite:true"), "{e}");
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        write_export(&path, b"second", true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
    }

    #[test]
    fn a_failed_write_leaves_no_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let path = blocker.join("out.csv"); // parent is a file -> create_dir_all fails
        assert!(write_export(&path, b"data", false).is_err());
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries,
            vec![std::ffi::OsString::from("blocker")],
            "no temp/partial file left behind"
        );
    }

    #[test]
    fn deny_list_rejects_protected_dirs_rc_files_and_allows_siblings() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
        std::fs::create_dir_all(home.path().join(".sshx")).unwrap();
        std::fs::write(home.path().join(".zshrc"), b"").unwrap();

        assert!(
            protected_location_in(&home.path().join(".ssh/authorized_keys"), home.path()).is_some()
        );
        assert!(protected_location_in(&home.path().join(".sshx/out.csv"), home.path()).is_none());
        assert!(protected_location_in(&home.path().join(".zshrc"), home.path()).is_some());
        assert!(protected_location_in(&home.path().join("exports/out.csv"), home.path()).is_none());
    }

    #[cfg(unix)] // `std::os::unix::fs::symlink`; the release matrix also builds Windows
    #[test]
    fn deny_list_rejects_a_symlinked_parent_into_a_protected_dir() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
        std::os::unix::fs::symlink(home.path().join(".ssh"), home.path().join("link")).unwrap();
        let p = home.path().join("link/out.csv");
        assert!(
            protected_location_in(&p, home.path()).is_some(),
            "a symlinked parent resolving into ~/.ssh must be rejected"
        );
    }

    /// Review Focus #1: `..` after a not-yet-existing directory made the old
    /// resolver return None, and the deny-list failed OPEN.
    #[test]
    fn parent_dir_components_are_rejected() {
        let e = parse_args(&json!({
            "sql": "SELECT 1",
            "path": "/Users/x/new/../Library/LaunchAgents/evil.plist"
        }))
        .unwrap_err();
        assert!(matches!(e, ToolError::InvalidArgs(_)), "{e:?}");
        assert!(e.to_string().contains(".."), "{e}");
    }

    /// Review Focus #2: APFS (macOS default) and NTFS are case-insensitive.
    #[test]
    fn deny_list_is_case_insensitive() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("Library/LaunchAgents")).unwrap();
        for p in ["library/launchagents/x.plist", ".ZSHRC", ".Bash_Profile"] {
            assert!(
                protected_location_in(&home.path().join(p), home.path()).is_some(),
                "{p}"
            );
        }
    }

    /// Spec amendment (review 2026-10-06): `.zshenv` is read by EVERY zsh,
    /// interactive or not — the same execution class as the spec's five.
    #[test]
    fn deny_list_covers_every_shell_startup_file() {
        let home = tempfile::tempdir().unwrap();
        for rc in [
            ".zshrc",
            ".zprofile",
            ".bashrc",
            ".bash_profile",
            ".profile",
            ".zshenv",
            ".zlogin",
            ".bash_login",
        ] {
            assert!(
                protected_location_in(&home.path().join(rc), home.path()).is_some(),
                "{rc}"
            );
        }
        assert!(
            protected_location_in(&home.path().join("notes/.zshrc"), home.path()).is_none(),
            "only in $HOME"
        );
    }

    /// Reproduced in review: dotfile managers symlink rc files out of $HOME.
    /// `persist` would replace the link with a regular file, and writing to
    /// the target edits the same file zsh reads. Both are refused.
    #[cfg(unix)]
    #[test]
    fn deny_list_rejects_symlinked_rc_files_and_their_targets() {
        let home = tempfile::tempdir().unwrap();
        let target = home.path().join("dotfiles/zsh/.zshrc");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"").unwrap();
        std::os::unix::fs::symlink(&target, home.path().join(".zshrc")).unwrap();
        assert!(
            protected_location_in(&home.path().join(".zshrc"), home.path()).is_some(),
            "the link"
        );
        assert!(
            protected_location_in(&target, home.path()).is_some(),
            "the link's target"
        );

        let ssh_target = home.path().join("dotfiles/ssh");
        std::fs::create_dir_all(&ssh_target).unwrap();
        std::os::unix::fs::symlink(&ssh_target, home.path().join(".ssh")).unwrap();
        assert!(
            protected_location_in(&ssh_target.join("authorized_keys"), home.path()).is_some(),
            "a symlinked ~/.ssh protects its target directory"
        );
    }

    /// Reproduced in review: APFS folds `ſ` (U+017F) to `s`. When the target
    /// does not exist yet there is no inode to compare, so names are folded.
    #[test]
    fn deny_list_folds_unicode_case_for_names_that_do_not_exist_yet() {
        let home = tempfile::tempdir().unwrap();
        for p in [
            ".ſſh/authorized_keys",
            ".zſhrc",
            ".config/autoſtart/x.desktop",
        ] {
            assert!(
                protected_location_in(&home.path().join(p), home.path()).is_some(),
                "{p}"
            );
        }
    }

    /// Reproduced in review: `/System/Volumes/Data/<path>` is the same inode
    /// as `/<path>` on APFS system/data splits, and `realpath` does not fold
    /// the firmlink back. Identity sees through it.
    #[cfg(target_os = "macos")]
    #[test]
    fn deny_list_sees_through_the_data_volume_firmlink() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
        let canon = home.path().canonicalize().unwrap(); // /private/var/folders/…
        let alias =
            std::path::Path::new("/System/Volumes/Data").join(canon.strip_prefix("/").unwrap());
        if !alias.exists() {
            return; // no system/data volume split here — nothing to alias
        }
        assert!(
            protected_location_in(&alias.join(".ssh/authorized_keys"), home.path()).is_some(),
            "existing dir"
        );
        assert!(
            protected_location_in(&alias.join(".zshenv"), home.path()).is_some(),
            "via home's identity"
        );
    }

    #[test]
    fn export_summary_reports_destination_rows_size_and_truncation() {
        let s = export_summary(
            3412,
            std::path::Path::new("/Users/x/invoices.csv"),
            ExportFormat::Csv,
            421_888,
            false,
            false,
        );
        assert!(
            s.contains("Exported 3,412 rows to /Users/x/invoices.csv (CSV, 412 KB)"),
            "{s}"
        );
        let t = export_summary(
            10_000,
            std::path::Path::new("/tmp/big.csv"),
            ExportFormat::Csv,
            1,
            true,
            true,
        );
        assert!(t.contains("first 10,000 rows"), "{t}");
        assert!(t.contains("narrow the query"), "{t}");
        assert!(t.contains("replaced existing file"), "{t}");
    }

    /// Spec §6.6 unit test (review A1) — the original plan only covered the
    /// DB-free half of the summary.
    #[test]
    fn summary_carries_the_enforcement_notice_and_lint_warnings() {
        let s = finalize_summary(
            "Exported 1 rows to /tmp/x.csv (CSV, 2 B)".into(),
            lucent_protocol::ReadOnlyMode::GuardOnly,
            &["JOIN WARNING: seat_no is not a declared FK".to_string()],
        );
        assert!(s.contains("Read-only is NOT enforced"), "{s}");
        assert!(s.contains("JOIN WARNING"), "{s}");
        let strong = finalize_summary(
            "Exported 1 rows".into(),
            lucent_protocol::ReadOnlyMode::TransactionScoped,
            &[],
        );
        assert_eq!(
            strong, "Exported 1 rows",
            "nothing appended when nothing is weakened"
        );
    }

    /// Review Focus #3: `format_json` keys records by column name.
    #[test]
    fn duplicate_column_names_are_detected() {
        let cols = |names: &[&str]| -> Vec<crate::export::ColumnMeta> {
            names
                .iter()
                .map(|n| crate::export::ColumnMeta {
                    name: n.to_string(),
                    type_name: "int4".into(),
                })
                .collect()
        };
        assert_eq!(
            duplicate_column_names(&cols(&["id", "name", "id", "id"])),
            vec!["id".to_string()]
        );
        assert!(duplicate_column_names(&cols(&["id", "name"])).is_empty());
    }
}
