use std::path::Path;

fn main() {
    // Check if sidecar binaries exist for the current target.
    // If not (e.g. during a clean `cargo test`, `cargo check`, or dev run where
    // sidecars have not been staged yet), clear `bundle.externalBin` via TAURI_CONFIG
    // so tauri-build does not fail on missing sidecar files.
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

    if !all_present {
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
