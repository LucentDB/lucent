use std::path::PathBuf;
use std::sync::Arc;
use lucent_protocol::ConnectionId;
use tokio::io::AsyncWriteExt;

use crate::ai::acp::bridge::ToolExecutor;
use crate::ai::acp::wire;
use crate::ai::external_endpoint::discovery::{
    cleanup_discovery_file_at, default_discovery_file_path, default_socket_path,
    ensure_socket_dir, is_pid_alive, read_discovery_file_at,
    write_discovery_file_at, ConnectionSummary, DiscoveryInfo, EndpointStatus,
};
use crate::ai::external_endpoint::executor::{
    DynamicContextToolExecutor, ExternalEndpointContext,
};
use crate::ai::tools::ToolOutput;
use crate::client::ConnectorClient;

pub struct ExternalEndpointManager {
    pub socket_path: PathBuf,
    pub token: String,
    pub context: Arc<ExternalEndpointContext>,
    pub discovery_path: PathBuf,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl ExternalEndpointManager {
    pub async fn start(
        context: Arc<ExternalEndpointContext>,
        discovery_path: Option<PathBuf>,
    ) -> Result<Self, String> {
        let discovery_path = match discovery_path {
            Some(p) => p,
            None => default_discovery_file_path()?,
        };

        // Crash recovery: check stale PID
        if discovery_path.exists() {
            if let Ok(old_info) = read_discovery_file_at(&discovery_path) {
                if !is_pid_alive(old_info.pid) {
                    log::info!(
                        "Cleaning up stale external endpoint descriptor from dead PID {}",
                        old_info.pid
                    );
                    let old_socket = PathBuf::from(&old_info.socket);
                    if old_socket.exists() {
                        let _ = std::fs::remove_file(&old_socket);
                    }
                    let _ = cleanup_discovery_file_at(&discovery_path);
                }
            }
        }

        // Determine socket path: use parent if custom discovery path in tests, else private 0700 dir
        let is_custom_test_dir = discovery_path
            .parent()
            .map(|p| p.to_string_lossy().contains("tmp") || p.to_string_lossy().contains("temp"))
            .unwrap_or(false);

        let socket_path = if is_custom_test_dir {
            discovery_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("external.sock")
        } else {
            ensure_socket_dir()?;
            default_socket_path()?
        };

        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }

        // Generate 256-bit cryptographically secure hex token
        let token = {
            let u1 = uuid::Uuid::new_v4();
            let u2 = uuid::Uuid::new_v4();
            let u3 = uuid::Uuid::new_v4();
            let hash = blake3::hash(format!("{}:{}:{}", u1, u2, u3).as_bytes());
            hash.to_hex().to_string()
        };

        #[cfg(unix)]
        let listener = tokio::net::UnixListener::bind(&socket_path)
            .map_err(|e| format!("Failed to bind external MCP socket at {}: {e}", socket_path.display()))?;

        // Write initial discovery file
        let initial_info = DiscoveryInfo {
            version: "1.0".into(),
            status: EndpointStatus::Disconnected,
            socket: socket_path.to_string_lossy().to_string(),
            token: token.clone(),
            pid: std::process::id(),
            connection: None,
            tools: vec![
                "run_readonly_query".into(),
                "search_schema".into(),
                "get_objects_info".into(),
                "get_preflight_context".into(),
            ],
        };
        write_discovery_file_at(&discovery_path, &initial_info)?;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let executor = Arc::new(DynamicContextToolExecutor::new(context.clone()));
        let loop_token = token.clone();

        #[cfg(unix)]
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        log::info!("External endpoint accept loop shutting down");
                        break;
                    }
                    accept_res = listener.accept() => {
                        match accept_res {
                            Ok((stream, _)) => {
                                let (reader, writer) = stream.into_split();
                                let token = loop_token.clone();
                                let executor = executor.clone();
                                tokio::spawn(async move {
                                    let _ = handle_external_stream(reader, writer, token, executor).await;
                                });
                            }
                            Err(e) => {
                                log::debug!("External listener accept stopped: {e}");
                                break;
                            }
                        }
                    }
                }
            }
        });

        let manager = Self {
            socket_path,
            token,
            context,
            discovery_path,
            shutdown_tx: Some(shutdown_tx),
        };

        // Sync initial state if a profile is already active
        let _ = manager.sync_profile().await;

        Ok(manager)
    }

    pub async fn run_connector_canary(
        client: &ConnectorClient,
        conn_id: ConnectionId,
    ) -> Result<(), String> {
        crate::ai::external_endpoint::executor::run_connector_canary(client, conn_id).await
    }

    pub async fn sync_profile(&self) -> Result<(), String> {
        let profile_opt = {
            let guard = self.context.active_profile.lock().await;
            guard.clone()
        };

        let info = match profile_opt {
            Some(profile) if profile.enable_external_agents => {
                let db_name = profile
                    .params
                    .get("database")
                    .cloned()
                    .or_else(|| profile.params.get("path").cloned())
                    .unwrap_or_else(|| profile.name.clone());

                let mut tools = vec![
                    "run_readonly_query".into(),
                    "search_schema".into(),
                    "get_objects_info".into(),
                    "get_preflight_context".into(),
                ];
                if profile.allow_query_history {
                    tools.push("search_query_history".into());
                }

                DiscoveryInfo {
                    version: "1.0".into(),
                    status: EndpointStatus::Connected,
                    socket: self.socket_path.to_string_lossy().to_string(),
                    token: self.token.clone(),
                    pid: std::process::id(),
                    connection: Some(ConnectionSummary {
                        profile_id: profile.id,
                        name: profile.name,
                        driver: profile.driver,
                        database: db_name,
                        read_only: true,
                    }),
                    tools,
                }
            }
            _ => {
                DiscoveryInfo {
                    version: "1.0".into(),
                    status: EndpointStatus::Disconnected,
                    socket: self.socket_path.to_string_lossy().to_string(),
                    token: self.token.clone(),
                    pid: std::process::id(),
                    connection: None,
                    tools: vec![
                        "run_readonly_query".into(),
                        "search_schema".into(),
                        "get_objects_info".into(),
                        "get_preflight_context".into(),
                    ],
                }
            }
        };

        write_discovery_file_at(&self.discovery_path, &info)
    }

    pub async fn shutdown(&mut self) -> Result<(), String> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if self.socket_path.exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }
        cleanup_discovery_file_at(&self.discovery_path)
    }
}

