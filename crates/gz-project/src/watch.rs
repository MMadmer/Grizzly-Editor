//! The project's one source of "a file changed": every open document, the content browser, the
//! validators and the caches hear about a change from here, so nothing polls.
//!
//! The file system watcher of the [`crate::TreeWatch`] feeds it. Changes are debounced (a burst
//! of writes to one file is one change, 100 ms after the last of them), coalesced per path, and
//! what is nobody's content - tooling entries (`.git`), the editor's scratch at the project root,
//! a write in progress - never gets here. The editor's own writes are told apart by the token
//! the save path leaves ([`ProjectWatch::note_written`]): a file whose size, write time and
//! content are what the editor wrote is no outside change.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use gz_base::{RelPath, SmolStr};
use gz_task::Executors;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use tokio::sync::Notify;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::rules::{is_editor_only_entry, is_tooling_entry};

/// How long a path stays quiet before its change is delivered.
pub const DEBOUNCE: Duration = Duration::from_millis(100);

/// How long the token of one of the editor's own writes waits for its event.
const OWN_WRITE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    /// The file appeared.
    Created,
    /// The file is there and was written.
    Changed,
    /// The file (or a folder holding it) is gone.
    Removed,
    /// The watcher lost track (its buffer overflowed): anything may have changed. A subscriber
    /// compares what it read with what is there.
    Rescan,
}

/// One delivered change, project-relative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: RelPath,
    pub kind: ChangeKind,
}

/// Which changes a subscriber hears about.
#[derive(Clone, Debug)]
pub enum Filter {
    All,
    /// One file (and the folders holding it going away).
    Path(RelPath),
    /// Everything at or below a folder.
    Under(RelPath),
    /// Files with one of these extensions (without the dot, any case).
    Extensions(Vec<SmolStr>),
}

impl Filter {
    fn matches(&self, c: &Change) -> bool {
        if c.kind == ChangeKind::Rescan {
            return true;
        }
        match self {
            Filter::All => true,
            Filter::Path(p) => c.path == *p || (c.kind == ChangeKind::Removed && p.starts_with(&c.path)),
            Filter::Under(dir) => c.path.starts_with(dir),
            Filter::Extensions(exts) => {
                let name = c.path.file_name();
                name.rfind('.')
                    .and_then(|i| name.get(i + 1..))
                    .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Pending {
    last: Option<Instant>,
    created: bool,
}

/// What the editor wrote: the file as it stood right after the write.
#[derive(Clone, Copy)]
struct OwnWrite {
    size: u64,
    modified: Option<SystemTime>,
    hash: u64,
    at: Instant,
}

struct Sub {
    filter: Filter,
    tx: UnboundedSender<Change>,
}

struct Hub {
    root: PathBuf,
    pending: Mutex<FxHashMap<RelPath, Pending>>,
    rescan: Mutex<bool>,
    own: Mutex<FxHashMap<RelPath, OwnWrite>>,
    subs: Mutex<Vec<Sub>>,
    kick: Arc<Notify>,
    live: AtomicBool,
}

/// FNV-1a: the content of a write, to tell it from another write of the same size and time.
fn hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn stamp(path: &Path) -> Option<(u64, Option<SystemTime>, bool)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok(), m.is_dir()))
}

/// Whether a project path is nobody's content: tooling at any depth, the editor's scratch at
/// the root, a write in progress.
fn ignored(rel: &RelPath) -> bool {
    if rel.segments().any(is_tooling_entry) {
        return true;
    }
    if rel.segments().next().is_some_and(is_editor_only_entry) {
        return true;
    }
    rel.file_name().ends_with(".gz~")
}

/// The changes of the open project. Lives in its [`crate::TreeWatch`].
#[derive(Clone)]
pub struct ProjectWatch {
    hub: Arc<Hub>,
}

impl ProjectWatch {
    pub(crate) fn start(root: PathBuf, exec: &Executors) -> Self {
        let kick = Arc::new(Notify::new());
        let hub = Arc::new(Hub {
            root,
            pending: Mutex::new(FxHashMap::default()),
            rescan: Mutex::new(false),
            own: Mutex::new(FxHashMap::default()),
            subs: Mutex::new(Vec::new()),
            kick: kick.clone(),
            live: AtomicBool::new(false),
        });
        exec.spawn(run(Arc::downgrade(&hub), kick, exec.clone()));
        Self { hub }
    }

