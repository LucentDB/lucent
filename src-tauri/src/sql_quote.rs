//! PostgreSQL literal and identifier quoting.
//!
//! Reached through `crate::sql_builder::PostgresSqlBuilder`, not called
//! directly by feature code — quoting is a per-driver decision.
//!
//! PostgreSQL uses standard-conforming string literals by default
//! (`standard_conforming_strings = on`), which means the backslash is an
//! ordinary character — only the single quote needs escaping (by doubling).
//! Escaping backslashes (as an earlier export copy did) corrupts data on
//! round-trip: `hello\world` would become `hello\\world`.

/// Quote a string as a SQL literal: wrap in single quotes, doubling any
/// embedded single quote. Backslashes are left untouched.
pub fn quote_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Quote an identifier (table/column name): wrap in double quotes, doubling any
/// embedded double quote.
pub fn quote_identifier(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_single_quote() {
        assert_eq!(quote_string("O'Brien"), "'O''Brien'");
    }

    #[test]
    fn preserves_backslash() {
        // Standard-conforming strings: backslash is literal, never doubled.
        assert_eq!(quote_string("hello\\world"), "'hello\\world'");
    }

    #[test]
    fn quotes_identifier() {
        assert_eq!(quote_identifier("we\"ird"), "\"we\"\"ird\"");
    }
}