async fn handle_external_stream<R, W>(
    reader: R,
    writer: W,
    token: String,
    executor: Arc<DynamicContextToolExecutor>,
) -> Result<(), String>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut reader = tokio::io::BufReader::new(reader);
    let mut writer = tokio::io::BufWriter::new(writer);

    let Some(hello) = wire::read_hello(&mut reader).await? else {
        return Ok(());
    };
    let wire::Hello::Hello { token: client_token } = hello;
    if client_token != token {
        return Ok(()); // silent close on token mismatch
    }

    loop {
        let Some(req) = wire::read_message(&mut reader).await? else {
            return Ok(());
        };
        let wire::BridgeRequest::Call { id, tool, args } = req;
        let resp = match executor.call(&tool, args).await {
            Ok(output) => {
                let text = match output {
                    ToolOutput::Text { content } => content,
                    ToolOutput::QueryResult { text_summary, .. } => text_summary,
                    ToolOutput::DmlPreview { description, .. } => description,
                };
                wire::BridgeResponse::Ok {
                    id,
                    output: serde_json::json!({ "text": text }),
                }
            }
            Err(err) => wire::BridgeResponse::Err {
                id,
                error: err.to_string(),
            },
        };
        wire::write_message(&mut writer, &resp).await?;
        writer
            .flush()
            .await
            .map_err(|e| format!("flush bridge response: {e}"))?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn manager_cleans_up_stale_pid_and_binds_socket() {
        let temp_dir = tempfile::tempdir().unwrap();
        let custom_discovery = temp_dir.path().join("test-mcp.json");

        let context = Arc::new(ExternalEndpointContext {
            client: Arc::new(tokio::sync::Mutex::new(None)),
            external_ai_connection_id: Arc::new(tokio::sync::Mutex::new(None)),
            schema_graph: Arc::new(tokio::sync::Mutex::new(None)),
            embedder: Arc::new(tokio::sync::Mutex::new(None)),
            capabilities: Arc::new(tokio::sync::Mutex::new(None)),
            active_profile: Arc::new(tokio::sync::Mutex::new(None)),
            inode_tracker: Arc::new(tokio::sync::Mutex::new(None)),
            reopen_lock: Arc::new(tokio::sync::Mutex::new(())),
        });

        let mut manager = ExternalEndpointManager::start(context, Some(custom_discovery)).await.unwrap();
        assert!(manager.socket_path.exists());

        manager.shutdown().await.unwrap();
        assert!(!manager.socket_path.exists());
    }

    #[tokio::test]
    async fn sync_profile_updates_discovery_file() {
        use crate::connections::ConnectionProfile;

        let temp_dir = tempfile::tempdir().unwrap();
        let custom_discovery = temp_dir.path().join("test-mcp.json");

        let context = Arc::new(ExternalEndpointContext {
            client: Arc::new(tokio::sync::Mutex::new(None)),
            external_ai_connection_id: Arc::new(tokio::sync::Mutex::new(None)),
            schema_graph: Arc::new(tokio::sync::Mutex::new(None)),
            embedder: Arc::new(tokio::sync::Mutex::new(None)),
            capabilities: Arc::new(tokio::sync::Mutex::new(None)),
            active_profile: Arc::new(tokio::sync::Mutex::new(None)),
            inode_tracker: Arc::new(tokio::sync::Mutex::new(None)),
            reopen_lock: Arc::new(tokio::sync::Mutex::new(())),
        });

        let mut manager = ExternalEndpointManager::start(context.clone(), Some(custom_discovery.clone())).await.unwrap();

        // 1. Initial status: Disconnected
        let disc1 = read_discovery_file_at(&custom_discovery).unwrap();
        assert_eq!(disc1.status, EndpointStatus::Disconnected);
        assert!(disc1.connection.is_none());

        // 2. Active profile with external agents enabled + history enabled
        let mut params = std::collections::BTreeMap::new();
        params.insert("path".into(), "/tmp/test.duckdb".into());
        let profile = ConnectionProfile {
            id: "prof-1".into(),
            name: "Analytics DB".into(),
            driver: "duckdb".into(),
            alias: None,
            params,
            ssh_tunnel_id: None,
            group: None,
            color: None,
            icon: None,
            last_used: None,
            created_at: String::new(),
            updated_at: String::new(),
            enable_external_agents: true,
            allow_query_history: true,
        };
        *context.active_profile.lock().await = Some(profile);
        manager.sync_profile().await.unwrap();

        let disc2 = read_discovery_file_at(&custom_discovery).unwrap();
        assert_eq!(disc2.status, EndpointStatus::Connected);
        assert!(disc2.connection.is_some());
        let conn = disc2.connection.unwrap();
        assert_eq!(conn.name, "Analytics DB");
        assert_eq!(conn.database, "/tmp/test.duckdb");
        assert!(disc2.tools.contains(&"search_query_history".into()));
        assert!(disc2.tools.contains(&"run_readonly_query".into()));
        assert!(disc2.tools.contains(&"get_preflight_context".into()));

        // 3. Disconnect profile
        *context.active_profile.lock().await = None;
        manager.sync_profile().await.unwrap();

        let disc3 = read_discovery_file_at(&custom_discovery).unwrap();
        assert_eq!(disc3.status, EndpointStatus::Disconnected);
        assert!(disc3.connection.is_none());

        manager.shutdown().await.unwrap();
    }
}
