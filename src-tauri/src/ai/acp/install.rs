use crate::ai::acp::registry::{AgentManifest, BinaryDist};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How to launch an installed agent. `cmd` is the bare program (never a
/// full command line); the launcher (phase C) runs `cmd` with `args` and
/// `env` — the manifest's npx/uvx entries are resolved here at install time
/// into this shape so the launcher never has to re-parse a shell line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LaunchSpec {
    pub cmd: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// A successfully installed agent, persisted as `installed.json` next to its
/// extracted files under `~/.lucent/agents/<id>/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledAgent {
    pub id: String,
    pub version: String,
    pub launch: LaunchSpec,
}

/// Resolves the manifest's binary triple from the current platform, or `None`
/// for an unsupported OS/arch combination.
pub fn target_triple() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("darwin-aarch64"),
        ("macos", "x86_64") => Some("darwin-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("windows", "aarch64") => Some("windows-aarch64"),
        _ => None,
    }
}

/// Where installed agents live: `~/.lucent/agents`.
pub fn agents_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    Ok(PathBuf::from(home).join(".lucent").join("agents"))
}

/// `cmd` from the manifest must resolve inside the extraction root. Rejects
/// absolute paths and any `..` component. The check runs on the manifest
/// string before extraction — it cannot see files inside the archive itself;
/// those are guarded separately (zip: the crate's own entry-path check,
/// pinned by `zip_slip_entries_are_rejected`; tar: the crate skips `..`).
pub(crate) fn cmd_inside_root(cmd: &str) -> Result<PathBuf, String> {
    let p = Path::new(cmd);
    if p.is_absolute()
        || p.components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("unsafe install path in manifest: {cmd}"));
    }
    Ok(p.to_path_buf())
}

/// Verifies a downloaded file against the manifest's sha256. Case-insensitive
/// on the hex; a mismatch is a hard failure (corrupt download or wrong manifest).
pub(crate) fn verify_sha256(file: &Path, expected_hex: &str) -> Result<(), String> {
    use sha2::Digest;
    let bytes = std::fs::read(file).map_err(|e| format!("read downloaded file: {e}"))?;
    let actual = format!("{:x}", sha2::Sha256::digest(&bytes));
    if actual != expected_hex.to_lowercase() {
        return Err("sha256 mismatch — download corrupted or the manifest is wrong".into());
    }
    Ok(())
}

/// Agent ids come from the registry feed and from the `uninstall_acp_agent`
/// command. They are joined into filesystem paths (`~/.lucent/agents/<id>`),
/// so they must be a single normal path component — otherwise `uninstall("..")`
/// would delete `~/.lucent` wholesale. Mirrors the `cmd_inside_root` guard's
/// discipline for the other manifest-controlled path input.
pub(crate) fn validate_agent_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("agent id must not be empty".into());
    }
    let mut components = Path::new(id).components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(()),
        _ => Err(format!("unsafe agent id: {id}")),
    }
}

/// Extracts an archive into `dest`. Supports `.zip`, `.tar.gz`/`.tgz`, and
/// `.tar.bz2`/`.tbz2`; recognized-but-unhandled compressed suffixes are a hard
/// error (a silently copied archive is a broken install with no diagnostic);
/// a plain binary (no compression suffix) is copied as-is, keeping its file
/// name. Dispatch is by the archive's file name extension — the staging name
/// must preserve the download URL's extension (see `staging_file_name`).
fn extract_archive(archive: &Path, dest: &Path) -> Result<(), String> {
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let f = std::fs::File::open(archive).map_err(|e| format!("open archive: {e}"))?;
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("bad zip: {e}"))?;
        zip.extract(dest).map_err(|e| format!("extract zip: {e}"))?;
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let gz = flate2::read::GzDecoder::new(f);
        let mut tar = tar::Archive::new(gz);
        tar.unpack(dest)
            .map_err(|e| format!("extract tar.gz: {e}"))?;
    } else if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") {
        let bz = bzip2::read::BzDecoder::new(f);
        let mut tar = tar::Archive::new(bz);
        tar.unpack(dest)
            .map_err(|e| format!("extract tar.bz2: {e}"))?;
    } else if is_unhandled_compressed(name) {
        return Err(format!(
            "unsupported archive type {} — install it manually or choose another agent",
            unsupported_ext(name)
        ));
    } else {
        // Plain binary archive (no compression suffix): copy as-is.
        std::fs::create_dir_all(dest).map_err(|e| format!("create dest dir: {e}"))?;
        let out = dest.join(name);
        std::fs::copy(archive, &out).map_err(|e| format!("copy binary: {e}"))?;
    }
    Ok(())
}

