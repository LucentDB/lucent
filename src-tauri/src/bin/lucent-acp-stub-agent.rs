//! `lucent-acp-stub-agent` — a scripted ACP v1 agent over stdio, used by the
//! phase C/D integration tests to drive `AcpChatDriver` deterministically
//! without any real agent.
//!
//! Speaks real JSON-RPC JSON-lines on stdin/stdout:
//! - `initialize` → responds with `protocolVersion: 1` and the legacy
//!   `capabilities.session.mcp.stdio` advertisement (pre-1.0 capability tree
//!   — the crate ignores it, the client's raw-wire MCP gate accepts it).
//! - `session/new` → responds with a fresh `stub-sess-N` id; logs the
//!   received `mcpServers` count to stderr.
//! - `session/prompt` → executes the script's steps for the session, then
//!   responds with the scripted `stopReason` (or `cancelled` when a
//!   `session/cancel` notification — or a `Cancelled` permission outcome —
//!   arrived during the turn).
//! - `session/request_permission` requests sent by the stub are awaited: the
//!   step machine pauses until the client's response arrives, so a pending
//!   permission genuinely blocks the turn (that is what cancellation tests
//!   exercise).
//! - `session/cancel` (notification) → flags the in-flight prompt as
//!   cancelled.
//! - any other request method → `-32601` method not found.
//!
//! Script: `--script <path>` or `STUB_SCRIPT` env. JSON shape:
//! ```json
//! {
//!   "stopReason": "end_turn",
//!   "steps": [
//!     { "notify": { "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Hel" } } },
//!     { "permission": { "title": "Read ~/.zshrc", "options": [ { "optionId": "allow_once", "name": "Allow once", "kind": "allow_once" } ] } }
//!   ]
//! }
//! ```
//! With no script the agent behaves minimally: no updates, `end_turn`.
//!
//! Everything observable is logged to stderr as `STUB <event>: <json>`
//! lines — the client's `with_debug` callback surfaces them through the
//! stderr tail, and the tests can assert on the wire behavior.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::rc::Rc;

#[derive(Debug, serde::Deserialize)]
struct Script {
    #[serde(rename = "stopReason", default = "default_stop_reason")]
    stop_reason: String,
    #[serde(default)]
    steps: Vec<Step>,
}

fn default_stop_reason() -> String {
    "end_turn".to_string()
}

#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum Step {
    Notify {
        notify: Value,
    },
    Permission {
        permission: Value,
    },
    /// Terminates the process mid-turn (crash-recovery tests). The client
    /// sees stdout EOF and treats the connection as dead.
    Exit {
        exit: Value,
    },
}

/// The in-flight prompt turn: which session it serves, the request id to
/// respond to, where the step machine is, and whether a cancel arrived.
struct PromptState {
    session_id: String,
    prompt_id: Value,
    cancelled: bool,
    step_index: usize,
    script: Option<Rc<Script>>,
}

