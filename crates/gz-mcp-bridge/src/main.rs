//! `grizzly-mcp`: the MCP server over stdio that drives the Grizzly Editor (`tools/mcp/grizzly_mcp.py`,
//! the Python bridge it replaced, is now a launcher of this binary).
//!
//! JSON-RPC, one message per line: `initialize`, `notifications/initialized`, `tools/list`,
//! `tools/call`. Each tool call goes to the editor's endpoint on 127.0.0.1 - one JSON line in,
//! one out - and comes back as a tool result. The tools, their descriptions, schemas and
//! annotations and the server's instructions are `crates/gz-mcp/tools.json`, word for word: the
//! safety paragraphs in the descriptions reach the client unchanged.
//!
//! `GRIZZLY_MCP_PORT` names another port of the editor than its own 28016;
//! `GRIZZLY_MCP_CLIENT_TIMEOUT_MS` / `GRIZZLY_MCP_REQUEST_TIMEOUT_MS` how long one answer may take.

use std::collections::{BTreeSet, HashMap};
use std::io::{self, BufRead, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};

const CATALOG: &str = include_str!("../../gz-mcp/tools.json");
const HOST: [u8; 4] = [127, 0, 0, 1];
const DEFAULT_PORT: u16 = 28016;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// What a tool is to the bridge: the command it sends and the arguments its schema declares.
struct Tool {
    cmd: String,
    args: BTreeSet<String>,
}

struct Bridge {
    /// `tools/list`'s answer: the catalogue's tools without the bridge's own `cmd` key.
    listed: Vec<Value>,
    tools: HashMap<String, Tool>,
    instructions: String,
    port: u16,
    request_timeout: Duration,
}

fn request_timeout() -> Duration {
    let raw = std::env::var("GRIZZLY_MCP_CLIENT_TIMEOUT_MS")
        .or_else(|_| std::env::var("GRIZZLY_MCP_REQUEST_TIMEOUT_MS"))
        .unwrap_or_else(|_| "180000".to_owned());
    match raw.trim().parse::<f64>() {
        Ok(ms) if ms.is_finite() => Duration::from_secs_f64((ms / 1000.0 + 5.0).max(5.0)),
        _ => Duration::from_secs(185),
    }
}

impl Bridge {
    fn load() -> Result<Self, String> {
        let catalog: Value =
            serde_json::from_str(CATALOG).map_err(|e| format!("the tool catalogue does not read: {e}"))?;
        let instructions = catalog.get("instructions").and_then(Value::as_str).unwrap_or_default().to_owned();
        let mut listed = Vec::new();
        let mut tools = HashMap::new();
        for t in catalog.get("tools").and_then(Value::as_array).into_iter().flatten() {
            let Some(name) = t.get("name").and_then(Value::as_str) else { continue };
            let Some(cmd) = t.get("cmd").and_then(Value::as_str) else { continue };
            let args = t
                .pointer("/inputSchema/properties")
                .and_then(Value::as_object)
                .map(|p| p.keys().cloned().collect())
                .unwrap_or_default();
            tools.insert(name.to_owned(), Tool { cmd: cmd.to_owned(), args });
            let mut shown = t.as_object().cloned().unwrap_or_default();
            shown.remove("cmd");
            listed.push(Value::Object(shown));
        }
        let port = std::env::var("GRIZZLY_MCP_PORT")
            .ok()
            .and_then(|p| p.trim().parse().ok())
            .unwrap_or(DEFAULT_PORT);
        Ok(Self { listed, tools, instructions, port, request_timeout: request_timeout() })
    }

