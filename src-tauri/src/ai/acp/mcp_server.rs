//! The MCP surface of the `lucent-db-tools-mcp` binary: pure request → response
//! mapping over JSON-RPC 2.0 (JSON-lines). Kept free of sockets and stdio so
//! it is unit-testable without a subprocess; the binary wires stdin/stdout and
//! the bridge socket around it.

use crate::ai::tools::AiToolContext;

/// The static description of one MCP tool.
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Builds the four Lucent tools' schemas from a real tool context (used by
/// the capstone conformance test, which compares against `LucentToolEnum`
/// directly).
pub fn lucent_tools_schema(ctx: AiToolContext) -> Vec<ToolSchema> {
    crate::ai::tools::all_tools(ctx)
        .iter()
        .map(|t| ToolSchema {
            name: t.name().to_string(),
            description: t.description(),
            input_schema: t.parameters(),
        })
        .collect()
}

/// The tool schemas are static — `description()`/`parameters()` never touch
/// the connection — so the binary builds them once from a neutral context.
pub fn static_tool_schemas() -> Vec<ToolSchema> {
    fn dummy_ctx() -> AiToolContext {
        AiToolContext {
            db: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            connection_id: None,
            capabilities: None,
            config: crate::ai::config::AiConfig::default(),
            schema_graph: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            embedder: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            reranker: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        }
    }
    lucent_tools_schema(dummy_ctx())
}

/// Maps one MCP JSON-RPC request to its response envelope.
///
/// `tools/call` is only validated and routed here (the binary fills `content`
/// after the bridge socket round-trip, so the response carries a `_pending`
/// marker when the tool exists). Everything else is answered in full.
pub fn handle_mcp_request(req: serde_json::Value, tools: &[ToolSchema]) -> serde_json::Value {
    // Notifications (id-less requests) must not be answered (JSON-RPC §5.2):
    // returning Null is the binary's "skip the envelope" signal. Non-object
    // garbage is not a notification — it still gets an error envelope so the
    // client sees the parse failure.
    if req.get("id").is_none() && req.is_object() {
        return serde_json::Value::Null;
    }
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let params = req
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let error = |code: i64, msg: String| serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": msg } });
    let result =
        |r: serde_json::Value| serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": r });
    match method {
        "initialize" => {
            let version = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or("2024-11-05");
            result(serde_json::json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "lucent-db-tools-mcp", "version": env!("CARGO_PKG_VERSION") },
            }))
        }
        "ping" => result(serde_json::json!({})),
        "logging/setLevel" => result(serde_json::json!({})),
        "resources/list" => result(serde_json::json!({ "resources": [] })),
        "prompts/list" => result(serde_json::json!({ "prompts": [] })),
        "tools/list" => result(serde_json::json!({
            "tools": tools.iter().map(|t| serde_json::json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })).collect::<Vec<_>>(),
        })),
        "tools/call" => {
            let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            match tools.iter().find(|t| t.name == name) {
                Some(_) => serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "content": [], "isError": false, "_pending": true },
                }),
                None => error(-32602, format!("unknown tool: {name}")),
            }
        }
        _ => error(-32601, "method not found".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools() -> Vec<ToolSchema> {
        static_tool_schemas()
    }

    #[test]
    fn initialize_echoes_protocol_version() {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": { "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } },
        });
        let resp = handle_mcp_request(req, &tools());
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(
            resp["result"]["capabilities"]["tools"],
            serde_json::json!({})
        );
        assert_eq!(resp["result"]["serverInfo"]["name"], "lucent-db-tools-mcp");
    }

    #[test]
    fn initialize_defaults_version_when_missing() {
        let req =
            serde_json::json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {} });
        let resp = handle_mcp_request(req, &tools());
        assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    }

    #[test]
    fn tools_list_returns_the_four_tools_in_order() {
        let req = serde_json::json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" });
        let resp = handle_mcp_request(req, &tools());
        let listed: Vec<String> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            listed,
            vec![
                "search_schema",
                "get_objects_info",
                "run_readonly_query",
                "preview_dml"
            ]
        );
        // Each schema is a JSON-schema object; search_schema requires "query".
        let search = &resp["result"]["tools"][0];
        assert_eq!(search["inputSchema"]["type"], "object");
        assert_eq!(search["inputSchema"]["required"][0], "query");
    }

    #[test]
    fn tools_call_unknown_tool_is_an_error() {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": { "name": "nope", "arguments": {} },
        });
        let resp = handle_mcp_request(req, &tools());
        assert_eq!(resp["id"], 4);
        assert_eq!(resp["error"]["code"], -32602);
        assert!(resp["error"]["message"].as_str().unwrap().contains("nope"));
    }

    #[test]
    fn tools_call_known_tool_routes_to_pending() {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": { "name": "run_readonly_query", "arguments": { "sql": "select 1" } },
        });
        let resp = handle_mcp_request(req, &tools());
        assert_eq!(resp["id"], 5);
        assert_eq!(resp["result"]["_pending"], true);
        assert_eq!(resp["result"]["isError"], false);
    }

    #[test]
    fn non_object_input_yields_error_with_null_id() {
        // A garbage line is not JSON-RPC; the mapper must answer with an error
        // and a null id (the client cannot correlate a response to anything).
        let resp = handle_mcp_request(serde_json::json!(42), &tools());
        assert_eq!(resp["error"]["code"], -32601);
        assert!(resp["id"].is_null());
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let req = serde_json::json!({ "jsonrpc": "2.0", "id": 6, "method": "unsupported/method", "params": {} });
        let resp = handle_mcp_request(req, &tools());
        assert_eq!(resp["error"]["code"], -32601);
        assert_eq!(resp["id"], 6);
    }

    #[test]
    fn ping_and_discovery_methods_return_empty_results() {
        let req = serde_json::json!({ "jsonrpc": "2.0", "id": 7, "method": "ping" });
        let resp = handle_mcp_request(req, &tools());
        assert_eq!(resp["id"], 7);
        assert_eq!(resp["result"], serde_json::json!({}));

        let req = serde_json::json!({ "jsonrpc": "2.0", "id": 8, "method": "resources/list" });
        let resp = handle_mcp_request(req, &tools());
        assert_eq!(resp["id"], 8);
        assert_eq!(resp["result"]["resources"], serde_json::json!([]));
    }

    #[test]
    fn initialized_notification_has_no_id() {
        let req = serde_json::json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let resp = handle_mcp_request(req, &tools());
        assert!(
            resp.get("id").is_none(),
            "notifications must not be answered"
        );
    }

    #[test]
    fn schemas_match_lucent_tool_enum_exactly() {
        // Schema parity with the rig path is a hard contract: the agent's
        // prompt sees the same tool shapes whether tools run via rig or MCP.
        let ctx = AiToolContext {
            db: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            connection_id: None,
            capabilities: None,
            config: crate::ai::config::AiConfig::default(),
            schema_graph: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            embedder: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            reranker: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        };
        let schemas = lucent_tools_schema(ctx);
        let direct = crate::ai::tools::all_tools(crate::ai::tools::AiToolContext {
            db: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            connection_id: None,
            capabilities: None,
            config: crate::ai::config::AiConfig::default(),
            schema_graph: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            embedder: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            reranker: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        });
        assert_eq!(schemas.len(), direct.len());
        for (s, t) in schemas.iter().zip(direct.iter()) {
            assert_eq!(s.name, t.name());
            assert_eq!(s.description, t.description());
            assert_eq!(s.input_schema, t.parameters());
        }
    }
}
