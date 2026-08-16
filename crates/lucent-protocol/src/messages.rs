use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::LucentErrorKind;

/// Bump whenever the wire format changes (new variants, changed fields).
/// Worker and app must agree; the handshake rejects mismatches loudly.
pub const PROTOCOL_VERSION: u32 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConnectionId(pub Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QueryId(pub Uuid);

/// How to reach one database.
///
/// A driver-tagged parameter bag rather than a fixed field set: `host`/`port`
/// mean nothing to a DuckDB file or a BigQuery dataset. Each driver validates
/// the keys it needs, which is why `require` names both the field and the
/// driver in its error.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub driver: String,
    pub params: std::collections::BTreeMap<String, String>,
    /// From the keychain. Never logged — see the `Debug` impl.
    pub secret: Option<String>,
}

impl ConnectionConfig {
    pub fn new(driver: impl Into<String>) -> Self {
        Self {
            driver: driver.into(),
            params: Default::default(),
            secret: None,
        }
    }

    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

    pub fn with_secret(mut self, secret: impl Into<String>) -> Self {
        self.secret = Some(secret.into());
        self
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(String::as_str)
    }

    /// A parameter this driver cannot work without.
    pub fn require(&self, key: &str) -> Result<&str, String> {
        self.get(key).ok_or_else(|| {
            format!(
                "the {} driver requires a {key:?} connection parameter",
                self.driver
            )
        })
    }

    /// Convenience for the common numeric parameter. `None` when absent or
    /// unparseable — never a panic on user-entered text.
    pub fn port(&self) -> Option<u16> {
        self.get("port")?.parse().ok()
    }
}

impl std::fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionConfig")
            .field("driver", &self.driver)
            .field("params", &self.params)
            .field("secret", &self.secret.as_ref().map(|_| "***"))
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: String,
    /// Static per driver. Carried on the connect reply so the app has it before
    /// the first query — the read-only ladder needs it immediately.
    pub capabilities: crate::DriverCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnMeta {
    pub name: String,
    pub type_name: String,
}

