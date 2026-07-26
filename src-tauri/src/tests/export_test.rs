use super::super::export::*;

fn sample_columns() -> Vec<ColumnMeta> {
    vec![
        ColumnMeta {
            name: "id".into(),
            type_name: "int4".into(),
        },
        ColumnMeta {
            name: "name".into(),
            type_name: "text".into(),
        },
        ColumnMeta {
            name: "score".into(),
            type_name: "float8".into(),
        },
        ColumnMeta {
            name: "active".into(),
            type_name: "bool".into(),
        },
        ColumnMeta {
            name: "notes".into(),
            type_name: "text".into(),
        },
    ]
}

fn sample_rows() -> Vec<Vec<serde_json::Value>> {
    vec![
        vec![
            serde_json::json!(1),
            serde_json::json!("Alice"),
            serde_json::json!(95.5),
            serde_json::json!(true),
            serde_json::json!("Hello, world!"),
        ],
        vec![
            serde_json::json!(2),
            serde_json::json!("Bob"),
            serde_json::json!(null),
            serde_json::json!(false),
            serde_json::json!(null),
        ],
        vec![
            serde_json::json!(3),
            serde_json::json!("Charlie"),
            serde_json::json!(88.0),
            serde_json::json!(true),
            serde_json::json!("Line 1\nLine 2"),
        ],
    ]
}

// ─── CSV Tests ──────────────────────────────────────────────────────────

#[test]
fn test_csv_basic() {
    let options = ExportOptions::default();
    let result = format_csv(&sample_columns(), &sample_rows(), &options);

    assert!(result.contains("id,name,score,active,notes"));
    assert!(result.contains("1,Alice,95.5,true,\"Hello, world!\""));
    assert!(result.contains("2,Bob,\\N,false,\\N"));
}

#[test]
fn test_csv_rfc_4180_quoting() {
    let cols = vec![ColumnMeta {
        name: "col".into(),
        type_name: "text".into(),
    }];
    let rows = vec![
        vec![serde_json::json!("contains, comma")],
        vec![serde_json::json!("contains \"quotes\"")],
        vec![serde_json::json!("has\nnewline")],
    ];

    let options = ExportOptions::default();
    let result = format_csv(&cols, &rows, &options);

    assert!(result.contains("\"contains, comma\""));
    assert!(result.contains("\"contains \"\"quotes\"\"\""));
    assert!(result.contains("\"has\nnewline\""));
}

#[test]
fn test_csv_no_header() {
    let options = ExportOptions {
        include_header: Some(false),
        ..Default::default()
    };
    let result = format_csv(&sample_columns(), &sample_rows(), &options);

    assert!(!result.contains("id,name,score"));
    assert!(result.starts_with("1"));
}

#[test]
fn test_csv_custom_delimiter() {
    let options = ExportOptions {
        delimiter: Some('|'),
        ..Default::default()
    };
    let result = format_csv(&sample_columns(), &sample_rows(), &options);

    assert!(result.contains("1|Alice|95.5|true"));
}

#[test]
fn test_csv_custom_null_string() {
    let options = ExportOptions {
        null_string: Some("NULL".into()),
        ..Default::default()
    };
    let result = format_csv(&sample_columns(), &sample_rows(), &options);

    assert!(result.contains("NULL"));
    assert!(!result.contains("\\N"));
}

// ─── JSON Tests ─────────────────────────────────────────────────────────

#[test]
fn test_json_basic() {
    let result = format_json(&sample_columns(), &sample_rows(), &ExportOptions::default());
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();

    assert_eq!(parsed.len(), 3);
    assert_eq!(parsed[0]["name"], "Alice");
    assert_eq!(parsed[1]["score"], serde_json::Value::Null);
    assert_eq!(parsed[2]["notes"], "Line 1\nLine 2");
}

#[test]
fn test_json_field_names() {
    let result = format_json(&sample_columns(), &sample_rows(), &ExportOptions::default());
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&result).unwrap();

    let first = &parsed[0];
    assert!(first.get("id").is_some());
    assert!(first.get("name").is_some());
    assert!(first.get("score").is_some());
    assert!(first.get("active").is_some());
}

