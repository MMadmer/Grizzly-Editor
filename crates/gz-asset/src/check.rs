//! `asset.check`: every problem of a file, a folder or the project, positioned - the kinds'
//! checks per file, and the module manifest's own and its areas' checks.

use std::path::Path;
use std::sync::Arc;

use gz_base::{Problem, RelPath, Severity};
use gz_build::{BuildChecks, BuildWalk, WalkScope};
use gz_editor::{CmdError, Editor};
use gz_project::{Project, is_tooling_entry};
use serde::Serialize;

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

fn walk_disk(root: &Path, rel: &RelPath, out: &mut Vec<RelPath>) {
    let Ok(rd) = std::fs::read_dir(rel.to_fs(root)) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if is_tooling_entry(&name) {
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

/// The line of `[section]` and of `key` in it, in a manifest's text (1-based).
fn ltx_line(text: &str, section: &str, key: Option<&str>) -> Option<(u32, u32)> {
    let mut in_section = false;
    let mut section_line = None;
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        let n = u32::try_from(i + 1).unwrap_or(u32::MAX);
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest.split(']').next().unwrap_or("").trim();
            in_section = name.eq_ignore_ascii_case(section);
            if in_section {
                section_line = Some((n, u32::try_from(raw.len() - raw.trim_start().len() + 1).unwrap_or(1)));
                if key.is_none() {
                    return section_line;
                }
            }
            continue;
        }
        if in_section
            && let Some(k) = key
            && line.split(['=', ' ', '\t']).next().is_some_and(|x| x.eq_ignore_ascii_case(k))
        {
            return Some((n, u32::try_from(raw.len() - raw.trim_start().len() + 1).unwrap_or(1)));
        }
    }
    section_line
}

/// Puts a manifest problem at the line it is about: `mod.<key>` codes in `[module]`, a
/// command's problems at its section, an area's at the section it reads.
fn place_manifest(p: Problem, text: &str, section: &str) -> Problem {
    let (sec, key): (String, Option<String>) = if let Some(k) = p.code.strip_prefix("mod.") {
        ("module".into(), Some(k.to_owned()))
    } else if p.field.as_deref() == Some("commands")
        && let Some(s) = &p.subject
    {
        (format!("command.{s}"), None)
    } else if let Some((s, k)) = p.field.as_deref().and_then(|f| f.split_once('.')) {
        (s.to_owned(), Some(k.to_owned()))
    } else {
        (section.to_owned(), p.field.as_ref().map(ToString::to_string))
    };
    let at = ltx_line(text, &sec, key.as_deref())
        .or_else(|| ltx_line(text, section, None))
        .or_else(|| ltx_line(text, "module", None));
    let p = p.in_file(RelPath::new(MANIFEST).unwrap_or_else(|_| RelPath::root()));
    match at {
        Some((line, col)) => p.at(line, col, line, col + 1),
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
    let text = editor.exec.io(move || std::fs::read_to_string(probe).ok()).await;
    let Some(text) = text else {
        return vec![Problem::error("mod.ltx", "the project has no mod.ltx: it is no module")];
    };
    let Ok(service) = gz_mod::ManifestService::of_session(editor, &session).await else { return Vec::new() };
    let _ = service.sync().await;
    let snap = service.snapshot();
    let Some(m) = snap.manifest() else { return Vec::new() };
    let mut out: Vec<Problem> =
        gz_mod::validate(m).into_iter().map(|p| place_manifest(p, &text, "module")).collect();
    let (_, commands) = gz_mod::command_check(editor, &session, m).await;
    out.extend(commands.into_iter().map(|p| place_manifest(p, &text, "commands")));
    for check in Kinds::of(editor).manifest_checks() {
        out.extend(check(editor.clone()).await.into_iter().map(|p| place_manifest(p, &text, "module")));
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
                out.extend(problems.into_iter().map(|p| place_manifest(p, &text, section)));
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