    /// One request to the editor and its answer; a failure is an answer of its own.
    fn editor_call(&self, cmd: &str, extra: &Map<String, Value>) -> Value {
        let mut payload = Map::new();
        payload.insert("cmd".into(), json!(cmd));
        for (k, v) in extra {
            payload.insert(k.clone(), v.clone());
        }
        let unreachable = |e: &dyn std::fmt::Display| json!({ "ok": false, "error": format!("editor not reachable on 127.0.0.1:{} ({e})", self.port) });
        let addr = SocketAddr::from((HOST, self.port));
        let mut stream = match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(s) => s,
            Err(e) => return unreachable(&e),
        };
        let _ = stream.set_read_timeout(Some(self.request_timeout));
        let _ = stream.set_write_timeout(Some(self.request_timeout));
        let mut line = Value::Object(payload).to_string();
        line.push('\n');
        if let Err(e) = stream.write_all(line.as_bytes()) {
            return unreachable(&e);
        }
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = vec![0u8; 1 << 16];
        while buf.last() != Some(&b'\n') {
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(chunk.get(..n).unwrap_or(&[])),
                Err(e) => return unreachable(&e),
            }
        }
        let text = match std::str::from_utf8(&buf) {
            Ok(t) => t,
            Err(e) => {
                return json!({ "ok": false, "error": format!("editor returned malformed UTF-8 at response byte {}", e.valid_up_to()) });
            }
        };
        match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => json!({ "ok": false, "error": format!("editor returned malformed JSON ({e})") }),
        }
    }
}

/// Python's truth of a JSON value: what the Python bridge's `if data.get(..)` tested.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

fn ok(data: &Value) -> bool {
    truthy(data.get("ok"))
}

fn pending(data: &Value) -> bool {
    truthy(data.get("pending"))
}

fn str_of<'a>(args: &'a Map<String, Value>, key: &str, default: &'a str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or(default)
}

fn obj(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
}

/// A JSON text the way Python's `json.dumps(.., ensure_ascii=False)` writes it: `", "` between
/// items, `": "` after a key.
fn py_dumps(v: &Value) -> String {
    let mut out = String::new();
    write_py(v, &mut out);
    out
}

fn write_py(v: &Value, out: &mut String) {
    match v {
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_py(x, out);
            }
            out.push(']');
        }
        Value::Object(o) => {
            out.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&Value::String(k.clone()).to_string());
                out.push_str(": ");
                write_py(x, out);
            }
            out.push('}');
        }
        other => out.push_str(&other.to_string()),
    }
}

