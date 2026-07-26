use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

use lucent_protocol::{new_framed, read_message, write_message, WorkerRequest, WorkerResponse};
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

    let received_token: Option<String> = read_message(&mut framed).await.ok().flatten();
    if received_token.as_deref() != Some(handshake_token.as_str()) {
        return Ok(());
    }

    let connector = Arc::new(connector);
    // TODO(multi-connection): support concurrent queries via HashMap<QueryId, Receiver>
    let mut batch_rx: Option<mpsc::Receiver<ExecutionEvent>> = None;
    let mut active_query = None;
    let mut sequence: u32 = 0;

    loop {
        let next_batch = async {
            match batch_rx.as_mut() {
                Some(rx) => rx.recv().await,
                None => std::future::pending().await,
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
                        let response = match connector.connect(connection_id, config).await {
                            Ok(server_info) => WorkerResponse::Connected { connection_id, server_info },
                            Err(e) => WorkerResponse::Error { kind: e.kind, message: e.message },
                        };
                        let _ = write_message(&mut framed, &response).await;
                    }
                    WorkerRequest::Execute { connection_id, query_id, command } => {
                        let (tx, rx) = mpsc::channel(4);
                        batch_rx = Some(rx);
                        active_query = Some(query_id);
                        sequence = 0;
                        let connector = connector.clone();
                        tokio::spawn(async move {
                            connector.execute(connection_id, query_id, command, tx).await;
                        });
                    }
                    WorkerRequest::Cancel { connection_id, query_id } => {
                        let response = match connector.cancel(connection_id, query_id).await {
                            Ok(()) => WorkerResponse::Cancelled { query_id },
                            Err(e) => WorkerResponse::Error { kind: e.kind, message: e.message },
                        };
                        let _ = write_message(&mut framed, &response).await;
                    }
                    WorkerRequest::Disconnect { connection_id } => {
                        let response = match connector.disconnect(connection_id).await {
                            Ok(()) => WorkerResponse::Disconnected { connection_id },
                            Err(e) => WorkerResponse::Error { kind: e.kind, message: e.message },
                        };
                        let _ = write_message(&mut framed, &response).await;
                    }
                }
            }
            batch = next_batch => {
                match batch {
                    Some(ExecutionEvent::Batch(shape, is_final)) => {
                        if let Some(query_id) = active_query {
                            let response = WorkerResponse::ResultBatch { query_id, shape, sequence, is_final };
                            sequence += 1;
                            let _ = write_message(&mut framed, &response).await;
                            if is_final {
                                batch_rx = None;
                                active_query = None;
                            }
                        }
                    }
                    Some(ExecutionEvent::Failed(e)) => {
                        let response = WorkerResponse::Error { kind: e.kind, message: e.message };
                        let _ = write_message(&mut framed, &response).await;
                        batch_rx = None;
                        active_query = None;
                    }
                    None => {}
                }
            }
        }
    }

    Ok(())
}
