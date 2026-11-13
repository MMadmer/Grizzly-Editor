//! One read path and one save path for every kind.
//!
//! Reading checks that the header agrees with the extension, refuses a version newer than the
//! kind's, migrates an older one in memory and lists the keys the kind does not know (kept, as
//! notes). Saving writes the difference between two values of the kind's model into the
//! document ([`LuaDoc::apply_diff`]), adds a missing header, and writes the document - only
//! when its text changed - atomically, past the project's write guard, with a token that tells
//! the project's watcher the change is the editor's own.

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::{Problem, RelPath, Severity};
use gz_editor::{CmdError, Editor};
use gz_luadata::path::show;
use gz_luadata::{LuaDoc, Seg, Slot, Span, Value};
use gz_project::Project;
use smol_str::SmolStr;

use crate::kinds::KindOps;

/// The largest asset a kind reading the game's limits takes.
const MAX_ASSET: u64 = 16 << 20;

/// A text asset as the editor holds it: its document (migrated to the current version) and
/// what reading it found.
#[derive(Clone)]
pub struct AssetFile {
    pub path: RelPath,
    pub abs: PathBuf,
    pub doc: LuaDoc,
    pub ops: Arc<KindOps>,
    /// The version the file was written in; the document holds the current one.
    pub read_version: u32,
    /// The document as the file holds it, when a migration changed it: where problems point.
    source: Option<LuaDoc>,
    /// The file has no header key: saving adds it.
    pub headerless: bool,
    /// What reading found: the header, an old version, keys the kind does not know, a byte
    /// order mark. Positioned.
    pub notes: Vec<Problem>,
    /// The text on disk as it was read or last written, to write only what changed.
    disk: Arc<str>,
}

/// Why a file is not read: a code to branch on and the problem, positioned.
#[derive(Clone, Debug)]
pub struct ReadFailure {
    /// `syntax`, `newer_version`, `encoding`, `io`, `migration`.
    pub code: &'static str,
    pub problem: Box<Problem>,
}

impl ReadFailure {
    #[must_use]
    pub fn new(code: &'static str, problem: Problem) -> Self {
        Self { code, problem: Box::new(problem) }
    }
}

impl From<ReadFailure> for CmdError {
    fn from(f: ReadFailure) -> Self {
        let data = serde_json::to_value(&f.problem).unwrap_or_default();
        CmdError::new(f.code, f.problem.message).with_data(serde_json::json!({ "problem": data }))
    }
}

/// The base name of a path without its extension: the chunk name of the reader's messages.
fn chunk_name(rel: &RelPath) -> String {
    let name = rel.file_name();
    name.rfind('.').map_or(name, |i| &name[..i]).to_owned()
}

/// A problem at a span of the file.
#[must_use]
pub fn at(p: Problem, file: &RelPath, s: Span) -> Problem {
    p.in_file(file.clone()).at(s.line, s.column, s.end_line, s.end_column)
}

/// A problem at what a path names in a document, or at the nearest thing that is written.
#[must_use]
pub fn at_path(p: Problem, file: &RelPath, doc: &LuaDoc, path: &[Seg]) -> Problem {
    at(p, file, doc.nearest_span(path))
}

fn version_of(v: &Value) -> Option<u32> {
    match v {
        Value::Number(n) if n.fract() == 0.0 && *n >= 1.0 && *n <= f64::from(u32::MAX) => Some(*n as u32),
        _ => None,
    }
}

/// The patterns of `known` as the paths of the tables they name in `v`.
fn tables_of(v: &Value, pattern: &[&str], at: &mut Vec<Seg>, out: &mut Vec<(Vec<Seg>, Value)>) {
    let Some((first, rest)) = pattern.split_first() else {
        if v.is_table() {
            out.push((at.clone(), v.clone()));
        }
        return;
    };
    let Value::Table(t) = v else { return };
    if *first == "*" {
        for (i, x) in t.arr.iter().enumerate() {
            at.push(Seg::Index(i));
            tables_of(x, rest, at, out);
            at.pop();
        }
        for (k, x) in &t.fields {
            at.push(Seg::Key(k.clone()));
            tables_of(x, rest, at, out);
            at.pop();
        }
    } else if let Some(x) = t.get(first) {
        at.push(Seg::key(first));
        tables_of(x, rest, at, out);
        at.pop();
    }
}

