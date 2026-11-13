//! What the graph area keeps while a project is open: the open documents, and the catalog, the
//! pickers, the item capabilities, the project index and the problems as `Query`s keyed by the
//! revisions they read (`NqDocs`, `NqCatalog::Ensure`, `NqPickers::Ensure`, `NqProjectIndex`).
//! Nothing is dropped by hand: a link change, a file the tree sees change, an edit of a
//! document or of `mod.ltx` moves a revision, and what read it is computed anew on its next use.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::SystemTime;

use gz_base::{Origin, Problem, RelPath, Rev, RevCounter};
use gz_editor::{CmdError, DirtyDoc, DirtySource, Editor, Executors, Log, ProjectSession};
use gz_game::{Bundled, GameLink};
use gz_mod::ManifestService;
use gz_project::{AssetKind, EntryKind, FileTree, Filter, Project};
use gz_query::Query;
use gz_store::{Store, StoreOptions};
use gz_vfs::{GameFiles, ModuleRoot};
use parking_lot::{Mutex, RwLock};
use rustc_hash::FxHashMap;

use crate::asset::{self, Graph};
use crate::caps::ItemCaps;
use crate::catalog::{Catalog, SourceFile};
use crate::disk::{DiskState, OnDisk};
use crate::doc::{DocCmd, DocOp, DocReply, GraphDoc};
use crate::index::{self, IndexEntry, IndexSnapshot};
pub use crate::pickers::Transition;
use crate::pickers::{self, GameIndex, PickerEntry, Pickers, ProjectFiles, ProjectPart};
pub use crate::validate::CharacterBrief;
use crate::validate::{self, Context, NqProblem};

/// The project's characters, filled by the characters area (`EditorCharacters::Briefs`).
pub trait CharacterLookup: Send + Sync {
    /// Moves whenever the briefs do.
    fn rev(&self) -> Rev;
    fn briefs(&self) -> Arc<[CharacterBrief]>;
}

/// What the open scene adds, filled by the scene area: its restrictors, its authored story ids,
/// its way paths and the level it is (`SetSceneRestrictors`, `SetSceneStories`,
/// `SetSceneWayPaths`, `NqDocs::CurrentLevel`).
pub trait SceneLookup: Send + Sync {
    fn rev(&self) -> Rev;
    fn restrictors(&self) -> Vec<PickerEntry>;
    fn stories(&self) -> Vec<PickerEntry>;
    fn way_paths(&self) -> Vec<PickerEntry>;
    /// The scene's level when the linked game knows it (`E031`).
    fn level(&self) -> Option<String>;
}

/// What the world area reads out of the linked game's `all.spawn` (`EditorWorld::Transitions`,
/// `EditorWorld::GamePatrols`).
pub trait WorldLookup: Send + Sync {
    fn rev(&self) -> Rev;
    fn transitions(&self) -> Vec<Transition>;
    fn patrols(&self) -> Vec<String>;
    /// Resolves once the linked game's world of the current revision is read: a command that
    /// lists level changers waits for it, as the C++ read `all.spawn` when asked.
    fn ready(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>;
}

/// Where an open document's file is.
#[derive(Clone, Debug)]
pub struct DocPath {
    pub abs: PathBuf,
    /// What the wire says: project-relative with forward slashes, or the absolute path.
    pub wire: String,
    /// The lower-case absolute path with backslashes the documents are told apart by.
    pub key: String,
    pub inside: bool,
}

/// One open document: its store, its file and what the file says the document does not show.
pub struct DocHandle {
    pub store: Store<GraphDoc>,
    path: RwLock<DocPath>,
    /// The file as read (its lossless document) and the model's value of it.
    pub(crate) on_disk: Mutex<Option<OnDisk>>,
    disk: RwLock<DiskState>,
    /// One read or write of the file at a time.
    pub(crate) io: tokio::sync::Mutex<()>,
    /// Which follower of the file is the current one (a rename starts another).
    pub(crate) follow: AtomicU64,
}

impl DocHandle {
    pub(crate) fn new(exec: &Executors, path: DocPath) -> Self {
        Self {
            store: Store::spawn("graph", GraphDoc::default(), exec, StoreOptions { history_limit: 200 }),
            path: RwLock::new(path),
            on_disk: Mutex::new(None),
            disk: RwLock::new(DiskState::default()),
            io: tokio::sync::Mutex::new(()),
            follow: AtomicU64::new(0),
        }
    }

    #[must_use]
    pub fn path(&self) -> DocPath {
        self.path.read().clone()
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.store.is_dirty()
    }

    /// What the file on disk says that the document does not show: it does not read, it is
    /// gone, values both sides changed.
    #[must_use]
    pub fn disk(&self) -> DiskState {
        self.disk.read().clone()
    }

    pub(crate) fn set_disk(&self, f: impl FnOnce(&mut DiskState)) {
        let mut d = self.disk.write();
        f(&mut d);
        d.rev += 1;
    }

