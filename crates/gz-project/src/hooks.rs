//! What other areas hang on the project area: work to do when a project opens or closes, the
//! module skeleton a project gets, files that follow a rename, the question a second start's
//! project asks in the window, the templates `file.create` writes and the library lists
//! `file.list` answers.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use gz_editor::{Editor, ProjectSession};
use parking_lot::RwLock;
use rustc_hash::FxHashMap;

type SessionHook = Arc<dyn Fn(&Arc<Editor>, &Arc<ProjectSession>) + Send + Sync>;
pub(crate) type ScaffoldHook = Arc<dyn Fn(&Path, &str) -> Result<(), String> + Send + Sync>;
type RenameHook = Arc<dyn Fn(&Arc<Editor>, &Path, &Path) + Send + Sync>;
type BeforeCloseHook =
    Arc<dyn Fn(Arc<Editor>, Arc<ProjectSession>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;
type LiveSceneFn = Arc<dyn Fn(&Editor) -> bool + Send + Sync>;
type FolderFn = Arc<dyn Fn(&Editor) -> Result<String, String> + Send + Sync>;
type LaunchFn = Arc<dyn Fn(&Arc<Editor>, &str) -> bool + Send + Sync>;

/// Hooks other areas register in `editor.services`; this crate's open, close, create and rename
/// call them.
#[derive(Default)]
pub struct ProjectHooks {
    open: RwLock<Vec<SessionHook>>,
    before_close: RwLock<Vec<BeforeCloseHook>>,
    close: RwLock<Vec<SessionHook>>,
    content_folder: RwLock<Option<FolderFn>>,
    live_scene: RwLock<Option<LiveSceneFn>>,
    scaffold: RwLock<Vec<ScaffoldHook>>,
    renamed: RwLock<Vec<RenameHook>>,
    launch: RwLock<Option<LaunchFn>>,
}

impl ProjectHooks {
    /// The hooks of this editor, made on first use.
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<ProjectHooks> {
        editor.services.get_or_init(ProjectHooks::default)
    }

    /// Runs once a project is open: `editor.project()` is the new session. Restoring tabs,
    /// opening the last scene.
    pub fn on_open(&self, f: impl Fn(&Arc<Editor>, &Arc<ProjectSession>) + Send + Sync + 'static) {
        self.open.write().push(Arc::new(f));
    }

    /// Runs first when a project closes - before the other close hooks and before anything of
    /// the session is dropped - and is awaited: what still needs the open scene
    /// (`preview.png`) is taken here.
    pub fn on_before_close<F>(
        &self,
        f: impl Fn(Arc<Editor>, Arc<ProjectSession>) -> F + Send + Sync + 'static,
    ) where
        F: Future<Output = ()> + Send + 'static,
    {
        self.before_close.write().push(Arc::new(move |ed, session| Box::pin(f(ed, session))));
    }

    /// What `project.open from_launch=true` does when unsaved work is in the way: the window asks
    /// Save / Don't Save / Cancel and opens the project (`folder`) after the answer. The provider
    /// answers whether it asked; without one (no window) the open answers `unsaved`.
    pub fn provide_launch_prompt(&self, f: impl Fn(&Arc<Editor>, &str) -> bool + Send + Sync + 'static) {
        *self.launch.write() = Some(Arc::new(f));
    }

    pub(crate) fn ask_before_launch(&self, editor: &Arc<Editor>, folder: &str) -> bool {
        let f = self.launch.read().clone();
        f.is_some_and(|f| f(editor, folder))
    }

    /// Where `file.create` makes an asset when the caller names no folder: the folder the
    /// Content Browser shows (project-relative, `""` the root), or why it cannot (the browser
    /// shows a source that is not the project). Without a provider: the project root.
    pub fn provide_content_folder(
        &self,
        f: impl Fn(&Editor) -> Result<String, String> + Send + Sync + 'static,
    ) {
        *self.content_folder.write() = Some(Arc::new(f));
    }

    pub(crate) fn content_folder(&self, editor: &Editor) -> Result<String, String> {
        let f = self.content_folder.read().clone();
        f.map_or_else(|| Ok(String::new()), |f| f(editor))
    }

    /// Whether a scene with something in it is open: a
    /// different game is not linked under it.
    pub fn provide_live_scene(&self, f: impl Fn(&Editor) -> bool + Send + Sync + 'static) {
        *self.live_scene.write() = Some(Arc::new(f));
    }

    /// The scene area's answer; `false` without one.
    #[must_use]
    pub fn live_scene(&self, editor: &Editor) -> bool {
        let f = self.live_scene.read().clone();
        f.is_some_and(|f| f(editor))
    }

    /// Runs while the closing project is still open, after its unsaved work was settled.
    pub fn on_close(&self, f: impl Fn(&Arc<Editor>, &Arc<ProjectSession>) + Send + Sync + 'static) {
        self.close.write().push(Arc::new(f));
    }

    /// Runs on the IO pool for every project that opens or is created, before its files are
    /// read: `(root, project name)`. The module area writes the skeleton `mod.ltx` here. An
    /// `Err` is logged and does not stop the open.
    pub fn on_scaffold(&self, f: impl Fn(&Path, &str) -> Result<(), String> + Send + Sync + 'static) {
        self.scaffold.write().push(Arc::new(f));
    }

    /// Runs after a file or folder of the project was renamed: `(from, to)`, absolute. Open
    /// documents follow it.
    pub fn on_renamed(&self, f: impl Fn(&Arc<Editor>, &Path, &Path) + Send + Sync + 'static) {
        self.renamed.write().push(Arc::new(f));
    }

    pub(crate) fn run_open(&self, editor: &Arc<Editor>, session: &Arc<ProjectSession>) {
        let hooks: Vec<SessionHook> = self.open.read().clone();
        for h in hooks {
            h(editor, session);
        }
    }

    pub(crate) async fn run_before_close(&self, editor: &Arc<Editor>, session: &Arc<ProjectSession>) {
        let hooks: Vec<BeforeCloseHook> = self.before_close.read().clone();
        for h in hooks {
            h(editor.clone(), session.clone()).await;
        }
    }

    pub(crate) fn run_close(&self, editor: &Arc<Editor>, session: &Arc<ProjectSession>) {
        let hooks: Vec<SessionHook> = self.close.read().clone();
        for h in hooks {
            h(editor, session);
        }
    }

    /// The scaffold hooks, to be run on the IO pool.
    pub(crate) fn scaffolds(&self) -> Vec<ScaffoldHook> {
        self.scaffold.read().clone()
    }

    pub(crate) fn run_renamed(&self, editor: &Arc<Editor>, from: &Path, to: &Path) {
        let hooks: Vec<RenameHook> = self.renamed.read().clone();
        for h in hooks {
            h(editor, from, to);
        }
    }
}

/// The kinds `file.create` makes, with the first free name of each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TemplateKind {
    Quest,
    Behaviour,
    Character,
    Pda,
    Item,
    Model,
    Texture,
    Sound,
}

