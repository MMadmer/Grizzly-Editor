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

/// What `-catalogue` prints: `catalogue fnv1a64:<16 hex> tools:<n>` of the catalogue this build
/// embeds. The bridge prints the same line of its own copy, so a check tells a binary built
/// from another `tools.json` from a current one (`tools/qa/catalogue.py`). Worked out only
/// when asked.
#[must_use]
pub fn catalogue_line() -> String {
    catalogue_line_of(TOOLS_JSON)
}

/// [`catalogue_line`] of any catalogue text.
#[must_use]
pub fn catalogue_line_of(text: &str) -> String {
    let hash =
        text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    let tools = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("tools").and_then(serde_json::Value::as_array).map(Vec::len))
        .unwrap_or(0);
    format!("catalogue fnv1a64:{hash:016x} tools:{tools}")
}

/// The port the editor has always listened on.
pub const DEFAULT_PORT: u16 = 28016;

/// A request line longer than this is refused and the connection closed, as the C++ endpoint
/// did ("request buffer exceeds 1 MiB"): no command takes more.
const MAX_REQUEST: usize = 1024 * 1024;
const OVERFLOW_REPLY: &[u8] = b"{\"ok\":false,\"error\":\"request buffer exceeds 1 MiB\"}\n";

/// How long a request may take, from `GRIZZLY_MCP_REQUEST_TIMEOUT_MS` (three minutes unset).
fn request_timeout() -> Duration {
    let ms = std::env::var("GRIZZLY_MCP_REQUEST_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(180_000);
    Duration::from_millis(ms.clamp(100, 3_600_000))
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
        let n = match read_line_limited(&mut read, &mut line).await {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                write.write_all(OVERFLOW_REPLY).await?;
                write.flush().await?;
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        if n == 0 {
            return Ok(());
        }
        let text = line.trim_ascii();
        if text.is_empty() {
            continue;
        }
        let reply = match parse_request(text) {
            Ok((cmd, args)) => run_request(&editor, cmd, args, timeout).await,
            Err(error) => json!({ "ok": false, "error": error }),
        };
        let mut out = serde_json::to_vec(&reply)
            .unwrap_or_else(|_| br#"{"ok":false,"error":"reply could not be encoded"}"#.to_vec());
        out.push(b'\n');
        write.write_all(&out).await?;
        write.flush().await?;
    }
}

/// Runs one command as a task of its own: the caller gets a timeout answer when it takes too
/// long, and the command runs to its end whatever the connection does - a timeout or a client
/// that went away never cuts a command that changes state half way.
async fn run_request(editor: &Arc<Editor>, cmd: String, args: Args, timeout: Duration) -> Value {
    let ed = editor.clone();
    let task = editor.exec.spawn(async move { ed.call(&cmd, args, Origin::Mcp).await });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(reply)) => reply,
        Ok(Err(_)) => json!({ "ok": false, "error": "internal error: the command did not finish" }),
        Err(_) => json!({ "ok": false, "error": "editor busy (timeout)" }),
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
                if line.len() > MAX_REQUEST + 1 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "request too large"));
                }
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn the_catalogue_line_is_fnv1a64_and_the_tool_count() {
        // FNV-1a 64 of "a" (the reference value; the bridge and tools/qa/catalogue.py agree)
        assert_eq!(catalogue_line_of(r#"{"tools":[{},{}]}"#).rsplit_once(' ').unwrap().1, "tools:2");
        assert_eq!(catalogue_line_of("a"), "catalogue fnv1a64:af63dc4c8601ec8c tools:0");
        assert!(catalogue_line().ends_with(&format!(
            " tools:{}",
            serde_json::from_str::<Value>(TOOLS_JSON).unwrap()["tools"].as_array().unwrap().len()
        )));
    }

    #[test]
    fn a_command_runs_to_its_end_past_the_timeout() {
        let rt = gz_task::Runtime::start(gz_task::ExecOptions::default()).unwrap();
        let (log, _guard) = gz_editor::Log::start(None, false).unwrap();
        let done = Arc::new(AtomicBool::new(false));
        let mut reg = gz_editor::Registry::new();
        let d2 = done.clone();
        reg.add("test", "slow_edit", move |_ed, _a, _o| {
            let d = d2.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                d.store(true, Ordering::Release);
                Ok(json!({ "edited": true }))
            }
        });
        let editor = Editor::new(gz_editor::EditorOptions::default(), rt.exec(), log, reg);
        let reply =
            rt.block_on(run_request(&editor, "slow_edit".into(), Args::new(), Duration::from_millis(50)));
        assert_eq!(reply["error"], "editor busy (timeout)");
        assert!(!done.load(Ordering::Acquire));
        // the caller was answered; the command was not cut: it ends by itself
        rt.block_on(async { tokio::time::sleep(Duration::from_millis(600)).await });
        assert!(done.load(Ordering::Acquire), "the command ran to its end");
        let reply =
            rt.block_on(run_request(&editor, "slow_edit".into(), Args::new(), Duration::from_secs(5)));
        assert_eq!(reply["edited"], true);
    }
}
