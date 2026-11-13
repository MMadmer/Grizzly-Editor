//! The `asset.*` commands and the `assets` topic of `help`.

use std::sync::Arc;

use gz_base::{Problem, RelPath, Severity};
use gz_editor::{CmdError, Editor, NoParams, Origin, Registry, Reply, Spec, Topic};
use gz_luadata::path::show;
use gz_luadata::{LuaDoc, Seg, Value};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::check::{self, Counts, MANIFEST};
use crate::file;
use crate::kinds::Kinds;

/// Which problems make `asset.check` itself fail.
#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailOn {
    Error,
    Warning,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckParams {
    /// A file or folder of the project, project-relative. Left out: the whole project.
    #[serde(default)]
    pub path: Option<String>,
    /// Fail the command (code `failed`) when a problem of this severity or a worse one exists:
    /// what a shell's exit code reads.
    #[serde(default)]
    pub fail_on: Option<FailOn>,
    /// How many problems to answer (default 100); `total` says how many there are.
    #[serde(default)]
    pub limit: Option<usize>,
    /// How many problems to skip first.
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceParams {
    /// The kind: its name (`quest`), its extension (`nqasset`) or its header key (`nq`).
    pub kind: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindParams {
    /// Text to look for in the files' lines, any case.
    #[serde(default)]
    pub query: Option<String>,
    /// An id - a character, a quest, a task, a variable, a node: the values and keys of the
    /// assets that name it.
    #[serde(default)]
    pub refs_to: Option<String>,
    /// A file or folder to look in. Left out: the whole project.
    #[serde(default)]
    pub path: Option<String>,
    /// How many places to answer (default 100); `total` says how many there are.
    #[serde(default)]
    pub limit: Option<usize>,
    /// How many places to skip first.
    #[serde(default)]
    pub offset: Option<usize>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MigrateParams {
    /// A file or folder of the project. Left out: the whole project.
    #[serde(default)]
    pub path: Option<String>,
    /// List the files of older versions without writing them.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TakeTheirsParams {
    /// The file of the open document, project-relative.
    pub path: String,
    /// Which of the document's conflicts (from 0, as its answer lists them); left out: every
    /// one.
    #[serde(default)]
    pub index: Option<usize>,
}

async fn asset_take_theirs(ed: Arc<Editor>, p: TakeTheirsParams, _o: Origin) -> Reply {
    let rel = rel_of(&ed, Some(&p.path))?.ok_or_else(|| CmdError::invalid("path is empty"))?;
    let ops = Kinds::of(&ed)
        .of_file(rel.file_name())
        .ok_or_else(|| CmdError::invalid(format!("{rel} is no asset of a kind the editor knows")))?;
    let take = ops
        .take_theirs
        .ok_or_else(|| CmdError::new("refused", format!("an open {} keeps no conflicts", ops.name)))?;
    let left = take(ed.clone(), rel.clone(), p.index).await.map_err(|e| CmdError::new("refused", e))?;
    Ok(json!({ "path": rel, "conflicts_left": left }))
}

/// A path of a command: project-relative (`/` or `\`), or absolute inside the project.
fn rel_of(editor: &Editor, path: Option<&str>) -> Result<Option<RelPath>, CmdError> {
    let Some(p) = path.map(str::trim).filter(|p| !p.is_empty()) else { return Ok(None) };
    let candidate = std::path::Path::new(p);
    if candidate.is_absolute() {
        let session = editor.project()?;
        return RelPath::from_fs(&session.root, candidate)
            .map(Some)
            .ok_or_else(|| CmdError::invalid(format!("{p} is outside the project")));
    }
    RelPath::new(p).map(Some).map_err(|e| CmdError::invalid(format!("{p}: {e}")))
}

fn page<T: Clone>(items: &[T], offset: Option<usize>, limit: Option<usize>) -> Vec<T> {
    let from = offset.unwrap_or(0).min(items.len());
    let n = limit.unwrap_or(100);
    items[from..(from + n).min(items.len())].to_vec()
}

async fn asset_check(ed: Arc<Editor>, p: CheckParams, _o: Origin) -> Reply {
    let path = rel_of(&ed, p.path.as_deref())?;
    let report = check::run(&ed, path.as_ref()).await?;
    let counts = Counts::of(&report.problems);
    let shown = page(&report.problems, p.offset, p.limit);
    let answer = json!({
        "files": report.files,
        "counts": counts,
        "total": report.problems.len(),
        "problems": shown,
    });
    let worst = match p.fail_on {
        Some(FailOn::Error) => counts.error,
        Some(FailOn::Warning) => counts.error + counts.required + counts.warning,
        None => 0,
    };
    if worst > 0 {
        let what = if p.fail_on == Some(FailOn::Error) { "error" } else { "error or warning" };
        let files = report
            .problems
            .iter()
            .filter_map(|p| p.file.as_ref())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let lines: Vec<String> = report
            .problems
            .iter()
            .filter(|x| {
                x.severity == Severity::Error
                    || p.fail_on == Some(FailOn::Warning) && x.severity != Severity::Note
            })
            .take(20)
            .map(Problem::line_text)
            .collect();
        return Err(CmdError::new(
            "failed",
            format!("{worst} problem(s) of the kind {what} in {files} file(s):\n{}", lines.join("\n")),
        )
        .with_data(answer));
    }
    Ok(answer)
}

async fn asset_reference(ed: Arc<Editor>, p: ReferenceParams, _o: Origin) -> Reply {
    let kinds = Kinds::of(&ed);
    let ops = kinds.named(&p.kind).ok_or_else(|| {
        let names: Vec<&str> = kinds.all().iter().map(|k| k.name).collect();
        CmdError::not_found(format!("'{}' is no kind of asset; the kinds are: {}", p.kind, names.join(", ")))
    })?;
    let text = (ops.reference)(ed.clone()).await;
    Ok(json!({ "kind": ops.name, "text": text }))
}

/// A line of a text: its number (1-based) and its text.
fn line_at(text: &str, line: u32) -> String {
    text.lines().nth(line.saturating_sub(1) as usize).unwrap_or("").trim().to_owned()
}

fn whole_word(hay: &str, word: &str) -> bool {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut from = 0;
    while let Some(i) = hay.get(from..).and_then(|r| r.find(word)) {
        let at = from + i;
        let before = hay[..at].chars().next_back().is_none_or(|c| !is_word(c));
        let after = hay[at + word.len()..].chars().next().is_none_or(|c| !is_word(c));
        if before && after {
            return true;
        }
        from = at + word.len();
    }
    false
}

/// The paths of `v` whose key is `id`, or whose string value is `id` or names it as a word.
fn refs_in(v: &Value, id: &str, at: &mut Vec<Seg>, out: &mut Vec<Vec<Seg>>) {
    match v {
        Value::String(s) if s == id || whole_word(s, id) => out.push(at.clone()),
        Value::Table(t) => {
            for (i, x) in t.arr.iter().enumerate() {
                at.push(Seg::Index(i));
                refs_in(x, id, at, out);
                at.pop();
            }
            for (k, x) in &t.fields {
                at.push(Seg::Key(k.clone()));
                if k == id {
                    out.push(at.clone());
                } else {
                    refs_in(x, id, at, out);
                }
                at.pop();
            }
        }
        _ => {}
    }
}

async fn asset_find(ed: Arc<Editor>, p: FindParams, _o: Origin) -> Reply {
    let query = p.query.as_deref().map(str::trim).filter(|q| !q.is_empty()).map(str::to_lowercase);
    let refs = p.refs_to.as_deref().map(str::trim).filter(|q| !q.is_empty()).map(str::to_owned);
    if query.is_none() && refs.is_none() {
        return Err(CmdError::invalid("asset.find needs query, refs_to or both"));
    }
    let path = rel_of(&ed, p.path.as_deref())?;
    let files = check::targets(&ed, path.as_ref()).await?;
    let session = ed.project()?;
    let mut found: Vec<serde_json::Value> = Vec::new();
    for rel in files {
        let abs = session.fs(&rel);
        let Ok(text) = ed.exec.io(move || std::fs::read_to_string(abs)).await else { continue };
        if let Some(q) = &query {
            for (i, line) in text.lines().enumerate() {
                if let Some(col) = line.to_lowercase().find(q.as_str()) {
                    let column = line.get(..col).map_or(0, |s| s.chars().count()) + 1;
                    found.push(json!({ "file": rel, "line": i + 1, "column": column, "text": line.trim(), "match": "text" }));
                }
            }
        }
        let Some(id) = &refs else { continue };
        if rel.file_name().eq_ignore_ascii_case(MANIFEST) {
            for (i, line) in text.lines().enumerate() {
                if whole_word(line, id) {
                    found.push(json!({ "file": rel, "line": i + 1, "column": 1, "text": line.trim(), "match": "ref" }));
                }
            }
            continue;
        }
        let Ok(doc) = LuaDoc::parse(&text) else { continue };
        let mut paths = Vec::new();
        refs_in(&doc.value(), id, &mut Vec::new(), &mut paths);
        for pth in paths {
            let Some(s) = doc.span(&pth) else { continue };
            found.push(json!({
                "file": rel, "line": s.line, "column": s.column, "path": show(&pth),
                "text": line_at(&text, s.line), "match": "ref",
            }));
        }
    }
    let total = found.len();
    Ok(json!({ "total": total, "results": page(&found, p.offset, p.limit) }))
}

async fn asset_migrate(ed: Arc<Editor>, p: MigrateParams, _o: Origin) -> Reply {
    let path = rel_of(&ed, p.path.as_deref())?;
    let files = check::targets(&ed, path.as_ref()).await?;
    let mut listed = Vec::new();
    let mut written = 0usize;
    let mut refused = Vec::new();
    for rel in files.iter().filter(|f| !f.file_name().eq_ignore_ascii_case(MANIFEST)) {
        let mut f = match file::read(&ed, rel).await {
            Ok(f) => f,
            Err(e) => {
                if e.code == "newer_version" || e.code == "migration" {
                    refused.push(json!({ "file": rel, "error": e.problem.message }));
                }
                continue;
            }
        };
        if f.read_version >= f.ops.version {
            continue;
        }
        listed.push(json!({ "file": rel, "from": f.read_version, "to": f.ops.version }));
        if !p.dry_run && file::write(&ed, &mut f).await? {
            written += 1;
        }
    }
    Ok(json!({ "files": listed, "written": written, "dry_run": p.dry_run, "refused": refused }))
}

fn kinds_answer(ed: &Editor) -> Reply {
    let mut list: Vec<serde_json::Value> = Kinds::of(ed)
        .all()
        .iter()
        .map(|k| {
            json!({
                "name": k.name, "extension": k.ext, "header": k.header, "version": k.version,
                "reference": k.reference_doc, "read_by": if k.limits.game { "the game, the editor" } else { "the editor" },
            })
        })
        .collect();
    list.push(json!({
        "name": "module manifest", "extension": MANIFEST, "header": "[module]", "version": 1,
        "reference": "the game's docs/dead-air/MODDING.md", "read_by": "the game, the editor",
    }));
    Ok(json!({ "kinds": list }))
}

const TOPIC: &str = "\
Assets are text files. Everything an author makes - quests (.nqasset), behaviour graphs
(.behasset), characters (.npcasset), PDA interfaces (.pdaasset), the module manifest (mod.ltx) -
is a file of the project an agent creates and edits with its own file tools. No command edits
an asset's content; the editor follows every change to the files at once.

The syntax, for every Lua asset: one `return { ... }` of literals - tables, strings, numbers,
booleans - with comments (`--`), trailing commas and long strings (`[[ ... ]]`). No code: an
identifier as a value, a call or an operator is refused. The first key is the kind's header and
the format version (`nq = 1`); the extension is the kind and the header agrees with it. Keys the
editor does not know are kept where they are; anything with a default may be left out. Things
are named by `id` / `name`, never by position.

The loop:
  1. asset.reference kind=<kind>   the format of a kind and what the linked game adds to it
                                   (the node kinds, the pickers' lists)
  2. edit the file
  3. asset.check [path=<file or folder>]   every problem, with file, line, column, code, text
                                   and a fix where it is mechanical; fail_on=error for a shell
  4. asset.find query=<text> | refs_to=<id>   where something is mentioned

asset.kinds lists the kinds; asset.migrate writes files of older format versions in the current
one. From a shell: Grizzly-Cmd.exe check [path], Grizzly-Cmd.exe reference <kind>.
The contract is docs/ASSETS.md.
";

/// The `asset` area, its commands and its topic.
pub fn register(reg: &mut Registry) {
    reg.area("asset", "The text assets: their kinds, checks with positions, references, search and versions");
    reg.cmd_sync(
        Spec::read(
            "asset.kinds",
            "Lists the kinds of assets: extension, header key, version, where the reference is.",
        ),
        |ed, _: &NoParams, _| kinds_answer(ed),
    );
    reg.cmd(
        Spec::read(
            "asset.check",
            "Checks a file, a folder or the project: every problem with its file, line and column.",
        )
        .example("path=quests/wolf.nqasset fail_on=error"),
        asset_check,
    );
    reg.cmd(
        Spec::read("asset.reference", "The format reference of a kind, with what the linked game adds.")
            .example("kind=quest"),
        asset_reference,
    );
    reg.cmd(
        Spec::read("asset.find", "Finds the files and lines that mention a text or an id.")
            .example("refs_to=wolf"),
        asset_find,
    );
    reg.cmd(
        Spec::write("asset.migrate", "Writes files of older format versions in the current one.")
            .example("dry_run=true"),
        asset_migrate,
    );
    reg.cmd(
        Spec::write(
            "asset.take_theirs",
            "Takes the file's value of a value both the file and an open document's unsaved edits changed.",
        )
        .example("path=quests/wolf.nqasset index=0"),
        asset_take_theirs,
    );
    reg.topic(Topic {
        name: "assets",
        summary: "Assets are text files: the syntax, the loop of edit, check and reference",
        text: |_| TOPIC.to_owned(),
    });
}