    /// What reading the file found that the graph's own check does not say: the header, a byte
    /// order mark, an old version.
    #[must_use]
    pub fn read_problems(&self) -> Vec<Problem> {
        self.on_disk.lock().as_ref().map_or_else(Vec::new, |o| o.asset.notes.clone())
    }
}

/// `"a\\b"` and `/` normalised to single backslashes, `\.\` dropped, a trailing separator cut
/// (`NqUtil::NormalizePath`).
#[must_use]
pub fn normalize_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.chars() {
        let c = if c == '/' { '\\' } else { c };
        if c == '\\' && out.ends_with('\\') {
            continue;
        }
        out.push(c);
    }
    while let Some(at) = out.find("\\.\\") {
        out.replace_range(at..at + 2, "");
    }
    while out.len() > 3 && out.ends_with('\\') {
        out.pop();
    }
    out
}

/// The key a path is compared by (`NqUtil::PathKey`).
#[must_use]
pub fn path_key(p: &str) -> String {
    normalize_path(p).to_lowercase()
}

/// A file the index reads: its wire path, its key, where it is, its stamp.
type ToRead = (String, String, PathBuf, (u64, Option<SystemTime>));

/// The answers of a reader that must not wait: the parse of a file, by its stamp.
#[derive(Clone)]
struct Parsed {
    stamp: (u64, Option<SystemTime>),
    catalog: u32,
    entry: IndexEntry,
}

/// What validation reads besides the graph.
pub struct ValidateEnv {
    pub catalog: Arc<Catalog>,
    pub pickers: Arc<Pickers>,
    pub caps: Option<Arc<ItemCaps>>,
    pub characters: Arc<[CharacterBrief]>,
    pub globals: Vec<String>,
    pub level: Option<String>,
    pub index: Arc<IndexSnapshot>,
    /// The fingerprint of all of it.
    pub rev: Rev,
}

impl ValidateEnv {
    /// The problems of a graph at `wire` (`NqDocs::Validate`).
    #[must_use]
    pub fn validate(&self, g: &Graph, wire: &str) -> Vec<NqProblem> {
        let others = self.index.other_ids(Some(wire));
        let behaviours = self.index.ids_of_kind(true);
        let ctx = Context {
            file_path: Some(wire),
            project_quest_ids: Some(&others),
            project_behaviour_ids: Some(&behaviours),
            level: self.level.as_deref(),
            pickers: Some(&self.pickers),
            caps: self.caps.as_deref(),
            characters: &self.characters,
            globals: &self.globals,
        };
        validate::run(g, &self.catalog, &ctx)
    }
}

/// The graph area of the open project (`session.services`).
pub struct NqService {
    pub(crate) exec: Executors,
    log: Log,
    root: String,
    root_key: String,
    pub(crate) project: Arc<Project>,
    pub(crate) docs: Mutex<Vec<Arc<DocHandle>>>,
    pub(crate) docs_rev: RevCounter,
    /// Moves with every change of a graph file the project's watcher reports.
    graphs_rev: Arc<RevCounter>,
    catalog: Query<(), Catalog>,
    catalog_gen: AtomicU32,
    game_pickers: Query<(), Option<Arc<GameIndex>>>,
    project_pickers: Query<(), ProjectPart>,
    pickers: Query<(), Pickers>,
    caps: Query<(), Option<Arc<ItemCaps>>>,
    index: Query<bool, IndexSnapshot>,
    parsed: Mutex<FxHashMap<String, Parsed>>,
    generations: Mutex<[(u64, u32, bool); 2]>,
    problems: Query<String, Vec<NqProblem>>,
    characters: RwLock<Option<Arc<dyn CharacterLookup>>>,
    scene: RwLock<Option<Arc<dyn SceneLookup>>>,
    world: RwLock<Option<Arc<dyn WorldLookup>>>,
}

/// Reads a graph file for the index: at most 16 MB, a byte order mark left out (the read path
/// says what is wrong with one).
fn read_graph_file(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|_| "cannot open file".to_owned())?;
    if meta.len() > 16 << 20 {
        return Err("file too large".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "read error".to_owned())?;
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    Ok(String::from_utf8_lossy(body).into_owned())
}

fn decode(bytes: &[u8]) -> String {
    gz_base::text::cp1251_to_utf8(bytes).into_owned()
}

impl NqService {
    /// The graph area of the open project, made on first use.
    pub fn of(editor: &Editor) -> Result<Arc<NqService>, CmdError> {
        let (session, project) = Project::of(editor)?;
        Ok(Self::of_session(editor, &session, project))
    }

    fn of_session(editor: &Editor, session: &ProjectSession, project: Arc<Project>) -> Arc<NqService> {
        let exec = editor.exec.clone();
        session.services.get_or_init(|| {
            let root = project.root().to_owned();
            let graphs_rev = follow_graphs(editor, &project);
            NqService {
                root_key: path_key(&root),
                root,
                project,
                log: editor.log.clone(),
                docs: Mutex::new(Vec::new()),
                docs_rev: RevCounter::new(),
                graphs_rev,
                catalog: Query::new(exec.clone()),
                catalog_gen: AtomicU32::new(0),
                game_pickers: Query::new(exec.clone()),
                project_pickers: Query::new(exec.clone()),
                pickers: Query::new(exec.clone()),
                caps: Query::new(exec.clone()),
                index: Query::new(exec.clone()),
                parsed: Mutex::new(FxHashMap::default()),
                generations: Mutex::new([(0, 0, false); 2]),
                problems: Query::new(exec.clone()),
                characters: RwLock::new(None),
                scene: RwLock::new(None),
                world: RwLock::new(None),
                exec,
            }
        })
    }

