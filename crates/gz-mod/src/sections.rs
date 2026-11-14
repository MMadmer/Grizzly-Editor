//! The sections the module itself brings: every `[section]` of the `.ltx` files under the
//! project's `gamedata\configs` and of what `[vfs]` mounts under `configs\`.
//!
//! The validators that warn about an item, a section or a squad the linked game lacks ask this
//! index too ([`OwnSections::has`]): a module that brings `od_parcel` is not told that the game
//! has no `od_parcel`. The index is built once per project on the IO pool and dropped when one
//! of the project's `.ltx` files changes (the project's watcher), so a frame never waits for it
//! and never reads a stale one for long.

use std::path::Path;
use std::sync::Arc;

use gz_base::RelPath;
use gz_editor::{Editor, ProjectSession};
use gz_project::{FileTree, Filter, Project};
use parking_lot::Mutex;
use rustc_hash::FxHashSet;

use crate::doc::ManifestDoc;
use crate::exclude::outside_build;
use crate::model::{FILE, Manifest};

/// The section names of the module, lower case.
#[derive(Clone, Debug, Default)]
pub struct SectionSet {
    names: FxHashSet<String>,
    /// Moves with every build of the index: what a cache of the checks keys on.
    generation: u64,
}

impl SectionSet {
    /// Whether the module defines `section` (case does not matter).
    #[must_use]
    pub fn has(&self, section: &str) -> bool {
        !section.is_empty() && self.names.contains(&section.to_ascii_lowercase())
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.names.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The sections of one `.ltx` text: `[name]` and `[name]:parent` headers.
    pub fn add_text(&mut self, text: &str) {
        for line in text.lines() {
            let line = line.trim_start_matches('\u{feff}').trim_start();
            let Some(rest) = line.strip_prefix('[') else { continue };
            let Some(end) = rest.find(']') else { continue };
            let name = rest.get(..end).unwrap_or("").trim();
            if !name.is_empty() {
                self.names.insert(name.to_ascii_lowercase());
            }
        }
    }
}

/// The `.ltx` files the game reads as the module's configs: under `gamedata\configs`, and what
/// a `[vfs]` entry for `configs\...` maps to (a file or a folder of the project). Files the build
/// leaves out are not the module's.
#[must_use]
pub fn config_files(tree: &FileTree, doc: Option<&ManifestDoc>, exclude: &[String]) -> Vec<RelPath> {
    let mut out: Vec<RelPath> = Vec::new();
    let take = |under: &RelPath, out: &mut Vec<RelPath>| {
        if tree.entry(under).is_some_and(|e| e.is_file()) {
            out.push(under.clone());
            return;
        }
        for (rel, entry) in tree.files_under(under) {
            if entry.is_file() && rel.as_str().to_ascii_lowercase().ends_with(".ltx") {
                out.push(rel);
            }
        }
    };
    if let Ok(configs) = RelPath::new("gamedata/configs") {
        take(&configs, &mut out);
    }
    if let Some(doc) = doc {
        for (virtual_path, inside) in doc.section_entries("vfs") {
            let v = virtual_path.trim().replace('/', "\\").to_ascii_lowercase();
            if !(v == "configs" || v.starts_with("configs\\")) {
                continue;
            }
            if let Ok(rel) = RelPath::new(inside.trim()) {
                take(&rel, &mut out);
            }
        }
    }
    out.retain(|rel| !outside_build(exclude, rel.as_str()));
    out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    out.dedup();
    out
}

/// Reads the sections of `files`. Blocking: call it on the IO pool.
#[must_use]
pub fn read_sections(root: &Path, files: &[RelPath]) -> SectionSet {
    let mut set = SectionSet::default();
    for rel in files {
        if let Ok(Some(bytes)) = gz_editor::fsx::read_opt(&rel.to_fs(root)) {
            set.add_text(&gz_base::text::decode_guess(&bytes));
        }
    }
    set
}

/// The index of the open project (`session.services`). Made on first use; it follows the
/// project's watcher from then on.
pub struct OwnSections {
    current: Mutex<Option<Arc<SectionSet>>>,
    builds: std::sync::atomic::AtomicU64,
    /// One build at a time; a second asker waits for it.
    building: tokio::sync::Mutex<()>,
}

impl OwnSections {
    fn new() -> Self {
        Self {
            current: Mutex::new(None),
            builds: std::sync::atomic::AtomicU64::new(0),
            building: tokio::sync::Mutex::new(()),
        }
    }

    /// The index of `session`, made on first use.
    pub fn of_session(editor: &Editor, session: &ProjectSession) -> Arc<Self> {
        if let Some(found) = session.services.get::<Self>() {
            return found;
        }
        let made = session.services.get_or_init(Self::new);
        made.follow(editor, session);
        made
    }

    /// The index as it is now, without waiting: `None` before it was built or after a change
    /// dropped it. What a check that must not wait reads.
    #[must_use]
    pub fn now(&self) -> Option<Arc<SectionSet>> {
        self.current.lock().clone()
    }

    /// The index, built on the IO pool when it is not there.
    pub async fn get(&self, editor: &Editor, session: &ProjectSession) -> Arc<SectionSet> {
        if let Some(set) = self.now() {
            return set;
        }
        let _one = self.building.lock().await;
        if let Some(set) = self.now() {
            return set;
        }
        let Some(project) = session.services.get::<Project>() else { return Arc::default() };
        project.tree.refresh().await;
        let tree = project.tree.snapshot();
        let root = session.root.clone();
        let set = editor
            .exec
            .io(move || {
                let text = gz_editor::fsx::read_opt(&root.join(FILE)).ok().flatten();
                let doc = text.map(|b| ManifestDoc::parse(&gz_base::text::decode_guess(&b)));
                let exclude = doc.as_ref().map(|d| Manifest::parse(d).exclude).unwrap_or_default();
                let files = config_files(&tree, doc.as_ref(), &exclude);
                read_sections(&root, &files)
            })
            .await;
        let mut set = set;
        set.generation = self.builds.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        let set = Arc::new(set);
        *self.current.lock() = Some(set.clone());
        set
    }

    /// Drops the index whenever a `.ltx` of the project changes (`mod.ltx` included: its `[vfs]`
    /// and `[exclude]` decide what counts).
    fn follow(self: &Arc<Self>, editor: &Editor, session: &ProjectSession) {
        let Some(project) = session.services.get::<Project>() else { return };
        let mut rx = project.tree.changes().subscribe(Filter::Extensions(vec!["ltx".into()]));
        let me = Arc::downgrade(self);
        editor.exec.spawn(async move {
            while rx.recv().await.is_some() {
                match me.upgrade() {
                    Some(me) => *me.current.lock() = None,
                    None => break,
                }
            }
        });
    }
}

/// The module's own sections of the open project, built when they are not known yet.
pub async fn own_sections(editor: &Editor) -> Arc<SectionSet> {
    let Ok(session) = editor.project() else { return Arc::default() };
    OwnSections::of_session(editor, &session).get(editor, &session).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_with_parents_and_comments() {
        let mut s = SectionSet::default();
        s.add_text("\u{feff}[od_parcel]:identity_immunities\nkey = 1\n ; [not_this]\n[ od_key ]\n[broken\n");
        assert!(s.has("od_parcel"));
        assert!(s.has("OD_KEY"));
        assert!(!s.has("not_this"));
        assert!(!s.has("broken"));
        assert!(!s.has(""));
        assert_eq!(s.len(), 2);
    }
}
