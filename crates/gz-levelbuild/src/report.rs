//! `build.report`: what the last build of a level did and made, as Lua data beside `build.state`.
//!
//! ```lua
//! -- What the last build of this level did and made. The editor writes it, for reading.
//! return {
//!   report = 1,
//!   level = "mp_test",
//!   time = "2026-11-15 12:30:05",
//!   quality = "standard",
//!   result = "built",           -- or "failed" (with failed_step and error) or "stopped"
//!   seconds = 64.8,
//!   steps = { { step = "geometry", seconds = 1.2, ... }, ... },
//!   lighter = "cpu",            -- when the light step's note names one
//!   phases = { { step = "light", name = "Lighting", seconds = 60.1 }, ... },
//!   counts = { visuals = { total = 812, by_kind = { ... } }, ... },
//!   files = { { name = "level", size = 123456 }, ... },
//!   warnings = { { step = "light", code = "compiler", text = "..." } },
//! }
//! ```
//!
//! An agent reads it with its own file tools, or through `level.inspect`'s summary. It is written
//! once per build that ran a step - ended well, failed or stopped - through the same guarded write
//! the other build files take. A report that cannot be written is a line in the log, never a
//! failed build. A `build.*` file never ships (`gz-project`'s `is_level_build_file`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::Severity;
use gz_editor::{Editor, Value, json};
use gz_luadata::{Table, Value as Lua};
use serde_json::Value as Json;

use crate::inspect::{Counts, list_files};
use crate::service::LevelProblem;
use crate::steps::{Quality, Step};

/// The file's name in the level folder.
pub const FILE: &str = "build.report";
/// The version of the file this editor writes.
pub const VERSION: u32 = 1;
/// How many files and warnings the report lists.
const LIST_CAP: usize = 500;
const HEAD: &str =
    "-- What the last build of this level did and made. The editor writes it, for people and agents to read.
";

/// How the build ended.
pub(crate) enum End<'a> {
    Built,
    Failed {
        step: Step,
        seconds: f64,
        text: &'a str,
    },
    /// Stopped by the author; the step it was in, when one had begun.
    Stopped {
        step: Option<Step>,
    },
}

/// Writes the report of a build that ran at least one step. Never fails the build.
pub(crate) async fn write(
    editor: &Arc<Editor>,
    dir: &Path,
    level: &str,
    quality: Quality,
    ran: &[Value],
    problems: &[LevelProblem],
    end: End<'_>,
) {
    if ran.is_empty() && matches!(end, End::Built | End::Stopped { step: None }) {
        return;
    }
    let (d2, level2) = (dir.to_path_buf(), level.to_owned());
    let (ran2, found) = (ran.to_vec(), problems_json(problems));
    let (result, failed) = match &end {
        End::Built => ("built", None),
        End::Failed { step, seconds, text } => ("failed", Some((step.name(), *seconds, (*text).to_owned()))),
        End::Stopped { step } => ("stopped", step.map(|s| (s.name(), 0.0, String::new()))),
    };
    let quality = quality.name();
    let text = editor.exec.io(move || text_of(&d2, &level2, quality, result, failed, &ran2, &found)).await;
    if let Err(e) =
        crate::pipeline::write_files(editor, PathBuf::from(dir), vec![(FILE.to_owned(), text.into_bytes())])
            .await
    {
        editor.log.warn(format!("{FILE} of {level} could not be written: {}", e.message));
    }
}

fn problems_json(problems: &[LevelProblem]) -> Vec<Json> {
    problems
        .iter()
        .filter(|p| matches!(p.severity, Severity::Warning))
        .take(LIST_CAP)
        .map(|p| json!({ "step": p.step.name(), "code": p.code, "text": p.text }))
        .collect()
}

/// The first string under `key` anywhere in a note.
fn find_string(v: &Json, key: &str) -> Option<String> {
    match v {
        Json::Object(o) => o
            .get(key)
            .and_then(Json::as_str)
            .map(str::to_owned)
            .or_else(|| o.values().find_map(|x| find_string(x, key))),
        Json::Array(a) => a.iter().find_map(|x| find_string(x, key)),
        _ => None,
    }
}

