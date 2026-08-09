//! Generation-side dialect handling.
//!
//! A parse registry is not enough here: this code *emits* SQL. `LIMIT/OFFSET`,
//! `ILIKE ... ESCAPE`, `::text`, and identifier quoting are all syntax choices
//! that differ across engines, so each driver gets a builder rather than a
//! flag.

use lucent_protocol::DriverCapabilities;

pub trait SqlBuilder: Send + Sync {
    fn quote_identifier(&self, s: &str) -> String;
    fn quote_string(&self, s: &str) -> String;
    /// Append the engine's row-window clause. `limit` and `offset` are clamped
    /// to zero — a negative bound is a syntax error on some engines and silently
    /// unbounded on others.
    fn page(&self, sql: &str, limit: i64, offset: i64) -> String;
    /// A predicate matching rows where `col` contains `needle`, case-insensitively.
    /// `col` arrives already quoted; `needle` is raw user text.
    fn case_insensitive_contains(&self, col: &str, needle: &str) -> String;
    /// A predicate matching rows where `col` starts with `needle`, case-insensitively.
    fn case_insensitive_starts_with(&self, col: &str, needle: &str) -> String;
    /// A predicate matching rows where `col` ends with `needle`, case-insensitively.
    fn case_insensitive_ends_with(&self, col: &str, needle: &str) -> String;
    /// Cast an already-quoted column reference to text.
    fn cast_to_text(&self, col: &str) -> String;
}

/// Resolve the builder for a connection.
///
/// Falls back to the Postgres builder for an unknown driver. Unlike the
/// read-only guard, a wrong builder produces a failed query the user sees
/// immediately — it cannot silently execute a write — so a usable default beats
/// refusing to render the grid.
pub fn for_driver(capabilities: &DriverCapabilities) -> Box<dyn SqlBuilder> {
    match capabilities.id.as_str() {
        "postgres" => Box::new(PostgresSqlBuilder),
        _ => {
            log::warn!(
                "no SQL builder for driver {:?}; falling back to PostgreSQL syntax",
                capabilities.id
            );
            Box::new(PostgresSqlBuilder)
        }
    }
}

pub struct PostgresSqlBuilder;

/// Escape `%`, `_`, and `\` in a value before it goes into a LIKE pattern.
fn escape_like_pattern(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

impl SqlBuilder for PostgresSqlBuilder {
    fn quote_identifier(&self, s: &str) -> String {
        crate::sql_quote::quote_identifier(s)
    }

    fn quote_string(&self, s: &str) -> String {
        crate::sql_quote::quote_string(s)
    }

    fn page(&self, sql: &str, limit: i64, offset: i64) -> String {
        format!("{sql} LIMIT {} OFFSET {}", limit.max(0), offset.max(0))
    }

    fn case_insensitive_contains(&self, col: &str, needle: &str) -> String {
        format!(
            "{} ILIKE {} ESCAPE '\\'",
            self.cast_to_text(col),
            self.quote_string(&format!("%{}%", escape_like_pattern(needle)))
        )
    }

    fn case_insensitive_starts_with(&self, col: &str, needle: &str) -> String {
        format!(
            "{} ILIKE {} ESCAPE '\\'",
            self.cast_to_text(col),
            self.quote_string(&format!("{}%", escape_like_pattern(needle)))
        )
    }

    fn case_insensitive_ends_with(&self, col: &str, needle: &str) -> String {
        format!(
            "{} ILIKE {} ESCAPE '\\'",
            self.cast_to_text(col),
            self.quote_string(&format!("%{}", escape_like_pattern(needle)))
        )
    }

    fn cast_to_text(&self, col: &str) -> String {
        format!("{col}::text")
    }
}

#[cfg(test)]
mod tests {
    use super::{PostgresSqlBuilder, SqlBuilder};

    fn pg() -> PostgresSqlBuilder {
        PostgresSqlBuilder
    }

    #[test]
    fn quotes_identifiers_by_doubling_the_quote_character() {
        assert_eq!(pg().quote_identifier("we\"ird"), "\"we\"\"ird\"");
        assert_eq!(pg().quote_identifier("plain"), "\"plain\"");
    }

    #[test]
    fn standard_conforming_strings_leave_backslashes_alone() {
        // Escaping backslashes corrupts data on round-trip: `hello\world`
        // would become `hello\\world`. This is a regression guard, not a style
        // preference — an earlier export copy had exactly this bug.
        assert_eq!(pg().quote_string("O'Brien"), "'O''Brien'");
        assert_eq!(pg().quote_string("hello\\world"), "'hello\\world'");
    }

    #[test]
    fn pages_with_limit_offset() {
        assert_eq!(
            pg().page("SELECT * FROM t", 50, 100),
            "SELECT * FROM t LIMIT 50 OFFSET 100"
        );
    }

    #[test]
    fn negative_page_bounds_are_clamped_not_emitted() {
        // `LIMIT -1` is a syntax error on some engines and unbounded on others.
        assert_eq!(pg().page("SELECT 1", -5, -10), "SELECT 1 LIMIT 0 OFFSET 0");
    }

    #[test]
    fn case_insensitive_contains_escapes_wildcards_in_the_needle() {
        // A search for "50%" must match a literal percent sign, not act as a
        // wildcard.
        let sql = pg().case_insensitive_contains("\"col\"", "50%");
        assert!(sql.contains("ILIKE"), "{sql}");
        assert!(sql.contains("ESCAPE"), "{sql}");
        assert!(sql.contains("\\%"), "the % must be escaped: {sql}");
    }

    #[test]
    fn case_insensitive_starts_with_escapes_wildcards_in_the_needle() {
        let sql = pg().case_insensitive_starts_with("\"col\"", "50%");
        assert!(sql.contains("ILIKE"), "{sql}");
        assert!(sql.contains("ESCAPE"), "{sql}");
        assert!(sql.contains("\\%"), "the % must be escaped: {sql}");
        assert!(sql.ends_with("'50\\%%' ESCAPE '\\'"), "{sql}");
    }

    #[test]
    fn case_insensitive_ends_with_escapes_wildcards_in_the_needle() {
        let sql = pg().case_insensitive_ends_with("\"col\"", "50%");
        assert!(sql.contains("ILIKE"), "{sql}");
        assert!(sql.contains("ESCAPE"), "{sql}");
        assert!(sql.contains("\\%"), "the % must be escaped: {sql}");
        assert!(sql.ends_with("'%50\\%' ESCAPE '\\'"), "{sql}");
    }

    #[test]
    fn cast_to_text_is_a_builder_concern_not_a_hardcoded_double_colon() {
        assert_eq!(pg().cast_to_text("\"col\""), "\"col\"::text");
    }
}