/// Recognizes compressed-archive suffixes we cannot unpack, so a feed entry
/// that ships one fails loudly instead of silently installing the compressed
/// bytes as the "binary". Only consulted after `.zip`/`.tar.gz`/`.tgz`/
/// `.tar.bz2`/`.tbz2` have been dispatched.
fn is_unhandled_compressed(name: &str) -> bool {
    if let Some(idx) = name.rfind(".tar.") {
        // Any `.tar.<ext>` we don't handle above (e.g. `.tar.xz`).
        if !name[idx + ".tar.".len()..].is_empty() {
            return true;
        }
    }
    const STANDALONE: &[&str] = &[
        ".txz", ".xz", ".bz2", ".gz", ".zst", ".zstd", ".lz4", ".7z", ".rar",
    ];
    STANDALONE.iter().any(|s| name.ends_with(s))
}

/// The extension to name in the "unsupported archive type" error, e.g.
/// `.tar.xz` for `agent.tar.xz` and `.xz` for `agent.xz`.
fn unsupported_ext(name: &str) -> String {
    if let Some(idx) = name.rfind(".tar.") {
        return name[idx..].to_string();
    }
    match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => format!(".{ext}"),
        _ => name.to_string(),
    }
}

/// Derives the staging file name for a downloaded archive from its URL: the
/// final path segment, so the extension survives and `extract_archive`'s
/// extension-based dispatch actually fires. Falls back to `"download"` when
/// the URL has no (non-empty) path segment — including bare `scheme://host`
/// URLs, whose last `/`-segment is the host, not a file name.
fn staging_file_name(archive_url: &str) -> String {
    let after_scheme = archive_url.rsplit("://").next().unwrap_or(archive_url);
    if !after_scheme.contains('/') {
        return "download".to_string();
    }
    let segment = after_scheme
        .rsplit('/')
        .next()
        .filter(|seg| !seg.is_empty())
        .unwrap_or("download");
    // Strip any query string / fragment so the extension survives (a URL like
    // `opencode.zip?token=x` must still stage as `opencode.zip`).
    let end = segment.find(['?', '#']).unwrap_or(segment.len());
    segment[..end].to_string()
}

