//! The file tree of the open project: the index every later area reads instead of touching the
//! disk.
//!
//! A [`FileTree`] is an immutable snapshot - folders by their project path, each with its
//! entries (name, size, write time, the editor's asset kind) - built by a parallel walk that
//! skips tooling entries (`.git` and the like are never entered) and never follows a link. A
//! [`TreeWatch`] keeps it current: the watcher only marks folders dirty, a task re-lists those on
//! the IO pool and publishes the next snapshot, which shares every unchanged folder by `Arc`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime};

use arc_swap::ArcSwap;
use gz_base::{RelPath, Rev, RevCounter, SmolStr};
use gz_store::imbl::OrdMap;
use gz_task::Executors;
use parking_lot::Mutex;
use rayon::prelude::*;
use rustc_hash::FxHashSet;
use tokio::sync::{Notify, watch};

use crate::asset_kind::{self, AssetKind};
use crate::fsys;
use crate::rules::is_tooling_entry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    File {
        size: u64,
        modified: Option<SystemTime>,
        asset: Option<AssetKind>,
    },
    Dir,
    /// A junction or a symbolic link: listed, never followed.
    Link,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: SmolStr,
    pub kind: EntryKind,
}

impl Entry {
    #[must_use]
    pub fn is_file(&self) -> bool {
        matches!(self.kind, EntryKind::File { .. })
    }
}

/// One folder: its entries sorted by name, ASCII case ignored.
#[derive(Clone, Debug)]
pub struct Dir {
    pub path: RelPath,
    pub entries: Arc<[Entry]>,
}

impl Dir {
    pub fn subdirs(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.kind == EntryKind::Dir)
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }
}

/// A snapshot of the project's files.
#[derive(Clone, Debug)]
pub struct FileTree {
    root: PathBuf,
    rev: Rev,
    dirs: OrdMap<RelPath, Arc<Dir>>,
}

/// A file the editor is writing (`fsx::write_atomic`'s `<name>.gz~`, renamed over the file when
/// whole): no entry of the project, so a build that walks the tree meanwhile never meets it.
fn is_write_in_progress(name: &str) -> bool {
    name.ends_with(".gz~")
}

fn list_dir(root: &Path, rel: &RelPath) -> Option<Dir> {
    let abs = rel.to_fs(root);
    let items = fsys::list_folder(&abs).ok()?;
    let mut entries: Vec<Entry> = Vec::with_capacity(items.len());
    for it in items {
        if is_tooling_entry(&it.name) || is_write_in_progress(&it.name) {
            continue;
        }
        let kind = if it.link {
            EntryKind::Link
        } else if it.is_folder() {
            EntryKind::Dir
        } else {
            let head = if asset_kind::needs_head(&it.name) { read_head(&abs.join(&it.name)) } else { None };
            EntryKind::File {
                size: it.size,
                modified: it.meta.modified().ok(),
                asset: asset_kind::kind_of(&it.name, head.as_deref()),
            }
        };
        entries.push(Entry { name: SmolStr::new(&it.name), kind });
    }
    entries.sort_by_key(|e| e.name.to_ascii_lowercase());
    Some(Dir { path: rel.clone(), entries: entries.into() })
}

fn read_head(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut buf = Vec::with_capacity(asset_kind::HEAD_LEN);
    file.take(asset_kind::HEAD_LEN as u64).read_to_end(&mut buf).ok()?;
    Some(buf)
}

fn child(dir: &RelPath, name: &str) -> Option<RelPath> {
    dir.join(name).ok()
}

/// Every folder below (and including) `start`, listed level by level in parallel on the pool
/// the caller runs on.
fn walk(root: &Path, start: Vec<RelPath>) -> Vec<Arc<Dir>> {
    let mut out = Vec::new();
    let mut frontier = start;
    while !frontier.is_empty() {
        let listed: Vec<Arc<Dir>> =
            frontier.par_iter().filter_map(|rel| list_dir(root, rel).map(Arc::new)).collect();
        frontier = listed
            .iter()
            .flat_map(|d| d.subdirs().filter_map(|e| child(&d.path, &e.name)).collect::<Vec<_>>())
            .collect();
        out.extend(listed);
    }
    out
}

