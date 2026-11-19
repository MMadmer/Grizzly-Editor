//! The sections a project's assets become in the game.
//!
//! The game makes config sections of what a module ships as assets: a character
//! (`.npcasset`) is the spawn section `<module>_<id>` and its squad. The editor reads the
//! linked game's configs to know a section, and those hold none of them - so whoever owns the
//! assets says here what the game will make ([`OwnSections::add`]), and whatever reads sections
//! (the scene's entities, the spawn roster, the pickers) takes them as the game's own. A spawn
//! point of a module's character is then a spawn point like any other.

use std::sync::Arc;

use futures::future::BoxFuture;
use gz_editor::Editor;
use gz_ltx::Ltx;
use parking_lot::RwLock;

/// One section the game makes of a project's asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnSection {
    /// The section's name, as the game makes it.
    pub name: String,
    /// The section it starts from (`stalker`): every key it does not say is that section's.
    pub parent: String,
    /// Its own lines, the key as written.
    pub lines: Vec<(String, String)>,
    /// The project file it is made of (`characters/guard.npcasset`).
    pub origin: String,
}

impl OwnSection {
    /// The value of one of its own lines, the key in any case.
    #[must_use]
    pub fn line(&self, key: &str) -> Option<&str> {
        self.lines.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
    }
}

/// What answers a project's sections: asked on every read of the settings, so it answers from
/// what it already holds.
pub type OwnSource = Arc<dyn for<'a> Fn(&'a Editor) -> BoxFuture<'a, Vec<OwnSection>> + Send + Sync>;

/// The editor's sources of a project's own sections.
#[derive(Default)]
pub struct OwnSections {
    sources: RwLock<Vec<OwnSource>>,
}

impl OwnSections {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    /// Registers a source: an area does, once, as the editor starts.
    pub fn add(&self, source: OwnSource) {
        self.sources.write().push(source);
    }

    /// Every section the open project's assets become, in the sources' order.
    pub async fn all(editor: &Editor) -> Vec<OwnSection> {
        let sources: Vec<OwnSource> = Self::of(editor).sources.read().clone();
        let mut out = Vec::new();
        for source in sources {
            out.extend(source(editor).await);
        }
        out
    }
}

/// A key of an own section as the game would answer it: its own line, else the parent's -
/// another own section or one of the configs.
#[must_use]
pub fn own_value<'a>(own: &'a [OwnSection], ltx: &'a Ltx, section: &str, key: &str) -> Option<&'a str> {
    let mut name = section.to_ascii_lowercase();
    // a chain of own sections is short; the bound only stops a loop
    for _ in 0..8 {
        match own.iter().find(|o| o.name.eq_ignore_ascii_case(&name)) {
            Some(o) => match o.line(key) {
                Some(v) => return Some(v),
                None => name = o.parent.to_ascii_lowercase(),
            },
            None => break,
        }
    }
    if !ltx.line_exist(&name, key) {
        return None;
    }
    Some(ltx.r_string_raw(&name, key).ok().flatten().unwrap_or(""))
}

/// What answers the game's config tree with the open project's module laid over it; `None`
/// when the module changes no config (the game's own tree is the answer then).
pub type ComposedSource = Arc<dyn for<'a> Fn(&'a Editor) -> BoxFuture<'a, Option<Arc<Ltx>>> + Send + Sync>;

/// The game's configs as a game with the open project's module would read them.
///
/// A module makes and changes sections of the game's configs: an item asset is a section, a
/// config of `configs\xms` and a patch are laid over the tree. Whoever composes the module
/// (the items area) says the result here, and whatever reads sections for the scene - the
/// entities of spawn points, the spawn roster - reads that tree: a section the module makes is
/// a spawn section like any of the game's.
#[derive(Default)]
pub struct ComposedConfigs {
    source: RwLock<Option<ComposedSource>>,
    /// The spawn roster of the last composed tree, by the tree. The tree is not held: a project
    /// that closes takes its composed configs with it.
    roster: parking_lot::Mutex<Option<RosterOf>>,
}

/// A composed tree, by where it is, and the spawnable sections listed from it.
type RosterOf = (std::sync::Weak<Ltx>, Arc<[crate::SpawnSection]>);

impl ComposedConfigs {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    /// Registers the one source: the area that composes a module does, once, as the editor
    /// starts.
    pub fn provide(&self, source: ComposedSource) {
        *self.source.write() = Some(source);
    }

    /// The composed tree of the open project, when its module changes the game's configs.
    pub async fn get(editor: &Editor) -> Option<Arc<Ltx>> {
        let source = Self::of(editor).source.read().clone()?;
        source(editor).await
    }

    /// The spawnable sections of a composed tree, listed once per tree.
    #[must_use]
    pub fn roster_of(&self, tree: &Arc<Ltx>) -> Arc<[crate::SpawnSection]> {
        // a tree that is gone and another made at its place are told apart: the one held must
        // still be alive to be this one
        if let Some((of, roster)) = &*self.roster.lock()
            && of.upgrade().is_some_and(|live| Arc::ptr_eq(&live, tree))
        {
            return roster.clone();
        }
        // listed outside the lock: nobody waits behind a walk of every section
        let roster: Arc<[crate::SpawnSection]> = Arc::from(crate::spawn::sections(tree));
        *self.roster.lock() = Some((Arc::downgrade(tree), roster.clone()));
        roster
    }
}