impl TemplateKind {
    /// `quest`, `behaviour`, `character`, `pda`, `item`, `model`, `texture` or `sound`, any case.
    #[must_use]
    pub fn parse(kind: &str) -> Option<Self> {
        [
            Self::Quest,
            Self::Behaviour,
            Self::Character,
            Self::Pda,
            Self::Item,
            Self::Model,
            Self::Texture,
            Self::Sound,
        ]
        .into_iter()
        .find(|k| k.key().eq_ignore_ascii_case(kind))
    }

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Quest => "quest",
            Self::Behaviour => "behaviour",
            Self::Character => "character",
            Self::Pda => "pda",
            Self::Item => "item",
            Self::Model => "model",
            Self::Texture => "texture",
            Self::Sound => "sound",
        }
    }

    /// `NewQuest.nqasset`, `NewQuest2.nqasset`, ...
    #[must_use]
    pub fn leaf(self, n: u32) -> String {
        let (base, ext) = match self {
            Self::Quest => ("NewQuest", ".nqasset"),
            Self::Behaviour => ("NewBehaviour", ".behasset"),
            Self::Character => ("NewCharacter", ".npcasset"),
            Self::Pda => ("NewPda", ".pdaasset"),
            Self::Item => ("NewItem", ".itemasset"),
            Self::Model => ("NewModel", ".meshasset"),
            Self::Texture => ("NewTexture", ".texasset"),
            Self::Sound => ("NewSound", ".sndasset"),
        };
        if n <= 1 { format!("{base}{ext}") } else { format!("{base}{n}{ext}") }
    }

    /// What the author calls the made thing, in the messages.
    #[must_use]
    pub fn noun(self) -> &'static str {
        match self {
            Self::Quest => "quest",
            Self::Behaviour => "behaviour graph",
            Self::Character => "character",
            Self::Pda => "PDA interface",
            Self::Item => "item",
            Self::Model => "model",
            Self::Texture => "texture",
            Self::Sound => "sound",
        }
    }

    /// The refusal when every name up to 999 is taken.
    #[must_use]
    pub fn too_many(self) -> &'static str {
        match self {
            Self::Quest => "Too many unnamed quests here.",
            Self::Behaviour => "Too many unnamed behaviour graphs here.",
            Self::Character => "Too many unnamed characters here.",
            Self::Pda => "no free NewPda name in the folder",
            Self::Item => "Too many unnamed items here.",
            Self::Model => "Too many unnamed models here.",
            Self::Texture => "Too many unnamed textures here.",
            Self::Sound => "Too many unnamed sounds here.",
        }
    }
}

/// What a template is made from: the file's name without its extension and its number in the
/// `NewQuest`, `NewQuest2`, ... row.
pub struct TemplateCx<'a> {
    pub stem: &'a str,
    pub n: u32,
}

