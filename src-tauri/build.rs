use std::path::Path;

fn main() {
    // `bundle.externalBin` serves two masters: `tauri build` bundles those
    // sidecar files, and tauri-build's `copy_binaries` copies the *staged
    // release sidecars* next to the app binary on every app build — including
    // debug ones. That copy is wrong for dev: it silently overwrites the
    // debug workers in `target/debug` with whatever release sidecars were
    // staged last, so a `PROTOCOL_VERSION` bump leaves `tauri dev` spawning a
    // protocol-mismatched worker. Dev workers must come from `cargo build`
    // (`npm run build:workers:debug`, wired into `beforeDevCommand`); a debug
    // *bundle* is deliberately unsupported.
    //
    // Clear `externalBin` (via TAURI_CONFIG, before tauri-build reads it) when:
    //   * the profile is not `release` — dev/test/check builds never copy or
    //     bundle sidecars, or
    //   * a sidecar for the current target is not staged — a clean checkout's
    //     release-profile `cargo test` / `cargo check` would otherwise fail on
    //     the missing files.
    let is_release = std::env::var("PROFILE")
        .map(|p| p == "release")
        .unwrap_or(false);

    let target = std::env::var("TARGET").unwrap_or_default();
    let is_windows = target.contains("windows");
    let ext = if is_windows { ".exe" } else { "" };

    let sidecars = [
        "lucent-driver-postgres",
        "lucent-driver-duckdb",
        "lucent-db-tools-mcp",
    ];

    let binaries_dir = Path::new("binaries");
    let all_present = !target.is_empty()
        && sidecars.iter().all(|name| {
            let filename = format!("{name}-{target}{ext}");
            binaries_dir.join(filename).exists()
        });

    if !is_release || !all_present {
        if let Ok(existing) = std::env::var("TAURI_CONFIG") {
            if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&existing) {
                if let Some(bundle) = json.get_mut("bundle") {
                    if let Some(obj) = bundle.as_object_mut() {
                        obj.insert("externalBin".into(), serde_json::Value::Null);
                    }
                } else if let Some(obj) = json.as_object_mut() {
                    obj.insert(
                        "bundle".into(),
                        serde_json::json!({ "externalBin": serde_json::Value::Null }),
                    );
                }
                if let Ok(serialized) = serde_json::to_string(&json) {
                    std::env::set_var("TAURI_CONFIG", serialized);
                }
            }
        } else {
            std::env::set_var("TAURI_CONFIG", r#"{"bundle":{"externalBin":null}}"#);
        }
    }

    tauri_build::build();
}
