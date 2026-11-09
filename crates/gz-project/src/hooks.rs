//! What other areas hang on the project area: work to do when a project opens or closes, the
//! module skeleton a project gets, files that follow a rename, the templates `content_create`
//! writes and the lists `list_assets` answers.

use std::path::Path;
use std::sync::Arc;

use gz_editor::{Editor, ProjectSession};
use parking_lot::RwLock;
use rustc_hash::FxHashMap;

type SessionHook = Arc<dyn Fn(&Arc<Editor>, &Arc<ProjectSession>) + Send + Sync>;
pub(crate) type ScaffoldHook = Arc<dyn Fn(&Path, &str) -> Result<(), String> + Send + Sync>;
type RenameHook = Arc<dyn Fn(&Arc<Editor>, &Path, &Path) + Send + Sync>;

/// Hooks other areas register in `editor.services`; this crate's open, close, create and rename
/// call them.
#[derive(Default)]
pub struct ProjectHooks {
    open: RwLock<Vec<SessionHook>>,
    close: RwLock<Vec<SessionHook>>,
    scaffold: RwLock<Vec<ScaffoldHook>>,
    renamed: RwLock<Vec<RenameHook>>,
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

/// The kinds `content_create` makes, with the first free name of each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TemplateKind {
    Quest,
    Behaviour,
    Character,
    Pda,
}

impl TemplateKind {
    /// `quest`, `behaviour`, `character` or `pda`, any case.
    #[must_use]
    pub fn parse(kind: &str) -> Option<Self> {
        [Self::Quest, Self::Behaviour, Self::Character, Self::Pda].into_iter().find(|k| k.key().eq_ignore_ascii_case(kind))
    }

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Quest => "quest",
            Self::Behaviour => "behaviour",
            Self::Character => "character",
            Self::Pda => "pda",
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

    pub fn provide(&self, kind: TemplateKind, f: impl Fn(&TemplateCx<'_>) -> Result<Vec<u8>, String> + Send + Sync + 'static) {
        self.map.write().insert(kind, Arc::new(f));
    }

    pub(crate) fn get(&self, kind: TemplateKind) -> Option<TemplateFn> {
        self.map.read().get(&kind).cloned()
    }
}

/// The content browser's categories, in its order, as `list_assets` names them.
pub const CATEGORIES: [&str; 10] = ["Objects", "Levels", "Groups", "Visuals", "Textures", "Textures (raw)", "Particles", "Sounds", "Entities", "Light Anims"];

type ListFn = Arc<dyn Fn(&Editor) -> Vec<String> + Send + Sync>;

/// The enumerators of the library categories, given by the areas that read them (the object
/// library, the linked game). `Levels` is this crate's own. A provider runs on the IO pool.
#[derive(Default)]
pub struct AssetLists {
    map: RwLock<FxHashMap<&'static str, ListFn>>,
}

impl AssetLists {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<AssetLists> {
        editor.services.get_or_init(AssetLists::default)
    }

    /// `category` is one of [`CATEGORIES`]; another name is ignored.
    pub fn provide(&self, category: &str, f: impl Fn(&Editor) -> Vec<String> + Send + Sync + 'static) {
        if let Some(name) = CATEGORIES.iter().find(|c| c.eq_ignore_ascii_case(category)) {
            self.map.write().insert(name, Arc::new(f));
        }
    }

    pub(crate) fn get(&self, category: &str) -> Option<ListFn> {
        self.map.read().get(category).cloned()
    }
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
}
