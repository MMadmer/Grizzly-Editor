//! The application (docs/UPDATES.md, section 5): the stage verified once more, then a transaction
//! of renames journalled in `apply.journal` - a line per move, on disk before the move - that is
//! rolled back in reverse on any failure, so the installation is either the new version or
//! exactly what it was. Nothing is ever copied: every move is a rename on one volume.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::hashes::HashCache;
use crate::manifest::{MANIFEST, Manifest, path_key};
use crate::stage::{Stage, StageState};
use crate::sys;

const HEADER: &str = "grizzly-editor.update/1 journal";

/// What a journal says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalState {
    pub from: String,
    pub to: String,
    /// Every move happened: the installation is the new version.
    pub complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    /// A folder the transaction made (relative to the root).
    Dir(String),
    /// A rename, both relative to the root.
    Move(String, String),
}

fn rel_of(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/")
}

fn abs(root: &Path, rel: &str) -> PathBuf {
    root.join(rel.replace('/', "\\"))
}

/// Why a move failed, for the author.
fn why(e: &io::Error) -> String {
    match e.raw_os_error() {
        Some(32 | 33) => "it is in use by another program".to_owned(),
        Some(5) => "access was denied".to_owned(),
        _ => e.to_string(),
    }
}

/// Checks the stage once more against the manifest and every file the delta did not need against
/// the installation. Answers the paths the stage brings.
pub fn verify(
    root: &Path,
    stage: &Stage,
    state: &StageState,
    new: &Manifest,
    cache: &HashCache,
) -> Result<Vec<String>, String> {
    let mut staged = Vec::new();
    for row in &new.rows {
        if state.passed.contains_key(&path_key(&row.path)) {
            let file = stage.file(&row.path);
            let (size, sha) = crate::hashes::sha256_file(&file).map_err(|_| {
                format!("{} is missing from the update's stage: check for the update again", row.path)
            })?;
            if size != row.size || sha != row.sha256 {
                return Err(format!(
                    "{} in the update's stage is not what the manifest wants: check for the update again",
                    row.path
                ));
            }
            staged.push(row.path.clone());
            continue;
        }
        match cache.of(root, &row.path) {
            Ok(Some((size, sha))) if size == row.size && sha == row.sha256 => {}
            _ => {
                return Err(format!(
                    "{} changed since the update was downloaded: check for the update again",
                    row.path
                ));
            }
        }
    }
    cache.save();
    Ok(staged)
}

struct Journal {
    file: File,
    root: PathBuf,
    done: Vec<Action>,
    wait: Duration,
}