fn main() {
    let script: Option<Rc<Script>> = std::env::args()
        .position(|a| a == "--script")
        .and_then(|i| std::env::args().nth(i + 1))
        .or_else(|| std::env::var("STUB_SCRIPT").ok())
        .and_then(|path| std::fs::read_to_string(&path).ok())
        .and_then(|json| serde_json::from_str(&json).ok())
        .map(Rc::new);

    // When set, the stub plays a real agent's MCP-client role for the FIRST
    // stdio server in session/new's mcpServers: it spawns the command with
    // the given args/env and keeps its stdin open, so the bridge hello
    // lands and Lucent's connect signal fires. (The stub does not speak MCP
    // JSON-RPC to the child — the child only needs the bridge socket.)
    let spawn_mcp = std::env::var("STUB_SPAWN_MCP")
        .map(|v| v == "1")
        .unwrap_or(false);
    let mut mcp_children: Vec<std::process::Child> = Vec::new();
    let mut mcp_stds: Vec<std::process::ChildStdin> = Vec::new();

    eprintln!("STUB ready: script={}", script.is_some());

    let stdin = std::io::stdin();
    let mut session_counter = 0usize;
    let mut request_counter = 9000usize;
    // The in-flight prompt turn, if any. `None` = no prompt in flight.
    let mut prompt: Option<PromptState> = None;
    // Outstanding session/request_permission request id (at most one in
    // flight — the step machine pauses until the client answers). Ids are
    // echoed as opaque JSON values: the client sends UUID-string ids.
    let mut pending_permission_id: Option<Value> = None;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("STUB bad line: {e}");
                continue;
            }
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(|v| v.as_str());

        match (&id, method, msg.get("result"), msg.get("error")) {
            // ── A response to one of OUR requests (request_permission) ──
            (Some(resp_id), None, Some(result), None)
                if pending_permission_id.as_ref() == Some(resp_id) =>
            {
                pending_permission_id = None;
                eprintln!("STUB permission outcome: {result}");
                // A Cancelled outcome means the client is cancelling the
                // turn (normative: the client MUST resolve pending
                // permissions with Cancelled before sending session/cancel)
                // — treat it as the cancel signal so the prompt response
                // reports "cancelled" deterministically. The crate encodes
                // the unit variant as {"outcome":{"outcome":"cancelled"}}.
                let outcome = result.get("outcome");
                let cancelled = outcome
                    .map(|o| {
                        o == "cancelled"
                            || o.get("outcome").map(|i| i == "cancelled").unwrap_or(false)
                    })
                    .unwrap_or(false);
                if cancelled {
                    if let Some(state) = &mut prompt {
                        state.cancelled = true;
                        eprintln!("STUB permission resolved cancelled (turn being cancelled)");
                    }
                }
                // Resume the step machine.
                run_next_step(
                    &mut prompt,
                    &mut request_counter,
                    &mut pending_permission_id,
                );
            }
            // ── Notifications (no id) ──
            (None, Some("session/cancel"), _, _) => {
                if let Some(state) = &mut prompt {
                    state.cancelled = true;
                    eprintln!("STUB session/cancel (cancelling current prompt)");
                } else {
                    eprintln!("STUB session/cancel (no prompt in flight)");
                }
            }
            (None, Some(_), _, _) => {
                eprintln!("STUB notification ignored: {method:?}");
            }
            // ── Requests (id + method) ──
            (Some(req_id), Some(method), _, _) => {
                let params = msg.get("params").cloned().unwrap_or(Value::Null);
                let req_id = req_id.clone();
                match method {
                    "initialize" => {
                        eprintln!("STUB initialize");
                        respond(
                            req_id,
                            json!({
                                "protocolVersion": 1,
                                "capabilities": { "session": { "mcp": { "stdio": {} } } },
                                "agentInfo": {
                                    "name": "lucent-acp-stub-agent",
                                    "version": "0.1.0"
                                }
                            }),
                        );
                    }
                    "session/new" => {
                        session_counter += 1;
                        let mcp_count = params
                            .get("mcpServers")
                            .and_then(|v| v.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0);
                        eprintln!("STUB session/new mcpServers={mcp_count}");
                        if spawn_mcp && mcp_children.is_empty() {
                            spawn_first_mcp_server(
                                params.get("mcpServers"),
                                &mut mcp_children,
                                &mut mcp_stds,
                            );
                        }
                        respond(
                            req_id,
                            json!({ "sessionId": format!("stub-sess-{session_counter}") }),
                        );
                    }
                    "session/prompt" => {
                        let session_id = params
                            .get("sessionId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("stub-sess-?")
                            .to_string();
                        let text = params
                            .get("prompt")
                            .and_then(|v| v.as_array())
                            .and_then(|a| a.first())
                            .and_then(|b| b.get("text"))
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        // Truncate: the first prompt carries the whole system
                        // preamble + schema, which would flood the stderr
                        // tail. 2 KB is plenty for the assertions (tool
                        // claims vs honest preamble markers).
                        let shown: String = text.chars().take(2000).collect();
                        eprintln!("STUB prompt text: {shown}");
                        eprintln!("STUB session/prompt session={session_id}");
                        prompt = Some(PromptState {
                            session_id,
                            prompt_id: req_id,
                            cancelled: false,
                            step_index: 0,
                            script: script.clone(),
                        });
                        run_next_step(
                            &mut prompt,
                            &mut request_counter,
                            &mut pending_permission_id,
                        );
                    }
                    _ => {
                        eprintln!("STUB unknown method: {method}");
                        respond_error(req_id, -32601, "method not found");
                    }
                }
            }
            _ => {
                eprintln!("STUB unparseable message: {line}");
            }
        }
    }
    eprintln!("STUB stdin EOF, exiting");
}