fn text_result(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

/// The tool call: the arguments checked against the schema, the fixed arguments of the tools
/// that are one command with one argument set, the editor's answer, the waiting loops for an
/// answer that says it is not done, and the result content.
fn tool_result(b: &Bridge, name: &str, tool: &Tool, given: &Map<String, Value>) -> Value {
    let extra: Vec<&str> = given.keys().map(String::as_str).filter(|k| !tool.args.contains(*k)).collect();
    if !extra.is_empty() {
        let known: Vec<&str> = tool.args.iter().map(String::as_str).collect();
        let quoted: Vec<String> = extra.iter().map(|a| format!("'{a}'")).collect();
        let refusal = json!({
            "ok": false,
            "error": format!("{name} has no argument {}; it takes {}", quoted.join(", "), if known.is_empty() { "no arguments".to_owned() } else { known.join(", ") }),
        });
        return text_result(py_dumps(&refusal), true);
    }
    let mut args = given.clone();
    let mut wait = true;
    if matches!(
        name,
        "grizzly_mod_publish"
            | "grizzly_mod_releases"
            | "grizzly_mod_release_delete"
            | "grizzly_mod_release_edit"
            | "grizzly_mod_catalog"
            | "grizzly_mod_catalog_submit"
            | "grizzly_mod_catalog_withdraw"
    ) {
        // 'wait' is this bridge's own: the editor answers at once and works on a worker
        wait = args.remove("wait").as_ref().is_none_or(|w| truthy(Some(w)));
    }
    match name {
        // a tool of its own so that a client can tell the one destructive call from the reads
        "grizzly_mod_release_delete" => {
            args.insert("action".into(), json!("delete"));
        }
        "grizzly_mod_release_edit" => {
            args.insert("action".into(), json!("edit"));
        }
        // the one call that opens a public issue, a tool of its own like the release deletion
        "grizzly_mod_catalog_submit" => {
            args.insert("action".into(), json!("submit"));
        }
        // takes the module out of the Mod Browser: a tool of its own, marked destructive
        "grizzly_mod_catalog_withdraw" => {
            args.insert("action".into(), json!("withdraw"));
        }
        // losing unsaved work is only ever a `*_discard` tool, each marked destructive: no other
        // tool's schema takes discard
        "grizzly_ui_tab_discard" | "grizzly_window_close_discard" => {
            args.insert("action".into(), json!("close"));
            args.insert("discard".into(), json!(true));
        }
        n if n.ends_with("_discard") => {
            args.insert("discard".into(), json!(true));
        }
        _ => {}
    }
    let mut data = b.editor_call(&tool.cmd, &args);

    let follows_job = name == "grizzly_mod_publish"
        && wait
        && matches!(str_of(&args, "action", "start"), "start" | "status");
    if follows_job && ok(&data) && pending(&data) {
        let started = data.clone();
        let deadline = Instant::now() + Duration::from_secs(150);
        while pending(&data) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(500));
            data = b.editor_call("mod_publish", &obj(&[("action", json!("status"))]));
        }
        // what was packaged is only in the first answer
        if let Some(d) = data.as_object_mut() {
            for key in ["assets", "files", "requires_game", "baked", "package_warnings"] {
                if let Some(v) = started.get(key) {
                    d.entry(key).or_insert_with(|| v.clone());
                }
            }
        }
    }
    if name == "grizzly_mod_publish"
        && args.get("action").and_then(Value::as_str) == Some("sign_in")
        && ok(&data)
    {
        // the code arrives a moment after the start; an answer without it is of no use
        let deadline = Instant::now() + Duration::from_secs(20);
        while data.get("state").and_then(Value::as_str) == Some("starting") && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(250));
            data = b.editor_call("mod_publish", &obj(&[("action", json!("sign_in_status"))]));
        }
    }
    if tool.cmd == "mod_releases" && wait && ok(&data) && pending(&data) {
        let deadline = Instant::now() + Duration::from_secs(90);
        while pending(&data) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(250));
            data = b.editor_call("mod_releases", &obj(&[("action", json!("status"))]));
        }
    }
    // 'show' starts a check when no job runs: a screenshot after it should show the results
    let follows_catalog = tool.cmd == "mod_catalog"
        && wait
        && matches!(
            str_of(&args, "action", "status"),
            "status" | "check" | "vt" | "submit" | "submission" | "withdraw" | "show"
        );
    let catalog_pending = |d: &Value| d.get("job").is_some_and(|j| truthy(j.get("pending")));
    if follows_catalog && ok(&data) && catalog_pending(&data) {
        // the catalog's status nests the job; a job that outlasts this comes back still pending
        let deadline = Instant::now() + Duration::from_secs(150);
        while catalog_pending(&data) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(500));
            data = b.editor_call("mod_catalog", &obj(&[("action", json!("status"))]));
        }
    }
    if name == "grizzly_quest_view" && ok(&data) && pending(&data) {
        // the answer is the last frame drawn: a held request makes the editor draw (a background one too)
        let path = args.get("path").cloned().unwrap_or(Value::Null);
        b.editor_call("titlebar_info", &Map::new());
        data = b.editor_call("quest_view", &obj(&[("path", path.clone())]));
        let deadline = Instant::now() + Duration::from_secs(1);
        while ok(&data) && pending(&data) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            data = b.editor_call("quest_view", &obj(&[("path", path.clone())]));
        }
    }
    if name == "grizzly_recovery"
        && matches!(str_of(&args, "action", "get"), "restore" | "discard")
        && ok(&data)
        && pending(&data)
    {
        let operation = data.get("operation_id").cloned();
        let deadline = Instant::now() + Duration::from_secs(3);
        while pending(&data) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            let mut current = b.editor_call("recovery", &obj(&[("action", json!("get"))]));
            if !ok(&current) {
                data = current;
                break;
            }
            if current.get("operation_id").cloned() != operation {
                if let Some(c) = current.as_object_mut() {
                    c.insert("ok".into(), json!(false));
                    c.insert("error".into(), json!("recovery operation was superseded before completion"));
                }
                data = current;
                break;
            }
            data = current;
        }
        if data.get("state").and_then(Value::as_str) == Some("error")
            && let Some(d) = data.as_object_mut()
        {
            let why = d
                .get("last_error")
                .and_then(Value::as_str)
                .filter(|e| !e.is_empty())
                .unwrap_or("recovery operation failed")
                .to_owned();
            d.insert("ok".into(), json!(false));
            d.insert("error".into(), json!(why));
        }
    }

    let png = data.as_object_mut().and_then(|d| d.remove("png_base64"));
    let is_error = !ok(&data);
    let mut content = Vec::new();
    match png {
        Some(png) if truthy(Some(&png)) && ok(&data) => {
            content.push(json!({ "type": "image", "data": png, "mimeType": "image/png" }));
            let mut text = "screenshot captured".to_owned();
            if let Some(modal) = data.get("modal").filter(|m| truthy(Some(m))) {
                text.push_str("; the dialog on top: ");
                text.push_str(modal.get("title").and_then(Value::as_str).unwrap_or(""));
            }
            // a panel dragged out of the editor lives in a window of its own, which the picture does not hold
            if let Some(outside) = data.get("outside").and_then(Value::as_array).filter(|o| !o.is_empty()) {
                let names: Vec<String> =
                    outside.iter().map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned)).collect();
                text.push_str("; not in it - drawn in windows of their own outside the editor: ");
                text.push_str(&names.join(", "));
            }
            content.push(json!({ "type": "text", "text": text }));
        }
        _ => content.push(json!({ "type": "text", "text": py_dumps(&data) })),
    }
    json!({ "content": content, "isError": is_error })
}

