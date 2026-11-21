//! The package a report carries (docs/REPORTS.md, "The package"): a ZIP whose `report.json` says the
//! build, the runtime, the system and the crash in the keys the hub reads, beside the editor's state
//! (`editor.json`), the tail of its log, its settings and, for a crash, the crash's own files. Every
//! text goes through the machine's [`Sanitizer`] and the dump through [`minidump::sanitize`] first;
//! what does not fit the hub's limits is left out and said so.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gz_editor::{Args, Editor, JobCtx, Origin, Value, json};
use schemars::JsonSchema;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::crashes::{self, Crash};
use crate::minidump;
use crate::sanitize::{Folder, Sanitizer};
use crate::service::FeedbackService;
use crate::sys;
use crate::zipw::Zip;

/// The schema `report.json` declares.
pub const SCHEMA: &str = "grizzly-editor.report/1";

/// The hub takes 5 MiB packed and 16 MiB expanded (its defaults); a report stays under both.
const MAX_PACKED: usize = 5 * 1024 * 1024 - 256 * 1024;
const MAX_EXPANDED: usize = 15 * 1024 * 1024;
const LOG_SHORT: usize = 1 << 20;
const SETTINGS_MAX: u64 = 1 << 20;
const ANSWER_MAX: usize = 512 * 1024;
/// A dump larger than this (a build of the sources writes whole ones) is left out before it is packed.
const DUMP_MAX: usize = 8 << 20;

/// The editor's commands whose answers say how it stands: each asked once, briefly. None of them
/// changes what it answers by being asked (`perf.frames` would: it measures since its last call).
const SNAPSHOT: [&str; 9] = [
    "state",
    "app.about",
    "project.info",
    "game.info",
    "level.status",
    "doc.list",
    "job.list",
    "perf.stalls",
    "update.status",
];

/// A problem or a wish.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Something does not work as it should.
    #[default]
    Bug,
    /// Something the author would like the editor to do.
    Suggestion,
}

impl Kind {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::Suggestion => "suggestion",
        }
    }
}

/// What the author asked to send.
pub struct Ask {
    pub kind: Kind,
    pub title: String,
    pub text: String,
    pub diagnostics: bool,
    pub crash: Option<Crash>,
}

/// A report ready to go.
pub struct Package {
    pub report_id: String,
    /// None: the report is its words alone.
    pub zip: Option<Vec<u8>>,
    /// (name in the package, bytes) of every file it holds.
    pub files: Vec<(String, usize)>,
    pub left_out: Vec<String>,
    /// `report.json` as it is in the package.
    pub manifest: Value,
}

/// A new report's id: 32 hex digits no other report shares (`count`: the service's reports so far).
fn new_id(count: u64) -> String {
    let mut h = Sha256::new();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    h.update(now.as_nanos().to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    h.update(count.to_le_bytes());
    h.update(format!("{:p}", &now).as_bytes());
    hex::encode(&h.finalize()[..16])
}

fn now_utc() -> String {
    time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).unwrap_or_default()
}

fn utc_of(secs: u64) -> String {
    i64::try_from(secs)
        .ok()
        .and_then(|s| time::OffsetDateTime::from_unix_timestamp(s).ok())
        .and_then(|t| t.format(&time::format_description::well_known::Rfc3339).ok())
        .unwrap_or_default()
}

/// The exception's name the hub groups by: `EXCEPTION_ACCESS_VIOLATION` of
/// `0xC0000005 EXCEPTION_ACCESS_VIOLATION`, `panic` for a panic (a crash without an exception).
fn exception_name(exception: &str) -> String {
    match exception.split_whitespace().last() {
        Some(name) => name.to_owned(),
        None => "panic".to_owned(),
    }
}

/// How this copy of the editor was put on the machine: Setup, the portable archive, or a build of
/// the sources (no installation manifest).
#[must_use]
pub fn edition(exe_dir: &Path) -> &'static str {
    if !exe_dir.join("update-manifest.txt").is_file() {
        "development"
    } else if exe_dir.join("unins000.dat").is_file() || exe_dir.join("unins000.exe").is_file() {
        "setup"
    } else {
        "portable"
    }
}

fn sha256_file(path: &Path) -> Option<String> {
    use std::io::Read as _;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(hex::encode(h.finalize()))
}

/// The build a report is about.
pub fn build_of(editor: &Editor, svc: &FeedbackService) -> Value {
    let id = svc
        .build_id
        .get_or_init(|| std::env::current_exe().ok().and_then(|p| sha256_file(&p)).unwrap_or_default());
    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "commit": svc.commit,
        "architecture": std::env::consts::ARCH,
        "build_id": id,
        "edition": edition(&editor.paths.exe_dir),
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
    })
}

