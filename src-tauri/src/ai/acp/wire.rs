//! Bridge socket wire protocol: one JSON object per line (JSON-lines framing).
//!
//! Shared between the `lucent-db-tools-mcp` binary (client side) and the main
//! Lucent process (server side). Newline-delimited so the whole feature stays
//! debuggable with `cat` — the same framing family as ACP and MCP themselves.

use serde::{Deserialize, Serialize};

/// Client → server. One JSON object per line; embedded newlines are a framing
/// error (the line protocol forbids them).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeRequest {
    Call {
        id: u64,
        tool: String,
        args: serde_json::Value,
    },
}

/// Server → client.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BridgeResponse {
    /// `output` carries the serialized tool output (a `{ "text": … }` envelope
    /// for every output kind in v1).
    Ok {
        id: u64,
        output: serde_json::Value,
    },
    Err {
        id: u64,
        error: String,
    },
}

/// First line on the socket: `{"type":"hello","token":"<hex>"}`. A mismatch
/// is answered with a silent close — never confirm a wrong token. Single-variant
/// enum (not a struct) so serde validates the tag value strictly: `{"type":"Hello"}`
/// or any other casing is a parse error, keeping the wire format pinned.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Hello {
    Hello { token: String },
}

/// Reads one request line. `Ok(None)` on EOF.
pub async fn read_message(
    reader: &mut (impl tokio::io::AsyncBufRead + Unpin),
) -> Result<Option<BridgeRequest>, String> {
    let mut line = String::new();
    let n = tokio::io::AsyncBufReadExt::read_line(reader, &mut line)
        .await
        .map_err(|e| format!("read bridge request: {e}"))?;
    if n == 0 {
        return Ok(None); // EOF
    }
    let trimmed = line.trim_end();
    if trimmed.contains('\n') {
        return Err("embedded newline in bridge message".into());
    }
    serde_json::from_str(trimmed)
        .map(Some)
        .map_err(|e| format!("parse bridge request: {e}"))
}

/// Writes one response line.
pub async fn write_message(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    msg: &BridgeResponse,
) -> Result<(), String> {
    let mut line =
        serde_json::to_string(msg).map_err(|e| format!("serialize bridge response: {e}"))?;
    line.push('\n');
    tokio::io::AsyncWriteExt::write_all(writer, line.as_bytes())
        .await
        .map_err(|e| format!("write bridge response: {e}"))
}

/// Reads the hello line. `Ok(None)` on EOF.
pub async fn read_hello(
    reader: &mut (impl tokio::io::AsyncBufRead + Unpin),
) -> Result<Option<Hello>, String> {
    let mut line = String::new();
    let n = tokio::io::AsyncBufReadExt::read_line(reader, &mut line)
        .await
        .map_err(|e| format!("read bridge hello: {e}"))?;
    if n == 0 {
        return Ok(None); // EOF
    }
    let trimmed = line.trim_end();
    if trimmed.contains('\n') {
        return Err("embedded newline in bridge hello".into());
    }
    serde_json::from_str(trimmed)
        .map(Some)
        .map_err(|e| format!("parse bridge hello: {e}"))
}

/// Writes the hello line (client side).
pub async fn write_hello(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    token: &str,
) -> Result<(), String> {
    let mut line = serde_json::to_string(&Hello::Hello {
        token: token.to_string(),
    })
    .map_err(|e| format!("serialize bridge hello: {e}"))?;
    line.push('\n');
    tokio::io::AsyncWriteExt::write_all(writer, line.as_bytes())
        .await
        .map_err(|e| format!("write bridge hello: {e}"))
}

/// Writes one request line (client side).
pub async fn write_request(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    req: &BridgeRequest,
) -> Result<(), String> {
    let mut line =
        serde_json::to_string(req).map_err(|e| format!("serialize bridge request: {e}"))?;
    line.push('\n');
    tokio::io::AsyncWriteExt::write_all(writer, line.as_bytes())
        .await
        .map_err(|e| format!("write bridge request: {e}"))
}

