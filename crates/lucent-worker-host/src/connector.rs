use async_trait::async_trait;
use lucent_protocol::{
    ConnectionConfig, ConnectionId, LucentError, QueryId, ResultShape, ServerInfo,
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
}