    /// Whether a watcher feeds it. Without one nothing is ever delivered and a subscriber keeps
    /// the stamps it compared before.
    #[must_use]
    pub fn live(&self) -> bool {
        self.hub.live.load(Ordering::Relaxed)
    }

    pub(crate) fn set_live(&self, live: bool) {
        self.hub.live.store(live, Ordering::Relaxed);
    }

    /// The changes that match `filter`, from now on, until the receiver is dropped.
    #[must_use]
    pub fn subscribe(&self, filter: Filter) -> UnboundedReceiver<Change> {
        let (tx, rx) = unbounded_channel();
        self.hub.subs.lock().push(Sub { filter, tx });
        rx
    }

    /// The editor wrote `bytes` to `rel` just now: the change this write causes is not an
    /// outside one. Stats the file: call it on the IO pool, right after the write.
    pub fn note_written(&self, rel: &RelPath, bytes: &[u8]) {
        let Some((size, modified, _)) = stamp(&rel.to_fs(&self.hub.root)) else { return };
        let token = OwnWrite { size, modified, hash: hash(bytes), at: Instant::now() };
        self.hub.own.lock().insert(rel.clone(), token);
    }

    /// [`Self::note_written`] for an absolute path; nothing when it is outside the project.
    pub fn note_written_fs(&self, path: &Path, bytes: &[u8]) {
        if let Some(rel) = RelPath::from_fs(&self.hub.root, path) {
            self.note_written(&rel, bytes);
        }
    }

    /// A raw event of the watcher at an absolute path.
    pub(crate) fn event(&self, path: &Path, created: bool) {
        let Ok(rel) = path.strip_prefix(&self.hub.root) else { return };
        let Some(text) = rel.to_str() else { return };
        let Ok(rel) = RelPath::new(text) else { return };
        if rel.is_root() || ignored(&rel) {
            return;
        }
        {
            let mut pending = self.hub.pending.lock();
            let p = pending.entry(rel).or_default();
            p.last = Some(Instant::now());
            p.created |= created;
        }
        self.hub.kick.notify_one();
    }

    /// The watcher lost events: every subscriber is told to look again.
    pub(crate) fn rescan(&self) {
        *self.hub.rescan.lock() = true;
        self.hub.kick.notify_one();
    }
}

impl Hub {
    /// The paths quiet for [`DEBOUNCE`], taken out; whether others still wait.
    fn take_due(&self, now: Instant) -> (Vec<(RelPath, Pending)>, bool) {
        let mut pending = self.pending.lock();
        let due: Vec<RelPath> = pending
            .iter()
            .filter(|(_, p)| p.last.is_none_or(|t| now.duration_since(t) >= DEBOUNCE))
            .map(|(k, _)| k.clone())
            .collect();
        let out = due.into_iter().filter_map(|k| pending.remove(&k).map(|p| (k, p))).collect();
        (out, !pending.is_empty())
    }

    /// What the due paths come to on disk: the editor's own writes drop out.
    fn classify(&self, due: Vec<(RelPath, Pending)>) -> Vec<Change> {
        let mut own = self.own.lock();
        own.retain(|_, t| t.at.elapsed() < OWN_WRITE_TTL);
        let mut out = Vec::with_capacity(due.len());
        for (rel, p) in due {
            let abs = rel.to_fs(&self.root);
            match stamp(&abs) {
                None => {
                    own.remove(&rel);
                    out.push(Change { path: rel, kind: ChangeKind::Removed });
                }
                // a folder's own events say nothing a file of it does not say itself
                Some((_, _, true)) => {}
                Some((size, modified, false)) => {
                    if let Some(t) = own.get(&rel).copied()
                        && t.size == size
                        && t.modified == modified
                        && std::fs::read(&abs).is_ok_and(|b| hash(&b) == t.hash)
                    {
                        continue;
                    }
                    own.remove(&rel);
                    let kind = if p.created { ChangeKind::Created } else { ChangeKind::Changed };
                    out.push(Change { path: rel, kind });
                }
            }
        }
        out
    }