/// Reads one response line. `Ok(None)` on EOF.
pub async fn read_response(
    reader: &mut (impl tokio::io::AsyncBufRead + Unpin),
) -> Result<Option<BridgeResponse>, String> {
    let mut line = String::new();
    let n = tokio::io::AsyncBufReadExt::read_line(reader, &mut line)
        .await
        .map_err(|e| format!("read bridge response: {e}"))?;
    if n == 0 {
        return Ok(None); // EOF
    }
    let trimmed = line.trim_end();
    if trimmed.contains('\n') {
        return Err("embedded newline in bridge response".into());
    }
    serde_json::from_str(trimmed)
        .map(Some)
        .map_err(|e| format!("parse bridge response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn duplex() -> (tokio::io::DuplexStream, tokio::io::DuplexStream) {
        tokio::io::duplex(1024)
    }

    #[tokio::test]
    async fn request_round_trips_over_duplex() {
        let (mut a, mut b) = duplex();
        let req = BridgeRequest::Call {
            id: 7,
            tool: "run_readonly_query".into(),
            args: serde_json::json!({"sql": "select 1"}),
        };
        let line = serde_json::to_string(&req).unwrap();
        let mut writer = tokio::io::BufWriter::new(&mut a);
        writer.write_all(line.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();
        drop(writer);
        let mut reader = tokio::io::BufReader::new(&mut b);
        let got = read_message(&mut reader).await.unwrap().unwrap();
        match got {
            BridgeRequest::Call { id, tool, .. } => {
                assert_eq!(id, 7);
                assert_eq!(tool, "run_readonly_query");
            }
        }
    }

    #[tokio::test]
    async fn response_round_trips_over_duplex() {
        let (mut a, mut b) = duplex();
        let resp = BridgeResponse::Err {
            id: 3,
            error: "boom".into(),
        };
        let mut writer = tokio::io::BufWriter::new(&mut a);
        write_message(&mut writer, &resp).await.unwrap();
        writer.flush().await.unwrap();
        drop(writer);
        let mut reader = tokio::io::BufReader::new(&mut b);
        let got = read_response(&mut reader).await.unwrap().unwrap();
        match got {
            BridgeResponse::Err { id, error } => {
                assert_eq!(id, 3);
                assert_eq!(error, "boom");
            }
            other => panic!("expected Err, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn eof_yields_none() {
        let (mut a, mut b) = duplex();
        drop(a);
        let mut reader = tokio::io::BufReader::new(&mut b);
        assert!(read_message(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn embedded_newline_is_rejected() {
        // A message containing a real '\n' byte inside the JSON must be treated
        // as a framing error (the line protocol forbids embedded newlines): the
        // reader stops at the embedded newline and the partial line is not JSON.
        let (mut a, mut b) = duplex();
        let mut writer = tokio::io::BufWriter::new(&mut a);
        writer
            .write_all(b"{\"type\":\"call\",\"id\":1,\"tool\":\"x\",\"args\":{\"a\":\"b\nc\"}}\n")
            .await
            .unwrap();
        writer.flush().await.unwrap();
        drop(writer);
        let mut reader = tokio::io::BufReader::new(&mut b);
        assert!(read_message(&mut reader).await.is_err());
    }

    #[tokio::test]
    async fn hello_round_trips() {
        let (mut a, mut b) = duplex();
        let mut writer = tokio::io::BufWriter::new(&mut a);
        write_hello(&mut writer, "deadbeef").await.unwrap();
        writer.flush().await.unwrap();
        drop(writer);
        let mut reader = tokio::io::BufReader::new(&mut b);
        let hello = read_hello(&mut reader).await.unwrap().unwrap();
        assert!(matches!(hello, Hello::Hello { token } if token == "deadbeef"));
    }

    #[tokio::test]
    async fn hello_tag_value_is_strict() {
        // The wire format is pinned to `{"type":"hello",...}` — serde must
        // reject any other tag value (a struct would silently accept it).
        let (mut a, mut b) = duplex();
        let mut writer = tokio::io::BufWriter::new(&mut a);
        writer
            .write_all(b"{\"type\":\"Hello\",\"token\":\"deadbeef\"}\n")
            .await
            .unwrap();
        writer.flush().await.unwrap();
        drop(writer);
        let mut reader = tokio::io::BufReader::new(&mut b);
        assert!(read_hello(&mut reader).await.is_err());
    }

    #[tokio::test]
    async fn hello_eof_yields_none() {
        let (mut a, mut b) = duplex();
        drop(a);
        let mut reader = tokio::io::BufReader::new(&mut b);
        assert!(read_hello(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn write_request_read_message_round_trip() {
        let (mut a, mut b) = duplex();
        let req = BridgeRequest::Call {
            id: 42,
            tool: "preview_dml".into(),
            args: serde_json::json!({"sql": "insert into t values (1)"}),
        };
        let mut writer = tokio::io::BufWriter::new(&mut a);
        write_request(&mut writer, &req).await.unwrap();
        writer.flush().await.unwrap();
        drop(writer);
        let mut reader = tokio::io::BufReader::new(&mut b);
        let got = read_message(&mut reader).await.unwrap().unwrap();
        match got {
            BridgeRequest::Call { id, tool, .. } => {
                assert_eq!(id, 42);
                assert_eq!(tool, "preview_dml");
            }
        }
    }

    #[tokio::test]
    async fn garbage_line_is_a_parse_error() {
        let (mut a, mut b) = duplex();
        let mut writer = tokio::io::BufWriter::new(&mut a);
        writer.write_all(b"this is not json\n").await.unwrap();
        writer.flush().await.unwrap();
        drop(writer);
        let mut reader = tokio::io::BufReader::new(&mut b);
        let err = read_message(&mut reader).await.unwrap_err();
        assert!(err.contains("parse bridge request"), "err: {err}");
    }

    #[tokio::test]
    async fn hello_embedded_newline_is_rejected() {
        let (mut a, mut b) = duplex();
        let mut writer = tokio::io::BufWriter::new(&mut a);
        writer
            .write_all(b"{\"type\":\"hello\",\"token\":\"a\nb\"}\n")
            .await
            .unwrap();
        writer.flush().await.unwrap();
        drop(writer);
        let mut reader = tokio::io::BufReader::new(&mut b);
        assert!(read_hello(&mut reader).await.is_err());
    }

    #[tokio::test]
    async fn write_message_is_flushable_with_shared_writer() {
        // read_message/write_message use the write half of a duplex split —
        // the generic bounds must accept an owned BufWriter<DuplexStream>.
        let (a, mut b) = duplex();
        let mut writer = tokio::io::BufWriter::new(a);
        write_message(
            &mut writer,
            &BridgeResponse::Ok {
                id: 1,
                output: serde_json::json!({"text": "hi"}),
            },
        )
        .await
        .unwrap();
        writer.flush().await.unwrap();
        drop(writer);
        let mut buf = Vec::new();
        b.read_to_end(&mut buf).await.unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "{\"type\":\"ok\",\"id\":1,\"output\":{\"text\":\"hi\"}}\n"
        );
    }
}