/// Reads `text` as a file of `ops`. `others` are the header keys of the other kinds (a file
/// that carries one of them is in the wrong file).
pub fn read_text(
    ops: &Arc<KindOps>,
    path: RelPath,
    abs: PathBuf,
    text: &str,
    others: &[(&'static str, &'static str)],
) -> Result<AssetFile, ReadFailure> {
    let mut doc = LuaDoc::parse_with(text, &chunk_name(&path), ops.limits).map_err(|e| {
        ReadFailure::new(
            "syntax",
            Problem::error("syntax", e.message).in_file(path.clone()).at(
                e.line.max(1),
                e.column.max(1),
                e.line.max(1),
                e.column.max(1) + 1,
            ),
        )
    })?;
    let mut notes = Vec::new();
    let head = doc.head_span();
    let header = [Seg::key(ops.header)];
    let current = ops.version;
    let fix = format!("{} = {current}", ops.header);
    let mut headerless = false;
    let read_version = match doc.get(&header) {
        None => {
            headerless = true;
            if let Some((h, kind)) = others.iter().find(|(h, _)| doc.get(&[Seg::key(h)]).is_some()) {
                let span = doc.span(&[Seg::key(h)]).unwrap_or(head);
                notes.push(at(
                    Problem::error(
                        "E001",
                        format!(
                            "'{h}' is the header of a {kind} - this file's extension says it is a {}",
                            ops.name
                        ),
                    )
                    .with_fix(fix.clone()),
                    &path,
                    span,
                ));
            } else {
                notes.push(at(
                    Problem::error(
                        "E001",
                        format!("the header '{fix}' is missing: the game refuses the file without it (saving adds it)"),
                    )
                    .with_fix(fix.clone()),
                    &path,
                    head,
                ));
            }
            current
        }
        Some(v) => match version_of(&v) {
            None => {
                let span = doc.value_span(&header).unwrap_or(head);
                notes.push(at(
                    Problem::error("E001", format!("'{}' is not a version number", ops.header))
                        .with_fix(fix.clone()),
                    &path,
                    span,
                ));
                current
            }
            Some(n) if n > current => {
                let span = doc.span(&header).unwrap_or(head);
                return Err(ReadFailure::new(
                    "newer_version",
                    at(
                        Problem::error(
                            "newer_version",
                            format!(
                                "{path} was made by a newer Grizzly Editor ({} = {n}; this one reads up to {current}): update the editor to open it",
                                ops.header
                            ),
                        ),
                        &path,
                        span,
                    ),
                ));
            }
            Some(n) => n,
        },
    };
    let source = (read_version < current).then(|| doc.clone());
    if read_version < current {
        for step in read_version..current {
            let Some(migrate) = ops.migrations.get((step - 1) as usize) else {
                return Err(ReadFailure::new(
                    "migration",
                    Problem::error(
                        "migration",
                        format!(
                            "{path}: version {step} of a {} cannot be read: the editor has no step from it",
                            ops.name
                        ),
                    )
                    .in_file(path.clone()),
                ));
            };
            migrate(&mut doc).and_then(|()| doc.set(&header, &Value::from(step + 1))).map_err(|e| {
                ReadFailure::new(
                    "migration",
                    Problem::error(
                        "migration",
                        format!("{path}: lifting version {step} to {} failed: {}", step + 1, e.message),
                    )
                    .in_file(path.clone()),
                )
            })?;
        }
        notes.push(at(
            Problem::note(
                "old_version",
                format!(
                    "written in version {read_version} of the format: read as version {current}; saving (or asset.migrate) writes version {current}"
                ),
            ),
            &path,
            doc.span(&header).unwrap_or(head),
        ));
    }
    if doc.has_bom() && ops.limits.game {
        notes.push(at(
            Problem::error("bom", "the file starts with a byte order mark, which the game refuses: save it as UTF-8 without one"),
            &path,
            Span { line: 1, column: 1, end_line: 1, end_column: 2 },
        ));
    }
    notes.extend(unknown_keys(ops, &path, source.as_ref().unwrap_or(&doc)));
    Ok(AssetFile {
        path,
        abs,
        doc,
        ops: ops.clone(),
        read_version,
        headerless,
        notes,
        source,
        disk: Arc::from(text),
    })
}