    /// The service when it has been made, without making it.
    #[must_use]
    pub fn get(session: &ProjectSession) -> Option<Arc<NqService>> {
        session.services.get::<NqService>()
    }

    /// The characters area fills this; empty, the characters come from the file tree.
    pub fn set_characters(&self, lookup: Arc<dyn CharacterLookup>) {
        *self.characters.write() = Some(lookup);
    }

    pub fn set_scene(&self, lookup: Arc<dyn SceneLookup>) {
        *self.scene.write() = Some(lookup);
    }

    pub fn set_world(&self, lookup: Arc<dyn WorldLookup>) {
        *self.world.write() = Some(lookup);
    }

    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }

    // ---- paths ---------------------------------------------------------------------------------

    /// Whether an absolute path lies inside the project (`NqDocs::InsideProject`).
    #[must_use]
    pub fn inside(&self, abs: &str) -> bool {
        let k = path_key(abs);
        k.len() > self.root_key.len() + 1
            && k.starts_with(&self.root_key)
            && k.as_bytes().get(self.root_key.len()) == Some(&b'\\')
    }

    /// The wire form of an absolute path: project-relative with `/`, or itself with `/`
    /// (`NqUtil::ProjectRelative`).
    #[must_use]
    pub fn wire(&self, abs: &str) -> String {
        let n = normalize_path(abs);
        let rel = if self.inside(&n) { n.get(self.root_key.len() + 1..).unwrap_or(&n).to_owned() } else { n };
        rel.replace('\\', "/")
    }

    /// A path as a document is named by (`NqDocs::Resolve`): project-relative or absolute,
    /// `.nqasset` added without an extension; `for_write` keeps it inside the project and a graph.
    pub fn resolve(&self, path: &str, for_write: bool) -> Result<DocPath, String> {
        if path.is_empty() {
            return Err("path is empty".into());
        }
        let n = normalize_path(path);
        let b = n.as_bytes();
        let absolute = (b.len() >= 2 && b.get(1) == Some(&b':')) || n.starts_with("\\\\");
        let mut abs = if absolute {
            n
        } else {
            if n.contains("..") {
                return Err("path may not contain '..'".into());
            }
            format!("{}\\{n}", normalize_path(&self.root))
        };
        if asset::extension(&abs).is_empty() {
            abs.push_str(".nqasset");
        }
        let inside = self.inside(&abs);
        if for_write && !inside {
            return Err(
                "path is outside the project folder - graphs are written into the project only".into()
            );
        }
        if for_write && !asset::is_graph_path(&abs) {
            return Err("a graph document is a .nqasset (quest) or a .behasset (behaviour graph)".into());
        }
        Ok(DocPath { wire: self.wire(&abs), key: path_key(&abs), abs: PathBuf::from(&abs), inside })
    }

    // ---- the catalog ---------------------------------------------------------------------------

    fn manifest(&self, editor: &Editor) -> Option<Arc<ManifestService>> {
        editor.project_opt().and_then(|s| ManifestService::get(&s))
    }

    async fn manifest_now(&self, editor: &Editor) -> Option<Arc<ManifestService>> {
        let session = editor.project_opt()?;
        ManifestService::of_session(editor, &session).await.ok()
    }

    fn tree(&self) -> Arc<FileTree> {
        self.project.tree.snapshot()
    }

    /// The catalog of the linked game (the bundled copy without one), the installed modules'
    /// kinds and the project's own, in that order (`NqCatalog::Load`).
    pub async fn catalog(&self, editor: &Editor) -> Arc<Catalog> {
        let manifest = self.manifest_now(editor).await;
        let own =
            manifest.as_ref().and_then(|m| m.snapshot().manifest().map(|x| x.id.clone())).unwrap_or_default();
        let link = GameLink::of(editor).ok();
        let game = match &link {
            Some(l) => l.snapshot().await.ok(),
            None => None,
        };
        let game_rev = game.as_ref().map_or(Rev::ZERO, |(_, r)| *r);
        let tree = self.tree();
        // only what the catalog reads moves it: the project's kinds and which module is its own
        let rev = game_rev
            .mix(stamp(&tree, &[("gamedata/configs/nq/kinds", "ltx")]))
            .mix(Rev(index::content_hash(&own.to_ascii_lowercase())))
            .mix(Rev(u64::from(game.is_some())));
        if let Some(c) = self.catalog.peek_at(&(), rev) {
            return c;
        }
        let bundled = Bundled::of(editor);
        let files = game.map(|(f, _)| f);
        let root = PathBuf::from(&self.root);
        let generation = self.catalog_gen.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        let built = self
            .exec
            .io(move || load_catalog(files.as_deref(), &bundled, &root, &tree, &own, generation))
            .await;
        self.catalog.fresh(&(), rev, move || built).await
    }

    // ---- the pickers ---------------------------------------------------------------------------

    fn world_lookup(&self) -> Option<Arc<dyn WorldLookup>> {
        self.world.read().clone()
    }

    fn scene_lookup(&self) -> Option<Arc<dyn SceneLookup>> {
        self.scene.read().clone()
    }

    /// The pickers: the linked game's index, the project's part and the open scene's on top.
    pub async fn pickers(&self, editor: &Editor) -> Arc<Pickers> {
        let game = self.game_index(editor).await;
        let project = self.project_part().await;
        let scene = self.scene_lookup();
        let rev = self
            .game_rev(editor)
            .mix(self.world_lookup().map_or(Rev::ZERO, |w| w.rev()))
            .mix(stamp(&self.tree(), PROJECT_PICKER_FILES))
            .mix(scene.as_ref().map_or(Rev::ZERO, |s| s.rev()))
            .mix(Rev(u64::from(game.is_some())));
        if let Some(p) = self.pickers.peek_at(&(), rev) {
            return p;
        }
        let (restrictors, stories) =
            scene.map_or_else(|| (Vec::new(), Vec::new()), |s| (s.restrictors(), s.stories()));
        self.pickers
            .fresh(&(), rev, move || Pickers::compose(game.as_deref(), &project, &restrictors, &stories))
            .await
    }

    fn game_rev(&self, _editor: &Editor) -> Rev {
        self.project.game_rev()
    }

    async fn game_index(&self, editor: &Editor) -> Option<Arc<GameIndex>> {
        let link = GameLink::of(editor).ok()?;
        let (files, rev) = link.snapshot().await.ok()?;
        let world = self.world_lookup();
        if let Some(w) = &world {
            w.ready().await;
        }
        let key = rev.mix(world.as_ref().map_or(Rev::ZERO, |w| w.rev()));
        if let Some(g) = self.game_pickers.peek_at(&(), key) {
            return g.as_ref().clone();
        }
        let ltx = link.configs().await.ok();
        let lang = link.language().await.unwrap_or_else(|_| "rus".to_owned());
        let strings = link.strings(&lang).await.ok();
        let transitions = world.map(|w| w.transitions()).unwrap_or_default();
        let built = self
            .exec
            .io(move || {
                let empty = gz_fmt_xml::strings::StringTable::default();
                let inp = pickers::GameInput {
                    files: &files,
                    ltx: ltx.as_deref(),
                    strings: strings.as_deref().unwrap_or(&empty),
                    transitions: &transitions,
                };
                Some(Arc::new(pickers::build_game(&inp)))
            })
            .await;
        self.game_pickers.fresh(&(), key, move || built).await.as_ref().clone()
    }

    async fn project_part(&self) -> Arc<ProjectPart> {
        let tree = self.tree();
        let rev = stamp(&tree, PROJECT_PICKER_FILES);
        if let Some(p) = self.project_pickers.peek_at(&(), rev) {
            return p;
        }
        let root = PathBuf::from(&self.root);
        let exec = self.exec.clone();
        let part =
            self.exec.io(move || pickers::build_project(&read_project_files(&exec, &root, &tree))).await;
        self.project_pickers.fresh(&(), rev, move || part).await
    }

    /// What the linked game's item sections can carry; `None` without a linked game.
    pub async fn caps(&self, editor: &Editor) -> Option<Arc<ItemCaps>> {
        let link = GameLink::of(editor).ok()?;
        let (files, rev) = link.snapshot().await.ok()?;
        if let Some(c) = self.caps.peek_at(&(), rev) {
            return c.as_ref().clone();
        }
        let ltx = link.configs().await.ok();
        let built = match ltx {
            None => None,
            Some(ltx) => {
                let script = self
                    .exec
                    .io(move || {
                        files.read("scripts/class_registrator.script").map(|b| decode(&b)).unwrap_or_default()
                    })
                    .await;
                Some(Arc::new(ItemCaps::new(ltx, &script)))
            }
        };
        self.caps.fresh(&(), rev, move || built).await.as_ref().clone()
    }

    /// [`way_paths`](Self::way_paths) once the linked game's patrols are read.
    pub async fn way_paths_ready(&self) -> Vec<PickerEntry> {
        if let Some(w) = self.world_lookup() {
            w.ready().await;
        }
        self.way_paths()
    }

    /// The way paths of the open scene, then the linked game's own patrols (`SceneWayPaths`).
    #[must_use]
    pub fn way_paths(&self) -> Vec<PickerEntry> {
        let mut out = self.scene_lookup().map(|s| s.way_paths()).unwrap_or_default();
        if let Some(w) = self.world_lookup() {
            for name in w.patrols() {
                if !out.iter().any(|e| e.id == name) {
                    out.push(PickerEntry { id: name, name: String::new(), extra: "game".into() });
                }
            }
        }
        out
    }

    // ---- characters and globals ----------------------------------------------------------------

    /// The project's characters: the characters area's briefs, or the file tree's asset kinds.
    #[must_use]
    pub fn characters(&self) -> (Rev, Arc<[CharacterBrief]>) {
        if let Some(c) = self.characters.read().clone() {
            return (c.rev(), c.briefs());
        }
        let tree = self.tree();
        let mut out: Vec<CharacterBrief> = Vec::new();
        for rel in tree.files_with_extension("npcasset") {
            let template = tree.entry(&rel).is_some_and(|e| {
                matches!(e.kind, EntryKind::File { asset: Some(AssetKind::CharacterTemplate), .. })
            });
            out.push(CharacterBrief {
                id: crate::text::id_from_name(rel.stem()),
                template,
                name: String::new(),
                file: rel.as_str().to_owned(),
            });
        }
        (tree.rev(), Arc::from(out))
    }

    /// The module's global variables, names only, with the manifest's revision.
    async fn globals(&self, editor: &Editor) -> (Rev, Vec<String>, Vec<(String, String)>) {
        match self.manifest_now(editor).await {
            Some(m) => {
                let (rev, vars) = m.globals();
                (
                    rev,
                    vars.iter().map(|v| v.name.clone()).collect(),
                    vars.iter().map(|v| (v.name.clone(), v.value.clone())).collect(),
                )
            }
            None => (Rev::ZERO, Vec::new(), Vec::new()),
        }
    }

    /// The globals with their defaults as written.
    pub async fn global_vars(&self, editor: &Editor) -> Vec<(String, String)> {
        self.globals(editor).await.2
    }

    // ---- the project index ---------------------------------------------------------------------

    fn overlay_rev(&self) -> Rev {
        let docs = self.docs.lock().clone();
        let mut r = self.docs_rev.get();
        for d in docs {
            let snap = d.store.snapshot();
            r = r.mix(Rev(snap.edit)).mix(Rev(u64::from(d.is_dirty())));
        }
        r
    }

    /// Every graph of the project (`NqProjectIndex::Snapshot`); `overlay` lays dirty open
    /// documents over their files.
    pub async fn index(&self, editor: &Editor, overlay: bool) -> Arc<IndexSnapshot> {
        let catalog = self.catalog(editor).await;
        let manifest = self.manifest(editor);
        let tree = self.tree();
        let rules: Vec<String> = manifest
            .as_ref()
            .and_then(|m| m.snapshot().manifest().map(|x| x.exclude.clone()))
            .unwrap_or_default();
        // the graphs of the project are the graphs its build ships
        let mut files: Vec<RelPath> = tree.files_with_extension("nqasset");
        files.extend(tree.files_with_extension("behasset"));
        files.retain(|rel| !gz_mod::outside_build(&rules, rel.as_str()));
        // the watcher says when a graph file changed; the list of them, when one came or went
        // with a folder; without a watcher the tree's stamps say it all
        let files_rev = if self.project.tree.changes().live() {
            let mut h = index::FNV_OFFSET;
            for f in &files {
                h = index::hash_bytes(h, f.key().as_bytes());
            }
            self.graphs_rev.get().mix(Rev(h))
        } else {
            tree.rev()
        };
        let rev = files_rev
            .mix(manifest.as_ref().map_or(Rev::ZERO, |m| m.rev()))
            .mix(Rev(u64::from(catalog.generation)))
            .mix(if overlay { self.overlay_rev() } else { Rev::ZERO });
        if let Some(s) = self.index.peek_at(&overlay, rev) {
            return s;
        }
        let root_norm = normalize_path(&self.root);
        let mut todo: Vec<ToRead> = Vec::new();
        let mut entries: Vec<IndexEntry> = Vec::new();
        let dirty: Vec<Arc<DocHandle>> = if overlay {
            self.docs.lock().iter().filter(|d| d.is_dirty()).cloned().collect()
        } else {
            Vec::new()
        };
        {
            let cache = self.parsed.lock();
            for rel in &files {
                let abs = format!("{root_norm}\\{}", rel.to_backslash());
                let key = path_key(&abs);
                if let Some(d) = dirty.iter().find(|d| d.path.read().key == key) {
                    entries.push(memory_entry(&catalog, d, rel.as_str(), &key));
                    continue;
                }
                let stamp = match tree.entry(rel).map(|e| e.kind) {
                    Some(EntryKind::File { size, modified, .. }) => (size, modified),
                    _ => (0, None),
                };
                match cache.get(&key) {
                    Some(c) if c.stamp == stamp && c.catalog == catalog.generation && stamp.1.is_some() => {
                        entries.push(c.entry.clone())
                    }
                    _ => todo.push((rel.as_str().to_owned(), key, rel.to_fs(Path::new(&self.root)), stamp)),
                }
            }
        }
        if !todo.is_empty() {
            let cat = catalog.clone();
            let exec = self.exec.clone();
            let read: Vec<(String, (u64, Option<SystemTime>), IndexEntry)> = self
                .exec
                .io(move || {
                    exec.par_io(todo, |(rel, key, abs, stamp)| {
                        let entry = match read_graph_file(&abs) {
                            Ok(text) => index::disk_entry(&cat, &rel, &key, &text),
                            Err(e) => IndexEntry {
                                rel: rel.clone(),
                                key: key.clone(),
                                graph: Arc::new(Graph::default()),
                                error: e,
                                readable: false,
                                complete: false,
                                memory: false,
                                content_hash: 0,
                            },
                        };
                        (key, stamp, entry)
                    })
                })
                .await;
            let mut cache = self.parsed.lock();
            for (key, stamp, entry) in read {
                cache.insert(key, Parsed { stamp, catalog: catalog.generation, entry: entry.clone() });
                entries.push(entry);
            }
        }
        // dirty documents of the project the tree does not list (yet)
        for d in &dirty {
            let p = d.path();
            if !p.inside || !asset::is_graph_path(&p.wire) || entries.iter().any(|e| e.key == p.key) {
                continue;
            }
            entries.push(memory_entry(&catalog, d, &p.wire, &p.key));
        }
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        let fingerprint = index::fingerprint(&self.root_key, &entries);
        let generation = {
            let mut g = self.generations.lock();
            let slot = &mut g[usize::from(overlay)];
            if !slot.2 || slot.0 != fingerprint {
                slot.0 = fingerprint;
                slot.1 = slot.1.wrapping_add(1).max(1);
                slot.2 = true;
            }
            slot.1
        };
        let snap = IndexSnapshot { entries, generation, fingerprint };
        self.index.fresh(&overlay, rev, move || snap).await
    }

    // ---- validation ----------------------------------------------------------------------------

    /// Everything validation reads, with its fingerprint. `overlay`: the index with the dirty
    /// documents laid over (the editor's own checks); without it, what is on disk (the build).
    pub async fn env(&self, editor: &Editor, overlay: bool) -> Arc<ValidateEnv> {
        let catalog = self.catalog(editor).await;
        let pickers = self.pickers(editor).await;
        let caps = if pickers.available() { self.caps(editor).await } else { None };
        let (crev, characters) = self.characters();
        let (grev, globals, _) = self.globals(editor).await;
        let level = self.scene_lookup().and_then(|s| s.level());
        let index = self.index(editor, overlay).await;
        let rev = Rev(u64::from(catalog.generation))
            .mix(self.project.game_rev())
            .mix(self.tree().rev())
            .mix(self.scene_lookup().map_or(Rev::ZERO, |s| s.rev()))
            .mix(crev)
            .mix(grev)
            .mix(Rev(index.fingerprint));
        Arc::new(ValidateEnv { catalog, pickers, caps, characters, globals, level, index, rev })
    }

    /// The problems of an open document (`NqDoc::Validate`): computed once per content and per
    /// state of what it reads; the reader's shape problems of its load come first.
    pub async fn problems(&self, editor: &Editor, doc: &DocHandle) -> Arc<Vec<NqProblem>> {
        let env = self.env(editor, true).await;
        let snap = doc.store.snapshot();
        let p = doc.path();
        let read = doc.read_problems();
        let rev = Rev(snap.edit)
            .mix(env.rev)
            .mix(Rev(index::content_hash(&p.key)))
            .mix(Rev(doc.disk().rev))
            .mix(Rev(read.len() as u64));
        let wire = p.wire.clone();
        self.problems
            .fresh(&p.key, rev, move || {
                let mut out: Vec<NqProblem> =
                    read.iter().filter(|r| r.severity != gz_base::Severity::Note).map(read_problem).collect();
                out.extend(validate::shape_errors(&snap.shape));
                // the header is the read path's: it says what the file holds, not the model
                out.extend(env.validate(&snap.graph, &wire).into_iter().filter(|x| x.code != "E001"));
                out
            })
            .await
    }

    // ---- documents -----------------------------------------------------------------------------

    /// The open document of a path (`NqDocs::Get`).
    #[must_use]
    pub fn get_doc(&self, path: &str) -> Option<Arc<DocHandle>> {
        let p = self.resolve(path, false).ok()?;
        self.docs.lock().iter().find(|d| d.path.read().key == p.key).cloned()
    }

    #[must_use]
    pub fn docs(&self) -> Vec<Arc<DocHandle>> {
        self.docs.lock().clone()
    }

    /// Sends one command to a document with the current catalog.
    pub async fn send(
        &self,
        editor: &Editor,
        doc: &DocHandle,
        op: DocOp,
        origin: Origin,
    ) -> Result<DocReply, CmdError> {
        let catalog = self.catalog(editor).await;
        doc.store.send(DocCmd { catalog, op }, origin).await
    }

    /// The open document of a path key.
    pub(crate) fn find_doc(&self, key: &str) -> Option<Arc<DocHandle>> {
        self.docs.lock().iter().find(|d| d.path.read().key == key).cloned()
    }

    /// Closes a document; a dirty one only with `discard`.
    pub fn close(&self, path: &str, discard: bool) -> Result<(), String> {
        let d = self.get_doc(path).ok_or_else(|| "not open".to_owned())?;
        if d.is_dirty() && !discard {
            return Err("unsaved".into());
        }
        self.docs.lock().retain(|x| !Arc::ptr_eq(x, &d));
        self.docs_rev.bump();
        self.problems.retain(|k| *k != d.path.read().key);
        Ok(())
    }

    /// A file or a folder was renamed: open documents follow it, and their files anew.
    pub fn retarget(&self, editor: &Arc<Editor>, from: &Path, to: &Path) {
        let was = path_key(&from.to_string_lossy());
        let new_abs = normalize_path(&to.to_string_lossy());
        let old_abs = normalize_path(&from.to_string_lossy());
        let mut moved = Vec::new();
        for d in self.docs.lock().iter() {
            let mut p = d.path.write();
            let target = if p.key == was {
                Some(new_abs.clone())
            } else if p.key.len() > was.len()
                && p.key.starts_with(&was)
                && p.key.as_bytes().get(was.len()) == Some(&b'\\')
            {
                let abs = normalize_path(&p.abs.to_string_lossy());
                abs.get(old_abs.len()..).map(|rest| format!("{new_abs}{rest}"))
            } else {
                None
            };
            if let Some(abs) = target {
                *p = DocPath {
                    wire: self.wire(&abs),
                    key: path_key(&abs),
                    inside: self.inside(&abs),
                    abs: PathBuf::from(abs),
                };
                moved.push(d.clone());
            }
        }
        self.docs_rev.bump();
        for d in moved {
            self.follow(editor, &d);
        }
    }

    /// Waits until the tree holds every change marked so far.
    pub async fn refresh_tree(&self) {
        self.project.tree.refresh().await;
    }

    pub(crate) fn log(&self) -> &Log {
        &self.log
    }
}