fn reply(out: &mut impl Write, id: &Value, result: Result<Value, Value>) -> io::Result<()> {
    let mut msg = Map::new();
    msg.insert("jsonrpc".into(), json!("2.0"));
    msg.insert("id".into(), id.clone());
    match result {
        Ok(r) => msg.insert("result".into(), r),
        Err(e) => msg.insert("error".into(), e),
    };
    let mut line = Value::Object(msg).to_string();
    line.push('\n');
    out.write_all(line.as_bytes())?;
    out.flush()
}

fn handle(b: &Bridge, msg: &Value, out: &mut impl Write) -> io::Result<()> {
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    match method {
        "initialize" => {
            let version =
                msg.pointer("/params/protocolVersion").cloned().unwrap_or_else(|| json!("2024-11-05"));
            reply(
                out,
                &id,
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "grizzly-editor", "version": "1.0.0" },
                    "instructions": b.instructions,
                })),
            )
        }
        "notifications/initialized" => Ok(()),
        "tools/list" => reply(out, &id, Ok(json!({ "tools": b.listed }))),
        "tools/call" => {
            let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
            let args =
                msg.pointer("/params/arguments").and_then(Value::as_object).cloned().unwrap_or_default();
            match b.tools.get(name) {
                Some(tool) => {
                    let result = tool_result(b, name, tool, &args);
                    reply(out, &id, Ok(result))
                }
                None => {
                    reply(out, &id, Err(json!({ "code": -32602, "message": format!("unknown tool {name}") })))
                }
            }
        }
        _ if !id.is_null() => {
            reply(out, &id, Err(json!({ "code": -32601, "message": format!("unknown method {method}") })))
        }
        _ => Ok(()),
    }
}

/// What `-catalogue` prints, the editor's `gz_mcp::catalogue_line` of this binary's copy:
/// `catalogue fnv1a64:<16 hex> tools:<n>`. A suite compares it with the file before it drives
/// the bridge (`tools/qa/catalogue.py`).
fn catalogue_line() -> String {
    let hash = fnv1a64(CATALOG);
    let tools = serde_json::from_str::<Value>(CATALOG)
        .ok()
        .and_then(|v| v.get("tools").and_then(Value::as_array).map(Vec::len))
        .unwrap_or(0);
    format!("catalogue fnv1a64:{hash:016x} tools:{tools}")
}

