//! Connection-form field descriptors.
//!
//! UI metadata, not SQL: rendering the connection form must not require
//! spawning a worker, so the shapes live here rather than behind the seam.

use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    Text,
    Number,
    Password,
    /// A filesystem path; the form offers a file picker.
    Path,
    Select,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverField {
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
    pub required: bool,
    pub default: Option<&'static str>,
    pub options: &'static [&'static str],
    pub placeholder: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub fields: &'static [DriverField],
    /// True when the driver takes a keychain secret (`AuthModel::UserPassword`).
    pub has_secret: bool,
}

const POSTGRES_FIELDS: &[DriverField] = &[
    DriverField {
        key: "host",
        label: "Host",
        kind: FieldKind::Text,
        required: true,
        default: Some("127.0.0.1"),
        options: &[],
        placeholder: None,
    },
    DriverField {
        key: "port",
        label: "Port",
        kind: FieldKind::Number,
        required: true,
        default: Some("5432"),
        options: &[],
        placeholder: None,
    },
    DriverField {
        key: "user",
        label: "User",
        kind: FieldKind::Text,
        required: true,
        default: Some("postgres"),
        options: &[],
        placeholder: None,
    },
    DriverField {
        key: "database",
        label: "Database",
        kind: FieldKind::Text,
        required: true,
        default: Some("postgres"),
        options: &[],
        placeholder: None,
    },
    DriverField {
        key: "ssl_mode",
        label: "SSL Mode",
        kind: FieldKind::Select,
        required: false,
        default: Some("prefer"),
        options: &["disable", "prefer", "require"],
        placeholder: None,
    },
];

const DESCRIPTORS: &[DriverDescriptor] = &[DriverDescriptor {
    id: "postgres",
    display_name: "PostgreSQL",
    fields: POSTGRES_FIELDS,
    has_secret: true,
}];

pub fn descriptors() -> &'static [DriverDescriptor] {
    DESCRIPTORS
}

pub fn descriptor(id: &str) -> Option<&'static DriverDescriptor> {
    DESCRIPTORS.iter().find(|d| d.id == id)
}

/// The parameter map a new profile of this driver starts with.
pub fn default_params(driver: &str) -> std::collections::BTreeMap<String, String> {
    descriptor(driver)
        .map(|d| {
            d.fields
                .iter()
                .filter_map(|f| f.default.map(|v| (f.key.to_string(), v.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub fn list_drivers() -> &'static [DriverDescriptor] {
    descriptors()
}
