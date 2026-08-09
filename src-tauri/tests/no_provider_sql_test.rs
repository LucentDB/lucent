//! Architecture test: no provider-specific catalog SQL above the `Connector`
//! trait.
//!
//! `src-tauri/src` is the app. It talks to databases through normalized catalog
//! RPCs. The moment a `pg_catalog` or `information_schema` string reappears
//! here, a second driver silently breaks the schema browser and the whole AI
//! stack — which is exactly the failure this plan exists to prevent.

use std::path::{Path, PathBuf};

/// Substrings that betray provider-specific catalog access.
const FORBIDDEN: &[&str] = &[
    "information_schema",
    "pg_catalog",
    "pg_class",
    "pg_namespace",
    "pg_attribute",
    "pg_constraint",
    "pg_proc",
    "pg_inherits",
    "pg_attrdef",
    "pg_stat_user_tables",
    "pg_get_functiondef",
    "pg_get_viewdef",
    "pg_get_partkeydef",
    "pg_get_expr",
    "obj_description",
    "col_description",
    "format_type(",
    "reltuples",
    "relkind",
    "::regclass",
];

/// Files that may still mention these strings, with the reason.
///
/// Keep this list short and justified. Every entry is a place a future driver
/// author has to think about.
fn is_exempt(path: &Path) -> bool {
    let p = path.to_string_lossy();
    // This test names the forbidden strings in order to forbid them.
    p.ends_with("tests/no_provider_sql_test.rs")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_provider_catalog_sql_above_the_connector_seam() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(!files.is_empty(), "found no sources under {root:?}");

    let mut violations: Vec<String> = Vec::new();
    for file in files {
        if is_exempt(&file) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            for needle in FORBIDDEN {
                if line.contains(needle) {
                    violations.push(format!(
                        "{}:{}: {needle}\n    {}",
                        file.display(),
                        i + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "provider-specific catalog SQL found above the Connector trait.\n\
         Move it into a driver's catalog module and expose it as a CatalogRequest.\n\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_grep_test_can_actually_fail() {
    // A test that cannot fail is not a test. Prove the matcher works.
    let line = "SELECT * FROM information_schema.tables";
    assert!(FORBIDDEN.iter().any(|n| line.contains(n)));
}

#[test]
fn the_app_crate_does_not_link_a_database_driver() {
    // `test_connection` used to build a tokio_postgres client directly,
    // bypassing the worker entirely. The PRODUCTION app must never link a
    // database driver — otherwise "add a driver" means "add a dependency to
    // the app", and the seam is decorative.
    //
    // Test harnesses are exempt by design: the `integration-tests` and `evals`
    // features (plus dev-dependencies) legitimately speak the Postgres wire
    // protocol to seed containers and compute ground truth. What must never
    // happen is a NON-optional driver dependency in the app's production
    // dependency graph.
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .expect("read Cargo.toml");

    // Only driver names that must NEVER appear in the manifest at all.
    for forbidden in ["duckdb", "gcp-bigquery"] {
        assert!(
            !manifest.contains(forbidden),
            "src-tauri must not depend on {forbidden}: talk to databases through \
             the worker, not by linking their clients into the app"
        );
    }

    // tokio-postgres may appear only as an optional dependency (test features)
    // or in [dev-dependencies] — never as a non-optional normal dependency.
    let deps_section = manifest
        .split("[dependencies]")
        .nth(1)
        .and_then(|rest| rest.split("[dev-dependencies]").next())
        .unwrap_or("");

    let non_optional_tokio_postgres = deps_section
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("tokio-postgres"))
        .filter(|line| !line.contains("optional = true"))
        .count();

    assert_eq!(
        non_optional_tokio_postgres, 0,
        "tokio-postgres must be an optional test-only dependency or a \
         dev-dependency — a non-optional entry in [dependencies] links a \
         database driver into the production app"
    );
}
