//! `asset.check`: every problem of a file, a folder or the project, positioned - the kinds'
//! checks per file, and the module manifest's own and its areas' checks.

use std::path::Path;
use std::sync::Arc;

use gz_base::{Problem, RelPath, Severity};
use gz_build::{BuildChecks, BuildWalk, WalkScope};
use gz_editor::{CmdError, Editor};
use gz_project::{Project, is_editor_only_entry, is_tooling_entry};
use serde::Serialize;

use gz_mod::{ManifestDoc, Spot};

use crate::file;
use crate::kinds::Kinds;

/// The manifest's file name.
pub const MANIFEST: &str = "mod.ltx";

/// How many problems of each severity a check found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub error: usize,
    pub required: usize,
    pub warning: usize,
    pub note: usize,
}

impl Counts {
    #[must_use]
    pub fn of(problems: &[Problem]) -> Self {
        let mut c = Counts::default();
        for p in problems {
            match p.severity {
                Severity::Error => c.error += 1,
                Severity::Required => c.required += 1,
                Severity::Warning => c.warning += 1,
                Severity::Note => c.note += 1,
            }
        }
        c
    }
}

/// What a check found.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub files: usize,
    pub problems: Vec<Problem>,
}

/// Whether `path` lies in the editor's own scratch (`temp\`, `releases\`, the other folders the
/// project's rules call editor-only, at the project root): what a crashed packaging leaves there
/// is no quest of the module. A check that was asked for such a folder looks into it.
fn in_editor_scratch(under: &RelPath, path: &RelPath) -> bool {
    let first = |p: &RelPath| p.segments().next().is_some_and(is_editor_only_entry);
    first(path) && !first(under)
}

fn walk_disk(root: &Path, rel: &RelPath, out: &mut Vec<RelPath>) {
    let Ok(rd) = std::fs::read_dir(rel.to_fs(root)) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if is_tooling_entry(&name) || (rel.is_root() && is_editor_only_entry(&name)) {
            continue;
        }
        let Ok(child) = rel.join(&name) else { continue };
        match e.file_type() {
            Ok(t) if t.is_dir() => walk_disk(root, &child, out),
            Ok(t) if t.is_file() => out.push(child),
            _ => {}
        }
    }
}

/// The files a check of `path` covers (a file, a folder, `None`: the project): every file of
/// a kind the editor knows, and the manifest when it is in the scope.
pub async fn targets(editor: &Arc<Editor>, path: Option<&RelPath>) -> Result<Vec<RelPath>, CmdError> {
    let session = editor.project()?;
    let kinds = Kinds::of(editor);
    let under = path.cloned().unwrap_or_else(RelPath::root);
    let abs = session.fs(&under);
    let root = session.root.clone();
    let probe = abs.clone();
    let (is_file, is_dir) = editor.exec.io(move || (probe.is_file(), probe.is_dir())).await;
    if is_file {
        let name = under.file_name();
        if kinds.of_file(name).is_some() || name.eq_ignore_ascii_case(MANIFEST) {
            return Ok(vec![under]);
        }
        return Err(CmdError::invalid(format!(
            "{under} is no asset of a kind the editor knows (asset.kinds lists them)"
        )));
    }
    if !is_dir {
        return Err(CmdError::not_found(format!("{under} does not exist in the project")));
    }
    let files: Vec<RelPath> = match session.services.get::<Project>() {
        Some(project) => {
            project.tree.refresh().await;
            project.tree.snapshot().files_under(&under).into_iter().map(|(p, _)| p).collect()
        }
        None => {
            let start = under.clone();
            editor
                .exec
                .io(move || {
                    let mut out = Vec::new();
                    walk_disk(&root, &start, &mut out);
                    out
                })
                .await
        }
    };
    let mut out: Vec<RelPath> = files
        .into_iter()
        .filter(|p| !in_editor_scratch(&under, p))
        .filter(|p| {
            kinds.of_file(p.file_name()).is_some()
                || (p.parent().is_some_and(|d| d.is_root()) && p.file_name().eq_ignore_ascii_case(MANIFEST))
        })
        .collect();
    out.sort();
    Ok(out)
}

