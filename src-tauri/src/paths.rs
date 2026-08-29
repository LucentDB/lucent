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
    if let Some(over) = std::env::var_os(HOME_OVERRIDE_ENV) {
        return Ok(PathBuf::from(over));
    }
    dirs::home_dir()
        .map(|h| h.join(".lucent"))
        .ok_or_else(|| "could not determine the user's home directory".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guard that restores an env var on drop, so a panicking test cannot
    /// leak state into its neighbours.
    struct EnvGuard(&'static str, Option<std::ffi::OsString>);

    impl EnvGuard {
        fn set(key: &'static str, value: &std::path::Path) -> Self {
            let prior = std::env::var_os(key);
            std::env::set_var(key, value);
            Self(key, prior)
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match self.1.take() {
                Some(v) => std::env::set_var(self.0, v),
                None => std::env::remove_var(self.0),
            }
        }
    }

    #[test]
    fn override_env_is_used_verbatim() {
        let tmp = tempfile::tempdir().unwrap();
        let want = tmp.path().join(".lucent");
        let _g = EnvGuard::set(HOME_OVERRIDE_ENV, &want);
        assert_eq!(lucent_home().unwrap(), want);
    }

    /// The regression this whole module exists for: resolution must not
    /// depend on `HOME`, which is absent for GUI-launched Windows processes.
    #[test]
    fn resolves_without_the_home_variable() {
        let prior = std::env::var_os("HOME");
        std::env::remove_var("HOME");
        let over = std::env::var_os(HOME_OVERRIDE_ENV);
        std::env::remove_var(HOME_OVERRIDE_ENV);

        let got = lucent_home();

        if let Some(h) = prior {
            std::env::set_var("HOME", h);
        }
        if let Some(o) = over {
            std::env::set_var(HOME_OVERRIDE_ENV, o);
        }

        let got = got.expect("home resolves without HOME set");
        assert!(
            got.ends_with(".lucent"),
            "expected a path ending in .lucent, got {got:?}"
        );
    }
}