#[test]
fn test_json_empty_rows() {
    let result = format_json(&sample_columns(), &[], &ExportOptions::default());
    assert_eq!(result, "[]");
}

// ─── SQL INSERT Tests ───────────────────────────────────────────────────

#[test]
fn test_inserts_basic() {
    let cols = vec![
        ColumnMeta {
            name: "id".into(),
            type_name: "int4".into(),
        },
        ColumnMeta {
            name: "name".into(),
            type_name: "text".into(),
        },
    ];
    let rows = vec![
        vec![serde_json::json!(1), serde_json::json!("Alice")],
        vec![serde_json::json!(2), serde_json::json!("Bob")],
    ];

    let result = format_inserts("users", &cols, &rows, &ExportOptions::default());
    assert!(result.contains("INSERT INTO \"users\""));
    assert!(result.contains("\"id\", \"name\""));
    assert!(result.contains("(1, 'Alice')"));
    assert!(result.contains("(2, 'Bob')"));
}

#[test]
fn test_inserts_escaping() {
    let cols = vec![ColumnMeta {
        name: "name".into(),
        type_name: "text".into(),
    }];
    let rows = vec![
        vec![serde_json::json!("O'Brien")],
        vec![serde_json::json!("null")],
        vec![serde_json::json!(null)],
    ];

    let result = format_inserts("t", &cols, &rows, &ExportOptions::default());
    assert!(result.contains("'O''Brien'"));
    assert!(result.contains("'null'"));
    assert!(result.contains("NULL"));
}

#[test]
fn test_inserts_batching() {
    // Create more than 500 rows to test batching
    let cols = vec![ColumnMeta {
        name: "x".into(),
        type_name: "int4".into(),
    }];
    let rows: Vec<Vec<serde_json::Value>> = (0..550).map(|i| vec![serde_json::json!(i)]).collect();

    let result = format_inserts("nums", &cols, &rows, &ExportOptions::default());
    // Should have at least 2 INSERT statements
    assert!(result.matches("INSERT INTO").count() >= 2);
}

#[test]
fn test_inserts_special_characters() {
    let cols = vec![ColumnMeta {
        name: "data".into(),
        type_name: "text".into(),
    }];
    let rows = vec![
        vec![serde_json::json!("hello\\world")],
        vec![serde_json::json!("it's \"quoted\"")],
    ];

    let result = format_inserts("test", &cols, &rows, &ExportOptions::default());
    // Backslash is a literal in standard-conforming strings — preserved, not doubled.
    assert!(result.contains("hello\\world"));
    assert!(!result.contains("hello\\\\world"));
    // Single quote should be escaped by doubling.
    assert!(result.contains("it''s"));
}

// ─── Edge Cases ─────────────────────────────────────────────────────────

#[test]
fn test_empty_columns() {
    let result = format_csv(&[], &[], &ExportOptions::default());
    // CSV always has a trailing newline when header is included
    assert_eq!(result, "\n");

    let result = format_json(&[], &[], &ExportOptions::default());
    assert_eq!(result, "[]");

    let result = format_inserts("t", &[], &[], &ExportOptions::default());
    assert!(result.is_empty());
}

#[test]
fn test_null_handling() {
    let cols = vec![ColumnMeta {
        name: "val".into(),
        type_name: "text".into(),
    }];
    let rows = vec![
        vec![serde_json::Value::Null],
        vec![serde_json::json!("not null")],
    ];

    let csv = format_csv(&cols, &rows, &ExportOptions::default());
    assert!(csv.contains("\\N"));

    let json = format_json(&cols, &rows, &ExportOptions::default());
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
    assert!(parsed[0]["val"].is_null());

    let insert = format_inserts("t", &cols, &rows, &ExportOptions::default());
    assert!(insert.contains("NULL"));
    assert!(insert.contains("'not null'"));
}
