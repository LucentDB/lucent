//! DuckDB's capability declaration.
//!
//! Every value is verified by `tests/readonly_reality_test.rs` against a real
//! database, not asserted from documentation. Three of them contradict what the
//! source spec assumed; the tests are why we know.

use lucent_protocol::{
    AuthModel, CancelMode, DriverCapabilities, NamespaceModel, PagingStyle, ReadOnlyMode,
    SqlDialect, StringLiteralStyle, TimeoutSupport,
};

/// `read_only` reflects how the connection was opened, because it changes the
/// strength of the read-only guarantee — the one capability here that is not
/// static per driver.
pub fn duckdb(read_only: bool) -> DriverCapabilities {
    DriverCapabilities {
        id: "duckdb".into(),
        display_name: "DuckDB".into(),
        sql_dialect: SqlDialect::DuckDb,
        // catalog → schema → object. `memory.main`, `mydb.main`.
        namespace_model: NamespaceModel::CatalogSchema,
        readonly: if read_only {
            // access_mode = READ_ONLY: the engine refuses every write for the
            // whole connection. Broader than Postgres's per-transaction
            // guarantee, and coarser — the editor cannot write either.
            ReadOnlyMode::SessionFlag
        } else {
            // No SET TRANSACTION READ ONLY exists, so transaction-scoped
            // enforcement is unavailable; the AST guard is the only
            // protection, and Plan C's disclosure path says so out loud.
            // (A READ_ONLY handle CAN coexist with this one within the same
            // process — verified in tests/readonly_reality_test.rs — but
            // engine enforcement only exists when the connection itself was
            // opened read-only, so GuardOnly stands for read-write opens.)
            ReadOnlyMode::GuardOnly
        },
        // No server-side statement timeout exists. The driver sets a
        // client-side deadline and fires the interrupt handle.
        statement_timeout: TimeoutSupport::Interrupt,
        cancel: CancelMode::Interrupt,
        paging: PagingStyle::LimitOffset,
        identifier_quote: '"',
        // Same as Postgres: backslash is an ordinary character in a literal.
        string_literal: StringLiteralStyle::StandardConforming,
        // A file path, not a username and password.
        auth: AuthModel::FilePath,
    }
}