/// What the project's part of the pickers reads: folder and extension.
const PROJECT_PICKER_FILES: &[(&str, &str)] = &[
    ("spawn/custom_data", "ltx"),
    ("rawdata/levels", "part"),
    ("gamedata/configs/gameplay", "xml"),
    ("gamedata/configs/ui/textures_descr", "xml"),
];

/// The stamp of the files below some folders with an extension: their paths, sizes and write
/// times. It moves only when one of them does - an unrelated file of the project leaves it be.
fn stamp(tree: &FileTree, dirs: &[(&str, &str)]) -> Rev {
    let mut h = index::FNV_OFFSET;
    for (dir, ext) in dirs {
        let Ok(under) = RelPath::new(dir) else { continue };
        for (rel, e) in tree.files_under(&under) {
            if !rel.has_extension(ext) {
                continue;
            }
            h = index::hash_bytes(h, rel.key().as_bytes());
            if let EntryKind::File { size, modified, .. } = e.kind {
                h = index::hash_bytes(h, &size.to_le_bytes());
                let t = modified
                    .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                h = index::hash_bytes(h, &t.to_le_bytes());
            }
        }
    }
    Rev(h)
}

/// What reading a file found, as a problem of the graph.
fn read_problem(p: &Problem) -> NqProblem {
    NqProblem {
        severity: if p.is_error() { validate::Severity::Error } else { validate::Severity::Warning },
        code: p.code.clone(),
        node: String::new(),
        slot: String::new(),
        message: p.message.clone(),
    }
}

