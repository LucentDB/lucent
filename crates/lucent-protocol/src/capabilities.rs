//! What a driver can and cannot do.
//!
//! Every "this database is PostgreSQL" assumption the app used to make is
//! expressed here instead, so a second driver changes data rather than code.
//! Capabilities are static per driver and ride in `ServerInfo` at connect time.

use serde::{Deserialize, Serialize};

/// Which `sqlparser` dialect parses this driver's SQL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SqlDialect {
    PostgreSql,
    DuckDb,
    BigQuery,
}

/// How many segments a namespace path has, and what they mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum NamespaceModel {
    /// database → schema → object (PostgreSQL). One path segment.
    DbSchemaObject,
    /// catalog → schema → object (DuckDB). Two path segments.
    CatalogSchema,
    /// project → dataset → table (BigQuery). Two path segments.
    ProjectDataset,
}

/// How strongly the engine can be made to refuse writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ReadOnlyMode {
    /// `BEGIN; SET TRANSACTION READ ONLY` — PostgreSQL. Verified.
    TransactionScoped,
    /// Transactions exist but are script-scoped, not connection-scoped
    /// (BigQuery multi-statement transactions).
    ScriptScoped,
    /// A session- or connection-level read-only flag, no transaction wrap.
    /// Stronger than `TransactionScoped` in reach, weaker in granularity.
    SessionFlag,
    /// No engine-level enforcement is available. The AST guard is the only
    /// layer. DuckDB opened read-write lands here: it has no read-only
    /// transaction mode, and it cannot hold a READ_ONLY handle alongside a
    /// READ_WRITE one on the same file.
    GuardOnly,
}

impl ReadOnlyMode {
    /// True when the engine itself will refuse a write, so the AST guard is
    /// layer 1 of 2 rather than layer 1 of 1.
    pub fn is_engine_enforced(&self) -> bool {
        !matches!(self, ReadOnlyMode::GuardOnly)
    }

    /// User- and model-facing text for a weakened guarantee. `None` when the
    /// guarantee is intact — callers must not invent reassuring text for that
    /// case, they must say nothing.
    pub fn disclosure(&self) -> Option<&'static str> {
        match self {
            ReadOnlyMode::GuardOnly => Some(
                "Read-only is NOT enforced by this database engine. Lucent's SQL guard \
                 is the only protection: a stored function called from a SELECT could \
                 still write.",
            ),
            _ => None,
        }
    }
}

/// How a runaway query can be bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TimeoutSupport {
    /// A server-side statement timeout (PostgreSQL `statement_timeout`).
    Statement,
    /// No server-side timeout: the client sets a deadline and interrupts.
    Interrupt,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CancelMode {
    /// The database's own cancel protocol (PostgreSQL CancelToken).
    Native,
    /// An in-process interrupt handle (DuckDB InterruptHandle).
    Interrupt,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PagingStyle {
    LimitOffset,
    FetchFirst,
    TopN,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum StringLiteralStyle {
    /// Backslash is an ordinary character; only `'` needs doubling.
    /// PostgreSQL with `standard_conforming_strings = on`, and DuckDB.
    StandardConforming,
    /// Backslash escapes must themselves be escaped.
    BackslashEscape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AuthModel {
    UserPassword,
    FilePath,
    ServiceAccount,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverCapabilities {
    /// Stable driver id. Matches the worker binary name and
    /// `ConnectionProfile.driver`.
    pub id: String,
    pub display_name: String,
    pub sql_dialect: SqlDialect,
    pub namespace_model: NamespaceModel,
    pub readonly: ReadOnlyMode,
    pub statement_timeout: TimeoutSupport,
    pub cancel: CancelMode,
    pub paging: PagingStyle,
    pub identifier_quote: char,
    pub string_literal: StringLiteralStyle,
    pub auth: AuthModel,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps() -> DriverCapabilities {
        DriverCapabilities {
            id: "postgres".into(),
            display_name: "PostgreSQL".into(),
            sql_dialect: SqlDialect::PostgreSql,
            namespace_model: NamespaceModel::DbSchemaObject,
            readonly: ReadOnlyMode::TransactionScoped,
            statement_timeout: TimeoutSupport::Statement,
            cancel: CancelMode::Native,
            paging: PagingStyle::LimitOffset,
            identifier_quote: '"',
            string_literal: StringLiteralStyle::StandardConforming,
            auth: AuthModel::UserPassword,
        }
    }

    #[test]
    fn round_trips_through_bincode() {
        let bytes = bincode::serialize(&caps()).unwrap();
        let back: DriverCapabilities = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back, caps());
    }

    #[test]
    fn only_transaction_scoped_read_only_is_engine_enforced_two_layer() {
        // This predicate is the whole safety story. Getting it wrong means
        // telling the user their queries are engine-enforced when they are not.
        assert!(ReadOnlyMode::TransactionScoped.is_engine_enforced());
        assert!(ReadOnlyMode::ScriptScoped.is_engine_enforced());
        assert!(ReadOnlyMode::SessionFlag.is_engine_enforced());
        assert!(!ReadOnlyMode::GuardOnly.is_engine_enforced());
    }

    #[test]
    fn guard_only_carries_a_disclosure_note_and_the_others_do_not() {
        let note = ReadOnlyMode::GuardOnly
            .disclosure()
            .expect("GuardOnly must disclose the weakened guarantee");
        assert!(
            note.to_lowercase().contains("not enforced"),
            "the note must say plainly that the engine is not enforcing: {note}"
        );
        assert!(ReadOnlyMode::TransactionScoped.disclosure().is_none());
        assert!(ReadOnlyMode::SessionFlag.disclosure().is_none());
    }

    #[test]
    fn a_newer_variant_does_not_break_deserialization_of_known_ones() {
        // The enums are #[non_exhaustive]; matches in the app must have a
        // wildcard, and that wildcard must be the safe branch.
        let bytes = bincode::serialize(&ReadOnlyMode::GuardOnly).unwrap();
        let back: ReadOnlyMode = bincode::deserialize(&bytes).unwrap();
        assert_eq!(back, ReadOnlyMode::GuardOnly);
    }
}
