use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::error::{LucentError, LucentErrorKind};

pub fn new_framed<S>(stream: S) -> Framed<S, LengthDelimitedCodec>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    Framed::new(stream, LengthDelimitedCodec::new())
}

pub async fn write_message<T, S>(
    framed: &mut Framed<S, LengthDelimitedCodec>,
    message: &T,
) -> Result<(), LucentError>
where
    T: serde::Serialize,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let bytes = bincode::serialize(message).map_err(|e| {
        LucentError::new(LucentErrorKind::Internal, format!("serialize failed: {e}"))
    })?;
    framed
        .send(bytes.into())
        .await
        .map_err(|e| LucentError::new(LucentErrorKind::Internal, format!("write failed: {e}")))
}

pub async fn read_message<T, S>(
    framed: &mut Framed<S, LengthDelimitedCodec>,
) -> Result<Option<T>, LucentError>
where
    T: serde::de::DeserializeOwned,
    S: AsyncRead + AsyncWrite + Unpin,
{
    match framed.next().await {
        Some(Ok(bytes)) => {
            let message = bincode::deserialize(&bytes).map_err(|e| {
                LucentError::new(
                    LucentErrorKind::Internal,
                    format!("deserialize failed: {e}"),
                )
            })?;
            Ok(Some(message))
        }
        Some(Err(e)) => Err(LucentError::new(
            LucentErrorKind::Internal,
            format!("read failed: {e}"),
        )),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ConnectionConfig, ConnectionId, WorkerRequest};
    use uuid::Uuid;

    #[tokio::test]
    async fn round_trips_a_message_over_a_duplex_stream() {
        let (client_stream, server_stream) = tokio::io::duplex(4096);
        let mut client = new_framed(client_stream);
        let mut server = new_framed(server_stream);

        let request = WorkerRequest::Connect {
            connection_id: ConnectionId(Uuid::new_v4()),
            config: ConnectionConfig::new("postgres")
                .with("host", "localhost")
                .with("port", "5432")
                .with("user", "postgres")
                .with("database", "postgres")
                .with("ssl_mode", "prefer"),
        };

        write_message(&mut client, &request).await.unwrap();
        let received: WorkerRequest = read_message(&mut server).await.unwrap().unwrap();

        match received {
            WorkerRequest::Connect { config, .. } => {
                assert_eq!(config.get("host"), Some("localhost"))
            }
            _ => panic!("expected Connect variant"),
        }
    }

    #[tokio::test]
    async fn read_message_returns_none_after_clean_close() {
        let (client_stream, server_stream) = tokio::io::duplex(4096);
        let mut server = new_framed(server_stream);
        drop(client_stream);

        let received: Option<WorkerRequest> = read_message(&mut server).await.unwrap();
        assert!(received.is_none());
    }
}