/// A counter of the graph files' changes, moved by the project's watcher: the index reads it
/// instead of the whole tree's stamp.
fn follow_graphs(editor: &Editor, project: &Arc<Project>) -> Arc<RevCounter> {
    let counter = Arc::new(RevCounter::new());
    let mut rx =
        project.tree.changes().subscribe(Filter::Extensions(vec!["nqasset".into(), "behasset".into()]));
    let (weak, tree) = (Arc::downgrade(&counter), Arc::downgrade(project));
    editor.exec.spawn(async move {
        while rx.recv().await.is_some() {
            // the tree lists what the index reads: it is current before the counter moves
            if let Some(p) = tree.upgrade() {
                p.tree.refresh().await;
            }
            match weak.upgrade() {
                Some(c) => {
                    c.bump();
                }
                None => break,
            }
        }
    });
    counter
}

fn memory_entry(catalog: &Catalog, d: &DocHandle, rel: &str, key: &str) -> IndexEntry {
    let snap = d.store.snapshot();
    let text = asset::write(catalog, &snap.graph);
    IndexEntry {
        rel: rel.to_owned(),
        key: key.to_owned(),
        graph: Arc::new(snap.graph.clone()),
        error: if snap.partial {
            "open document is an incomplete model of a malformed source file".to_owned()
        } else {
            String::new()
        },
        readable: true,
        complete: !snap.partial,
        memory: true,
        content_hash: index::content_hash(&text),
    }
}

