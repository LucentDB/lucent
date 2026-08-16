use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LucentErrorKind {
    Protocol,
    ConnectionRefused,
    AuthenticationFailed,
    QuerySyntaxError,
    QueryCancelled,
    Timeout,
    WorkerCrashed,
    Internal,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ErrorContext {
    pub connection_id: Option<String>,
    pub query_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LucentError {
    pub kind: LucentErrorKind,
    pub message: String,
    pub context: ErrorContext,
}

impl LucentError {
    pub fn new(kind: LucentErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            context: ErrorContext::default(),
        }
    }
}

impl std::fmt::Display for LucentErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protocol => write!(f, "Protocol"),
            Self::ConnectionRefused => write!(f, "Connection refused"),
            Self::AuthenticationFailed => write!(f, "Authentication failed"),
            Self::QuerySyntaxError => write!(f, "Query syntax error"),
            Self::QueryCancelled => write!(f, "Query cancelled"),
            Self::Timeout => write!(f, "Timeout"),
            Self::WorkerCrashed => write!(f, "Worker process crashed"),
            Self::Internal => write!(f, "Internal error"),
        }
    }
}

impl std::fmt::Display for LucentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for LucentError {}
