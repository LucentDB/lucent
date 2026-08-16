mod capabilities;
mod catalog;
mod error;
mod framing;
mod messages;

pub use capabilities::{
    AuthModel, CancelMode, DriverCapabilities, NamespaceModel, PagingStyle, ReadOnlyMode,
    SqlDialect, StringLiteralStyle, TimeoutSupport,
};
pub use catalog::{
    CatalogRequest, CatalogResult, ColumnDetail, ColumnPath, ForeignKey, ForeignKeyTarget,
    Namespace, NamespacePath, ObjectDetail, ObjectKind, ObjectProperty, ObjectRef, ObjectSummary,
    PartitionInfo, SearchHit,
};
pub use error::{ErrorContext, LucentError, LucentErrorKind};
pub use framing::{new_codec, new_framed, read_message, write_message, MAX_FRAME_LENGTH};
pub use messages::{
    ColumnMeta, ConnectionConfig, ConnectionId, QueryId, ResultShape, ServerInfo, Value,
    WorkerRequest, WorkerResponse, PROTOCOL_VERSION,
};
