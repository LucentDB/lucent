//! Cross-platform resolution of Lucent's user-data root.
//!
//! Named for the `CARGO_HOME` → `~/.cargo` precedent: `lucent_home()` is the
//! `~/.lucent` directory itself, not the user's home directory.
//!
//! Why this module exists: five call sites independently reached for
//! `std::env::var("HOME")`, which Windows does not set for GUI-launched
//! processes (it sets `USERPROFILE`). Every one of them was on the AI/ACP
//! path, so the db-tools MCP feature was wholly non-functional on Windows.
//! One helper, one convention.

use std::path::PathBuf;

/// Names the `.lucent` directory itself. Tests set this; users generally
/// do not need to.
const HOME_OVERRIDE_ENV: &str = "LUCENT_HOME";

/// Lucent's user-data root: `<home>/.lucent`.
pub fn lucent_home() -> Result<PathBuf, String> {
    resolve(std::env::var_os(HOME_OVERRIDE_ENV), dirs::home_dir())
}

/// The decision, separated from the two impure reads above.
///
/// Kept pure on purpose: `std::env::set_var` is process-global, so a test that
/// sets `HOME` or `LUCENT_HOME` to exercise this logic reaches into every other
/// test running concurrently. Taking both inputs as parameters lets the tests
/// cover every branch without touching the environment at all.
fn resolve(
    override_value: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Result<PathBuf, String> {
    if let Some(over) = override_value {
        return Ok(PathBuf::from(over));
    }
    home.map(|h| h.join(".lucent"))
        .ok_or_else(|| "could not determine the user's home directory".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_override_wins_and_is_used_verbatim() {
        let got = resolve(
            Some(std::ffi::OsString::from("/tmp/fixture/.lucent")),
            Some(PathBuf::from("/Users/real")),
        );
        assert_eq!(got.unwrap(), PathBuf::from("/tmp/fixture/.lucent"));
    }

    /// The regression this whole module exists for: the root is derived from
    /// the OS's notion of a home directory, never from `HOME`, which Windows
    /// does not set for GUI-launched processes. That `lucent_home()` reads
    /// only `LUCENT_HOME` and `dirs::home_dir()` is guaranteed structurally --
    /// `resolve` has no access to the environment -- and the absence of stray
    /// `HOME` reads elsewhere is enforced by the guard test below.
    #[test]
    fn without_an_override_the_root_is_dot_lucent_under_home() {
        let got = resolve(None, Some(PathBuf::from("/Users/real")));
        assert_eq!(got.unwrap(), PathBuf::from("/Users/real/.lucent"));
    }

    #[test]
    fn an_unresolvable_home_is_an_error_not_a_panic() {
        let err = resolve(None, None).unwrap_err();
        assert!(err.contains("home directory"), "{err}");
    }

    /// Architectural guard: the whole point of this module is that nothing
    /// else reads HOME directly. A new call site would silently reintroduce
    /// the Windows bug, so fail the build instead of waiting for a bug report.
    #[test]
    fn no_production_code_reads_the_home_variable_directly() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                // This module documents the pattern in prose and tests it.
                if path.file_name().and_then(|n| n.to_str()) == Some("paths.rs") {
                    continue;
                }
                let src = std::fs::read_to_string(&path).unwrap();
                for (i, line) in src.lines().enumerate() {
                    if !line.contains(r#"var("HOME")"#) && !line.contains(r#"var_os("HOME")"#) {
                        continue;
                    }
                    // Tests legitimately manipulate HOME to build fixtures, and
                    // some read it only to restore it afterwards. Those carry an
                    // explicit `// home-ok` marker so the exemption is deliberate
                    // rather than an accident of how the line is written.
                    if line.contains("set_var")
                        || line.contains("remove_var")
                        || line.contains("// home-ok")
                    {
                        continue;
                    }
                    offenders.push(format!("{}:{}", path.display(), i + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "these read HOME directly - use crate::paths::lucent_home() instead:\n{}",
            offenders.join("\n")
        );
    }
}