impl Journal {
    fn line(&mut self, text: &str) -> io::Result<()> {
        self.file.write_all(text.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.file.sync_data()
    }

    /// Makes the folders `target` needs, each one journalled before it is made.
    fn parents(&mut self, target: &Path) -> io::Result<()> {
        let mut missing = Vec::new();
        let mut dir = target.parent();
        while let Some(d) = dir {
            if d == self.root || d.exists() {
                break;
            }
            missing.push(d.to_path_buf());
            dir = d.parent();
        }
        for d in missing.into_iter().rev() {
            let rel = rel_of(&self.root, &d);
            self.line(&format!("dir {rel}"))?;
            fs::create_dir(&d)?;
            self.done.push(Action::Dir(rel));
        }
        Ok(())
    }

    /// One rename, journalled first, tried for `wait` (a scanner reading a fresh program is the
    /// usual reason one fails).
    fn mv(&mut self, from: &Path, to: &Path) -> Result<(), String> {
        let (a, b) = (rel_of(&self.root, from), rel_of(&self.root, to));
        self.line(&format!("move {a}\t{b}")).map_err(|e| format!("the journal could not be written: {e}"))?;
        rename_patiently(from, to, self.wait).map_err(|e| format!("{a} could not be moved: {}", why(&e)))?;
        self.done.push(Action::Move(a, b));
        Ok(())
    }
}

fn rename_patiently(from: &Path, to: &Path, wait: Duration) -> io::Result<()> {
    let started = Instant::now();
    loop {
        match sys::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if started.elapsed() >= wait || e.kind() == io::ErrorKind::NotFound => return Err(e),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// Undoes actions in reverse. A move that never happened (its target is not there) is skipped.
fn undo(root: &Path, actions: &[Action], wait: Duration) -> Result<(), String> {
    let mut first_error = None;
    for a in actions.iter().rev() {
        match a {
            Action::Move(from, to) => {
                let (f, t) = (abs(root, from), abs(root, to));
                if t.exists() && !f.exists() {
                    if let Some(dir) = f.parent() {
                        let _ = fs::create_dir_all(dir);
                    }
                    if let Err(e) = rename_patiently(&t, &f, wait) {
                        first_error.get_or_insert(format!("{to} could not be moved back: {}", why(&e)));
                    }
                }
            }
            Action::Dir(d) => {
                let _ = fs::remove_dir(abs(root, d));
            }
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Runs the transaction. `old` is the installed manifest. On failure everything done is undone
/// and the error says what stopped it; the stage is then whole again.
pub fn apply(
    root: &Path,
    stage: &Stage,
    staged: &[String],
    old: Option<&Manifest>,
    new: &Manifest,
    from: &str,
    wait: Duration,
) -> Result<(), String> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(stage.journal())
        .map_err(|e| format!("the update's journal could not be written: {e}"))?;
    let mut j = Journal { file, root: root.to_path_buf(), done: Vec::new(), wait };
    let result = run(&mut j, root, stage, staged, old, new, from);
    match result {
        Ok(()) => {
            tidy_dropped(root, old, new);
            Ok(())
        }
        Err(e) => {
            let undone = undo(root, &j.done, wait);
            drop(j);
            let _ = fs::remove_file(stage.journal());
            match undone {
                Ok(()) => Err(format!("{e}. The installation is as it was.")),
                Err(u) => Err(format!("{e}, and the update could not be undone: {u}")),
            }
        }
    }
}

fn run(
    j: &mut Journal,
    root: &Path,
    stage: &Stage,
    staged: &[String],
    old: Option<&Manifest>,
    new: &Manifest,
    from: &str,
) -> Result<(), String> {
    let io = |e: io::Error| format!("the update's journal could not be written: {e}");
    j.line(HEADER).map_err(io)?;
    j.line(&format!("from {from}")).map_err(io)?;
    j.line(&format!("to {}", new.version)).map_err(io)?;
    let old_dir = stage.old();
    let aside = |rel: &str| -> Result<PathBuf, String> {
        let p = old_dir.join(rel.replace('/', "\\"));
        if let Some(dir) = p.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("the update's stage could not be written: {e}"))?;
        }
        Ok(p)
    };
    // the new files, each after the one it replaces moved aside
    for rel in staged {
        let target = abs(root, rel);
        if target.exists() {
            j.mv(&target, &aside(rel)?)?;
        }
        j.parents(&target).map_err(|e| format!("a folder for {rel} could not be made: {}", why(&e)))?;
        j.mv(&stage.file(rel), &target)?;
    }
    // what the new version no longer has
    if let Some(old) = old {
        for row in &old.rows {
            if new.get(&row.path).is_none() {
                let target = abs(root, &row.path);
                if target.is_file() {
                    j.mv(&target, &aside(&row.path)?)?;
                }
            }
        }
    }
    // the manifest last: until it is in, the installation does not claim the new version
    let installed = root.join(MANIFEST);
    if installed.exists() {
        j.mv(&installed, &aside(MANIFEST)?)?;
    }
    j.mv(&stage.manifest(), &installed)?;
    j.line("complete").map_err(io)?;
    Ok(())
}

/// The folders the dropped files leave empty go too (never the root).
fn tidy_dropped(root: &Path, old: Option<&Manifest>, new: &Manifest) {
    let Some(old) = old else { return };
    for row in old.rows.iter().filter(|r| new.get(&r.path).is_none()) {
        let mut dir = abs(root, &row.path).parent().map(Path::to_path_buf);
        while let Some(d) = dir {
            if d == root || fs::remove_dir(&d).is_err() {
                break;
            }
            dir = d.parent().map(Path::to_path_buf);
        }
    }
}

fn parse(text: &str) -> (Option<JournalState>, Vec<Action>) {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return (None, Vec::new());
    }
    let mut state = JournalState { from: String::new(), to: String::new(), complete: false };
    let mut actions = Vec::new();
    for line in lines {
        if let Some(v) = line.strip_prefix("from ") {
            state.from = v.to_owned();
        } else if let Some(v) = line.strip_prefix("to ") {
            state.to = v.to_owned();
        } else if let Some(v) = line.strip_prefix("dir ") {
            actions.push(Action::Dir(v.to_owned()));
        } else if let Some((a, b)) = line.strip_prefix("move ").and_then(|v| v.split_once('\t')) {
            actions.push(Action::Move(a.to_owned(), b.to_owned()));
        } else if line == "complete" {
            state.complete = true;
        }
    }
    (Some(state), actions)
}

/// What the journal of a stage says, when it has one.
#[must_use]
pub fn journal_state(stage: &Stage) -> Option<JournalState> {
    parse(&fs::read_to_string(stage.journal()).ok()?).0
}

/// Undoes a journalled transaction - one a crash cut short, or a complete one whose new version
/// did not start - and removes the journal. The stage is whole again afterwards.
pub fn roll_back(root: &Path, stage: &Stage, wait: Duration) -> Result<(), String> {
    let Ok(text) = fs::read_to_string(stage.journal()) else { return Ok(()) };
    let (_, actions) = parse(&text);
    undo(root, &actions, wait)?;
    fs::remove_file(stage.journal()).map_err(|e| format!("the update's journal could not be removed: {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::manifest::{Row, SCHEMA};
    use crate::stage::Mode;
    use crate::version::Version;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;

    fn row(path: &str, data: &[u8]) -> Row {
        Row { sha256: hex::encode(Sha256::digest(data)), size: data.len() as u64, path: path.into() }
    }

    fn manifest(v: &str, files: &[(&str, &[u8])]) -> Manifest {
        Manifest { version: Version::parse(v).unwrap(), rows: files.iter().map(|(p, d)| row(p, d)).collect() }
    }

    /// An installation of 0.7.0 and a stage of 0.7.1 that replaces a.txt, adds sub/new.txt and
    /// drops gone/old.txt; mine.txt is the author's.
    fn setup(dir: &Path) -> (Manifest, Manifest, Stage, StageState) {
        let old = manifest("0.7.0", &[("a.txt", b"a0"), ("b.txt", b"b"), ("gone/old.txt", b"x")]);
        let new = manifest("0.7.1", &[("a.txt", b"a1"), ("b.txt", b"b"), ("sub/new.txt", b"n")]);
        for (p, d) in [("a.txt", &b"a0"[..]), ("b.txt", b"b"), ("gone/old.txt", b"x"), ("mine.txt", b"mine")]
        {
            let f = dir.join(p.replace('/', "\\"));
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, d).unwrap();
        }
        fs::write(dir.join(MANIFEST), old.to_text()).unwrap();
        let stage = Stage::new(dir, "0.7.1");
        let mut state = StageState {
            version: "0.7.1".into(),
            asset: 1,
            size: 1,
            manifest_sha: String::new(),
            mode: Mode::Ranges,
            passed: BTreeMap::new(),
        };
        for (p, d) in [("a.txt", &b"a1"[..]), ("sub/new.txt", b"n")] {
            let f = stage.file(p);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, d).unwrap();
            let r = row(p, d);
            state.passed.insert(path_key(p), (r.sha256, r.size, p.into()));
        }
        fs::write(stage.manifest(), new.to_text()).unwrap();
        (old, new, stage, state)
    }

    fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        fn walk(root: &Path, d: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
            for e in fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.file_name().is_some_and(|n| n == ".update") {
                    continue;
                }
                if p.is_dir() {
                    out.insert(rel_of(root, &p) + "/", Vec::new());
                    walk(root, &p, out);
                } else {
                    out.insert(rel_of(root, &p), fs::read(&p).unwrap());
                }
            }
        }
        walk(dir, dir, &mut out);
        out
    }

