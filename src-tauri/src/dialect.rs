//! Parse-side dialect registry.
//!
//! Resolving a dialect can fail: `SqlDialect` is `#[non_exhaustive]`, so a
//! driver on a newer protocol can declare one this build does not know. Every
//! caller must treat `None` as "reject", never as "use something permissive" —
//! the read-only guard is the reason.

use lucent_protocol::SqlDialect;
use sqlparser::dialect::{BigQueryDialect, Dialect, DuckDbDialect, PostgreSqlDialect};

/// The `sqlparser` dialect for a driver's declared SQL dialect.
///
/// `None` means this build cannot parse that dialect. Fail closed.
pub fn parser_for(dialect: SqlDialect) -> Option<Box<dyn Dialect>> {
    match dialect {
        SqlDialect::PostgreSql => Some(Box::new(PostgreSqlDialect {})),
        SqlDialect::DuckDb => Some(Box::new(DuckDbDialect {})),
        SqlDialect::BigQuery => Some(Box::new(BigQueryDialect {})),
        // Deliberately no wildcard fallback to GenericDialect: a dialect this
        // build does not know is a dialect it cannot safely validate.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use lucent_protocol::SqlDialect;

    use super::parser_for;

    #[test]
    fn every_declared_dialect_resolves_to_a_parser() {
        // A driver may only declare a dialect the app can actually parse.
        // If this fails, the driver's capability declaration is a lie.
        for d in [
            SqlDialect::PostgreSql,
            SqlDialect::DuckDb,
            SqlDialect::BigQuery,
        ] {
            assert!(parser_for(d).is_some(), "no parser for {d:?}");
        }
    }

    #[test]
    fn the_postgres_parser_still_parses_postgres_specific_syntax() {
        let dialect = parser_for(SqlDialect::PostgreSql).unwrap();
        assert!(
            sqlparser::parser::Parser::parse_sql(dialect.as_ref(), "SELECT a::text FROM t").is_ok(),
            ":: casts are Postgres syntax and must still parse"
        );
    }
}