/// The editor's state as its own commands answer it, each within a few seconds.
async fn snapshot(editor: &Arc<Editor>) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for name in SNAPSHOT {
        let call = editor.call(name, Args::new(), Origin::Mcp);
        let answer = match tokio::time::timeout(Duration::from_secs(3), call).await {
            Ok(v) => v,
            Err(_) => json!({ "ok": false, "error": "no answer within 3 seconds" }),
        };
        // what a finished job answered is that job's business - a release's account, a module's
        // repository -, not the state of the editor: the list keeps what ran and how it ended
        let mut answer = answer;
        if name == "job.list"
            && let Some(jobs) = answer.get_mut("jobs").and_then(Value::as_array_mut)
        {
            for job in jobs.iter_mut().filter_map(Value::as_object_mut) {
                job.remove("result");
            }
        }
        let text = answer.to_string();
        let answer = if text.len() > ANSWER_MAX {
            json!({ "ok": answer.get("ok").cloned().unwrap_or(Value::Null), "cut": format!("an answer of {} bytes", text.len()) })
        } else {
            answer
        };
        out.insert(name.to_owned(), answer);
    }
    out
}

/// The GitHub account the project's module releases from (`mod.ltx [update] github = <owner>/<repo>`):
/// an identity the log and the answers name in words of their own, without an address.
fn github_owner(project: &Path) -> Option<String> {
    let text = std::fs::read_to_string(project.join("mod.ltx")).ok()?;
    let mut in_update = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_update = line.eq_ignore_ascii_case("[update]");
        } else if in_update
            && let Some((key, value)) = line.split_once('=')
            && key.trim().eq_ignore_ascii_case("github")
        {
            let owner = value.trim().trim_matches('"').split('/').next()?.trim().to_owned();
            return (!owner.is_empty()).then_some(owner);
        }
    }
    None
}

/// The machine's sanitizer, with the editor's folders, the project's and the games' as markers, and
/// the module's GitHub account as a word (`owner`, read on the IO pool).
fn sanitizer_of(editor: &Editor, state: &serde_json::Map<String, Value>, owner: Option<&str>) -> Sanitizer {
    let game = state.get("game.info");
    let games: Vec<String> = ["root", "default"]
        .iter()
        .filter_map(|k| game.and_then(|g| g.get(*k)).and_then(Value::as_str).map(str::to_owned))
        .filter(|s| !s.is_empty())
        .collect();
    let project = editor.project_root();
    let game_paths: Vec<std::path::PathBuf> = games.iter().map(std::path::PathBuf::from).collect();
    let p = &editor.paths;
    let mut folders = vec![
        Folder { path: &p.exe_dir, marker: "<editor>" },
        Folder { path: &p.root, marker: "<editor_data>" },
        Folder { path: &p.home, marker: "<home>" },
    ];
    if let Some(project) = project.as_deref() {
        folders.push(Folder { path: project, marker: "<project>" });
    }
    folders.extend(game_paths.iter().map(|g| Folder { path: g, marker: "<game>" }));
    let mut s = Sanitizer::of_machine(&folders);
    if let Some(owner) = owner {
        s.add_word(owner, "<owner>");
    }
    s
}

/// `text` cut to its last `limit` bytes from a whole line on.
fn last_bytes(text: &str, limit: usize) -> &str {
    if text.len() <= limit {
        return text;
    }
    let mut from = text.len() - limit;
    while !text.is_char_boundary(from) {
        from += 1;
    }
    let cut = &text[from..];
    cut.find('\n').map_or(cut, |nl| &cut[nl + 1..])
}

struct Entry {
    name: &'static str,
    bytes: Vec<u8>,
}