/// A single cell value on the wire.
///
/// Drivers decode their native types into these variants. Anything a driver
/// cannot map — arrays, ranges, composites, enums, domains, extension types —
/// becomes `Other`, which carries the source type name alongside the server's
/// own text rendering. That escape hatch is what stops this enum from having
/// to model every provider's type system.
///
/// Temporal conventions (drivers MUST follow these):
/// - `Timestamp { tz: false }` is a wall-clock reading, NOT an instant.
///   `micros` is that wall-clock value interpreted as if UTC.
/// - `Timestamp { tz: true }` is a true instant, normalized to UTC.
/// - `Date` is days since the Unix epoch (1970-01-01).
/// - `Time` is micros since midnight, with no zone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Value {
    Null,
    Bool(bool),
    Int64(i64),
    Float64(f64),
    /// Exact decimal as the server's canonical text. Never an f64 — that would
    /// lose precision. Tolerates `NaN`, `Infinity`, and exponent forms.
    Decimal(String),
    Text(String),
    Binary(Vec<u8>),
    Timestamp {
        micros: i64,
        tz: bool,
    },
    /// Days since the Unix epoch (1970-01-01).
    Date(i32),
    /// Micros since midnight, no zone.
    Time(i64),
    Interval(String),
    Uuid(uuid::Uuid),
    Json(String),
    Other {
        type_name: String,
        text: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ResultShape {
    Tabular {
        columns: Arc<Vec<ColumnMeta>>,
        rows: Vec<Vec<Value>>,
    },
    Affected {
        rows_affected: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WorkerRequest {
    Connect {
        connection_id: ConnectionId,
        config: ConnectionConfig,
    },
    Execute {
        connection_id: ConnectionId,
        query_id: QueryId,
        command: String,
    },
    Cancel {
        connection_id: ConnectionId,
        query_id: QueryId,
    },
    Disconnect {
        connection_id: ConnectionId,
    },
    /// A catalog question. `request_id` is a `QueryId` because catalog replies
    /// travel the same correlation path as query results — see
    /// `ConnectorClient::catalog`.
    Catalog {
        connection_id: ConnectionId,
        request_id: QueryId,
        request: crate::CatalogRequest,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum WorkerResponse {
    Connected {
        connection_id: ConnectionId,
        server_info: ServerInfo,
    },
    ResultBatch {
        query_id: QueryId,
        shape: ResultShape,
        sequence: u32,
        is_final: bool,
    },
    Error {
        kind: LucentErrorKind,
        message: String,
        query_id: Option<QueryId>,
    },
    Cancelled {
        query_id: QueryId,
    },
    Disconnected {
        connection_id: ConnectionId,
    },
    /// A Connect or Disconnect failure. Correlates on `connection_id` so the
    /// app's sync-path oneshot routing can resolve it. The generic `Error`
    /// variant carries `query_id: None` on these paths and was dropped by the
    /// client reader, which turned every auth failure into a 10s "connect
    /// response timed out" (C1).
    ConnectionError {
        connection_id: ConnectionId,
        kind: LucentErrorKind,
        message: String,
    },
    /// Sent by the worker immediately after a valid version+token handshake,
    /// before any Connect request. Lets the client surface a typed mismatch
    /// instead of a generic EOF.
    HandshakeAccepted,
    CatalogResult {
        request_id: QueryId,
        result: crate::CatalogResult,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_value_variant_through_bincode() {
        let variants = vec![
            Value::Null,
            Value::Bool(true),
            Value::Int64(-42),
            Value::Float64(1.5),
            Value::Decimal("1234.56".into()),
            Value::Text("hello".into()),
            Value::Binary(vec![0x00, 0xff, 0x10]),
            Value::Timestamp {
                micros: 1_754_568_896_789_000,
                tz: true,
            },
            Value::Timestamp {
                micros: 1_754_568_896_789_000,
                tz: false,
            },
            Value::Date(20_673),
            Value::Time(45_296_789_000),
            Value::Interval("1 day 02:03:04".into()),
            Value::Uuid(uuid::Uuid::nil()),
            Value::Json(r#"{"a":1}"#.into()),
            Value::Other {
                type_name: "int4range".into(),
                text: "[1,5)".into(),
            },
        ];

        for v in variants {
            let bytes = bincode::serialize(&v).expect("serialize");
            let back: Value = bincode::deserialize(&bytes).expect("deserialize");
            assert_eq!(
                format!("{v:?}"),
                format!("{back:?}"),
                "variant did not survive a bincode round trip"
            );
        }
    }

    #[test]
    fn float_nan_survives_the_wire_as_nan() {
        let bytes = bincode::serialize(&Value::Float64(f64::NAN)).unwrap();
        let back: Value = bincode::deserialize(&bytes).unwrap();
        match back {
            Value::Float64(f) => assert!(f.is_nan(), "NaN must survive as NaN"),
            other => panic!("expected Float64, got {other:?}"),
        }
    }

    #[test]
    fn the_builder_produces_a_driver_tagged_parameter_bag() {
        let config = ConnectionConfig::new("postgres")
            .with("host", "db.internal")
            .with("port", "5432")
            .with_secret("hunter2");

        assert_eq!(config.driver, "postgres");
        assert_eq!(config.get("host"), Some("db.internal"));
        assert_eq!(config.port(), Some(5432));
        assert_eq!(config.get("missing"), None);
    }

    #[test]
    fn a_required_parameter_that_is_missing_names_itself() {
        let config = ConnectionConfig::new("duckdb");
        let err = config.require("path").unwrap_err();
        assert!(err.contains("path"), "the error must name the field: {err}");
        assert!(err.contains("duckdb"), "and the driver: {err}");
    }

    #[test]
    fn debug_redacts_the_secret_and_nothing_else() {
        let config = ConnectionConfig::new("postgres")
            .with("host", "db.internal")
            .with_secret("hunter2");
        let rendered = format!("{config:?}");
        assert!(
            !rendered.contains("hunter2"),
            "secrets must never reach a log line: {rendered}"
        );
        assert!(rendered.contains("db.internal"), "{rendered}");
        assert!(rendered.contains("***"), "{rendered}");
    }

    #[test]
    fn a_non_numeric_port_is_none_rather_than_a_panic() {
        let config = ConnectionConfig::new("postgres").with("port", "not-a-number");
        assert_eq!(config.port(), None);
    }

    #[test]
    fn round_trips_a_connect_request_through_bincode() {
        let request = WorkerRequest::Connect {
            connection_id: ConnectionId(uuid::Uuid::new_v4()),
            config: ConnectionConfig::new("postgres")
                .with("host", "localhost")
                .with("port", "5432")
                .with("user", "postgres")
                .with("database", "postgres")
                .with("ssl_mode", "prefer"),
        };

        let bytes = bincode::serialize(&request).unwrap();
        let decoded: WorkerRequest = bincode::deserialize(&bytes).unwrap();

        match decoded {
            WorkerRequest::Connect { config, .. } => {
                assert_eq!(config.get("host"), Some("localhost"));
                assert_eq!(config.port(), Some(5432));
            }
            _ => panic!("expected Connect variant"),
        }
    }

    #[test]
    fn round_trips_a_tabular_result_batch() {
        let response = WorkerResponse::ResultBatch {
            query_id: QueryId(uuid::Uuid::new_v4()),
            shape: ResultShape::Tabular {
                columns: Arc::new(vec![ColumnMeta {
                    name: "id".to_string(),
                    type_name: "int4".to_string(),
                }]),
                rows: vec![vec![Value::Text("1".into())], vec![Value::Null]],
            },
            sequence: 0,
            is_final: true,
        };

        let bytes = bincode::serialize(&response).unwrap();
        let decoded: WorkerResponse = bincode::deserialize(&bytes).unwrap();

        match decoded {
            WorkerResponse::ResultBatch {
                shape, is_final, ..
            } => match shape {
                ResultShape::Tabular { rows, .. } => {
                    assert_eq!(rows.len(), 2);
                    assert!(is_final);
                }
                other => panic!("expected Tabular shape, got {other:?}"),
            },
            _ => panic!("expected ResultBatch variant"),
        }
    }

    #[test]
    fn round_trips_an_affected_shape() {
        let response = WorkerResponse::ResultBatch {
            query_id: QueryId(uuid::Uuid::new_v4()),
            shape: ResultShape::Affected { rows_affected: 14 },
            sequence: 0,
            is_final: true,
        };
        let bytes = bincode::serialize(&response).unwrap();
        let decoded: WorkerResponse = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerResponse::ResultBatch {
                shape: ResultShape::Affected { rows_affected },
                ..
            } => assert_eq!(rows_affected, 14),
            other => panic!("expected Affected: {other:?}"),
        }
    }

    #[test]
    fn handshake_accepted_roundtrips() {
        let msg = WorkerResponse::HandshakeAccepted;
        let bytes = bincode::serialize(&msg).unwrap();
        let back: WorkerResponse = bincode::deserialize(&bytes).unwrap();
        assert!(matches!(back, WorkerResponse::HandshakeAccepted));
    }

    #[test]
    fn protocol_version_is_seven() {
        assert_eq!(PROTOCOL_VERSION, 7);
    }

    #[test]
    fn connection_error_roundtrips_through_bincode() {
        let msg = WorkerResponse::ConnectionError {
            connection_id: ConnectionId(uuid::Uuid::new_v4()),
            kind: LucentErrorKind::AuthenticationFailed,
            message: "password authentication failed for user \"postgres\"".into(),
        };
        let bytes = bincode::serialize(&msg).unwrap();
        let back: WorkerResponse = bincode::deserialize(&bytes).unwrap();
        match back {
            WorkerResponse::ConnectionError {
                connection_id,
                kind,
                message,
            } => {
                assert_eq!(
                    message,
                    "password authentication failed for user \"postgres\""
                );
                assert!(matches!(kind, LucentErrorKind::AuthenticationFailed));
                assert!(!connection_id.0.is_nil());
            }
            other => panic!("expected ConnectionError, got {other:?}"),
        }
    }

    #[test]
    fn round_trips_a_catalog_request_through_bincode() {
        let request = WorkerRequest::Catalog {
            connection_id: ConnectionId(uuid::Uuid::new_v4()),
            request_id: QueryId(uuid::Uuid::new_v4()),
            request: crate::CatalogRequest::ListNamespaces,
        };
        let bytes = bincode::serialize(&request).unwrap();
        let decoded: WorkerRequest = bincode::deserialize(&bytes).unwrap();
        assert!(matches!(decoded, WorkerRequest::Catalog { .. }));
    }

    #[test]
    fn catalog_replies_carry_the_request_id_so_they_correlate_like_queries() {
        // The whole routing design rests on this: a catalog reply is keyed by
        // the same QueryId type the `pending` map already uses.
        let request_id = QueryId(uuid::Uuid::new_v4());
        let response = WorkerResponse::CatalogResult {
            request_id,
            result: crate::CatalogResult::Ddl("CREATE VIEW v AS SELECT 1".into()),
        };
        let bytes = bincode::serialize(&response).unwrap();
        let decoded: WorkerResponse = bincode::deserialize(&bytes).unwrap();
        match decoded {
            WorkerResponse::CatalogResult {
                request_id: got, ..
            } => {
                assert_eq!(got, request_id);
            }
            other => panic!("expected CatalogResult, got {other:?}"),
        }
    }
}
