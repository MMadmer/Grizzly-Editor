//! The editor's control endpoint.
//!
//! One request is one line of JSON on a TCP connection to 127.0.0.1: an object with `cmd` and
//! the command's arguments beside it. One reply is one line: `{"ok":true, ...}` or
//! `{"ok":false,"error":"..."}`. It is the wire the QA suites and the stdio bridge have always
//! spoken; a connection may carry one request or many.
//!
//! Every connection is its own task and every request runs beside the others: a read answers
//! from snapshots without waiting for a frame or for another command, and a long command (a
//! build, a held screenshot) holds only its own connection.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use gz_editor::{Args, Editor, Origin, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// The tool catalogue the stdio bridge serves: names, descriptions, schemas and the command
/// each tool maps to. The parity check reads it: every command named here must be registered.
pub const TOOLS_JSON: &str = include_str!("../tools.json");

/// The port the editor has always listened on.
pub const DEFAULT_PORT: u16 = 28016;

/// A request line longer than this is refused: no asset or picture comes near it.
const MAX_REQUEST: usize = 64 * 1024 * 1024;

/// How long a request may take, from `GRIZZLY_MCP_REQUEST_TIMEOUT_MS` (three minutes unset).
fn request_timeout() -> Duration {
    let ms = std::env::var("GRIZZLY_MCP_REQUEST_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(180_000);
    Duration::from_millis(ms.clamp(1_000, 3_600_000))
}

/// Binds the endpoint and serves it on the editor's runtime until the editor quits. Returns the
/// address it listens on (pass port 0 to let the system pick one).
pub async fn serve(editor: Arc<Editor>, port: u16) -> io::Result<SocketAddr> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
    let addr = listener.local_addr()?;
    let timeout = request_timeout();
    let exec = editor.exec.clone();
    exec.spawn(async move {
        let quit = editor.quit_token().clone();
        loop {
            tokio::select! {
                () = quit.cancelled() => break,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    let editor = editor.clone();
                    editor.exec.clone().spawn(async move {
                        // a broken connection is the client's business; nothing to report
                        let _ = connection(editor, stream, timeout).await;
                    });
                }
            }
        }
    });
    Ok(addr)
}

async fn connection(editor: Arc<Editor>, stream: TcpStream, timeout: Duration) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::with_capacity(64 * 1024, read);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = read_line_limited(&mut read, &mut line).await?;
        if n == 0 {
            return Ok(());
        }
        let text = line.trim_ascii();
        if text.is_empty() {
            continue;
        }
        let reply = match parse_request(text) {
            Ok((cmd, args)) => {
                match tokio::time::timeout(timeout, editor.call(&cmd, args, Origin::Mcp)).await {
                    Ok(reply) => reply,
                    Err(_) => json!({ "ok": false, "error": "editor busy (timeout)" }),
                }
            }
            Err(error) => json!({ "ok": false, "error": error }),
        };
        let mut out = serde_json::to_vec(&reply)
            .unwrap_or_else(|_| br#"{"ok":false,"error":"reply could not be encoded"}"#.to_vec());
        out.push(b'\n');
        write.write_all(&out).await?;
        write.flush().await?;
    }
}

/// Reads through the next `\n`. Returns 0 at the end of the stream; a line over the limit is
/// an error that closes the connection.
async fn read_line_limited(
    read: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
    line: &mut Vec<u8>,
) -> io::Result<usize> {
    loop {
        let chunk = read.fill_buf().await?;
        if chunk.is_empty() {
            return Ok(line.len());
        }
        match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => {
                line.extend_from_slice(&chunk[..=i]);
                read.consume(i + 1);
                return Ok(line.len());
            }
            None => {
                let n = chunk.len();
                line.extend_from_slice(chunk);
                read.consume(n);
            }
        }
        if line.len() > MAX_REQUEST {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "request too large"));
        }
    }
}

fn parse_request(text: &[u8]) -> Result<(String, Args), String> {
    let value: Value = serde_json::from_slice(text).map_err(|e| format!("malformed JSON request ({e})"))?;
    let Value::Object(mut map) = value else {
        return Err("the request must be a JSON object".to_owned());
    };
    match map.remove("cmd") {
        Some(Value::String(cmd)) if !cmd.is_empty() => Ok((cmd, map)),
        _ => Err("missing 'cmd'".to_owned()),
    }
}
