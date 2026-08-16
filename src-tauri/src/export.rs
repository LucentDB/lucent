use std::borrow::Cow;

use crate::sql_quote::{quote_identifier, quote_string};
use serde::{Deserialize, Serialize};

// ─── Types ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExportFormat {
    Csv,
    Json,
    SqlInsert,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub include_header: Option<bool>,
    pub delimiter: Option<char>,
    pub null_string: Option<String>,
    pub table_name: Option<String>,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            format: ExportFormat::Csv,
            include_header: Some(true),
            delimiter: Some(','),
            null_string: Some("\\N".into()),
            table_name: None,
        }
    }
}

// ─── CSV Formatting (RFC 4180) ─────────────────────────────────────────────

pub fn format_csv(
    columns: &[ColumnMeta],
    rows: &[Vec<serde_json::Value>],
    options: &ExportOptions,
) -> String {
    let delimiter = options.delimiter.unwrap_or(',');
    let null_str = options.null_string.as_deref().unwrap_or("\\N");
    let mut out = String::new();

    if options.include_header.unwrap_or(true) {
        let header: Vec<String> = columns
            .iter()
            .map(|c| csv_quote(&c.name, delimiter))
            .collect();
        out.push_str(&header.join(&delimiter.to_string()));
        out.push('\n');
    }

    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .map(|v| csv_format_value(v, null_str, delimiter))
            .collect();
        out.push_str(&cells.join(&delimiter.to_string()));
        out.push('\n');
    }
    out
}

fn csv_format_value(v: &serde_json::Value, null_str: &str, delimiter: char) -> String {
    match v {
        serde_json::Value::Null => null_str.to_string(),
        serde_json::Value::String(s) => csv_quote(s, delimiter),
        other => csv_quote(&other.to_string(), delimiter),
    }
}

/// Prefix a cell with `'` when it starts with a spreadsheet formula
/// trigger. Without this, Excel/Sheets evaluates exported cells as formulas
/// — CWE-1236 CSV injection (S2). Applied BEFORE quoting so the prefix also
/// protects cells that need RFC-4180 quoting.
fn neutralize_formula(s: &str) -> Cow<'_, str> {
    match s.chars().next() {
        Some('=' | '+' | '-' | '@' | '\t' | '\r') => {
            let mut out = String::with_capacity(s.len() + 1);
            out.push('\'');
            out.push_str(s);
            Cow::Owned(out)
        }
        _ => Cow::Borrowed(s),
    }
}

fn csv_quote(s: &str, delimiter: char) -> String {
    let s = neutralize_formula(s);
    if s.contains(delimiter) || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ─── JSON Formatting ────────────────────────────────────────────────────────

pub fn format_json(
    columns: &[ColumnMeta],
    rows: &[Vec<serde_json::Value>],
    _options: &ExportOptions,
) -> String {
    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        let mut map = serde_json::Map::new();
        for (i, col) in columns.iter().enumerate() {
            if let Some(val) = row.get(i) {
                map.insert(col.name.clone(), val.clone());
            }
        }
        records.push(serde_json::Value::Object(map));
    }
    serde_json::to_string_pretty(&serde_json::Value::Array(records)).unwrap_or_default()
}

// ─── SQL INSERT Formatting ─────────────────────────────────────────────────

pub fn format_inserts(
    table_name: &str,
    columns: &[ColumnMeta],
    rows: &[Vec<serde_json::Value>],
    _options: &ExportOptions,
) -> String {
    const BATCH_SIZE: usize = 500;
    let mut out = String::new();

    let col_names: Vec<String> = columns.iter().map(|c| quote_identifier(&c.name)).collect();

    for chunk in rows.chunks(BATCH_SIZE) {
        out.push_str(&format!(
            "INSERT INTO {} ({}) VALUES\n",
            quote_identifier(table_name),
            col_names.join(", ")
        ));

        let rows_str: Vec<String> = chunk
            .iter()
            .map(|row| {
                let vals: Vec<String> = columns
                    .iter()
                    .enumerate()
                    .map(|(i, _)| match row.get(i) {
                        Some(serde_json::Value::Null) => "NULL".into(),
                        Some(serde_json::Value::Bool(b)) => b.to_string(),
                        Some(serde_json::Value::Number(n)) => n.to_string(),
                        Some(serde_json::Value::String(s)) => quote_string(s),
                        Some(other) => quote_string(&other.to_string()),
                        None => "NULL".into(),
                    })
                    .collect();
                format!("({})", vals.join(", "))
            })
            .collect();

        out.push_str(&rows_str.join(",\n"));
        out.push_str(";\n\n");
    }
    out
}

// ─── ColumnMeta (mirrors lucent_protocol::ColumnMeta) ─────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnMeta {
    pub name: String,
    #[serde(rename = "typeName", alias = "type_name")]
    pub type_name: String,
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tests/export_test.rs"]
mod export_tests;
