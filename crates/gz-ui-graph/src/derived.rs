//! What a graph tab reads besides its document's snapshot, fetched off the UI thread: the
//! catalog, the problems, the pickers, the item capabilities, the project's characters, the
//! module's globals, the ids of the project's graphs (with a behaviour graph's roles), the way
//! paths, the scene's level, and what the file on disk says the document does not show (its
//! `DiskState`, kept by the project's watcher). All of it comes from `gz_nq::NqService`'s
//! queries, so a refresh of an unchanged document costs lookups only.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gz_base::Rev;
use gz_editor::Editor;
use gz_nq::caps::ItemCaps;
use gz_nq::{Catalog, CharacterBrief, DocHandle, NqProblem, NqService, PickerEntry, Pickers, Role};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

/// How often a tab that is drawn looks again without an edit (the manifest, the file's state).
const POLL: Duration = Duration::from_secs(1);

/// One project graph another one can name.
#[derive(Clone, Debug, Default)]
pub struct ProjectGraph {
    pub id: String,
    pub path: String,
    pub behaviour: bool,
    pub readable: bool,
    pub roles: Vec<Role>,
}

/// What the tab draws from besides the snapshot.
#[derive(Clone, Default)]
pub struct DocData {
    pub catalog: Option<Arc<Catalog>>,
    pub problems: Arc<Vec<NqProblem>>,
    /// The content serial the problems are of.
    pub problems_edit: u64,
    /// node id -> (1 warning / 2 error, every message of it, one per line).
    pub by_node: Arc<FxHashMap<SmolStr, (u8, String)>>,
    pub errors: usize,
    pub warnings: usize,
    pub pickers: Option<Arc<Pickers>>,
    pub caps: Option<Arc<ItemCaps>>,
    pub characters: Arc<[CharacterBrief]>,
    /// The module's globals, `name = default` as written.
    pub globals: Vec<(String, String)>,
    pub globals_rev: Rev,
    pub graphs: Arc<Vec<ProjectGraph>>,
    pub way_paths: Arc<Vec<PickerEntry>>,
    /// The open scene's level, for a fresh position.
    pub level: String,
    /// What the file on disk says the document does not show: it does not read, it is gone,
    /// values both sides changed.
    pub disk: gz_nq::DiskState,
    pub missing: bool,
    pub has_project: bool,
}

impl DocData {
    #[must_use]
    pub fn severity(&self, node: &str) -> Option<&(u8, String)> {
        self.by_node.get(node)
    }
}

/// Keeps a tab's [`DocData`] fresh: one refresh at a time, started by the frame when the
/// document changed or a second went by.
pub struct Refresher {
    pub data: Arc<Mutex<DocData>>,
    running: Arc<AtomicBool>,
    last_edit: Option<u64>,
    last_at: Option<Instant>,
}

impl Default for Refresher {
    fn default() -> Self {
        Self {
            data: Arc::new(Mutex::new(DocData::default())),
            running: Arc::new(AtomicBool::new(false)),
            last_edit: None,
            last_at: None,
        }
    }
}

impl Refresher {
    /// The current data (a cheap clone of `Arc`s).
    #[must_use]
    pub fn get(&self) -> DocData {
        self.data.lock().clone()
    }

    /// Starts a refresh when one is due. `force` refreshes whatever happened.
    pub fn tick(
        &mut self,
        editor: &Arc<Editor>,
        doc: &Arc<DocHandle>,
        ctx: &egui::Context,
        edit: u64,
        force: bool,
    ) {
        let due = force || self.last_edit != Some(edit) || self.last_at.is_none_or(|t| t.elapsed() >= POLL);
        if !due || self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        self.last_edit = Some(edit);
        self.last_at = Some(Instant::now());
        let (ed, doc, data, running, ctx) =
            (editor.clone(), doc.clone(), self.data.clone(), self.running.clone(), ctx.clone());
        let _task = editor.exec.spawn(async move {
            let fresh = fetch(&ed, &doc).await;
            if let Some(fresh) = fresh {
                *data.lock() = fresh;
            }
            running.store(false, Ordering::Release);
            ctx.request_repaint();
        });
    }
}

async fn fetch(ed: &Arc<Editor>, doc: &Arc<DocHandle>) -> Option<DocData> {
    let svc = NqService::of(ed).ok()?;
    let catalog = svc.catalog(ed).await;
    let snap = doc.store.snapshot();
    let problems = svc.problems(ed, doc).await;
    let pickers = svc.pickers(ed).await;
    let caps = if pickers.available() { svc.caps(ed).await } else { None };
    let (_, characters) = svc.characters();
    let globals = svc.global_vars(ed).await;
    let globals_rev = gz_mod::ManifestService::of(ed).await.map(|m| m.rev()).unwrap_or(Rev::ZERO);
    let index = svc.index(ed, true).await;
    let env = svc.env(ed, true).await;
    let mut graphs = Vec::with_capacity(index.entries.len());
    for e in &index.entries {
        graphs.push(ProjectGraph {
            id: if e.readable { e.graph.id.clone() } else { String::new() },
            path: e.rel.clone(),
            behaviour: e.behaviour(),
            readable: e.readable,
            roles: if e.behaviour() && e.readable { e.graph.roles.clone() } else { Vec::new() },
        });
    }
    let mut by_node: FxHashMap<SmolStr, (u8, String)> = FxHashMap::default();
    for p in problems.iter().filter(|p| !p.node.is_empty()) {
        let e = by_node.entry(SmolStr::new(&p.node)).or_insert((0, String::new()));
        e.0 = e.0.max(if p.is_error() { 2 } else { 1 });
        if !e.1.is_empty() {
            e.1.push('\n');
        }
        e.1.push_str(&p.text());
    }
    let disk = doc.disk();
    Some(DocData {
        errors: gz_nq::validate::count_errors(&problems),
        warnings: gz_nq::validate::count_warnings(&problems),
        catalog: Some(catalog),
        problems_edit: snap.edit,
        problems,
        by_node: Arc::new(by_node),
        pickers: Some(pickers),
        caps,
        characters,
        globals,
        globals_rev,
        graphs: Arc::new(graphs),
        way_paths: Arc::new(svc.way_paths()),
        level: env.level.clone().unwrap_or_default(),
        missing: disk.missing,
        disk,
        has_project: true,
    })
}
