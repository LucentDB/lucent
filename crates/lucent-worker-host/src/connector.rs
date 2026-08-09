use async_trait::async_trait;
use lucent_protocol::{
    CatalogRequest, CatalogResult, ConnectionConfig, ConnectionId, LucentError, QueryId,
    ResultShape, ServerInfo,
};
use tokio::sync::mpsc;

pub enum ExecutionEvent {
    Batch(ResultShape, bool),
    Failed(LucentError),
}

pub type BatchSender = mpsc::Sender<ExecutionEvent>;

#[async_trait]
pub trait Connector: Send + Sync {
    async fn connect(
        &self,
        connection_id: ConnectionId,
        config: ConnectionConfig,
    ) -> Result<ServerInfo, LucentError>;

    async fn execute(
        &self,
        connection_id: ConnectionId,
        query_id: QueryId,
        command: String,
        sender: BatchSender,
    );

    async fn cancel(
        &self,
        connection_id: ConnectionId,
        query_id: QueryId,
    ) -> Result<(), LucentError>;

    async fn disconnect(&self, connection_id: ConnectionId) -> Result<(), LucentError>;

    /// Answer a catalog question in normalized types.
    ///
    /// Deliberately has no default implementation: a driver that does not
    /// implement this must fail to compile, not fail at runtime in the schema
    /// browser. Same fail-closed discipline as the read-only guard.
    async fn catalog(
        &self,
        connection_id: ConnectionId,
        request: CatalogRequest,
    ) -> Result<CatalogResult, LucentError>;
}