/// The files of `dir` (project-relative) with an extension, listed by the tree, read and decoded.
fn read_tree_files(
    exec: &Executors,
    root: &Path,
    tree: &FileTree,
    dir: &str,
    ext: &str,
    name: Option<&str>,
    max: u64,
) -> Vec<(String, String)> {
    let Ok(under) = RelPath::new(dir) else { return Vec::new() };
    let items: Vec<(String, PathBuf)> = tree
        .files_under(&under)
        .into_iter()
        .filter(|(rel, e)| {
            rel.has_extension(ext)
                && name.is_none_or(|n| rel.stem().eq_ignore_ascii_case(n))
                && matches!(e.kind, EntryKind::File { size, .. } if size > 0 && size < max)
        })
        .map(|(rel, _)| (rel.as_str().to_owned(), rel.to_fs(root)))
        .collect();
    exec.par_io(items, |(rel, abs)| {
        std::fs::read(&abs).ok().map(|b| (rel, gz_base::text::decode_guess(&b).into_owned()))
    })
    .into_iter()
    .flatten()
    .collect()
}

fn read_project_files(exec: &Executors, root: &Path, tree: &FileTree) -> ProjectFiles {
    ProjectFiles {
        custom_data: read_tree_files(exec, root, tree, "spawn/custom_data", "ltx", None, 1 << 20),
        spawn_parts: read_tree_files(exec, root, tree, "rawdata/levels", "part", Some("spawn"), 64 << 20),
        gameplay: read_tree_files(exec, root, tree, "gamedata/configs/gameplay", "xml", None, 8 << 20),
        textures_descr: read_tree_files(
            exec,
            root,
            tree,
            "gamedata/configs/ui/textures_descr",
            "xml",
            None,
            8 << 20,
        ),
    }
}