fn fnv1a64(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

fn main() -> std::process::ExitCode {
    // the editor's spelling (one dash or two); every other argument is ignored, as before
    if std::env::args().skip(1).any(|a| a.trim_start_matches('-').eq_ignore_ascii_case("catalogue")) {
        println!("{}", catalogue_line());
        return std::process::ExitCode::SUCCESS;
    }
    let bridge = match Bridge::load() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("grizzly-mcp: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    let stdin = io::stdin();
    let mut out = io::stdout().lock();
    let mut line = Vec::new();
    let mut input = stdin.lock();
    loop {
        line.clear();
        match input.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        // MCP speaks UTF-8; a line that is not, or not JSON, is skipped like the Python bridge did
        let Ok(text) = std::str::from_utf8(&line) else { continue };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(text) else { continue };
        if handle(&bridge, &msg, &mut out).is_err() {
            break;
        }
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge() -> Bridge {
        Bridge::load().unwrap()
    }

    #[test]
    fn the_catalogue_reaches_the_client_word_for_word() {
        let b = bridge();
        let raw: Value = serde_json::from_str(CATALOG).unwrap();
        assert_eq!(b.instructions, raw["instructions"].as_str().unwrap());
        assert_eq!(b.listed.len(), raw["tools"].as_array().unwrap().len());
        let publish = b.listed.iter().find(|t| t["name"] == "grizzly_mod_publish").unwrap();
        assert!(publish["description"].as_str().unwrap().contains("SAFETY - this holds whatever the user"));
        assert!(publish.get("cmd").is_none());
        let withdraw = b.listed.iter().find(|t| t["name"] == "grizzly_mod_catalog_withdraw").unwrap();
        assert!(withdraw["description"].as_str().unwrap().contains("withdraw only the user's own module"));
    }

    #[test]
    fn an_unknown_argument_is_refused_with_the_python_text() {
        let b = bridge();
        let tool = b.tools.get("grizzly_ping").unwrap();
        let r = tool_result(&b, "grizzly_ping", tool, &obj(&[("x", json!(1))]));
        assert_eq!(r["isError"], json!(true));
        assert_eq!(
            r["content"][0]["text"],
            json!("{\"ok\": false, \"error\": \"grizzly_ping has no argument 'x'; it takes no arguments\"}")
        );
    }

    #[test]
    fn only_a_discard_tool_drops_work_and_every_one_is_marked() {
        let raw: Value = serde_json::from_str(CATALOG).unwrap();
        let destructive = |t: &Value| t["annotations"]["destructiveHint"].as_bool() == Some(true);
        let mut discard_tools = 0;
        for t in raw["tools"].as_array().unwrap() {
            let name = t["name"].as_str().unwrap();
            let takes = t["inputSchema"]["properties"].get("discard").is_some();
            assert!(!takes, "{name}: only a *_discard tool may drop unsaved work, and it takes no 'discard'");
            if name.ends_with("_discard") {
                discard_tools += 1;
                assert!(destructive(t), "{name} carries destructiveHint");
                let plain = name.trim_end_matches("_discard");
                let base = raw["tools"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|x| x["name"] == plain || x["cmd"] == t["cmd"]);
                assert!(base.is_some(), "{name} has a plain form");
            }
        }
        assert_eq!(discard_tools, 8);
        for name in [
            "grizzly_level_build",
            "grizzly_content_delete",
            "grizzly_quest_reload",
            "grizzly_project_delete",
        ] {
            let t = raw["tools"].as_array().unwrap().iter().find(|t| t["name"] == name).unwrap();
            assert!(destructive(t), "{name} carries destructiveHint");
        }
    }

    #[test]
    fn a_discard_tool_sends_discard() {
        let b = bridge();
        for name in ["grizzly_level_parts_discard", "grizzly_open_scene_discard", "grizzly_scene_new_discard"]
        {
            assert!(b.tools.contains_key(name), "{name} is listed");
        }
        // the arguments a discard tool sends: no editor answers here, so the call reports the refusal
        let tool = b.tools.get("grizzly_level_parts").unwrap();
        let r = tool_result(&b, "grizzly_level_parts", tool, &obj(&[("discard", json!(true))]));
        assert_eq!(r["isError"], json!(true), "the plain form takes no discard: {r}");
    }

    #[test]
    fn the_catalogue_line_names_the_tools() {
        let line = catalogue_line();
        let raw: Value = serde_json::from_str(CATALOG).unwrap();
        assert!(line.starts_with("catalogue fnv1a64:"), "{line}");
        assert!(line.ends_with(&format!(" tools:{}", raw["tools"].as_array().unwrap().len())), "{line}");
        // FNV-1a 64's reference values (the Python side computes the same)
        assert_eq!(fnv1a64(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn python_dumps_spacing() {
        assert_eq!(py_dumps(&json!({ "a": [1, "é"], "b": null })), "{\"a\": [1, \"é\"], \"b\": null}");
        assert!(truthy(Some(&json!(1))) && !truthy(Some(&json!(""))) && !truthy(None));
    }
}