/// Makes the package of `ask`.
pub async fn build(
    editor: &Arc<Editor>,
    svc: &Arc<FeedbackService>,
    ask: &Ask,
    ctx: &JobCtx,
) -> Result<Package, String> {
    let report_id = new_id(svc.next_report());
    ctx.stage("Describing the build");
    let (ed, s) = (editor.clone(), svc.clone());
    let build = ctx.exec.io(move || build_of(&ed, &s)).await;
    let mut manifest = json!({
        "schema": SCHEMA,
        "type": if ask.crash.is_some() { "crash" } else { "manual" },
        "kind": ask.kind.word(),
        "report_id": report_id,
        "created_utc": now_utc(),
        "build": build,
    });
    if !ask.diagnostics {
        return Ok(Package { report_id, zip: None, files: Vec::new(), left_out: Vec::new(), manifest });
    }

    ctx.stage("Reading the editor's state");
    let mut state = snapshot(editor).await;
    ctx.check().map_err(|_| "Stopped.".to_owned())?;
    let project = editor.project_root();
    let owner = ctx.exec.io(move || project.as_deref().and_then(github_owner)).await;
    let sanitizer = sanitizer_of(editor, &state, owner.as_deref());
    let provided = svc.provided(editor);
    let renderer =
        provided.get("gpu").and_then(|g| g.get("active")).and_then(Value::as_str).unwrap_or("").to_owned();
    let language =
        provided.get("ui").and_then(|u| u.get("language")).and_then(Value::as_str).unwrap_or("").to_owned();
    let location = state
        .get("state")
        .and_then(|s| s.get("active_tab"))
        .and_then(Value::as_str)
        .map(|t| t.split(':').next().unwrap_or(t).to_owned())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(
            || if editor.flags.headless { "headless".to_owned() } else { "start_page".to_owned() },
        );
    for (k, v) in provided {
        state.insert(k, v);
    }
    let mut state = Value::Object(state);
    sanitizer.clean_value(&mut state);

    manifest["runtime"] = json!({
        "renderer": sanitizer.clean(&renderer),
        "location": sanitizer.clean(&location),
        "language": language,
        "uptime_s": svc.started.elapsed().as_secs(),
        "window": !editor.flags.headless,
        "memory": sys::memory(),
    });
    manifest["system"] = sys::system();
    if let Some(c) = &ask.crash {
        let at = sanitizer.clean(c.field("at"));
        manifest["crash"] = json!({
            "name": c.name,
            "time_utc": utc_of(c.time),
            "fatal": c.fatal,
            "exception_name": exception_name(c.field("exception")),
            "exception": sanitizer.clean(c.field("exception")),
            "module": sanitizer.clean(c.field("module")),
            "module_rva": sanitizer.clean(c.field("module_rva")),
            "thread": sanitizer.clean(c.field("thread")),
            "at": at,
            "message": sanitizer.clean(c.field("message")),
            "version": sanitizer.clean(c.field("version")),
        });
    }

    ctx.stage("Reading the log and the settings");
    editor.log.flush();
    let log_file = svc.log_file.clone();
    let settings = editor.paths.home.join("grizzly_editor.ltx");
    let crash = ask.crash.clone();
    let clean = sanitizer.clone();
    let (log, settings, crash_files) = ctx
        .exec
        .io(move || {
            let text = |b: Vec<u8>| clean.clean(&String::from_utf8_lossy(&b));
            let log = crashes::tail(&log_file, crashes::LOG_TAIL).map(text).unwrap_or_default();
            let settings = crashes::tail(&settings, SETTINGS_MAX).map(text).unwrap_or_default();
            let crash_files = crash.map(|c| {
                let txt = c.text.as_ref().and_then(|p| std::fs::read(p).ok()).map(text).unwrap_or_default();
                let run_log = c.log.as_ref().and_then(|p| crashes::tail(p, crashes::LOG_TAIL)).map(text);
                let dump = c.dump.as_ref().and_then(|p| std::fs::read(p).ok()).map(|mut d| {
                    // a whole dump of a build of the sources is too large to go: not even made anonymous
                    let ok = if d.len() > DUMP_MAX {
                        Err("it is larger than a report takes")
                    } else {
                        minidump::sanitize(&mut d)
                    };
                    (d, ok)
                });
                (txt, run_log, dump)
            });
            (log, settings, crash_files)
        })
        .await;
    ctx.check().map_err(|_| "Stopped.".to_owned())?;

    let mut left_out = Vec::new();
    let mut entries = vec![
        Entry { name: "editor.json", bytes: serde_json::to_vec_pretty(&state).unwrap_or_default() },
        Entry { name: "logs/editor.log", bytes: log.into_bytes() },
    ];
    if !settings.is_empty() {
        entries.push(Entry { name: "settings/grizzly_editor.ltx", bytes: settings.into_bytes() });
    }
    if let Some((txt, run_log, dump)) = crash_files {
        entries.push(Entry { name: "crash/crash.txt", bytes: txt.into_bytes() });
        match run_log {
            Some(l) => entries.push(Entry { name: "crash/run.log", bytes: l.into_bytes() }),
            None => left_out.push("the crashed run's log: the next start found none to set aside".to_owned()),
        }
        match dump {
            Some((d, Ok(()))) => entries.push(Entry { name: "crash/crash.dmp", bytes: d }),
            Some((_, Err(why))) => left_out.push(format!("the crash's dump: {why}")),
            None => {}
        }
    }

    ctx.stage("Packing the report");
    let manifest_now = manifest.clone();
    let found = left_out;
    let (zip, files, left_out) = ctx
        .exec
        .cpu(move || pack(manifest_now, entries, found))
        .await
        .map_err(|e| format!("The report could not be packed: {e}"))?;
    manifest["files"] =
        json!(files.iter().map(|(n, b)| json!({ "name": n, "bytes": b })).collect::<Vec<_>>());
    manifest["left_out"] = json!(left_out);
    Ok(Package { report_id, zip: Some(zip), files, left_out, manifest })
}

