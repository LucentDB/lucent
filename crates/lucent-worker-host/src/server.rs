use std::collections::HashMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use lucent_protocol::{
    new_framed, read_message, write_message, QueryId, WorkerRequest, WorkerResponse,
};
use tokio::net::UnixListener;
use tokio::sync::mpsc;

use crate::connector::{Connector, ExecutionEvent};

pub fn bind(socket_path: impl AsRef<Path>) -> std::io::Result<UnixListener> {
    let _ = std::fs::remove_file(&socket_path);
    let listener = UnixListener::bind(&socket_path)?;
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o700))?;
    Ok(listener)
}

pub async fn serve<C>(
    listener: UnixListener,
    handshake_token: String,
    connector: C,
) -> std::io::Result<()>
where
    C: Connector + 'static,
{
    let (stream, _addr) = listener.accept().await?;
    let mut framed = new_framed(stream);

    let received_version: Option<u32> = read_message(&mut framed).await.ok().flatten();
    if received_version != Some(lucent_protocol::PROTOCOL_VERSION) {
        eprintln!(
            "protocol version mismatch: app sent {received_version:?}, worker expects {}",
            lucent_protocol::PROTOCOL_VERSION
        );
        return Ok(());
    }
    let received_token: Option<String> = read_message(&mut framed).await.ok().flatten();
    if received_token.as_deref() != Some(handshake_token.as_str()) {
        return Ok(());
    }

    let connector = Arc::new(connector);
    let mut batch_rxs: HashMap<QueryId, mpsc::Receiver<ExecutionEvent>> = HashMap::new();
    let mut sequences: HashMap<QueryId, u32> = HashMap::new();

    // Replies from spawned handlers (catalog RPCs). Catalog queries hit the
    // database, so answering them inline in the select loop would stall result
    // batches for every in-flight query sharing this socket.
    let (oob_tx, mut oob_rx) = mpsc::channel::<WorkerResponse>(16);

    loop {
        // Build a FuturesUnordered of all active batch receivers inside a
        // block so the borrow is released before the select body mutates batch_rxs.
        let next_batch = {
            let mut futs: FuturesUnordered<_> = batch_rxs
                .iter_mut()
                .map(|(&qid, rx)| {
                    let qid = qid;
                    Box::pin(async move { rx.recv().await.map(|ev| (qid, ev)) })
                })
                .collect();

            async move {
                if futs.is_empty() {
                    std::future::pending::<Option<(QueryId, ExecutionEvent)>>().await
                } else {
                    futs.next().await.flatten()
                }
            }
        };

        tokio::select! {
            request = read_message::<WorkerRequest, _>(&mut framed) => {
                let request = match request {
                    Ok(Some(r)) => r,
                    _ => break,
                };

                match request {
                    WorkerRequest::Shutdown => break,
                    WorkerRequest::Connect { connection_id, config } => {
                        const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
                        let response = match tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(connection_id, config)).await {
                            Ok(Ok(server_info)) => WorkerResponse::Connected { connection_id, server_info },
                            Ok(Err(e)) => WorkerResponse::Error { kind: e.kind, message: e.message, query_id: None },
                            Err(_) => WorkerResponse::Error {
                                kind: lucent_protocol::LucentErrorKind::Timeout,
                                message: format!("connect timed out after {CONNECT_TIMEOUT:?}"),
                                query_id: None,
                            },
                        };
                        let _ = write_message(&mut framed, &response).await;
                    }
                    WorkerRequest::Execute { connection_id, query_id, command } => {
                        let (tx, rx) = mpsc::channel(4);
                        batch_rxs.insert(query_id, rx);
                        sequences.insert(query_id, 0);
                        let connector = connector.clone();
                        tokio::spawn(async move {
                            connector.execute(connection_id, query_id, command, tx).await;
                        });
                    }
                    WorkerRequest::Cancel { connection_id, query_id } => {
                        let response = match connector.cancel(connection_id, query_id).await {
                            Ok(()) => WorkerResponse::Cancelled { query_id },
                            Err(e) => WorkerResponse::Error {
                                kind: e.kind,
                                message: e.message,
                                query_id: Some(query_id),
                            },
                        };
                        let _ = write_message(&mut framed, &response).await;
                    }
                    WorkerRequest::Disconnect { connection_id } => {
                        let response = match connector.disconnect(connection_id).await {
                            Ok(()) => WorkerResponse::Disconnected { connection_id },
                            Err(e) => WorkerResponse::Error {
                                kind: e.kind,
                                message: e.message,
                                query_id: None,
                            },
                        };
                        let _ = write_message(&mut framed, &response).await;
                    }
                    WorkerRequest::Catalog { connection_id, request_id, request } => {
                        let connector = connector.clone();
                        let oob_tx = oob_tx.clone();
                        tokio::spawn(async move {
                            let response = match connector.catalog(connection_id, request).await {
                                Ok(result) => WorkerResponse::CatalogResult { request_id, result },
                                // Reuse `query_id` for the request id so the
                                // app's existing error correlation applies
                                // unchanged.
                                Err(e) => WorkerResponse::Error {
                                    kind: e.kind,
                                    message: e.message,
                                    query_id: Some(request_id),
                                },
                            };
                            let _ = oob_tx.send(response).await;
                        });
                    }
                    // `WorkerRequest` is #[non_exhaustive]: a newer app could
                    // send a variant this worker does not know. Ignore it
                    // rather than killing the loop for every other connection.
                    other => {
                        eprintln!("worker: ignoring unsupported request {other:?}");
                    }
                }
            }
            batch = next_batch => {
                match batch {
                    Some((query_id, ExecutionEvent::Batch(shape, is_final))) => {
                        let seq = sequences.get(&query_id).copied().unwrap_or(0);
                        let response = WorkerResponse::ResultBatch {
                            query_id, shape, sequence: seq, is_final,
                        };
                        sequences.insert(query_id, seq + 1);
                        let _ = write_message(&mut framed, &response).await;
                        if is_final {
                            batch_rxs.remove(&query_id);
                            sequences.remove(&query_id);
                        }
                    }
                    Some((query_id, ExecutionEvent::Failed(e))) => {
                        let response = WorkerResponse::Error {
                            kind: e.kind,
                            message: e.message,
                            query_id: Some(query_id),
                        };
                        let _ = write_message(&mut framed, &response).await;
                        batch_rxs.remove(&query_id);
                        sequences.remove(&query_id);
                    }
                    None => {
                        // The sender dropped without a terminal event (e.g. the spawned
                        // execute task panicked). Clean up so the closed receiver can't
                        // resolve immediately and busy-spin the select loop, and the
                        // client's pending oneshot isn't orphaned.
                        for (qid, rx) in batch_rxs.drain() {
                            let _ = rx; // receiver dropped here closes the channel
                            sequences.remove(&qid);
                            // The app-side execute() waits on a oneshot — resolve it with an
                            // error so it doesn't hang forever.
                            let _ = write_message(&mut framed, &WorkerResponse::Error {
                                kind: lucent_protocol::LucentErrorKind::Internal,
                                message: "worker task exited unexpectedly".into(),
                                query_id: Some(qid),
                            }).await;
                        }
                    }
                }
            }
            oob = oob_rx.recv() => {
                match oob {
                    Some(response) => {
                        let _ = write_message(&mut framed, &response).await;
                    }
                    // This loop holds a sender, so `None` is unreachable.
                    // Treat it as shutdown rather than spinning.
                    None => break,
                }
            }
        }
    }

    Ok(())
}