/// Gathers and folds the catalog's files in the C++ order. Blocking: the IO pool.
fn load_catalog(
    game: Option<&GameFiles>,
    bundled: &Bundled,
    root: &Path,
    tree: &FileTree,
    own: &str,
    generation: u32,
) -> Catalog {
    let mut files: Vec<SourceFile> = Vec::new();
    let mut from_game = false;
    if let Some(g) = game
        && g.exists("configs/nq/catalog.ltx")
    {
        from_game = true;
        if let Ok(b) = g.read("configs/nq/catalog.ltx") {
            files.push(SourceFile { label: "game:configs\\nq\\catalog.ltx".into(), text: decode(&b) });
        }
        // extensions shipped inside the game content
        for f in g.find("configs/nq/kinds", "ltx") {
            if let Ok(b) = g.read_ref(f) {
                files.push(SourceFile {
                    label: format!("game:{}", f.path.replace('/', "\\")),
                    text: decode(&b),
                });
            }
        }
    }
    if !from_game {
        let path = bundled.path("configs/nq/catalog.ltx");
        match std::fs::read(&path) {
            Err(_) => {
                return Catalog::assemble(
                    &files,
                    false,
                    Some(format!("bundled catalog not found: {}", path.display())),
                    generation,
                );
            }
            Ok(b) => {
                files.push(SourceFile { label: "bundled:configs\\nq\\catalog.ltx".into(), text: decode(&b) })
            }
        }
        let dir = bundled.path("configs/nq/kinds");
        let mut kinds: Vec<PathBuf> = Vec::new();
        list_ltx(&dir, &mut kinds);
        kinds.sort();
        for k in kinds {
            if let Ok(b) = std::fs::read(&k) {
                files.push(SourceFile {
                    label: format!(
                        "bundled:configs\\nq\\kinds\\{}.ltx",
                        asset::base_name(&k.to_string_lossy())
                    ),
                    text: decode(&b),
                });
            }
        }
    }
    // installed modules extend it, all but the open project's own: that one is read from the
    // project itself, last, so what its author is writing wins over any earlier build of it
    if let Some(g) = game {
        for r in ModuleRoot::ALL {
            let mut mods: Vec<&gz_vfs::Module> = g.modules().iter().filter(|m| m.root == r).collect();
            mods.sort_by_key(|m| m.id.to_lowercase());
            for m in mods {
                if !own.is_empty() && m.id.eq_ignore_ascii_case(own) {
                    continue;
                }
                for f in m.find("gamedata/configs/nq/kinds", "ltx") {
                    if let Ok(b) = m.read(f.path) {
                        files.push(SourceFile {
                            label: format!("{}\\{}\\kinds\\{}", r.folder(), m.id, asset::base_name(f.path)),
                            text: decode(&b),
                        });
                    }
                }
            }
        }
    }
    if let Ok(under) = RelPath::new("gamedata/configs/nq/kinds") {
        for (rel, _) in tree.files_under(&under) {
            if !rel.has_extension("ltx") {
                continue;
            }
            if let Ok(b) = std::fs::read(rel.to_fs(root)) {
                files.push(SourceFile {
                    label: format!("project:kinds\\{}.ltx", rel.stem()),
                    text: decode(&b),
                });
            }
        }
    }
    Catalog::assemble(&files, from_game, None, generation)
}

