//! The editor's control endpoint.
//!
//! One request is one line of JSON on a TCP connection to 127.0.0.1: an object with `cmd` and
//! the command's parameters beside it or under `args`; `gate` names the door the request came
//! through when one applies (`run`, `destroy`, `release`: the stdio bridge's tools); `batch`
//! holds several requests to run in order. One reply is one line: `{"ok":true, ...}` or
//! `{"ok":false,"error":"...","code":"..."}`. A connection may carry one request or many.
//!
//! Every connection is its own task and every request runs beside the others: a read answers
//! from snapshots without waiting for a frame or for another command, and a long command (a
//! build, a held screenshot) holds only its own connection.

use std::io;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use gz_editor::{Args, Editor, Gate, Origin, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// The port the editor has always listened on.
pub const DEFAULT_PORT: u16 = 28016;

/// A request line longer than this is refused and the connection closed: no command takes more.
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
        let reply = match parse_line(text) {
            Ok(Line::One(request)) => run_request(&editor, request, timeout).await,
            Ok(Line::Batch { requests, keep_going }) => {
                run_batch(&editor, requests, keep_going, timeout).await
            }
            Err(error) => json!({ "ok": false, "error": error, "code": "invalid" }),
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
/// that went away never cuts a command that changes state half way. The command runs in a job
/// scope of its own, so a timeout answer can name the jobs it started ([`timed_out`]).
async fn run_request(editor: &Arc<Editor>, request: Request, timeout: Duration) -> Value {
    let ed = editor.clone();
    let scope = gz_task::JobScope::default();
    let name = request.cmd.clone();
    let task = editor.exec.spawn(gz_task::in_scope(scope.clone(), async move {
        ed.call_gated(&request.cmd, request.args, Origin::Mcp, request.gate).await
    }));
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(reply)) => reply,
        Ok(Err(_)) => {
            json!({ "ok": false, "error": "internal error: the command did not finish", "code": "internal" })
        }
        Err(_) => timed_out(editor, &name, &scope, timeout),
    }
}

/// Runs several requests in order and answers every reply under `results`. The first failure
/// stops the rest unless `keep_going`; `ok` says whether all of them worked.
async fn run_batch(
    editor: &Arc<Editor>,
    requests: Vec<Request>,
    keep_going: bool,
    timeout: Duration,
) -> Value {
    let total = requests.len();
    let mut results = Vec::with_capacity(total);
    let mut ok = true;
    for request in requests {
        let reply = run_request(editor, request, timeout).await;
        let worked = reply.get("ok").and_then(Value::as_bool) == Some(true);
        results.push(reply);
        if !worked {
            ok = false;
            if !keep_going {
                break;
            }
        }
    }
    let ran = results.len();
    json!({ "ok": ok, "results": results, "ran": ran, "of": total })
}

/// Where a job of a kind is followed besides `jobs`.
fn follow_of(kind: &str) -> Option<&'static str> {
    match kind {
        "level_build" | "level_run" => Some("level_build action=status"),
        "publish" => Some("mod_publish action=status"),
        "catalog" => Some("mod_catalog action=status"),
        _ => None,
    }
}