/// The keys the kind does not know at its known levels: kept, listed once each as a note.
fn unknown_keys(ops: &KindOps, path: &RelPath, doc: &LuaDoc) -> Vec<Problem> {
    if ops.known.is_empty() {
        return Vec::new();
    }
    let value = doc.value();
    let mut out = Vec::new();
    for (pattern, keys) in ops.known {
        let segs: Vec<&str> = if pattern.is_empty() { Vec::new() } else { pattern.split('.').collect() };
        let mut found = Vec::new();
        tables_of(&value, &segs, &mut Vec::new(), &mut found);
        for (at_path, table) in found {
            for (k, _) in table.fields() {
                if keys.contains(&k.as_str()) || (at_path.is_empty() && k.as_str() == ops.header) {
                    continue;
                }
                let mut p = at_path.clone();
                p.push(Seg::Key(SmolStr::new(k)));
                let where_ =
                    if at_path.is_empty() { String::new() } else { format!(" in {}", show(&at_path)) };
                out.push(at(
                    Problem::note("unknown_key", format!("unknown key '{k}'{where_} (kept)")),
                    path,
                    doc.nearest_span(&p),
                ));
            }
        }
    }
    out
}

impl AssetFile {
    /// The document problems point into: the file as it is on disk (before a migration).
    #[must_use]
    pub fn place_doc(&self) -> &LuaDoc {
        self.source.as_ref().unwrap_or(&self.doc)
    }

    /// A problem at what a path names in the file (or at the nearest thing that is written).
    #[must_use]
    pub fn place(&self, p: Problem, path: &[Seg]) -> Problem {
        at_path(p, &self.path, self.place_doc(), path)
    }

    /// A problem of the whole file: at its `return {`.
    #[must_use]
    pub fn place_head(&self, p: Problem) -> Problem {
        at(p, &self.path, self.place_doc().head_span())
    }

    /// Whether the text on disk differs from the document (a migration, an unsaved edit).
    #[must_use]
    pub fn changed(&self) -> bool {
        *self.disk != *self.doc.text()
    }

    /// The text that was read (or last written).
    #[must_use]
    pub fn disk_text(&self) -> &str {
        &self.disk
    }

    /// Every problem reading the file found, of at least `severity`.
    #[must_use]
    pub fn notes_of(&self, severity: Severity) -> Vec<Problem> {
        self.notes.iter().filter(|p| p.severity <= severity).cloned().collect()
    }

    /// Adds the header key when the file has none (before every other key).
    pub fn ensure_header(&mut self) -> Result<(), CmdError> {
        if !self.headerless {
            return Ok(());
        }
        self.doc
            .insert(&[], Slot::Head(SmolStr::new(self.ops.header)), &Value::from(self.ops.version))
            .map_err(|e| {
                CmdError::internal(format!("{}: the header could not be added: {}", self.path, e.message))
            })?;
        self.headerless = false;
        Ok(())
    }

    /// Writes `old -> new` (the model as it was read, as it is now) into the document; true
    /// when the document's text changed. Nothing is written to disk.
    pub fn apply(&mut self, old: &Value, new: &Value) -> Result<bool, CmdError> {
        let rules = (self.ops.diff_rules)();
        let changed = self.doc.apply_diff(old, new, &rules).map_err(|e| {
            CmdError::internal(format!("{}: the change could not be written: {}", self.path, e.message))
        })?;
        if changed {
            self.ensure_header()?;
        }
        Ok(changed)
    }
}