type Packed = (Vec<u8>, Vec<(String, usize)>, Vec<String>);

/// The ZIP of `entries` with `report.json` first, inside the hub's limits: the logs are cut to their
/// last megabyte first, then the dump, the settings and the state are left out, in that order.
/// `left_out` is what was left out before; `report.json` names every other entry and all of it.
fn pack(mut manifest: Value, mut entries: Vec<Entry>, mut left_out: Vec<String>) -> std::io::Result<Packed> {
    for step in 0..5 {
        let files: Vec<(String, usize)> =
            entries.iter().map(|e| (e.name.to_owned(), e.bytes.len())).collect();
        manifest["files"] =
            json!(files.iter().map(|(n, b)| json!({ "name": n, "bytes": b })).collect::<Vec<_>>());
        manifest["left_out"] = json!(left_out);
        let head = serde_json::to_vec_pretty(&manifest).unwrap_or_default();
        let expanded = head.len() + files.iter().map(|(_, b)| b).sum::<usize>();
        let mut zip = Zip::default();
        zip.add("report.json", &head)?;
        for e in &entries {
            zip.add(e.name, &e.bytes)?;
        }
        if (zip.len() <= MAX_PACKED && expanded <= MAX_EXPANDED) || step == 4 {
            let mut all = vec![("report.json".to_owned(), head.len())];
            all.extend(files);
            return Ok((zip.finish(), all, left_out));
        }
        match step {
            0 => {
                for e in entries.iter_mut().filter(|e| e.name.ends_with(".log")) {
                    let text = String::from_utf8_lossy(&e.bytes).into_owned();
                    e.bytes = last_bytes(&text, LOG_SHORT).as_bytes().to_vec();
                }
                left_out.push("the logs past their last megabyte: the report was too large".to_owned());
            }
            1 => drop_entry(&mut entries, &mut left_out, "crash/crash.dmp", "the crash's dump"),
            2 => drop_entry(&mut entries, &mut left_out, "settings/grizzly_editor.ltx", "the settings"),
            _ => drop_entry(&mut entries, &mut left_out, "editor.json", "the editor's state"),
        }
    }
    Err(std::io::Error::other("the report does not get under the hub's limits"))
}

fn drop_entry(entries: &mut Vec<Entry>, left_out: &mut Vec<String>, name: &str, what: &str) {
    let before = entries.len();
    entries.retain(|e| e.name != name);
    if entries.len() != before {
        left_out.push(format!("{what}: the report was too large"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_package_stays_inside_the_hub_limits() {
        // a log that does not compress: pseudo-random lines
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut log = String::new();
        while log.len() < 12 << 20 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            log.push_str(&format!("{x:016x}{:016x}\n", x.rotate_left(17)));
        }
        let entries = vec![
            Entry { name: "editor.json", bytes: b"{}".to_vec() },
            Entry { name: "logs/editor.log", bytes: log.into_bytes() },
        ];
        let Ok((zip, files, left)) = pack(json!({ "schema": SCHEMA }), entries, vec!["a dump: x".to_owned()])
        else {
            panic!("not packed")
        };
        assert!(zip.len() <= MAX_PACKED, "{}", zip.len());
        assert_eq!(files[0].0, "report.json");
        assert!(left.iter().any(|l| l.contains("last megabyte")));
        let read = crate::zipw::read(&zip);
        let head: Value = serde_json::from_slice(&read[0].1).unwrap_or_default();
        assert_eq!(head["schema"], SCHEMA);
        let left_in_zip = head["left_out"].as_array().cloned().unwrap_or_default();
        assert!(left_in_zip.iter().any(|l| l == "a dump: x"), "what was left out before is in the zip");
        assert_eq!(left_in_zip.len(), left.len(), "the zip and the answer say the same");
    }

    #[test]
    fn a_cut_keeps_whole_lines() {
        assert_eq!(last_bytes("one\ntwo\nthree\n", 8), "three\n");
        assert_eq!(last_bytes("short", 100), "short");
    }

    #[test]
    fn ids_differ() {
        let (a, b) = (new_id(0), new_id(1));
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}