type TemplateFn = Arc<dyn Fn(&TemplateCx<'_>) -> Result<Vec<u8>, String> + Send + Sync>;

/// The bytes a new asset of each kind starts as, given by the area that owns the kind (quests,
/// characters, PDA). Lives in `editor.services`.
#[derive(Default)]
pub struct AssetTemplates {
    map: RwLock<FxHashMap<TemplateKind, TemplateFn>>,
}

impl AssetTemplates {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<AssetTemplates> {
        editor.services.get_or_init(AssetTemplates::default)
    }

    pub fn provide(
        &self,
        kind: TemplateKind,
        f: impl Fn(&TemplateCx<'_>) -> Result<Vec<u8>, String> + Send + Sync + 'static,
    ) {
        self.map.write().insert(kind, Arc::new(f));
    }

    pub(crate) fn get(&self, kind: TemplateKind) -> Option<TemplateFn> {
        self.map.read().get(&kind).cloned()
    }
}

/// The library's categories, in the content browser's order (`file.list source=library`).
pub const CATEGORIES: [&str; 10] = [
    "Objects",
    "Levels",
    "Groups",
    "Visuals",
    "Textures",
    "Textures (raw)",
    "Particles",
    "Sounds",
    "Entities",
    "Light Anims",
];

type ListFn = Arc<dyn Fn(&Editor) -> Vec<String> + Send + Sync>;
type AsyncListFn =
    Arc<dyn Fn(Arc<Editor>) -> Pin<Box<dyn Future<Output = Vec<String>> + Send>> + Send + Sync>;

/// A provider of one category: work for the IO pool, or a computation that awaits the area's own
/// queries.
#[derive(Clone)]
pub(crate) enum Lister {
    Io(ListFn),
    Async(AsyncListFn),
}

/// The enumerators of the library categories, given by the areas that read them (the object
/// library, the linked game). `Levels` is this crate's own. A provider runs on the IO pool.
#[derive(Default)]
pub struct AssetLists {
    map: RwLock<FxHashMap<&'static str, Lister>>,
}

impl AssetLists {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<AssetLists> {
        editor.services.get_or_init(AssetLists::default)
    }

    /// `category` is one of [`CATEGORIES`]; another name is ignored.
    pub fn provide(&self, category: &str, f: impl Fn(&Editor) -> Vec<String> + Send + Sync + 'static) {
        if let Some(name) = CATEGORIES.iter().find(|c| c.eq_ignore_ascii_case(category)) {
            self.map.write().insert(name, Lister::Io(Arc::new(f)));
        }
    }

    /// As [`provide`](Self::provide), for a list an area computes through its own queries.
    pub fn provide_async<F>(&self, category: &str, f: impl Fn(Arc<Editor>) -> F + Send + Sync + 'static)
    where
        F: Future<Output = Vec<String>> + Send + 'static,
    {
        if let Some(name) = CATEGORIES.iter().find(|c| c.eq_ignore_ascii_case(category)) {
            self.map.write().insert(name, Lister::Async(Arc::new(move |ed| Box::pin(f(ed)))));
        }
    }

    pub(crate) fn get(&self, category: &str) -> Option<Lister> {
        self.map.read().get(category).cloned()
    }
}

/// A library listing as the editor's file system gives it (with the extension clamped):
/// every file under `root` with the extension `ext` (any case), as its path below `root` without
/// the extension, lower case, backslashes, in byte order.
#[must_use]
pub fn editor_file_list(root: &Path, ext: &str) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, ext: &str, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() {
                walk(root, &p, ext, out);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext))
                && let Ok(rel) = p.with_extension("").strip_prefix(root)
            {
                out.push(rel.to_string_lossy().replace('/', "\\").to_lowercase());
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, ext, &mut out);
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_names() {
        assert_eq!(TemplateKind::parse("Quest"), Some(TemplateKind::Quest));
        assert_eq!(TemplateKind::parse("folder"), None);
        assert_eq!(TemplateKind::Quest.leaf(1), "NewQuest.nqasset");
        assert_eq!(TemplateKind::Pda.leaf(3), "NewPda3.pdaasset");
        assert_eq!(TemplateKind::Behaviour.leaf(2), "NewBehaviour2.behasset");
    }

    #[test]
    fn file_lists_are_lower_case_in_byte_order() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("Light_preset")).unwrap();
        std::fs::create_dir_all(root.join("act")).unwrap();
        std::fs::write(root.join("Light_preset").join("Lamp.GROUP"), b"").unwrap();
        std::fs::write(root.join("act").join("a.group"), b"").unwrap();
        std::fs::write(root.join("act_b.group"), b"").unwrap();
        std::fs::write(root.join("act").join("skip.txt"), b"").unwrap();
        assert_eq!(editor_file_list(root, "group"), [r"act\a", "act_b", r"light_preset\lamp"]);
    }
}
