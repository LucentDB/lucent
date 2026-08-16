use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_util::codec::{Framed, LengthDelimitedCodec};

use crate::error::{LucentError, LucentErrorKind};

/// Maximum size of one IPC frame on the wire, both directions.
///
/// The tokio-util default is 8 MiB (enforced on encode AND decode), which a
/// 500-row batch of 20 KB cells (10 MiB) silently exceeded — the worker
/// swallowed the encode error and the query died with a lying "task exited
/// unexpectedly". 256 MiB covers every realistic batch; the driver truncates
/// single cells > 1 MiB and the worker splits oversized batches (Plan A
/// Tasks A4/A5), so only a pathological single row can exceed it.
pub const MAX_FRAME_LENGTH: usize = 256 * 1024 * 1024;

/// The length-delimited codec used on every worker/app socket, with the
/// raised frame ceiling. Use this — never `LengthDelimitedCodec::new()` — on
/// both ends and both halves of a split stream.
pub fn new_codec() -> LengthDelimitedCodec {
    LengthDelimitedCodec::builder()
        .max_frame_length(MAX_FRAME_LENGTH)
        .new_codec()
}

pub fn new_framed<S>(stream: S) -> Framed<S, LengthDelimitedCodec>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    Framed::new(stream, new_codec())
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

#[cfg(test)]
mod codec_tests {
    use super::*;
    use bytes::BytesMut;
    use tokio_util::codec::{Decoder, Encoder};

    /// C2 regression: the stock `LengthDelimitedCodec::new()` caps frames at
    /// 8 MiB on encode — a 500-row × 20 KB batch is 10 MiB and was silently
    /// dropped, killing wide-row queries with a lying "task exited
    /// unexpectedly" error. `new_codec()` must accept it, and the decode side
    /// must round-trip it. (The codec's `Encoder` item is `Bytes`; `BytesMut`
    /// converts with `.into()`.)
    #[test]
    fn new_codec_accepts_frames_the_default_codec_rejects() {
        let big = BytesMut::from(&vec![0x41u8; 9 * 1024 * 1024][..]);

        let mut default_buf = BytesMut::new();
        let mut default_codec = LengthDelimitedCodec::new();
        assert!(
            default_codec
                .encode(big.clone().into(), &mut default_buf)
                .is_err(),
            "the stock codec must reject a 9 MiB frame (pins the 8 MiB default)"
        );

        let mut buf = BytesMut::new();
        let mut codec = new_codec();
        codec
            .encode(big.clone().into(), &mut buf)
            .expect("new_codec must accept a 9 MiB frame");

        // Decode the length-prefixed frame back out. `Decoder::decode` takes
        // the source buffer only and returns the frame.
        let mut decode_codec = new_codec();
        let decoded = decode_codec
            .decode(&mut buf)
            .expect("decode must succeed")
            .expect("one complete frame must be available");
        assert_eq!(decoded.len(), big.len());
    }
}
