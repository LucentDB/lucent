//! `lucent-db-tools-mcp` — Lucent's four database tools as an MCP stdio
//! server for external ACP agents.
//!
//! The agent (spawned via ACP `session/new` `mcpServers`) starts this binary
//! and speaks MCP JSON-RPC (JSON-lines over stdio) to it. Every `tools/call`
//! is forwarded over a token-authenticated local socket to the main Lucent
//! process, where the real tool stack (guardrails included) executes it.
//! This binary is stateless and thin: no schema graph, no DB client, no
//! guard logic — all of that stays where the state lives.
//!
//! Usage: `lucent-db-tools-mcp --socket <path> --token <hex>`
//! Env fallback: `LUCENT_ACP_SOCKET`, `LUCENT_ACP_TOKEN` (argv wins).

use lucent_lib::ai::acp::{mcp_server, wire};

/// Splits the bridge connection into owned read/write halves. The concrete
/// transport follows the platform IPC pattern: Unix domain socket on Unix,
/// named pipe on Windows.
async fn connect_socket(
    socket: &str,
) -> Result<
    (
        Box<dyn tokio::io::AsyncRead + Send + Unpin>,
        Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    ),
    Box<dyn std::error::Error>,
> {
    #[cfg(unix)]
    {
        let stream = tokio::net::UnixStream::connect(socket)
            .await
            .map_err(|e| format!("connect bridge socket {socket}: {e}"))?;
        let (r, w) = stream.into_split();
        Ok((Box::new(r), Box::new(w)))
    }
    #[cfg(windows)]
    {
        let client = tokio::net::windows::named_pipe::ClientOptions::new()
            .open(socket)
            .map_err(|e| format!("connect bridge pipe {socket}: {e}"))?;
        let (r, w) = tokio::io::split(client);
        Ok((Box::new(r), Box::new(w)))
    }
}

fn parse_tool_arguments(tool: &str, raw_args: Option<&str>) -> serde_json::Value {
    let Some(raw) = raw_args else {
        return serde_json::json!({});
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return serde_json::json!({});
    }
    // If it's valid JSON object, use it directly
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if v.is_object() {
            return v;
        }
    }
    // Otherwise fallback based on tool type so raw strings from shell work smoothly
    match tool {
        "run_readonly_query" => serde_json::json!({ "sql": trimmed }),
        "search_schema" => serde_json::json!({ "query": trimmed }),
        "preview_dml" => serde_json::json!({ "sql": trimmed, "description": "" }),
        "get_objects_info" => serde_json::json!({ "objects": [{ "name": trimmed }] }),
        _ => serde_json::json!({ "arg": trimmed }),
    }
}