fn list_ltx(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if gz_project::is_tooling_entry(&name) {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let p = e.path();
        if ft.is_dir() {
            list_ltx(&p, out);
        } else if asset::extension(&name) == ".ltx" {
            out.push(p);
        }
    }
}

/// The unsaved graphs, as every command that closes work asks.
pub(crate) struct NqDirty;

impl DirtySource for NqDirty {
    fn dirty(&self, editor: &Editor) -> Vec<DirtyDoc> {
        let Some(session) = editor.project_opt() else { return Vec::new() };
        let Some(svc) = NqService::get(&session) else { return Vec::new() };
        svc.docs()
            .iter()
            .filter(|d| d.is_dirty())
            .map(|d| {
                let p = d.path();
                let behaviour = asset::is_behaviour_path(&p.wire);
                DirtyDoc {
                    kind: (if behaviour { "behaviour" } else { "quest" }).into(),
                    name: p
                        .abs
                        .file_name()
                        .map_or_else(|| p.wire.clone(), |n| n.to_string_lossy().into_owned()),
                    path: Some(p.wire),
                }
            })
            .collect()
    }

    fn save<'a>(&'a self, editor: &'a Arc<Editor>) -> gz_editor::SettleFuture<'a> {
        Box::pin(async move {
            let Some(svc) = editor.project_opt().and_then(|s| NqService::get(&s)) else { return Ok(()) };
            for d in svc.docs().into_iter().filter(|d| d.is_dirty()) {
                let name = d.path().wire;
                svc.save(editor, &d).await.map_err(|e| format!("{name}: {e}"))?;
            }
            Ok(())
        })
    }

    fn discard<'a>(&'a self, editor: &'a Arc<Editor>) -> gz_editor::SettleFuture<'a> {
        Box::pin(async move {
            let Some(svc) = editor.project_opt().and_then(|s| NqService::get(&s)) else { return Ok(()) };
            for d in svc.docs().into_iter().filter(|d| d.is_dirty()) {
                // back to the file; a document never saved has no file to go back to
                if svc.reload(editor, &d).await.is_err() {
                    let _ = svc.close(&d.path().wire, true);
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_normalise_like_the_cxx() {
        assert_eq!(normalize_path("a//b/./c\\"), "a\\b\\c");
        assert_eq!(normalize_path("C:\\"), "C:\\");
        assert_eq!(path_key("Q/Wolf.NQASSET"), "q\\wolf.nqasset");
    }
}