    fn deliver(&self, changes: &[Change]) {
        let mut subs = self.subs.lock();
        subs.retain(|s| {
            for c in changes {
                if s.filter.matches(c) && s.tx.send(c.clone()).is_err() {
                    return false;
                }
            }
            !s.tx.is_closed()
        });
    }
}

async fn run(weak: Weak<Hub>, kick: Arc<Notify>, exec: Executors) {
    loop {
        kick.notified().await;
        loop {
            tokio::time::sleep(DEBOUNCE).await;
            let Some(hub) = weak.upgrade() else { return };
            let rescan = std::mem::take(&mut *hub.rescan.lock());
            let (due, more) = hub.take_due(Instant::now());
            let mut changes = Vec::new();
            if rescan {
                changes.push(Change { path: RelPath::root(), kind: ChangeKind::Rescan });
            }
            if !due.is_empty() {
                let h = hub.clone();
                changes.extend(exec.io(move || h.classify(due)).await);
            }
            if !changes.is_empty() {
                hub.deliver(&changes);
                exec.wake().ping();
            }
            if !more {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(s: &str) -> RelPath {
        RelPath::new(s).unwrap()
    }

    #[test]
    fn filters_and_ignores() {
        let c = |p: &str, kind| Change { path: rel(p), kind };
        assert!(Filter::Path(rel("q/a.nqasset")).matches(&c("q/a.nqasset", ChangeKind::Changed)));
        assert!(!Filter::Path(rel("q/a.nqasset")).matches(&c("q/b.nqasset", ChangeKind::Changed)));
        assert!(Filter::Path(rel("q/a.nqasset")).matches(&c("q", ChangeKind::Removed)));
        assert!(Filter::Under(rel("q")).matches(&c("q/x/y.npcasset", ChangeKind::Created)));
        let ext = Filter::Extensions(vec!["NPCASSET".into()]);
        assert!(ext.matches(&c("x/b.npcasset", ChangeKind::Changed)));
        assert!(!ext.matches(&c("x/b.nqasset", ChangeKind::Changed)));
        assert!(ext.matches(&c("", ChangeKind::Rescan)));
        assert!(ignored(&rel(".git/index")));
        assert!(ignored(&rel("q/.vscode/x")));
        assert!(ignored(&rel("temp/session.ltx")));
        assert!(ignored(&rel("q/a.nqasset.gz~")));
        assert!(!ignored(&rel("q/temp/a.nqasset")));
    }

    #[test]
    fn delivers_outside_changes_and_not_the_editors_own() {
        use gz_task::{ExecOptions, Runtime};
        let dir = std::env::temp_dir().join(format!("gz-project-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("q")).unwrap();
        let rt = Runtime::start(ExecOptions { async_threads: 2, cpu_threads: 1, io_threads: 1 }).unwrap();
        let exec = rt.exec();
        let (tree, problem) = crate::TreeWatch::start(dir.clone(), &exec);
        assert!(problem.is_none(), "{problem:?}");
        assert!(tree.changes().live());
        let mut rx = tree.changes().subscribe(Filter::Extensions(vec!["nqasset".into()]));
        let next = |rx: &mut UnboundedReceiver<Change>| {
            rt.block_on(async {
                tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.ok().flatten()
            })
        };
        let quiet = |rx: &mut UnboundedReceiver<Change>| {
            rt.block_on(async { tokio::time::timeout(Duration::from_millis(600), rx.recv()).await.is_err() })
        };
        let file = dir.join("q").join("a.nqasset");
        // a burst of writes is one change
        for i in 0..5 {
            std::fs::write(&file, format!("return {{ n = {i} }}")).unwrap();
        }
        let c = next(&mut rx).expect("a change");
        assert_eq!(c.path, rel("q/a.nqasset"));
        assert!(matches!(c.kind, ChangeKind::Created | ChangeKind::Changed));
        assert!(quiet(&mut rx), "one change for the burst");
        // the editor's own write is no outside change
        let bytes = b"return { n = 9 }";
        std::fs::write(&file, bytes).unwrap();
        tree.changes().note_written(&rel("q/a.nqasset"), bytes);
        assert!(quiet(&mut rx), "the editor's own write was delivered");
        // another extension is not this subscriber's, a removal is
        std::fs::write(dir.join("q").join("b.npcasset"), "return {}").unwrap();
        std::fs::remove_file(&file).unwrap();
        let c = next(&mut rx).expect("the removal");
        assert_eq!((c.path, c.kind), (rel("q/a.nqasset"), ChangeKind::Removed));
        drop(tree);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