/// The header keys of every kind but `ops`, with the kind's name.
#[must_use]
pub fn other_headers(editor: &Editor, ops: &KindOps) -> Vec<(&'static str, &'static str)> {
    crate::Kinds::of(editor)
        .all()
        .iter()
        .filter(|k| k.header != ops.header)
        .map(|k| (k.header, k.name))
        .collect()
}

/// Reads a file of the open project. A file of no registered kind is `not_found`.
pub async fn read(editor: &Editor, path: &RelPath) -> Result<AssetFile, ReadFailure> {
    let fail = |code: &'static str, msg: String| {
        ReadFailure::new(code, Problem::error(code, msg).in_file(path.clone()))
    };
    let session = editor.project().map_err(|e| fail("io", e.message))?;
    let ops = crate::Kinds::of(editor)
        .of_file(path.file_name())
        .ok_or_else(|| fail("not_found", format!("{path} is no asset of a kind the editor knows")))?;
    let abs = session.fs(path);
    let file = abs.clone();
    let limit = if ops.limits.game { MAX_ASSET } else { u64::MAX };
    let bytes = editor
        .exec
        .io(move || -> Result<Vec<u8>, String> {
            let meta = std::fs::metadata(&file).map_err(|e| e.to_string())?;
            if meta.len() > limit {
                return Err("the file is larger than 16 MB".into());
            }
            std::fs::read(&file).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| fail("io", format!("{path} could not be read: {e}")))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| fail("encoding", format!("{path} is not UTF-8 text: save it as UTF-8")))?;
    let others = other_headers(editor, &ops);
    read_text(&ops, path.clone(), abs, &text, &others)
}

/// Writes a text to a file of the open project: past the write guard, atomically, with the
/// token that keeps the watcher from calling it an outside change; the tree hears of it.
pub async fn write_text(editor: &Editor, path: &RelPath, text: String) -> Result<(), CmdError> {
    let session = editor.project()?;
    let abs = session.fs(path);
    let project = session.services.get::<Project>();
    let guard =
        project.as_ref().map_or_else(|| gz_editor::fsx::WriteGuard::new("", None), |p| p.write_guard(None));
    guard.check(&abs).map_err(|e| CmdError::new("refused", e.0))?;
    let (file, rel, watch) = (abs.clone(), path.clone(), project.clone());
    editor
        .exec
        .io(move || -> std::io::Result<()> {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            gz_editor::fsx::write_atomic(&file, text.as_bytes())?;
            if let Some(p) = watch {
                p.tree.changes().note_written(&rel, text.as_bytes());
            }
            Ok(())
        })
        .await
        .map_err(|e| CmdError::io(format!("{path} could not be written: {e}")))?;
    if let Some(p) = project {
        p.tree.mark_fs(&abs);
    }
    Ok(())
}

/// Writes the file's document when its text differs from the disk's. True when it wrote.
pub async fn write(editor: &Editor, file: &mut AssetFile) -> Result<bool, CmdError> {
    if !file.changed() {
        return Ok(false);
    }
    let text = file.doc.text().to_owned();
    write_text(editor, &file.path, text.clone()).await?;
    file.disk = Arc::from(text);
    file.source = None;
    file.read_version = file.ops.version;
    file.notes.retain(|n| n.code != "old_version");
    Ok(true)
}

/// The one save path: the model's difference written into the document (a missing header
/// added with it), the document written when its text differs from the disk's. A model that
/// did not change writes nothing; a file migrated on read is written in the current version.
/// True when the file was written.
pub async fn save(editor: &Editor, file: &mut AssetFile, old: &Value, new: &Value) -> Result<bool, CmdError> {
    file.apply(old, new)?;
    write(editor, file).await
}
