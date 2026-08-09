use lucent_lib::notebook::cte::validate_referenceable;
use lucent_lib::notebook::types::CellError;

use lucent_protocol::SqlDialect;

const PG: SqlDialect = SqlDialect::PostgreSql;

// The old dml::is_dml_or_ddl module was removed (Task 26): referenceability is
// now a single wrappability predicate — "is this one SELECT/WITH/VALUES
// statement" — which subsumes the DML/DDL string-prefix check. These tests pin
// that predicate's behaviour at the integration level.

#[test]
fn test_validate_referenceable_rejects_dml_and_ddl() {
    for sql in [
        "INSERT INTO foo VALUES (1)",
        "  insert into foo (a) values (1)",
        "UPDATE foo SET x=1",
        "DELETE FROM foo",
        "TRUNCATE foo",
        "CREATE TABLE foo (id int)",
        "ALTER TABLE foo ADD COLUMN x int",
        "DROP TABLE foo",
        // Reads that are NOT a single Statement::Query cannot be wrapped either.
        "EXPLAIN SELECT 1",
        "SHOW search_path",
        "SELECT 1; SELECT 2",
    ] {
        let err = validate_referenceable("c1", sql, PG).unwrap_err();
        assert!(matches!(err, CellError::NotATable { .. }), "{sql}: {err:?}");
    }
}

#[test]
fn test_validate_referenceable_accepts_selects_and_reads() {
    for sql in [
        "SELECT * FROM foo",
        "WITH x AS (SELECT 1) SELECT * FROM x",
        "VALUES (1)",
        "SELECT ';' AS s;",
    ] {
        let body = validate_referenceable("c1", sql, PG).unwrap_or_else(|e| panic!("{sql}: {e:?}"));
        assert!(!body.is_empty());
    }
    assert!(validate_referenceable("c1", "", PG).is_err());
}

#[test]
fn test_cell_error_serialization_roundtrip() {
    let err = CellError::CyclicDependency {
        cycle: vec!["a".into(), "b".into(), "a".into()],
        hint: "circular reference detected".into(),
    };
    let json = serde_json::to_string(&err).unwrap();
    let parsed: CellError = serde_json::from_str(&json).unwrap();
    match parsed {
        CellError::CyclicDependency { cycle, .. } => {
            assert_eq!(cycle, vec!["a", "b", "a"]);
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn test_all_error_variants_serialize() {
    let errors = vec![
        CellError::NotExecuted {
            cell_id: "c1".into(),
            hint: "run first".into(),
        },
        CellError::TextNotReferencable {
            cell_id: "c2".into(),
            message: "text only".into(),
        },
        CellError::StaleReference {
            cell_id: "c3".into(),
            hint: "re-run".into(),
        },
        CellError::QueryError {
            message: "fail".into(),
            sql_error: "syntax error".into(),
        },
        CellError::NotATable {
            cell_id: "c5".into(),
            message: "not a table".into(),
        },
        CellError::NotExecutable {
            cell_id: "c6".into(),
            message: "can't exec".into(),
        },
        CellError::UnresolvedRef {
            cell_id: "c7".into(),
            ref_name: "foo".into(),
            hint: "check name".into(),
        },
        CellError::ConnectionLost {
            message: "connection lost".into(),
        },
    ];
    for err in &errors {
        let json = serde_json::to_string(err).unwrap();
        let parsed: CellError = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }
}