/// Executes script steps until the next permission step (which sends the
/// request and pauses — the prompt response is deferred until the client
/// answers) or the steps run out (then the prompt is answered).
fn run_next_step(
    prompt: &mut Option<PromptState>,
    request_counter: &mut usize,
    pending_permission_id: &mut Option<Value>,
) {
    let Some(state) = prompt else { return };
    loop {
        let step = state
            .script
            .as_ref()
            .and_then(|s| s.steps.get(state.step_index));
        match step {
            Some(Step::Notify { notify }) => {
                eprintln!("STUB notify: {notify}");
                notify_line(&state.session_id, notify.clone());
                state.step_index += 1;
            }
            Some(Step::Permission { permission }) => {
                *request_counter += 1;
                let req = json!({
                    "jsonrpc": "2.0",
                    "id": *request_counter,
                    "method": "session/request_permission",
                    "params": {
                        "sessionId": state.session_id,
                        "toolCall": {
                            "toolCallId": format!("perm-{request_counter}"),
                            "title": permission.get("title").cloned().unwrap_or(json!("permission")),
                            "status": "pending"
                        },
                        "options": permission.get("options").cloned().unwrap_or(json!([]))
                    }
                });
                *pending_permission_id = Some(json!(*request_counter));
                eprintln!("STUB request_permission: {req}");
                writeln!(std::io::stdout(), "{req}").expect("write line");
                std::io::stdout().flush().expect("flush");
                state.step_index += 1;
                // Pause here: the prompt response waits for the client's
                // decision (this is what makes a pending permission
                // genuinely block the turn).
                return;
            }
            Some(Step::Exit { exit }) => {
                let _ = exit;
                eprintln!("STUB exit step — terminating");
                std::process::exit(1);
            }
            None => {
                // Steps exhausted — answer the prompt.
                let state = prompt.take().expect("prompt in flight");
                let stop_reason = if state.cancelled {
                    "cancelled".to_string()
                } else {
                    state
                        .script
                        .as_ref()
                        .map(|s| s.stop_reason.clone())
                        .unwrap_or_else(default_stop_reason)
                };
                eprintln!("STUB prompt done stopReason={stop_reason}");
                respond(state.prompt_id, json!({ "stopReason": stop_reason }));
                return;
            }
        }
    }
}

/// Spawns the first stdio MCP server from session/new's `mcpServers` — the
/// agent role Lucent's integration tests need to exercise: the child's
/// `--socket`/`--token` args point at the real bridge, and the child
/// connects on startup. `stdin` is kept open (piped and held) so the child
/// doesn't see EOF and exit. HTTP/SSE entries and env-less configs are
/// tolerated; a missing command is logged and skipped.
fn spawn_first_mcp_server(
    servers: Option<&Value>,
    children: &mut Vec<std::process::Child>,
    stds: &mut Vec<std::process::ChildStdin>,
) {
    use std::process::{Command, Stdio};

    let Some(servers) = servers.and_then(|v| v.as_array()) else {
        return;
    };
    let Some(first) = servers.iter().find(|s| {
        s.get("type").map(|t| t == "stdio").unwrap_or(true) && s.get("command").is_some()
    }) else {
        eprintln!("STUB no stdio MCP server to spawn");
        return;
    };
    let command = first.get("command").and_then(|c| c.as_str()).unwrap_or("");
    let args: Vec<String> = first
        .get("args")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let envs: Vec<(String, String)> = first
        .get("env")
        .and_then(|e| e.as_array())
        .map(|e| {
            e.iter()
                .filter_map(|v| {
                    Some((
                        v.get("name")?.as_str()?.to_string(),
                        v.get("value")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut cmd = Command::new(command);
    cmd.args(&args)
        .envs(envs)
        .stdin(Stdio::piped())
        .stderr(Stdio::inherit());
    match cmd.spawn() {
        Ok(mut child) => {
            let stdin = child.stdin.take().expect("piped stdin");
            eprintln!("STUB spawned MCP server: {command}");
            children.push(child);
            stds.push(stdin);
        }
        Err(e) => eprintln!("STUB spawn MCP server failed: {e}"),
    }
    let _ = stds;
}

fn notify_line(session_id: &str, update: Value) {
    let notif = json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": { "sessionId": session_id, "update": update }
    });
    writeln!(std::io::stdout(), "{notif}").expect("write line");
    std::io::stdout().flush().expect("flush");
}

fn respond(id: Value, result: Value) {
    let resp = json!({ "jsonrpc": "2.0", "id": id, "result": result });
    writeln!(std::io::stdout(), "{resp}").expect("write line");
    std::io::stdout().flush().expect("flush");
}

fn respond_error(id: Value, code: i64, message: &str) {
    let resp = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    });
    writeln!(std::io::stdout(), "{resp}").expect("write line");
    std::io::stdout().flush().expect("flush");
}