/// The self-documentation a CLI-first agent reaches for before guessing.
///
/// pi-class runtimes have no `tools/list` to read, and the four tools' JSON
/// schemas are already available here (`mcp_server::static_tool_schemas`) — so
/// print them rather than making the model discover the argument shape by
/// trial and error.
fn usage() -> String {
    let mut out = String::from(
        "lucent-tool — Lucent's database tools, for agents without MCP.\n\
         \n\
         USAGE:\n\
         \x20 lucent-tool <tool> '<json_arguments>'\n\
         \x20 lucent-tool <tool> \"<primary field>\"   # shorthand, see below\n\
         \x20 lucent-tool help\n\
         \n\
         Every call runs against the database connection open in Lucent right now,\n\
         through Lucent's guardrails (read-only enforcement, row caps, DML approval).\n\
         \n\
         SHORTHAND: a bare, non-JSON argument is read as the tool's primary field —\n\
         `run_readonly_query`/`preview_dml` take the SQL, `search_schema` the query,\n\
         `get_objects_info` an object name. Avoids nesting JSON in shell quoting:\n\
         \x20 lucent-tool run_readonly_query \"SELECT count(*) FROM public.orders\"\n\
         \n\
         TOOLS:\n",
    );
    for t in lucent_lib::ai::acp::mcp_server::static_tool_schemas() {
        let schema = serde_json::to_string(&t.input_schema).unwrap_or_default();
        let summary = t
            .description
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        out.push_str(&format!(
            "\n  {}\n    {}\n    args: {}\n",
            t.name, summary, schema
        ));
    }
    out
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mut socket = None;
    let mut token = None;
    let mut cli_tool = None;
    let mut cli_tool_args = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--socket" => {
                socket = args.get(i + 1).cloned();
                i += 2;
            }
            "--token" => {
                token = args.get(i + 1).cloned();
                i += 2;
            }
            "call" => {
                cli_tool = args.get(i + 1).cloned();
                cli_tool_args = args.get(i + 2).cloned();
                break;
            }
            "search_schema" | "get_objects_info" | "run_readonly_query" | "preview_dml" => {
                cli_tool = Some(args[i].clone());
                cli_tool_args = args.get(i + 1).cloned();
                break;
            }
            "help" | "-h" | "--help" | "list" => {
                println!("{}", usage());
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}\n\n{}", usage());
                std::process::exit(2);
            }
        }
    }
    // The sandbox helper script forwards everything after `call`, so `help`
    // arrives as the tool name — answer it without needing the socket.
    if matches!(cli_tool.as_deref(), Some("help" | "-h" | "--help" | "list")) {
        println!("{}", usage());
        return Ok(());
    }
    let socket = socket
        .or_else(|| std::env::var("LUCENT_ACP_SOCKET").ok())
        .ok_or("--socket required")?;
    let token = token
        .or_else(|| std::env::var("LUCENT_ACP_TOKEN").ok())
        .ok_or("--token required")?;

    // CLI mode: execute a single tool call directly and exit
    if let Some(tool) = cli_tool {
        let (sock_r, mut sock_w) = connect_socket(&socket).await?;
        wire::write_hello(&mut sock_w, &token).await?;
        let mut sock_r = tokio::io::BufReader::new(sock_r);
        let parsed_args = parse_tool_arguments(&tool, cli_tool_args.as_deref());
        let fwd = wire::BridgeRequest::Call {
            id: 1,
            tool,
            args: parsed_args,
        };
        wire::write_request(&mut sock_w, &fwd).await?;
        match wire::read_response(&mut sock_r).await? {
            Some(wire::BridgeResponse::Ok { output, .. }) => {
                let text = output.get("text").and_then(|t| t.as_str()).unwrap_or("");
                println!("{text}");
                return Ok(());
            }
            Some(wire::BridgeResponse::Err { error, .. }) => {
                eprintln!("Error: {error}");
                std::process::exit(1);
            }
            None => {
                eprintln!("Error: bridge closed unexpectedly");
                std::process::exit(1);
            }
        }
    }

    // MCP stdio server mode (default)
    let (sock_r, mut sock_w) = connect_socket(&socket).await?;
    wire::write_hello(&mut sock_w, &token).await?;
    let mut sock_r = tokio::io::BufReader::new(sock_r);

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = tokio::io::BufReader::new(stdin);
    let mut writer = tokio::io::BufWriter::new(stdout);

    // The tool schemas are static: build once from a default context (no DB —
    // schemas don't need a live connection).
    let tools = mcp_server::static_tool_schemas();

    loop {
        let mut line = String::new();
        let n = tokio::io::AsyncBufReadExt::read_line(&mut reader, &mut line).await?;
        if n == 0 {
            break; // agent closed stdin
        }
        let req: serde_json::Value = serde_json::from_str(line.trim_end())?;
        if req.get("method").and_then(|m| m.as_str()) == Some("tools/call") {
            // Forward to the bridge and answer from its response. One call in
            // flight at a time (the bridge serves one connection sequentially).
            let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
            let name = req
                .pointer("/params/name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args = req
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(serde_json::json!({}));
            let fwd = wire::BridgeRequest::Call {
                id: 0,
                tool: name,
                args,
            };
            wire::write_request(&mut sock_w, &fwd).await?;
            let resp = match wire::read_response(&mut sock_r).await? {
                Some(wire::BridgeResponse::Ok { output, .. }) => {
                    let text = output
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "content": [{ "type": "text", "text": text }], "isError": false }
                    })
                }
                Some(wire::BridgeResponse::Err { error, .. }) => {
                    serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "content": [{ "type": "text", "text": error }], "isError": true }
                    })
                }
                None => break, // bridge gone
            };
            let mut out = serde_json::to_string(&resp)?;
            out.push('\n');
            tokio::io::AsyncWriteExt::write_all(&mut writer, out.as_bytes()).await?;
            tokio::io::AsyncWriteExt::flush(&mut writer).await?;
        } else {
            let resp = mcp_server::handle_mcp_request(req, &tools);
            // Notifications (no id) must not be answered — skip the envelope.
            if resp.get("id").is_some() {
                let mut out = serde_json::to_string(&resp)?;
                out.push('\n');
                tokio::io::AsyncWriteExt::write_all(&mut writer, out.as_bytes()).await?;
                tokio::io::AsyncWriteExt::flush(&mut writer).await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_object_arguments_pass_through_unchanged() {
        let v = parse_tool_arguments("run_readonly_query", Some(r#"{"sql":"select 1"}"#));
        assert_eq!(v, serde_json::json!({"sql": "select 1"}));
    }

    #[test]
    fn a_bare_argument_becomes_the_tools_primary_field() {
        // The shorthand the guidance and `help` both advertise: it spares the
        // model from nesting JSON (with its own quoted SQL) inside shell
        // quoting, which is where CLI-path calls tend to fail.
        assert_eq!(
            parse_tool_arguments("run_readonly_query", Some("SELECT count(*) FROM t")),
            serde_json::json!({"sql": "SELECT count(*) FROM t"})
        );
        assert_eq!(
            parse_tool_arguments("search_schema", Some("unpaid invoices")),
            serde_json::json!({"query": "unpaid invoices"})
        );
        assert_eq!(
            parse_tool_arguments("get_objects_info", Some("orders")),
            serde_json::json!({"objects": [{"name": "orders"}]})
        );
        assert_eq!(
            parse_tool_arguments("preview_dml", Some("UPDATE t SET x = 1")),
            serde_json::json!({"sql": "UPDATE t SET x = 1", "description": ""})
        );
    }

    #[test]
    fn missing_or_blank_arguments_become_an_empty_object() {
        assert_eq!(
            parse_tool_arguments("search_schema", None),
            serde_json::json!({})
        );
        assert_eq!(
            parse_tool_arguments("search_schema", Some("   ")),
            serde_json::json!({})
        );
    }

    #[test]
    fn a_non_object_json_scalar_is_still_treated_as_the_primary_field() {
        // `42` parses as JSON but isn't an argument map — and a bare SQL string
        // never parses at all. Both must land on the primary field.
        assert_eq!(
            parse_tool_arguments("run_readonly_query", Some("42")),
            serde_json::json!({"sql": "42"})
        );
    }

    #[test]
    fn usage_documents_every_tool_with_its_schema() {
        let u = usage();
        for tool in [
            "search_schema",
            "get_objects_info",
            "run_readonly_query",
            "preview_dml",
        ] {
            assert!(u.contains(tool), "{tool} missing from help: {u}");
        }
        // The schemas come from the same source the MCP path serves, so help
        // can't drift from `tools/list`.
        assert!(u.contains(r#""required":["sql"]"#), "{u}");
        assert!(u.contains("SHORTHAND"), "{u}");
    }
}