    #[test]
    fn apply_replaces_adds_and_drops_and_touches_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new, stage, state) = setup(dir.path());
        let cache = HashCache::load(dir.path().join("cache.ltx"));
        let staged = verify(dir.path(), &stage, &state, &new, &cache).unwrap();
        assert_eq!(staged, ["a.txt", "sub/new.txt"]);
        apply(dir.path(), &stage, &staged, Some(&old), &new, "0.7.0", Duration::ZERO).unwrap();
        let now = snapshot(dir.path());
        assert_eq!(now.get("a.txt").unwrap(), b"a1");
        assert_eq!(now.get("sub/new.txt").unwrap(), b"n");
        assert_eq!(now.get("mine.txt").unwrap(), b"mine");
        assert!(!now.contains_key("gone/old.txt") && !now.contains_key("gone/"), "{:?}", now.keys());
        assert_eq!(now.get(MANIFEST).unwrap(), new.to_text().as_bytes());
        assert_eq!(fs::read(stage.old().join("a.txt")).unwrap(), b"a0");
        let j = journal_state(&stage).unwrap();
        assert_eq!(j, JournalState { from: "0.7.0".into(), to: "0.7.1".into(), complete: true });
        // a new version that does not start is rolled back to exactly what was there
        roll_back(dir.path(), &stage, Duration::ZERO).unwrap();
        let back = snapshot(dir.path());
        assert_eq!(back.get("a.txt").unwrap(), b"a0");
        assert_eq!(back.get("gone/old.txt").unwrap(), b"x");
        assert!(!back.contains_key("sub/"));
        assert_eq!(back.get(MANIFEST).unwrap(), old.to_text().as_bytes());
        // and the stage is whole again
        assert!(verify(dir.path(), &stage, &state, &new, &cache).is_ok());
        assert!(journal_state(&stage).is_none());
    }

    #[test]
    fn a_file_in_the_way_rolls_the_whole_transaction_back() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new, stage, state) = setup(dir.path());
        let before = snapshot(dir.path());
        // the last move of the transaction cannot happen: the stage's manifest is gone
        fs::remove_file(stage.manifest()).unwrap();
        let err = apply(
            dir.path(),
            &stage,
            &["a.txt".into(), "sub/new.txt".into()],
            Some(&old),
            &new,
            "0.7.0",
            Duration::ZERO,
        )
        .unwrap_err();
        assert!(err.starts_with(".update/0.7.1/update-manifest.txt could not be moved"), "{err}");
        assert!(err.ends_with("The installation is as it was."), "{err}");
        assert_eq!(snapshot(dir.path()), before);
        assert!(stage.file("a.txt").is_file() && stage.file("sub/new.txt").is_file());
        assert!(journal_state(&stage).is_none());
        let _ = state;
    }

    #[test]
    fn a_crash_inside_the_transaction_is_undone_from_the_journal() {
        let dir = tempfile::tempdir().unwrap();
        let (_old, _new, stage, _state) = setup(dir.path());
        let before = snapshot(dir.path());
        // what a crash after two moves leaves: the journal and the moves, no "complete"
        fs::create_dir_all(stage.old()).unwrap();
        sys::rename(&dir.path().join("a.txt"), &stage.old().join("a.txt")).unwrap();
        sys::rename(&stage.file("a.txt"), &dir.path().join("a.txt")).unwrap();
        fs::write(
            stage.journal(),
            format!(
                "{HEADER}\nfrom 0.7.0\nto 0.7.1\nmove a.txt\t.update/0.7.1/old/a.txt\nmove .update/0.7.1/files/a.txt\ta.txt\nmove b.txt\t.update/0.7.1/old/b.txt\n"
            ),
        )
        .unwrap();
        assert!(!journal_state(&stage).unwrap().complete);
        roll_back(dir.path(), &stage, Duration::ZERO).unwrap();
        assert_eq!(snapshot(dir.path()), before);
        assert_eq!(fs::read(stage.file("a.txt")).unwrap(), b"a1");
        let _ = SCHEMA;
    }
}