/// The answer of a request past its timeout. A command whose jobs still run says so - which,
/// since when, where to follow them - with `ok: false`, `code: running` and `job`: it has not
/// answered, it is no failure and no result. A command with no job behind it is "editor busy".
fn timed_out(editor: &Editor, cmd: &str, scope: &gz_task::JobScope, timeout: Duration) -> Value {
    let running: Vec<gz_task::JobInfo> = scope
        .ids()
        .into_iter()
        .filter_map(|id| editor.jobs.get(id))
        .filter(gz_task::JobInfo::running)
        .collect();
    if running.is_empty() {
        return json!({ "ok": false, "error": "editor busy (timeout)", "code": "busy" });
    }
    let mut follow = vec!["job.wait".to_owned()];
    for j in &running {
        if let Some(f) = follow_of(&j.kind)
            && !follow.iter().any(|x| x == f)
        {
            follow.push(f.to_owned());
        }
    }
    let first = &running[0];
    let error = format!(
        "{cmd} is still running: its job {} #{} ({}) goes on past this request's {} s timeout, and this answer is \
         not its result. Follow it with {}.",
        first.kind,
        first.id.0,
        if first.stage.is_empty() {
            first.title.clone()
        } else {
            format!("{}: {}", first.title, first.stage)
        },
        timeout.as_secs_f32(),
        follow.join(" or "),
    );
    let jobs: Vec<Value> = running
        .iter()
        .map(|j| {
            json!({ "id": j.id.0, "kind": j.kind, "title": j.title, "stage": j.stage, "group": j.group,
                    "started_ms": j.started_ms })
        })
        .collect();
    json!({ "ok": false, "code": "running", "running": true, "command": cmd, "job": first.id.0, "error": error,
            "jobs": jobs, "follow": follow })
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

/// One command of a request line.
#[derive(Debug)]
struct Request {
    cmd: String,
    args: Args,
    gate: Gate,
}

/// What a request line asks for.
#[derive(Debug)]
enum Line {
    One(Request),
    Batch { requests: Vec<Request>, keep_going: bool },
}

fn gate_of(map: &mut Args) -> Result<Gate, String> {
    match map.remove("gate") {
        None | Some(Value::Null) => Ok(Gate::Any),
        Some(Value::String(word)) => {
            Gate::parse(&word).ok_or_else(|| format!("'gate' is run, destroy or release, not '{word}'"))
        }
        Some(_) => Err("'gate' is a word: run, destroy or release".to_owned()),
    }
}

/// `{"cmd": name, ...parameters}` or `{"cmd": name, "args": {...}}`. `gate` beside `cmd` names
/// the door; without one `outer` (a batch's) applies.
fn parse_request(mut map: Args, outer: Gate) -> Result<Request, String> {
    let cmd = match map.remove("cmd") {
        Some(Value::String(cmd)) if !cmd.trim().is_empty() => cmd.trim().to_owned(),
        _ => return Err("missing 'cmd'".to_owned()),
    };
    let gate = match gate_of(&mut map)? {
        Gate::Any => outer,
        own => own,
    };
    let args = match map.remove("args") {
        None | Some(Value::Null) => map,
        // the parameters under `args`: nothing else may stand beside them
        Some(Value::Object(args)) if map.is_empty() => args,
        Some(Value::Object(_)) => {
            return Err(format!("'{cmd}': write the parameters beside 'cmd' or under 'args', not both"));
        }
        Some(_) => return Err(format!("'{cmd}': 'args' is an object of the command's parameters")),
    };
    Ok(Request { cmd, args, gate })
}

fn parse_line(text: &[u8]) -> Result<Line, String> {
    let value: Value = serde_json::from_slice(text).map_err(|e| format!("malformed JSON request ({e})"))?;
    let Value::Object(mut map) = value else {
        return Err("the request must be a JSON object".to_owned());
    };
    let Some(batch) = map.remove("batch") else {
        return parse_request(map, Gate::Any).map(Line::One);
    };
    let Value::Array(items) = batch else {
        return Err("'batch' is a list of requests".to_owned());
    };
    let gate = gate_of(&mut map)?;
    let keep_going = map.get("keep_going").and_then(Value::as_bool).unwrap_or(false);
    let mut requests = Vec::with_capacity(items.len());
    for (i, item) in items.into_iter().enumerate() {
        let Value::Object(item) = item else {
            return Err(format!("batch item {}: a request is a JSON object", i + 1));
        };
        requests.push(parse_request(item, gate).map_err(|e| format!("batch item {}: {e}", i + 1))?);
    }
    Ok(Line::Batch { requests, keep_going })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

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
        let ask = |name: &str| Request { cmd: name.to_owned(), args: Args::new(), gate: Gate::Any };
        let reply = rt.block_on(run_request(&editor, ask("slow_edit"), Duration::from_millis(50)));
        // no job behind it: the busy answer
        assert_eq!(reply["error"], "editor busy (timeout)");
        assert!(reply.get("running").is_none());
        assert!(!done.load(Ordering::Acquire));
        // the caller was answered; the command was not cut: it ends by itself
        rt.block_on(async { tokio::time::sleep(Duration::from_millis(600)).await });
        assert!(done.load(Ordering::Acquire), "the command ran to its end");
        let reply = rt.block_on(run_request(&editor, ask("slow_edit"), Duration::from_secs(5)));
        assert_eq!(reply["edited"], true);
    }

    #[test]
    fn a_command_whose_job_outlasts_the_timeout_names_the_job() {
        let rt = gz_task::Runtime::start(gz_task::ExecOptions::default()).unwrap();
        let (log, _guard) = gz_editor::Log::start(None, false).unwrap();
        let mut reg = gz_editor::Registry::new();
        // a command that starts a job and answers when it ended, as `level_build` does
        reg.add("test", "slow_build", |ed, _a, _o| async move {
            let id = ed
                .jobs
                .start("level_build", Some("level_build"), "Building the level", |ctx| async move {
                    ctx.stage("Compiling");
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    Ok(json!({ "built": true }))
                })
                .map_err(|b| gz_editor::CmdError::busy(b.to_string()))?;
            let info = ed.jobs.wait(id).await.unwrap();
            Ok(json!({ "built": info.result.is_some() }))
        });
        let editor = Editor::new(gz_editor::EditorOptions::default(), rt.exec(), log, reg);
        let request = Request { cmd: "slow_build".to_owned(), args: Args::new(), gate: Gate::Any };
        let reply = rt.block_on(run_request(&editor, request, Duration::from_millis(100)));
        assert_eq!(reply["ok"], false);
        assert_eq!(reply["code"], "running", "{reply}");
        assert_eq!(reply["job"], reply["jobs"][0]["id"]);
        assert_eq!(reply["jobs"][0]["kind"], "level_build");
        assert_eq!(reply["jobs"][0]["stage"], "Compiling");
        assert_eq!(reply["follow"], json!(["job.wait", "level_build action=status"]));
        let error = reply["error"].as_str().unwrap();
        assert!(error.starts_with("slow_build is still running: its job level_build #"), "{error}");
        assert!(!error.contains("busy"));
        // the job ends by itself, and the next call answers its result
        let id = gz_task::JobId(reply["jobs"][0]["id"].as_u64().unwrap());
        let ended = rt.block_on(editor.jobs.wait(id)).unwrap();
        assert!(ended.result.is_some());
    }

    fn line(text: &str) -> Result<Line, String> {
        parse_line(text.as_bytes())
    }

    #[test]
    fn a_request_names_its_command_parameters_and_door() {
        let Ok(Line::One(r)) = line(r#"{"cmd":"scene.place","kind":"object","gate":"run"}"#) else {
            panic!()
        };
        assert_eq!((r.cmd.as_str(), r.gate), ("scene.place", Gate::Run));
        assert_eq!(Value::Object(r.args), json!({ "kind": "object" }));
        let Ok(Line::One(r)) = line(r#"{"cmd":"file.delete","args":{"path":"a"},"gate":"destroy"}"#) else {
            panic!()
        };
        assert_eq!(r.gate, Gate::Destroy);
        assert_eq!(Value::Object(r.args), json!({ "path": "a" }));
        // no door: the author's own socket
        let Ok(Line::One(r)) = line(r#"{"cmd":"ping"}"#) else { panic!() };
        assert_eq!(r.gate, Gate::Any);
        assert_eq!(line(r#"{"kind":1}"#).unwrap_err(), "missing 'cmd'");
        assert_eq!(
            line(r#"{"cmd":"a","args":{},"x":1}"#).unwrap_err(),
            "'a': write the parameters beside 'cmd' or under 'args', not both"
        );
        assert_eq!(
            line(r#"{"cmd":"a","gate":"all"}"#).unwrap_err(),
            "'gate' is run, destroy or release, not 'all'"
        );
        // a batch's door is every item's, unless an item names its own
        let Ok(Line::Batch { requests, keep_going }) =
            line(r#"{"batch":[{"cmd":"a"},{"cmd":"b","args":{"n":1},"gate":"destroy"}],"gate":"run"}"#)
        else {
            panic!()
        };
        assert!(!keep_going);
        assert_eq!(requests.iter().map(|r| r.gate).collect::<Vec<_>>(), [Gate::Run, Gate::Destroy]);
        assert_eq!(line(r#"{"batch":[{"x":1}]}"#).unwrap_err(), "batch item 1: missing 'cmd'");
    }

    #[test]
    fn a_batch_stops_at_the_first_failure() {
        let rt = gz_task::Runtime::start(gz_task::ExecOptions::default()).unwrap();
        let (log, _guard) = gz_editor::Log::start(None, false).unwrap();
        let mut reg = gz_editor::Registry::new();
        reg.add_sync("test", "fine", |_ed, _a, _o| Ok(json!({ "done": true })));
        reg.add_sync("test", "broken", |_ed, _a, _o| Err(gz_editor::CmdError::invalid("no")));
        let editor = Editor::new(gz_editor::EditorOptions::default(), rt.exec(), log, reg);
        let batch = |text: &str| {
            let Ok(Line::Batch { requests, keep_going }) = line(text) else { panic!() };
            rt.block_on(run_batch(&editor, requests, keep_going, Duration::from_secs(5)))
        };
        let reply = batch(r#"{"batch":[{"cmd":"fine"},{"cmd":"broken"},{"cmd":"fine"}]}"#);
        assert_eq!(
            (reply["ok"].clone(), reply["ran"].clone(), reply["of"].clone()),
            (json!(false), json!(2), json!(3))
        );
        assert_eq!(reply["results"][1]["code"], "invalid");
        let reply = batch(r#"{"batch":[{"cmd":"broken"},{"cmd":"fine"}],"keep_going":true}"#);
        assert_eq!((reply["ok"].clone(), reply["ran"].clone()), (json!(false), json!(2)));
        assert_eq!(reply["results"][1]["done"], true);
    }
}