/// The problems of one asset file: what reading it found, then its kind's check.
pub async fn check_file(editor: &Arc<Editor>, path: &RelPath) -> Vec<Problem> {
    let f = match file::read(editor, path).await {
        Ok(f) => f,
        Err(e) => return vec![*e.problem],
    };
    let mut out = f.notes.clone();
    let check = f.ops.check;
    let found = check(editor.clone(), Arc::new(f)).await;
    out.extend(found.into_iter().map(|p| if p.file.is_some() { p } else { p.in_file(path.clone()) }));
    out
}

/// Puts a manifest problem at the line it is about, from the document's own lines: a `mod.<key>`
/// code at its key of `[module]`, a command's problems at its section, `[pda] layout` at its
/// key, and an area's at the [`Spot`] it names (a key of its section, a layer's key, or the
/// section's header for a problem about the whole section); a problem that names nothing goes to
/// the header of the section its check reads.
fn place_manifest(p: Problem, doc: &ManifestDoc, section: &str) -> Problem {
    let spot = if let Some(spot) = Spot::of(&p) {
        spot
    } else if let Some(k) = p.code.strip_prefix("mod.") {
        Spot::key("module", k)
    } else if p.field.as_deref() == Some("commands")
        && let Some(s) = &p.subject
    {
        Spot::section(&format!("command.{s}"))
    } else if let Some((s, k)) = p.field.as_deref().and_then(|f| f.split_once('.')) {
        Spot::key(s, k)
    } else {
        match p.field.as_deref() {
            Some(k) => Spot::key(section, k),
            None => Spot::section(section),
        }
    };
    let at = doc
        .locate(&spot)
        .or_else(|| doc.locate(&Spot::section(section)))
        .or_else(|| doc.locate(&Spot::section("module")));
    let mut p = p.in_file(RelPath::new(MANIFEST).unwrap_or_else(|_| RelPath::root()));
    // the carrier has done its work: the problem names its key the way the manifest's own do
    if Spot::of(&p).is_some() {
        p.subject = None;
        p.field = spot
            .key
            .as_deref()
            .or(spot.scope.as_ref().map(|(k, _)| k.as_str()))
            .map(|k| format!("{}.{k}", spot.section).into());
    }
    match at {
        Some(place) => place.put(p),
        None => p,
    }
}

/// The section of the manifest a build check reads, by what it says it checks.
fn section_of(stage: &str) -> &'static str {
    let s = stage.to_ascii_lowercase();
    if s.contains("screen") || s.contains("music") {
        "screens"
    } else if s.contains("world") {
        "world"
    } else if s.contains("command") {
        "commands"
    } else {
        "module"
    }
}

/// The manifest's own check, its commands' and the checks of the areas that read it.
pub async fn check_manifest(editor: &Arc<Editor>) -> Vec<Problem> {
    let Ok(session) = editor.project() else { return Vec::new() };
    let file = session.root.join(MANIFEST);
    let probe = file.clone();
    let readable = editor.exec.io(move || std::fs::metadata(probe).is_ok_and(|m| m.is_file())).await;
    if !readable {
        return vec![Problem::error("mod.ltx", "the project has no mod.ltx: it is no module")];
    }
    let Ok(service) = gz_mod::ManifestService::of_session(editor, &session).await else { return Vec::new() };
    let _ = service.sync().await;
    let snap = service.snapshot();
    let Some(m) = snap.manifest() else { return Vec::new() };
    // the manifest's own problems come placed: each at its line
    let doc = snap.doc().cloned().unwrap_or_default();
    let mut out: Vec<Problem> = gz_mod::validate(&doc);
    let (_, commands) = gz_mod::command_check(editor, &session, m).await;
    out.extend(commands.into_iter().map(|p| place_manifest(p, &doc, "commands")));
    for check in Kinds::of(editor).manifest_checks() {
        out.extend(check(editor.clone()).await.into_iter().map(|p| place_manifest(p, &doc, "module")));
    }
    if let Some(project) = session.services.get::<Project>() {
        let kinds = Kinds::of(editor);
        let areas: Vec<_> =
            BuildChecks::of(editor).checks().into_iter().filter(|c| !kinds.covered(c.stage())).collect();
        if !areas.is_empty() {
            project.tree.refresh().await;
            let tree = project.tree.snapshot();
            let (rules, exec) = (m.exclude.clone(), editor.exec.clone());
            let walk = Arc::new(
                editor.exec.io(move || BuildWalk::scan(&tree, &rules, WalkScope::Module, &exec)).await,
            );
            let found = futures::future::join_all(
                areas.iter().map(|c| c.check(editor.clone(), session.clone(), walk.clone())),
            )
            .await;
            for (c, problems) in areas.iter().zip(found) {
                let section = section_of(c.stage());
                out.extend(problems.into_iter().map(|p| place_manifest(p, &doc, section)));
            }
        }
    }
    out
}