/// The compiler's phases of a step's note, in order, each with the step's name.
fn phases_of(step: &str, note: &Json, out: &mut Vec<Json>) {
    for part in note.get("parts").and_then(Json::as_array).into_iter().flatten() {
        for p in part.get("phases").and_then(Json::as_array).into_iter().flatten() {
            if let Some(name) = p.get("name").and_then(Json::as_str) {
                let seconds = p.get("seconds").and_then(Json::as_f64).unwrap_or(0.0);
                out.push(
                    json!({ "step": step, "name": name, "seconds": (seconds * 1000.0).round() / 1000.0 }),
                );
            }
        }
    }
}

/// The file's text. Blocking: it reads the level folder back.
fn text_of(
    dir: &Path,
    level: &str,
    quality: &str,
    result: &str,
    failed: Option<(&str, f64, String)>,
    ran: &[Value],
    warnings: &[Json],
) -> String {
    let mut top = Json::Object(serde_json::Map::new());
    let mut put = |k: &str, v: Json| {
        if let Json::Object(o) = &mut top {
            o.insert(k.to_owned(), v);
        }
    };
    put("report", json!(VERSION));
    put("level", json!(level));
    put("time", json!(crate::state::now_text()));
    put("quality", json!(quality));
    put("result", json!(result));
    match (&failed, result) {
        (Some((step, _, text)), "failed") => {
            put("failed_step", json!(step));
            put("error", json!(text));
        }
        (Some((step, _, _)), _) => put("stopped_in", json!(step)),
        _ => {}
    }

    let mut steps = Vec::new();
    let mut phases = Vec::new();
    let mut lighter = None;
    let mut total = 0.0;
    for r in ran {
        let name = r.get("step").and_then(Json::as_str).unwrap_or("");
        let seconds = r.get("seconds").and_then(Json::as_f64).unwrap_or(0.0);
        total += seconds;
        let mut s = json!({ "step": name, "seconds": seconds });
        if let Some(files) = r.get("files").filter(|f| f.as_array().is_some_and(|a| !a.is_empty())) {
            s["files"] = files.clone();
        }
        if let Some(note) = r.get("note") {
            phases_of(name, note, &mut phases);
            if let Some(l) = find_string(note, "lighter") {
                s["lighter"] = json!(l);
                lighter.get_or_insert(l);
            }
        }
        steps.push(s);
    }
    if let (Some((step, seconds, text)), "failed") = (&failed, result) {
        steps.push(json!({ "step": step, "seconds": (seconds * 10.0).round() / 10.0, "failed": text }));
        total += seconds;
    }
    put("seconds", json!((total * 10.0).round() / 10.0));
    put("steps", Json::Array(steps));
    if let Some(l) = lighter {
        put("lighter", json!(l));
    }
    put("phases", Json::Array(phases));
    put("counts", Counts::of(dir).json());
    let files = list_files(dir);
    put("files_total", json!(files.len()));
    put(
        "files",
        Json::Array(files.iter().take(LIST_CAP).map(|(n, s)| json!({ "name": n, "size": s })).collect()),
    );
    put("warnings", Json::Array(warnings.to_vec()));

    let Lua::Table(table) = lua_of_json(&top) else { return String::new() };
    gz_luadata::write_asset(HEAD, &table)
}

/// A JSON value as Lua data; a null is left out by its table.
pub(crate) fn lua_of_json(v: &Json) -> Lua {
    match v {
        Json::Null => Lua::Nil,
        Json::Bool(b) => Lua::Bool(*b),
        Json::Number(n) => Lua::Number(n.as_f64().unwrap_or(0.0)),
        Json::String(s) => Lua::String(s.as_str().into()),
        Json::Array(a) => {
            let mut t = Table::new();
            t.arr = a.iter().map(lua_of_json).filter(|x| !matches!(x, Lua::Nil)).collect();
            Lua::Table(t)
        }
        Json::Object(o) => {
            let mut t = Table::new();
            for (k, x) in o {
                let x = lua_of_json(x);
                if !matches!(x, Lua::Nil) {
                    t.put(k, x);
                }
            }
            Lua::Table(t)
        }
    }
}

