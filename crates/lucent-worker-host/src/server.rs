use std::collections::HashMap;
use std::sync::Arc;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use lucent_protocol::{
    new_framed, read_message, write_message, ConnectionId, QueryId, ResultShape, WorkerRequest,
    WorkerResponse, MAX_FRAME_LENGTH,
};
use tokio::sync::mpsc;

use crate::connector::{Connector, ExecutionEvent};
use crate::ipc::IpcListener;

pub async fn serve<C>(
    listener: IpcListener,
    handshake_token: String,
    connector: C,
) -> std::io::Result<()>
where
    C: Connector + 'static,
{
    let stream = listener.accept().await?;
    let mut framed = new_framed(stream);

    let received_version: Option<u32> = read_message(&mut framed).await.ok().flatten();
    if received_version != Some(lucent_protocol::PROTOCOL_VERSION) {
        let msg = format!(
            "protocol version mismatch: app sent {received_version:?}, worker expects {}",
            lucent_protocol::PROTOCOL_VERSION
        );
        eprintln!("{msg}");
        // Best-effort: the connection is about to close; the client handles a
        // dropped socket as EOF after surfacing this typed error.
        let _ = write_message(
            &mut framed,
            &WorkerResponse::Error {
                kind: lucent_protocol::LucentErrorKind::Protocol,
                message: msg,
                query_id: None,
            },
        )
        .await;
        return Ok(());
    }
    let received_token: Option<String> = read_message(&mut framed).await.ok().flatten();
    if received_token.as_deref() != Some(handshake_token.as_str()) {
        eprintln!("worker: handshake token mismatch");
        // Best-effort: the connection is about to close; the client handles a
        // dropped socket as EOF after surfacing this typed error.
        let _ = write_message(
            &mut framed,
            &WorkerResponse::Error {
                kind: lucent_protocol::LucentErrorKind::Protocol,
                message: "handshake token mismatch".into(),
                query_id: None,
            },
        )
        .await;
        return Ok(());
    }
    let _ = write_message(&mut framed, &WorkerResponse::HandshakeAccepted).await;

    let connector = Arc::new(connector);
    let mut batch_rxs: HashMap<QueryId, mpsc::Receiver<ExecutionEvent>> = HashMap::new();
    let mut sequences: HashMap<QueryId, u32> = HashMap::new();
    // Which connection each in-flight query belongs to, so a failed query task
    // can be evicted AND its DB-side statement cancelled without touching
    // sibling queries on the same socket.
    let mut query_conns: HashMap<QueryId, ConnectionId> = HashMap::new();

    // Replies from spawned handlers (catalog RPCs). Catalog queries hit the
    // database, so answering them inline in the select loop would stall result
    // batches for every in-flight query sharing this socket.
    let (oob_tx, mut oob_rx) = mpsc::channel::<WorkerResponse>(16);

    loop {
        // Build a FuturesUnordered of all active batch receivers inside a
        // block so the borrow is released before the select body mutates batch_rxs.
        // Each future yields (query_id, Option<event>): the None event arm means
        // the sender dropped without a terminal event, and the query id is what
        // lets us evict just that query instead of draining the whole set.
        let next_batch = {
            let mut futs: FuturesUnordered<_> = batch_rxs
                .iter_mut()
                .map(|(&qid, rx)| Box::pin(async move { (qid, rx.recv().await) }))
                .collect();

            async move {
                if futs.is_empty() {
                    std::future::pending::<Option<(QueryId, Option<ExecutionEvent>)>>().await
                } else {
                    // The set is non-empty, so poll resolves with a value; it
                    // can only yield None once every receiver has resolved.
                    futs.next().await
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
                        // Spawned with a timeout, exactly like catalog: a slow/unreachable DB
                        // must not stall batches for every other query sharing this socket
                        // (C8). Replies flow through the out-of-band channel; the app
                        // correlates them by connection_id.
                        let connector = connector.clone();
                        let oob_tx = oob_tx.clone();
                        tokio::spawn(async move {
                            const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
                            let response = match tokio::time::timeout(
                                CONNECT_TIMEOUT,
                                connector.connect(connection_id, config),
                            )
                            .await
                            {
                                Ok(Ok(server_info)) => WorkerResponse::Connected {
                                    connection_id,
                                    server_info,
                                },
                                Ok(Err(e)) => WorkerResponse::ConnectionError {
                                    connection_id,
                                    kind: e.kind,
                                    message: e.message,
                                },
                                Err(_) => WorkerResponse::ConnectionError {
                                    connection_id,
                                    kind: lucent_protocol::LucentErrorKind::Timeout,
                                    message: format!("connect timed out after {CONNECT_TIMEOUT:?}"),
                                },
                            };
                            let _ = oob_tx.send(response).await;
                        });
                    }
                    WorkerRequest::Execute { connection_id, query_id, command } => {
                        let (tx, rx) = mpsc::channel(4);
                        batch_rxs.insert(query_id, rx);
                        sequences.insert(query_id, 0);
                        query_conns.insert(query_id, connection_id);
                        let connector = connector.clone();
                        tokio::spawn(async move {
                            connector.execute(connection_id, query_id, command, tx).await;
                        });
                    }
                    WorkerRequest::Cancel { connection_id, query_id } => {
                        // The DB-native cancel opens a fresh connection and can block for the
                        // OS TCP timeout (~75s). Spawned with a timeout so it cannot stall the
                        // select loop (C8).
                        let connector = connector.clone();
                        let oob_tx = oob_tx.clone();
                        tokio::spawn(async move {
                            const CANCEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
                            let response = match tokio::time::timeout(
                                CANCEL_TIMEOUT,
                                connector.cancel(connection_id, query_id),
                            )
                            .await
                            {
                                Ok(Ok(())) => WorkerResponse::Cancelled { query_id },
                                Ok(Err(e)) => WorkerResponse::Error {
                                    kind: e.kind,
                                    message: e.message,
                                    query_id: Some(query_id),
                                },
                                Err(_) => WorkerResponse::Error {
                                    kind: lucent_protocol::LucentErrorKind::Timeout,
                                    message: format!("cancel timed out after {CANCEL_TIMEOUT:?}"),
                                    query_id: Some(query_id),
                                },
                            };
                            let _ = oob_tx.send(response).await;
                        });
                    }
                    WorkerRequest::Disconnect { connection_id } => {
                        // Same treatment as Connect (C8).
                        let connector = connector.clone();
                        let oob_tx = oob_tx.clone();
                        tokio::spawn(async move {
                            const DISCONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
                            let response = match tokio::time::timeout(
                                DISCONNECT_TIMEOUT,
                                connector.disconnect(connection_id),
                            )
                            .await
                            {
                                Ok(Ok(())) => WorkerResponse::Disconnected { connection_id },
                                Ok(Err(e)) => WorkerResponse::ConnectionError {
                                    connection_id,
                                    kind: e.kind,
                                    message: e.message,
                                },
                                Err(_) => WorkerResponse::ConnectionError {
                                    connection_id,
                                    kind: lucent_protocol::LucentErrorKind::Timeout,
                                    message: format!("disconnect timed out after {DISCONNECT_TIMEOUT:?}"),
                                },
                            };
                            let _ = oob_tx.send(response).await;
                        });
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
                    Some((query_id, Some(ExecutionEvent::Batch(shape, is_final)))) => {
                        let chunks =
                            split_shape_for_frame(query_id, shape, is_final, MAX_FRAME_LENGTH as u64);
                        let mut seq = sequences.get(&query_id).copied().unwrap_or(0);
                        for (chunk_shape, chunk_final) in chunks {
                            let response = WorkerResponse::ResultBatch {
                                query_id,
                                shape: chunk_shape,
                                sequence: seq,
                                is_final: chunk_final,
                            };
                            if write_message(&mut framed, &response).await.is_err() {
                                eprintln!("worker: write failed — dropping query {query_id:?}");
                                break;
                            }
                            seq += 1;
                        }
                        if is_final {
                            sequences.remove(&query_id);
                            batch_rxs.remove(&query_id);
                            query_conns.remove(&query_id);
                        } else {
                            sequences.insert(query_id, seq);
                        }
                    }
                    Some((query_id, Some(ExecutionEvent::Failed(e)))) => {
                        let response = WorkerResponse::Error {
                            kind: e.kind,
                            message: e.message,
                            query_id: Some(query_id),
                        };
                        if write_message(&mut framed, &response).await.is_err() {
                            // The socket is dead — end the loop rather than spinning.
                            eprintln!("worker: write failed — dropping query {query_id:?}");
                            break;
                        }
                        batch_rxs.remove(&query_id);
                        sequences.remove(&query_id);
                        query_conns.remove(&query_id);
                    }
                    Some((query_id, None)) => {
                        // The sender dropped without a terminal event (e.g. the
                        // spawned execute task panicked). Evict ONLY this query:
                        // siblings share the socket and must keep streaming.
                        // Reply FIRST — then cancel the DB-side statement in a
                        // spawned task so a slow/unreachable DB (cancel opens a
                        // fresh connection) cannot stall sibling queries on the
                        // single-threaded select loop.
                        let qid = query_conns.get(&query_id).copied();
                        batch_rxs.remove(&query_id);
                        sequences.remove(&query_id);
                        query_conns.remove(&query_id);
                        if write_message(&mut framed, &WorkerResponse::Error {
                            kind: lucent_protocol::LucentErrorKind::Internal,
                            message: "query task exited unexpectedly".into(),
                            query_id: Some(query_id),
                        })
                        .await
                        .is_err()
                        {
                            // The socket is dead — end the loop; the best-effort
                            // cancel below is moot because the DB connection drops
                            // with the process.
                            eprintln!("worker: write failed — dropping query {query_id:?}");
                            break;
                        }
                        if let Some(qid) = qid {
                            let connector = connector.clone();
                            tokio::spawn(async move {
                                const CANCEL_TIMEOUT: std::time::Duration =
                                    std::time::Duration::from_secs(5);
                                match tokio::time::timeout(CANCEL_TIMEOUT, connector.cancel(qid, query_id)).await {
                                    Ok(Ok(())) => {}
                                    Ok(Err(e)) => eprintln!(
                                        "worker: best-effort cancel of evicted query {query_id:?} failed: {e}"
                                    ),
                                    Err(_) => eprintln!(
                                        "worker: best-effort cancel of evicted query {query_id:?} timed out"
                                    ),
                                }
                            });
                        }
                    }
                    None => {
                        // All receivers resolved; the next loop iteration rebuilds
                        // the set (empty ⇒ pending forever). Unreachable in
                        // practice — kept for match exhaustiveness.
                    }
                }
            }
            oob = oob_rx.recv() => {
                match oob {
                    Some(response) => {
                        let id = match &response {
                            WorkerResponse::Connected { connection_id, .. }
                            | WorkerResponse::ConnectionError { connection_id, .. }
                            | WorkerResponse::Disconnected { connection_id } => {
                                format!("connection {connection_id:?}")
                            }
                            WorkerResponse::Cancelled { query_id }
                            | WorkerResponse::CatalogResult {
                                request_id: query_id,
                                ..
                            } => format!("query {query_id:?}"),
                            WorkerResponse::Error {
                                query_id: Some(query_id),
                                ..
                            } => format!("query {query_id:?}"),
                            _ => "reply".into(),
                        };
                        if write_message(&mut framed, &response).await.is_err() {
                            // The socket is dead — end the loop rather than spinning.
                            eprintln!("worker: write failed — dropping {id}");
                            break;
                        }
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

/// Split an oversized result batch into frame-sized chunks, in order.
///
/// C2: the IPC frame ceiling applies to the serialized `ResultBatch`. A batch
/// of many medium cells can exceed it even though every cell is individually
/// small (the driver caps cells at 1 MiB plus a truncation marker — Task A4).
/// Splitting preserves streaming: the client only appends rows until
/// `is_final`, so chunked batches are transparent. `max_frame_bytes` is a
/// parameter so unit tests can exercise splitting with tiny thresholds; the
/// caller passes `MAX_FRAME_LENGTH`.
fn split_shape_for_frame(
    query_id: QueryId,
    shape: ResultShape,
    is_final: bool,
    max_frame_bytes: u64,
) -> Vec<(ResultShape, bool)> {
    fn serialized_size(query_id: QueryId, shape: &ResultShape, is_final: bool) -> u64 {
        bincode::serialized_size(&WorkerResponse::ResultBatch {
            query_id,
            shape: shape.clone(),
            sequence: 0,
            is_final,
        })
        .unwrap_or(u64::MAX)
    }

    if serialized_size(query_id, &shape, is_final) <= max_frame_bytes {
        return vec![(shape, is_final)];
    }
    match shape {
        ResultShape::Tabular { columns, rows } if rows.len() > 1 => {
            let mut first_rows = rows;
            let second_rows = first_rows.split_off(first_rows.len() / 2);
            let mut chunks = split_shape_for_frame(
                query_id,
                ResultShape::Tabular {
                    columns: columns.clone(),
                    rows: first_rows,
                },
                false,
                max_frame_bytes,
            );
            chunks.extend(split_shape_for_frame(
                query_id,
                ResultShape::Tabular {
                    columns,
                    rows: second_rows,
                },
                is_final,
                max_frame_bytes,
            ));
            chunks
        }
        // A single row that still exceeds the frame (pathological: > 256
        // columns of 1 MiB cells). Send it anyway — the write error is
        // logged by the caller — rather than dropping the terminal event
        // silently.
        other => vec![(other, is_final)],
    }
}

#[cfg(test)]
mod split_tests {
    use super::*;
    use lucent_protocol::{ColumnMeta, Value};
    use std::sync::Arc;

    fn tabular(n: usize) -> ResultShape {
        ResultShape::Tabular {
            columns: Arc::new(vec![ColumnMeta {
                name: "c".into(),
                type_name: "text".into(),
            }]),
            rows: vec![vec![Value::Text("x".repeat(1000))]; n],
        }
    }

    fn serialized_size(shape: &ResultShape, is_final: bool) -> u64 {
        bincode::serialized_size(&WorkerResponse::ResultBatch {
            query_id: QueryId(uuid::Uuid::nil()),
            shape: shape.clone(),
            sequence: 0,
            is_final,
        })
        .unwrap()
    }

    #[test]
    fn small_batches_are_not_split() {
        let chunks = split_shape_for_frame(QueryId(uuid::Uuid::nil()), tabular(2), true, 100_000);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].1, "is_final must survive unsplit");
    }

    #[test]
    fn oversized_batches_split_until_every_chunk_fits() {
        let chunks = split_shape_for_frame(QueryId(uuid::Uuid::nil()), tabular(64), true, 20_000);
        assert!(
            chunks.len() > 1,
            "64 × ~1 KB rows at a 20 KB frame must split"
        );
        for (shape, is_final) in &chunks {
            assert!(
                serialized_size(shape, *is_final) <= 20_000,
                "chunk of {} bytes exceeds the frame",
                serialized_size(shape, *is_final)
            );
        }
        assert_eq!(
            chunks.last().map(|(_, f)| *f),
            Some(true),
            "only the last chunk may be final"
        );
        assert!(
            chunks[..chunks.len() - 1].iter().all(|(_, f)| !*f),
            "no earlier chunk may be final"
        );
        let total: usize = chunks
            .iter()
            .map(|(s, _)| match s {
                ResultShape::Tabular { rows, .. } => rows.len(),
                other => panic!("expected Tabular, got {other:?}"),
            })
            .sum();
        assert_eq!(total, 64, "no rows may be lost or duplicated");
    }

    #[test]
    fn affected_shapes_never_split() {
        let chunks = split_shape_for_frame(
            QueryId(uuid::Uuid::nil()),
            ResultShape::Affected { rows_affected: 7 },
            true,
            1,
        );
        assert_eq!(chunks.len(), 1);
        assert!(matches!(
            chunks[0].0,
            ResultShape::Affected { rows_affected: 7 }
        ));
    }
}