/// Every problem of `path` (a file, a folder, `None`: the project), sorted by file and place.
pub async fn run(editor: &Arc<Editor>, path: Option<&RelPath>) -> Result<Report, CmdError> {
    let files = targets(editor, path).await?;
    let manifest = files
        .iter()
        .any(|f| f.file_name().eq_ignore_ascii_case(MANIFEST) && f.parent().is_some_and(|d| d.is_root()));
    let assets: Vec<&RelPath> =
        files.iter().filter(|f| !f.file_name().eq_ignore_ascii_case(MANIFEST)).collect();
    let mut problems: Vec<Problem> = futures::future::join_all(assets.iter().map(|f| check_file(editor, f)))
        .await
        .into_iter()
        .flatten()
        .collect();
    if manifest {
        problems.extend(check_manifest(editor).await);
    }
    problems.sort_by(|a, b| {
        a.file
            .as_ref()
            .map(RelPath::as_str)
            .cmp(&b.file.as_ref().map(RelPath::as_str))
            .then(a.line.cmp(&b.line))
            .then(a.column.cmp(&b.column))
            .then(a.severity.cmp(&b.severity))
    });
    Ok(Report { files: files.len(), problems })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_manifest_problem_is_put_at_the_line_its_spot_names() {
        let doc = ManifestDoc::parse(
            "[module]
id = od
version = x
[pda]
layout = a.pdaasset
[screen.a]
layer = a.dds
  layer = b.dds
[command.s]
help = x
",
        );
        let at = |p: Problem, section: &str| {
            let p = place_manifest(p, &doc, section);
            (p.line, p.column, p.file.map(|f| f.as_str().to_owned()))
        };
        let file = Some("mod.ltx".to_owned());
        // an area names the layer: its own line, with the key's column
        let layer = Spot::within("screen.a", "layer", 1, None).mark(Problem::error("E401", "x"));
        let placed = place_manifest(layer.clone(), &doc, "screens");
        assert_eq!((placed.subject, placed.field.as_deref()), (None, Some("screen.a.layer")));
        assert_eq!(at(layer, "screens"), (Some(8), Some(3), file.clone()));
        // a whole section: its header
        let section = Spot::section("screen.a").mark(Problem::warning("W406", "x"));
        assert_eq!(at(section, "screens"), (Some(6), Some(1), file.clone()));
        // the older ways to say it
        assert_eq!(at(Problem::error("mod.version", "x"), "module").0, Some(3));
        assert_eq!(at(Problem::error("pda", "x").in_field("pda.layout"), "module").0, Some(5));
        assert_eq!(at(Problem::error("E405", "x").about("s").in_field("commands"), "commands").0, Some(9));
        // what names nothing is at the header of the section its check reads, else the module's
        assert_eq!(at(Problem::error("E201", "x"), "world").0, Some(1));
        assert_eq!(at(Problem::error("E000", "x"), "screens").0, Some(1));
    }

    #[test]
    fn the_editors_scratch_is_no_content_unless_asked_for() {
        let rel = |p: &str| RelPath::new(p).unwrap();
        let stage = rel("temp/release_stage_1/modules/od/quests/q.nqasset");
        assert!(in_editor_scratch(&RelPath::root(), &stage));
        assert!(in_editor_scratch(&rel("quests"), &stage));
        assert!(in_editor_scratch(&RelPath::root(), &rel("releases/1.0.0/q.nqasset")));
        // a check asked for the folder itself looks into it
        assert!(!in_editor_scratch(&rel("temp"), &stage));
        // the author's folders are checked, whatever a build leaves out
        assert!(!in_editor_scratch(&RelPath::root(), &rel("quests/q.nqasset")));
        assert!(!in_editor_scratch(&RelPath::root(), &rel("rawdata/q.nqasset")));
    }
}