/// Installs an agent from its manifest: resolves the launch spec (npx/uvx as
/// bare-program + args, binary via download + sha256 verify + extract),
/// writes `installed.json`, and returns the resolved `InstalledAgent`.
pub async fn install(
    agent: &AgentManifest,
    http: &reqwest::Client,
) -> Result<InstalledAgent, String> {
    validate_agent_id(&agent.id)?;
    let dir = agents_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create agents dir: {e}"))?;
    let root = dir.join(&agent.id);
    std::fs::create_dir_all(&root).map_err(|e| format!("create agent dir: {e}"))?;

    let (cmd, args, env) = if let Some(npx) = &agent.distribution.npx {
        let mut args = vec!["-y".to_string(), npx.package.clone()];
        args.extend(npx.args.clone());
        ("npx".to_string(), args, npx.env.clone())
    } else if let Some(uvx) = &agent.distribution.uvx {
        let mut args = vec![uvx.package.clone()];
        args.extend(uvx.args.clone());
        ("uvx".to_string(), args, uvx.env.clone())
    } else if let Some(binary) = &agent.distribution.binary {
        let triple = target_triple()
            .ok_or_else(|| format!("{} has no binary for this platform", agent.name))?;
        let dist: &BinaryDist = binary.get(triple).ok_or_else(|| {
            format!(
                "{} has no binary for {triple} (has: {})",
                agent.name,
                binary.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        let rel = cmd_inside_root(&dist.cmd)?;
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).map_err(|e| format!("create staging dir: {e}"))?;
        let archive_path = staging.join(staging_file_name(&dist.archive));
        let resp = http
            .get(&dist.archive)
            .send()
            .await
            .map_err(|e| format!("download failed: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("download returned HTTP {}", resp.status()));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("read download: {e}"))?;
        std::fs::write(&archive_path, &bytes).map_err(|e| format!("write download: {e}"))?;
        if let Some(want) = &dist.sha256 {
            verify_sha256(&archive_path, want)?;
        }
        extract_archive(&archive_path, &root)?;
        let _ = std::fs::remove_dir_all(&staging);
        let abs = root.join(&rel);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Err(e) = std::fs::set_permissions(&abs, std::fs::Permissions::from_mode(0o755)) {
                log::warn!("failed to chmod +x {}: {e}", abs.display());
            }
        }
        (
            abs.to_string_lossy().into_owned(),
            dist.args.clone(),
            dist.env.clone(),
        )
    } else {
        return Err(format!("{} has no installable distribution", agent.name));
    };

    let installed = InstalledAgent {
        id: agent.id.clone(),
        version: agent.version.clone(),
        launch: LaunchSpec { cmd, args, env },
    };
    let meta = serde_json::to_string_pretty(&installed)
        .map_err(|e| format!("serialize installed: {e}"))?;
    std::fs::write(root.join("installed.json"), meta)
        .map_err(|e| format!("write installed.json: {e}"))?;
    Ok(installed)
}

/// Reads the persisted install for an agent id. `Ok(None)` when nothing is
/// installed; `Err` for an unsafe id or an unreadable/parse-broken
/// `installed.json`.
pub fn read_installed(id: &str) -> Result<Option<InstalledAgent>, String> {
    validate_agent_id(id)?;
    let path = agents_dir()?.join(id).join("installed.json");
    match std::fs::read_to_string(&path) {
        Ok(json) => serde_json::from_str(&json)
            .map(Some)
            .map_err(|e| format!("parse installed.json: {e}")),
        Err(_) => Ok(None),
    }
}

/// Lists every installed agent (one entry per `~/.lucent/agents/<id>/installed.json`).
/// Entries that are not directories, lack an `installed.json`, or carry an
/// unsafe id are skipped — a leftover staging dir must not surface as an agent.
pub fn list_installed() -> Vec<InstalledAgent> {
    let Ok(dir) = agents_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let id = e.file_name().to_string_lossy().into_owned();
            read_installed(&id).ok().flatten()
        })
        .collect()
}

/// Removes an installed agent (its whole directory). No-op when not installed.
pub fn uninstall(id: &str) -> Result<(), String> {
    validate_agent_id(id)?;
    let dir = agents_dir()?.join(id);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("remove {id}: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::acp::registry::{AgentManifest, BinaryDist};
    use sha2::Digest;
    use std::io::Write;

    #[test]
    fn triple_resolves_for_current_platform() {
        let t = target_triple();
        #[cfg(target_os = "macos")]
        assert!(matches!(t, Some("darwin-aarch64") | Some("darwin-x86_64")));
        #[cfg(target_os = "linux")]
        assert!(matches!(t, Some("linux-aarch64") | Some("linux-x86_64")));
        #[cfg(target_os = "windows")]
        assert!(matches!(
            t,
            Some("windows-x86_64") | Some("windows-aarch64")
        ));
    }

    #[test]
    fn cmd_path_escape_is_rejected() {
        // The manifest says cmd = "../evil" — must be rejected before extraction.
        let fixture: BinaryDist =
            serde_json::from_str(r#"{"archive":"x","cmd":"../evil"}"#).unwrap();
        assert!(cmd_inside_root(&fixture.cmd).is_err());
        // Absolute paths are rejected too (the guard's documented contract).
        let abs: BinaryDist =
            serde_json::from_str(r#"{"archive":"x","cmd":"/etc/passwd"}"#).unwrap();
        assert!(cmd_inside_root(&abs.cmd).is_err());
    }

    #[test]
    fn sha256_mismatch_fails_install() {
        let tmp = tempfile::tempdir().unwrap();
        let payload = b"hello acp";
        let file = tmp.path().join("agent.bin");
        std::fs::write(&file, payload).unwrap();
        let want = "0000000000000000000000000000000000000000000000000000000000000000"; // wrong hash
        let got = verify_sha256(&file, want);
        assert!(got.is_err());
    }

    #[test]
    fn sha256_match_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let payload = b"hello acp";
        let file = tmp.path().join("agent.bin");
        std::fs::write(&file, payload).unwrap();
        let want = format!("{:x}", sha2::Sha256::digest(payload));
        let got = verify_sha256(&file, &want);
        assert!(got.is_ok(), "matching hash must verify: {got:?}");
    }

    #[tokio::test]
    async fn npx_install_round_trips_installed_json() {
        // The npx path never executes anything — install() only resolves the
        // launch spec and writes installed.json. No network is touched.
        let manifest: AgentManifest = serde_json::from_str(
            r#"{
                "id": "test-npx-agent",
                "name": "Test Npx Agent",
                "version": "9.9.9",
                "description": "test fixture",
                "license": "MIT",
                "distribution": {
                    "npx": {
                        "package": "@test/agent@9.9.9",
                        "args": ["--acp"],
                        "env": { "FOO": "bar" }
                    }
                }
            }"#,
        )
        .unwrap();
        let http = reqwest::Client::new(); // unused on the npx path

        let installed = install(&manifest, &http)
            .await
            .expect("npx install succeeds");
        assert_eq!(installed.id, "test-npx-agent");
        assert_eq!(installed.launch.cmd, "npx");
        assert_eq!(
            installed.launch.args,
            vec!["-y", "@test/agent@9.9.9", "--acp"]
        );
        assert_eq!(installed.launch.env.get("FOO"), Some(&"bar".to_string()));

        let read = read_installed("test-npx-agent")
            .expect("read_installed ok")
            .expect("installed.json round-trips");
        assert_eq!(read.version, "9.9.9");
        assert_eq!(read.launch.cmd, "npx");
        assert_eq!(read.launch.args, vec!["-y", "@test/agent@9.9.9", "--acp"]);

        uninstall("test-npx-agent").expect("cleanup ok");
        assert!(
            read_installed("test-npx-agent")
                .expect("read_installed ok")
                .is_none(),
            "uninstall removes the agent dir"
        );
    }

    #[test]
    fn list_installed_lists_every_agent_with_installed_json() {
        let tmp = tempfile::tempdir().unwrap();
        let agents = tmp.path().join(".lucent").join("agents");
        for (id, version, cmd) in [
            ("agent-one", "1.0.0", "npx"),
            ("agent-two", "2.0.0", "/usr/local/bin/goose"),
        ] {
            let dir = agents.join(id);
            std::fs::create_dir_all(&dir).unwrap();
            let installed = InstalledAgent {
                id: id.to_string(),
                version: version.to_string(),
                launch: LaunchSpec {
                    cmd: cmd.to_string(),
                    args: vec![],
                    env: HashMap::new(),
                },
            };
            std::fs::write(
                dir.join("installed.json"),
                serde_json::to_string_pretty(&installed).unwrap(),
            )
            .unwrap();
        }
        // A directory without installed.json (leftover staging dir, partial
        // install) must be skipped, not returned as a bogus entry.
        std::fs::create_dir_all(agents.join("no-metadata")).unwrap();

        let prev_home = std::env::var_os("HOME");
        std::env::set_var("HOME", tmp.path());
        let listed = list_installed();
        match prev_home {
            Some(h) => std::env::set_var("HOME", h),
            None => std::env::remove_var("HOME"),
        }

        let mut ids: Vec<_> = listed.iter().map(|a| a.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["agent-one", "agent-two"]);
        let one = listed.iter().find(|a| a.id == "agent-one").unwrap();
        assert_eq!(one.version, "1.0.0");
        assert_eq!(one.launch.cmd, "npx");
    }

    // --- Fix round 1 covering tests (archive extension dispatch + zip-slip) ---

    #[test]
    fn staging_file_name_derives_url_segment() {
        assert!(staging_file_name("https://example.com/dl/opencode.zip").ends_with(".zip"));
        assert!(
            staging_file_name("https://example.com/dl/opencode-darwin-aarch64.tar.gz")
                .ends_with(".tar.gz")
        );
        // Query strings / fragments must not leak into the staging name: the
        // extension has to survive for extract_archive's dispatch to fire.
        assert!(
            staging_file_name("https://example.com/dl/opencode.zip?token=abc123").ends_with(".zip")
        );
        assert!(
            staging_file_name("https://example.com/dl/opencode.tar.gz#frag").ends_with(".tar.gz")
        );
        assert_eq!(staging_file_name("https://example.com"), "download");
        assert_eq!(staging_file_name("https://example.com/"), "download");
        assert_eq!(staging_file_name(""), "download");
    }

    #[test]
    fn agent_id_path_escape_is_rejected() {
        // Agent ids are joined into filesystem paths (~/.lucent/agents/<id>)
        // by install/read_installed/uninstall and can come from the feed or
        // from the uninstall command — a `..` id would wipe ~/.lucent.
        for bad in ["../evil", "/abs", "a/b", "a/../b", "", ".", ".."] {
            assert!(
                validate_agent_id(bad).is_err(),
                "agent id must be rejected: {bad:?}"
            );
        }
        assert!(validate_agent_id("opencode").is_ok());
    }

    #[test]
    fn tar_bz2_archive_extracts_into_dest() {
        // goose (in the pinned snapshot) ships .tar.bz2 triples — this proves
        // the bzip2 dispatch fires and extraction actually produces files.
        let tmp = tempfile::tempdir().unwrap();
        let cases = [("agent.tar.bz2", "hello bz2"), ("agent.tbz2", "hello tbz2")];
        for (file, content) in cases {
            let bz_path = tmp.path().join(file);
            write_tar_bz2(&bz_path, &[("hello.txt", content.as_bytes())]);
            let dest = tmp.path().join(format!("out-{file}"));
            extract_archive(&bz_path, &dest)
                .unwrap_or_else(|e| panic!("{file} extraction must succeed: {e}"));
            assert_eq!(
                std::fs::read(dest.join("hello.txt")).unwrap(),
                content.as_bytes()
            );
        }
    }

    #[test]
    fn unsupported_archive_suffix_is_a_hard_error() {
        // A compressed suffix we can't unpack must fail loudly — silently
        // copying compressed bytes as the "binary" is a broken install with
        // zero diagnostic.
        let tmp = tempfile::tempdir().unwrap();
        for file in ["agent.tar.xz", "agent.txz"] {
            let xz_path = tmp.path().join(file);
            std::fs::write(&xz_path, b"not really xz").unwrap();
            let dest = tmp.path().join(format!("out-{file}"));
            let err = match extract_archive(&xz_path, &dest) {
                Ok(()) => panic!("{file} must be a hard error, got Ok"),
                Err(e) => e,
            };
            assert!(
                err.contains("unsupported archive type"),
                "error names the problem: {err}"
            );
            assert!(
                err.contains(".tar.xz") || err.contains(".txz"),
                "error names the extension: {err}"
            );
            assert!(!dest.exists(), "no output dir on error for {file}");
        }
    }

    #[test]
    fn extensionless_archive_copies_as_plain_binary() {
        // sigit (pinned snapshot) ships extensionless URLs (e.g.
        // sigit-linux-arm64) — the plain-copy fallback is load-bearing for them.
        let tmp = tempfile::tempdir().unwrap();
        let bin_path = tmp.path().join("sigit-linux-arm64");
        std::fs::write(&bin_path, b"ELF payload").unwrap();
        let dest = tmp.path().join("out");
        extract_archive(&bin_path, &dest).expect("plain binary copies");
        assert_eq!(
            std::fs::read(dest.join("sigit-linux-arm64")).unwrap(),
            b"ELF payload"
        );
    }

    #[test]
    fn zip_archive_extracts_into_dest() {
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("agent.zip");
        write_zip(&zip_path, &[("hello.txt", b"hello zip")]);
        let dest = tmp.path().join("out");
        extract_archive(&zip_path, &dest).expect("zip extraction succeeds");
        assert_eq!(std::fs::read(dest.join("hello.txt")).unwrap(), b"hello zip");
    }

    #[test]
    fn tar_gz_archive_extracts_into_dest() {
        let tmp = tempfile::tempdir().unwrap();
        let tgz_path = tmp.path().join("agent.tar.gz");
        write_tar_gz(&tgz_path, &[("hello.txt", b"hello tar")]);
        let dest = tmp.path().join("out");
        extract_archive(&tgz_path, &dest).expect("tar.gz extraction succeeds");
        assert_eq!(std::fs::read(dest.join("hello.txt")).unwrap(), b"hello tar");
    }

    #[test]
    fn zip_slip_entries_are_rejected() {
        // zip 2.4.2's `extract` refuses `../` entries via safe_prepare_path /
        // simplified_components (verified in the crate source): a ParentDir at
        // depth 0 yields InvalidArchive("Invalid file path"). This test pins
        // that behavior so a future crate upgrade that relaxes it is caught.
        let tmp = tempfile::tempdir().unwrap();
        let zip_path = tmp.path().join("evil.zip");
        write_zip(&zip_path, &[("../evil.txt", b"pwned")]);
        let dest = tmp.path().join("out");
        let err = extract_archive(&zip_path, &dest).expect_err("zip-slip entry must be rejected");
        assert!(err.contains("extract zip"), "unexpected error: {err}");
        assert!(
            !tmp.path().join("evil.txt").exists(),
            "no file may escape the dest dir"
        );
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    fn write_tar_gz(path: &Path, entries: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        let mut tar = tar::Builder::new(gz);
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, *name, *data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
    }

    fn write_tar_bz2(path: &Path, entries: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let bz = bzip2::write::BzEncoder::new(f, bzip2::Compression::default());
        let mut tar = tar::Builder::new(bz);
        for (name, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, *name, *data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
    }
}