/// Lua data as JSON: a table with keys is an object, one with only a list an array.
fn json_of_lua(v: &Lua) -> Json {
    match v {
        Lua::Nil => Json::Null,
        Lua::Bool(b) => Json::Bool(*b),
        Lua::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 9e15 {
                json!(*n as i64)
            } else {
                json!(n)
            }
        }
        Lua::String(s) => Json::String(s.to_string()),
        Lua::Table(t) if t.fields.is_empty() => Json::Array(t.arr.iter().map(json_of_lua).collect()),
        Lua::Table(t) => {
            let mut o = serde_json::Map::new();
            for (k, x) in &t.fields {
                o.insert(k.to_string(), json_of_lua(x));
            }
            if !t.arr.is_empty() {
                o.insert("list".into(), Json::Array(t.arr.iter().map(json_of_lua).collect()));
            }
            Json::Object(o)
        }
    }
}

/// A report's text as JSON, `None` for a file that is no Lua data.
pub(crate) fn read_json(text: &str) -> Option<Value> {
    gz_luadata::parse_named(text, FILE).ok().map(|v| json_of_lua(&v))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_text_reads_back_as_lua_data_with_what_it_says() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("level"), crate::inspect::sample::level()).unwrap();
        std::fs::write(dir.path().join("level.geom"), [0u8; 10]).unwrap();
        let ran = vec![
            json!({ "step": "geometry", "title": "Geometry", "seconds": 1.2, "files": ["build.prj"], "note": {} }),
            json!({ "step": "light", "title": "Light", "seconds": 60.5, "files": [], "note": {
                "level": "x", "parts": [ { "part": "level", "seconds": 60.0, "lighter": "gpu",
                    "phases": [ { "name": "Lighting", "seconds": 40.25 }, { "name": "Saving", "seconds": 2.0 } ] } ] } }),
        ];
        let warnings = vec![json!({ "step": "light", "code": "compiler", "text": "a \"quoted\" warning" })];
        let text = text_of(dir.path(), "mp_test", "standard", "built", None, &ran, &warnings);
        assert!(text.starts_with("-- What the last build"), "{text}");

        let v = read_json(&text).unwrap();
        assert_eq!(v["report"], 1);
        assert_eq!(v["level"], "mp_test");
        assert_eq!(v["result"], "built");
        assert_eq!(v["quality"], "standard");
        assert_eq!(v["seconds"], 61.7);
        assert_eq!(v["lighter"], "gpu");
        assert_eq!(v["steps"].as_array().unwrap().len(), 2);
        assert_eq!(v["steps"][1]["lighter"], "gpu");
        assert_eq!(v["phases"][0]["name"], "Lighting");
        assert_eq!(v["phases"][1]["seconds"], 2);
        assert_eq!(v["counts"]["visuals"]["total"], 3);
        assert_eq!(v["counts"]["sectors"], 1);
        assert_eq!(v["files_total"], 2);
        assert_eq!(v["files"][0]["name"], "level");
        assert_eq!(v["warnings"][0]["text"], "a \"quoted\" warning");
        assert!(v.get("error").is_none());
    }

    #[test]
    fn a_failed_and_a_stopped_build_say_where() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("build.prj"), b"x").unwrap();
        let ran = vec![json!({ "step": "game", "seconds": 0.5, "files": [], "note": null })];
        let text =
            text_of(dir.path(), "l", "draft", "failed", Some(("light", 3.0, "No sun.".into())), &ran, &[]);
        let v = read_json(&text).unwrap();
        assert_eq!(
            (v["result"].clone(), v["failed_step"].clone(), v["error"].clone()),
            (json!("failed"), json!("light"), json!("No sun."))
        );
        assert_eq!(v["steps"][1]["failed"], "No sun.");
        let text =
            text_of(dir.path(), "l", "draft", "stopped", Some(("light", 0.0, String::new())), &ran, &[]);
        let v = read_json(&text).unwrap();
        assert_eq!((v["result"].clone(), v["stopped_in"].clone()), (json!("stopped"), json!("light")));
    }
}
