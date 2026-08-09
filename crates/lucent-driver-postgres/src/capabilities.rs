//! PostgreSQL's capability declaration.
//!
//! Every value here is verified against a running server by
//! `tests/capabilities_test.rs`, not asserted from documentation.

use lucent_protocol::{
    AuthModel, CancelMode, DriverCapabilities, NamespaceModel, PagingStyle, ReadOnlyMode,
    SqlDialect, StringLiteralStyle, TimeoutSupport,
};

pub fn postgres() -> DriverCapabilities {
    DriverCapabilities {
        id: "postgres".into(),
        display_name: "PostgreSQL".into(),
        sql_dialect: SqlDialect::PostgreSql,
        // database → schema → object; one namespace path segment.
        namespace_model: NamespaceModel::DbSchemaObject,
        // BEGIN; SET TRANSACTION READ ONLY. The engine refuses the write.
        readonly: ReadOnlyMode::TransactionScoped,
        statement_timeout: TimeoutSupport::Statement,
        // tokio_postgres::CancelToken — a real out-of-band cancel request.
        cancel: CancelMode::Native,
        paging: PagingStyle::LimitOffset,
        identifier_quote: '"',
        // standard_conforming_strings = on since 9.1.
        string_literal: StringLiteralStyle::StandardConforming,
        auth: AuthModel::UserPassword,
    }
}