impl FileTree {
    /// A tree that holds nothing yet.
    #[must_use]
    pub fn empty(root: PathBuf) -> Self {
        Self { root, rev: Rev::ZERO, dirs: OrdMap::new() }
    }

    /// Walks the whole project. Blocking and parallel: run it on the IO pool.
    #[must_use]
    pub fn scan(root: &Path) -> Self {
        let mut dirs = OrdMap::new();
        for d in walk(root, vec![RelPath::root()]) {
            dirs.insert(d.path.clone(), d);
        }
        Self { root: root.to_path_buf(), rev: Rev(1), dirs }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Moves with every published change.
    #[must_use]
    pub fn rev(&self) -> Rev {
        self.rev
    }

    #[must_use]
    pub fn dir(&self, path: &RelPath) -> Option<&Arc<Dir>> {
        self.dirs.get(path)
    }

    #[must_use]
    pub fn entry(&self, path: &RelPath) -> Option<&Entry> {
        let parent = path.parent()?;
        self.dirs.get(&parent)?.get(path.file_name())
    }

    #[must_use]
    pub fn contains(&self, path: &RelPath) -> bool {
        path.is_root() || self.entry(path).is_some()
    }

    /// Every folder, in path order.
    pub fn dirs(&self) -> impl Iterator<Item = &Arc<Dir>> {
        self.dirs.values()
    }

    /// Every file at or below `under`, with its path.
    #[must_use]
    pub fn files_under(&self, under: &RelPath) -> Vec<(RelPath, Entry)> {
        let mut out = Vec::new();
        let mut stack = vec![under.clone()];
        while let Some(rel) = stack.pop() {
            let Some(dir) = self.dirs.get(&rel) else { continue };
            for e in dir.entries.iter() {
                let Some(path) = child(&rel, &e.name) else { continue };
                match e.kind {
                    EntryKind::Dir => stack.push(path),
                    EntryKind::File { .. } => out.push((path, e.clone())),
                    EntryKind::Link => {}
                }
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Every file whose extension is `ext` (without the dot, any case).
    #[must_use]
    pub fn files_with_extension(&self, ext: &str) -> Vec<RelPath> {
        let mut out: Vec<RelPath> = self
            .dirs
            .values()
            .flat_map(|d| {
                d.entries
                    .iter()
                    .filter(|e| e.is_file())
                    .filter(|e| {
                        e.name
                            .rfind('.')
                            .and_then(|i| e.name.get(i + 1..))
                            .is_some_and(|x| x.eq_ignore_ascii_case(ext) && !e.name.starts_with('.'))
                    })
                    .filter_map(|e| child(&d.path, &e.name))
                    .collect::<Vec<_>>()
            })
            .collect();
        out.sort();
        out
    }

    fn remove_subtree(dirs: &mut OrdMap<RelPath, Arc<Dir>>, top: &RelPath) {
        let doomed: Vec<RelPath> = dirs.keys().filter(|k| k.starts_with(top)).cloned().collect();
        for k in doomed {
            dirs.remove(&k);
        }
    }

    /// The next snapshot: the `dirty` folders re-listed, folders that appeared walked, folders
    /// that went removed with everything below them. `full` walks everything again.
    #[must_use]
    pub fn relist(&self, dirty: &FxHashSet<RelPath>, full: bool) -> FileTree {
        if full {
            let mut next = FileTree::scan(&self.root);
            next.rev = self.rev.next();
            return next;
        }
        let mut dirs = self.dirs.clone();
        // a folder the tree does not know yet is found by re-listing the nearest one it does
        let mut targets: FxHashSet<RelPath> = FxHashSet::default();
        for d in dirty {
            let mut at = d.clone();
            loop {
                if dirs.contains_key(&at) {
                    targets.insert(at);
                    break;
                }
                match at.parent() {
                    Some(p) => at = p,
                    None => break,
                }
            }
        }
        let targets: Vec<RelPath> = targets.into_iter().collect();
        let listed: Vec<(RelPath, Option<Dir>)> =
            targets.par_iter().map(|t| (t.clone(), list_dir(&self.root, t))).collect();
        let mut fresh: Vec<RelPath> = Vec::new();
        for (path, dir) in listed {
            let old = dirs.get(&path).cloned();
            match dir {
                None => {
                    if path.is_root() {
                        dirs = OrdMap::new();
                    } else {
                        Self::remove_subtree(&mut dirs, &path);
                    }
                }
                Some(dir) => {
                    if let Some(old) = &old {
                        for gone in old
                            .subdirs()
                            .filter(|e| dir.get(&e.name).is_none_or(|n| n.kind != EntryKind::Dir))
                        {
                            if let Some(p) = child(&path, &gone.name) {
                                Self::remove_subtree(&mut dirs, &p);
                            }
                        }
                    }
                    for sub in dir.subdirs() {
                        if let Some(p) = child(&path, &sub.name)
                            && !dirs.contains_key(&p)
                        {
                            fresh.push(p);
                        }
                    }
                    dirs.insert(path, Arc::new(dir));
                }
            }
        }
        for d in walk(&self.root, fresh) {
            dirs.insert(d.path.clone(), d);
        }
        FileTree { root: self.root.clone(), rev: self.rev.next(), dirs }
    }
}

#[derive(Default)]
struct Dirt {
    folders: FxHashSet<RelPath>,
    full: bool,
}

struct Shared {
    root: PathBuf,
    snap: ArcSwap<FileTree>,
    marked: RevCounter,
    dirt: Mutex<Dirt>,
    kick: Arc<Notify>,
    applied: watch::Sender<Rev>,
}

impl Shared {
    fn mark(&self, folder: RelPath) {
        self.dirt.lock().folders.insert(folder);
        self.marked.bump();
        self.kick.notify_one();
    }

    fn mark_all(&self) {
        self.dirt.lock().full = true;
        self.marked.bump();
        self.kick.notify_one();
    }

    /// What a change at `path` (absolute) dirties: its folder and itself. A change inside a
    /// tooling entry (`.git`) dirties nothing.
    fn on_change(&self, path: &Path) {
        let Ok(rel) = path.strip_prefix(&self.root) else { return };
        let Some(text) = rel.to_str() else { return };
        let Ok(rel) = RelPath::new(text) else { return };
        if rel.segments().any(is_tooling_entry) {
            return;
        }
        if let Some(parent) = rel.parent() {
            self.mark(parent);
        }
        self.mark(rel);
    }
}

/// The project's file tree, kept current. Lives in the `Project` service; the watcher and the
/// re-list task end with it.
pub struct TreeWatch {
    shared: Arc<Shared>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

impl TreeWatch {
    /// Starts the first walk and the watcher. Returns at once; [`Self::refresh`] waits for the
    /// walk. A watcher that cannot start is logged by the caller from the `Err` and the tree
    /// then follows only [`Self::mark`] and [`Self::mark_all`].
    pub fn start(root: PathBuf, exec: &Executors) -> (Self, Option<String>) {
        let (applied, _) = watch::channel(Rev::ZERO);
        let kick = Arc::new(Notify::new());
        let shared = Arc::new(Shared {
            snap: ArcSwap::from_pointee(FileTree::empty(root.clone())),
            root: root.clone(),
            marked: RevCounter::new(),
            dirt: Mutex::new(Dirt::default()),
            kick: kick.clone(),
            applied,
        });
        exec.spawn(run(Arc::downgrade(&shared), kick, exec.clone()));
        shared.mark_all();

        let weak = Arc::downgrade(&shared);
        let made = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Some(shared) = weak.upgrade() else { return };
            match res {
                Ok(event) if event.need_rescan() => shared.mark_all(),
                Ok(event) => {
                    for p in &event.paths {
                        shared.on_change(p);
                    }
                }
                Err(_) => shared.mark_all(),
            }
        });
        let (watcher, problem) = match made {
            Ok(mut w) => {
                use notify::Watcher;
                match w.watch(&root, notify::RecursiveMode::Recursive) {
                    Ok(()) => (Some(w), None),
                    Err(e) => (None, Some(format!("the project's files are not watched: {e}"))),
                }
            }
            Err(e) => (None, Some(format!("the project's files are not watched: {e}"))),
        };
        (Self { shared, watcher: Mutex::new(watcher) }, problem)
    }

    /// The current snapshot. Lock-free.
    #[must_use]
    pub fn snapshot(&self) -> Arc<FileTree> {
        self.shared.snap.load_full()
    }

    #[must_use]
    pub fn rev(&self) -> Rev {
        self.shared.snap.load().rev
    }

    /// Marks a folder (project-relative) to be re-listed: what an operation of the editor does
    /// after it wrote there, without waiting for the watcher.
    pub fn mark(&self, folder: RelPath) {
        self.shared.mark(folder);
    }

    /// Marks an absolute path's folder, when it lies in the project.
    pub fn mark_fs(&self, path: &Path) {
        if let Some(rel) = RelPath::from_fs(&self.shared.root, path) {
            self.shared.mark(rel);
        } else if path == self.shared.root {
            self.shared.mark(RelPath::root());
        }
    }

    /// Everything is walked again.
    pub fn mark_all(&self) {
        self.shared.mark_all();
    }

    /// Resolves once every change marked before the call is in the published snapshot.
    pub async fn refresh(&self) {
        let target = self.shared.marked.get();
        self.shared.kick.notify_one();
        let mut rx = self.shared.applied.subscribe();
        loop {
            if *rx.borrow_and_update() >= target {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Drop for TreeWatch {
    fn drop(&mut self) {
        self.watcher.lock().take();
        // wakes the task so that it sees the tree is gone
        self.shared.kick.notify_one();
    }
}

async fn run(weak: Weak<Shared>, kick: Arc<Notify>, exec: Executors) {
    loop {
        kick.notified().await;
        // a burst of events (a checkout, a build) becomes one re-list
        tokio::time::sleep(Duration::from_millis(30)).await;
        let Some(shared) = weak.upgrade() else { break };
        if Arc::strong_count(&shared) <= 1 {
            break;
        }
        let target = shared.marked.get();
        let dirt = std::mem::take(&mut *shared.dirt.lock());
        if dirt.folders.is_empty() && !dirt.full {
            shared.applied.send_replace(target);
            continue;
        }
        let old = shared.snap.load_full();
        let next = exec.io(move || old.relist(&dirt.folders, dirt.full)).await;
        shared.snap.store(Arc::new(next));
        shared.applied.send_replace(target);
        exec.wake().ping();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn rel(s: &str) -> RelPath {
        RelPath::new(s).unwrap()
    }

    #[test]
    fn walks_without_tooling_and_relists() {
        let dir = std::env::temp_dir().join(format!("gz-project-tree-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".git\\objects")).unwrap();
        fs::create_dir_all(dir.join("quests\\main")).unwrap();
        fs::write(dir.join(".git\\objects\\x"), "x").unwrap();
        fs::write(dir.join("Thumbs.db"), "x").unwrap();
        fs::write(dir.join("quests\\main\\Intro.nqasset"), "return {}").unwrap();
        fs::write(dir.join("quests\\Base.npcasset"), "return { abstract = true }").unwrap();

        let tree = FileTree::scan(&dir);
        assert!(tree.dir(&rel(".git")).is_none());
        assert!(tree.entry(&rel("Thumbs.db")).is_none());
        let intro = tree.entry(&rel("quests/main/intro.nqasset")).unwrap();
        assert!(matches!(intro.kind, EntryKind::File { asset: Some(AssetKind::Quest), .. }));
        let base = tree.entry(&rel("quests/Base.npcasset")).unwrap();
        assert!(matches!(base.kind, EntryKind::File { asset: Some(AssetKind::CharacterTemplate), .. }));
        assert_eq!(tree.files_with_extension("NQASSET"), [rel("quests/main/Intro.nqasset")]);
        assert_eq!(tree.files_under(&rel("quests")).len(), 2);

        fs::create_dir_all(dir.join("quests\\side\\deep")).unwrap();
        fs::write(dir.join("quests\\side\\deep\\a.nqasset"), "").unwrap();
        fs::remove_dir_all(dir.join("quests\\main")).unwrap();
        let mut dirty = FxHashSet::default();
        dirty.insert(rel("quests/side/deep"));
        dirty.insert(rel("quests"));
        let next = tree.relist(&dirty, false);
        assert!(next.rev() > tree.rev());
        assert!(next.dir(&rel("quests/main")).is_none());
        assert!(next.entry(&rel("quests/side/deep/a.nqasset")).is_some());
        // the folders nobody touched are the same Arc
        assert!(Arc::ptr_eq(tree.dir(&RelPath::root()).unwrap(), next.dir(&RelPath::root()).unwrap()));
        fs::remove_dir_all(&dir).unwrap();
    }
}
