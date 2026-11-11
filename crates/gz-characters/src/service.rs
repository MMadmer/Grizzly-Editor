//! What the characters area keeps while a project is open: the project's characters as one
//! resolved [`Roster`], computed once per state of their files, and what validation reads of
//! the linked game, computed once per game revision.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use gz_base::{RelPath, Rev};
use gz_editor::{CmdError, Editor, Executors};
use gz_game::GameLink;
use gz_mod::ManifestService;
use gz_nq::{CharacterBrief, CharacterLookup, NqService, Pickers};
use gz_project::{AssetKind, EntryKind, FileTree, Project};
use gz_query::Query;
use gz_vfs::GameFiles;
use parking_lot::{Mutex, RwLock};
use rustc_hash::FxHashMap;

use crate::game::{TRADE_PRESETS, ogf_bone_names, stalker_damage_bones, trade_categories};
use crate::model::{Character, id_from_name, load_text, read_file, text_in};
use crate::resolve::resolve;
use crate::validate::{Cx, Lookups, Models, validate};

/// Every character of the project, read, sorted by id and resolved (`LoadResolved`).
#[derive(Clone, Debug, Default)]
pub struct Roster {
    /// The state of the files it was read from.
    pub stamp: Rev,
    /// As the files say, sorted by id: what an id is looked up in, and what the tab shows a
    /// character inherits from.
    pub raw: Vec<Character>,
    /// The same characters with their base chains flattened: what validation, the build and the
    /// answers describe.
    pub resolved: Vec<Character>,
    /// The files that did not read (`E100`), ids taken twice (`E101`), broken chains (`E111`,
    /// `E112`).
    pub problems: Vec<String>,
    /// What a quest names when it spawns a character and what the pickers offer.
    pub briefs: Arc<[CharacterBrief]>,
}

impl Roster {
    /// The roster of these files: `(file, text or the read's error)`, in path order
    /// (`LoadAll` + `Resolve`).
    #[must_use]
    pub fn build(stamp: Rev, files: Vec<(String, Result<String, String>)>) -> Self {
        let mut problems = Vec::new();
        let mut raw: Vec<Character> = Vec::with_capacity(files.len());
        for (file, text) in files {
            match text.and_then(|t| load_text(&t, &file)) {
                Ok(c) => raw.push(c),
                Err(e) => problems.push(format!("E100 {file}: {e}")),
            }
        }
        // a stable order: the list reads the same every time
        raw.sort_by(|a, b| a.id.cmp(&b.id));
        for pair in raw.windows(2) {
            if let [a, b] = pair
                && a.id == b.id
            {
                problems
                    .push(format!("E101 two characters share the id '{}': {} and {}", a.id, a.file, b.file));
            }
        }
        let mut resolved = raw.clone();
        resolve(&mut resolved, &mut problems);
        let briefs: Vec<CharacterBrief> = resolved
            .iter()
            .map(|c| CharacterBrief {
                id: c.id.clone(),
                template: c.is_abstract,
                name: text_in(&c.name, "rus"),
                file: c.file.clone(),
            })
            .collect();
        Self { stamp, raw, resolved, problems, briefs: Arc::from(briefs) }
    }

    /// The resolved character of an id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Character> {
        self.resolved.iter().find(|c| c.id == id)
    }

    /// The character of an id as its file says.
    #[must_use]
    pub fn get_raw(&self, id: &str) -> Option<&Character> {
        self.raw.iter().find(|c| c.id == id)
    }
}

// ---- the linked game -----------------------------------------------------------------------

/// The bones of the models read so far, by lower-case path.
type BoneCache = Arc<Mutex<FxHashMap<String, Arc<[String]>>>>;

/// The linked game's models with their bones read once per model.
#[derive(Clone)]
struct GameModels {
    files: Arc<GameFiles>,
    damage: Arc<Vec<String>>,
    cache: BoneCache,
}

impl Models for GameModels {
    fn exists(&self, rel: &str) -> bool {
        self.files.exists(rel)
    }

    fn bones(&self, rel: &str) -> Arc<[String]> {
        let key = rel.to_ascii_lowercase().replace('/', "\\");
        if let Some(b) = self.cache.lock().get(&key) {
            return b.clone();
        }
        let bones: Arc<[String]> = self
            .files
            .read(rel)
            .ok()
            .and_then(|data| ogf_bone_names(&data))
            .map_or_else(|| Arc::from(Vec::new()), Arc::from);
        self.cache.lock().insert(key, bones.clone());
        bones
    }

    fn damage_bones(&self) -> &[String] {
        &self.damage
    }
}

/// What validation reads of the linked game, gathered once per call: the pickers, the trade
/// categories, the models. Blocking to use (a model's bones are read on first need): validate
/// on the IO pool.
#[derive(Clone, Default)]
pub struct Checker {
    /// A game is linked and mounted.
    pub linked: bool,
    pub pickers: Option<Arc<Pickers>>,
    pub trade_categories: Arc<Vec<String>>,
    models: Option<GameModels>,
}

impl Checker {
    /// The checks that need no game (the build's).
    #[must_use]
    pub fn unlinked() -> Self {
        Self::default()
    }

    /// The problems of a resolved character, appended.
    pub fn validate(&self, c: &Character, problems: &mut Vec<String>) {
        let cx = Cx {
            linked: self.linked,
            pickers: self.pickers.as_deref().map(|p| p as &dyn Lookups),
            trade_categories: &self.trade_categories,
            models: self.models.as_ref().map(|m| m as &dyn Models),
        };
        validate(c, &cx, problems);
    }

    /// The problems of many characters, in their order, checked in parallel on the caller's
    /// pool.
    #[must_use]
    pub fn validate_all(&self, all: &[Character]) -> Vec<String> {
        use rayon::prelude::*;
        let per: Vec<Vec<String>> = all
            .par_iter()
            .map(|c| {
                let mut p = Vec::new();
                self.validate(c, &mut p);
                p
            })
            .collect();
        per.into_iter().flatten().collect()
    }
}

// ---- the service ---------------------------------------------------------------------------

/// A seed of the FNV-1a hash the stamps are made with.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Marks the briefs made from the file tree alone, before the first roster.
const FROM_TREE: Rev = Rev(0x7472_6565);

/// The characters of the open project (`session.services`). Made on first use, which a project
/// open asks for (it fills `gz_nq`'s `CharacterLookup`).
pub struct Characters {
    exec: Executors,
    root: String,
    project: Arc<Project>,
    manifest: Arc<ManifestService>,
    roster: Query<(), Roster>,
    /// The stamp of the files for a state of the tree and the manifest.
    memo: Mutex<Option<(Rev, Rev)>>,
    /// The briefs of the newest roster, for the readers that cannot wait.
    latest: RwLock<Option<(Rev, Arc<[CharacterBrief]>)>>,
    refreshing: AtomicBool,
    presets: Query<(), Vec<String>>,
    damage: Query<(), Vec<String>>,
    bones: Mutex<(Rev, BoneCache)>,
}

/// What `gz_nq` reads of the characters.
struct Lookup(Arc<Characters>);

impl CharacterLookup for Lookup {
    fn rev(&self) -> Rev {
        self.0.lookup_rev()
    }

    fn briefs(&self) -> Arc<[CharacterBrief]> {
        self.0.lookup_briefs()
    }
}

impl Characters {
    /// The characters of the open project, made on first use; `no active project` without one.
    pub async fn of(editor: &Editor) -> Result<Arc<Self>, CmdError> {
        let (session, project) = Project::of(editor)?;
        if let Some(found) = session.services.get::<Self>() {
            return Ok(found);
        }
        let manifest = ManifestService::of_session(editor, &session).await?;
        let mut made = false;
        let svc = session.services.get_or_init(|| {
            made = true;
            Self {
                exec: editor.exec.clone(),
                root: project.root().to_owned(),
                project,
                manifest,
                roster: Query::new(editor.exec.clone()),
                memo: Mutex::new(None),
                latest: RwLock::new(None),
                refreshing: AtomicBool::new(false),
                presets: Query::new(editor.exec.clone()),
                damage: Query::new(editor.exec.clone()),
                bones: Mutex::new((Rev::ZERO, Arc::new(Mutex::new(FxHashMap::default())))),
            }
        });
        if made && let Ok(nq) = NqService::of(editor) {
            nq.set_characters(Arc::new(Lookup(svc.clone())));
        }
        Ok(svc)
    }

    /// The project folder, as it was opened.
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }

    #[must_use]
    pub fn project(&self) -> &Arc<Project> {
        &self.project
    }

    #[must_use]
    pub fn manifest(&self) -> &Arc<ManifestService> {
        &self.manifest
    }

    /// `[exclude]` as the manifest has it now.
    fn rules(&self) -> Vec<String> {
        self.manifest.snapshot().manifest().map(|m| m.exclude.clone()).unwrap_or_default()
    }

    /// The `.npcasset` files of the build, in path order (`List`): a character in an excluded
    /// folder, the editor's scratch or tooling is no character of the module.
    fn files(tree: &FileTree, rules: &[String]) -> Vec<RelPath> {
        tree.files_with_extension("npcasset")
            .into_iter()
            .filter(|r| !gz_mod::outside_build(rules, r.as_str()))
            .collect()
    }

    fn stamp_of(tree: &FileTree, files: &[RelPath], rules: &[String]) -> Rev {
        let mut h = FNV_OFFSET;
        for rule in rules {
            h = fnv(h, rule.as_bytes());
            h = fnv(h, b"\n");
        }
        for rel in files {
            h = fnv(h, rel.key().as_bytes());
            if let Some(EntryKind::File { size, modified, .. }) = tree.entry(rel).map(|e| e.kind) {
                h = fnv(h, &size.to_le_bytes());
                let t = modified
                    .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                h = fnv(h, &t.to_le_bytes());
            }
        }
        Rev(h)
    }

    /// The stamp of the files now, worked out once per state of the tree and the manifest.
    fn current_stamp(&self) -> Rev {
        let tree = self.project.tree.snapshot();
        let cheap = tree.rev().mix(self.manifest.rev());
        if let Some((key, stamp)) = *self.memo.lock()
            && key == cheap
        {
            return stamp;
        }
        let rules = self.rules();
        let stamp = Self::stamp_of(&tree, &Self::files(&tree, &rules), &rules);
        *self.memo.lock() = Some((cheap, stamp));
        stamp
    }

    /// The roster of the files as the tree has them now: every file read in parallel on the IO
    /// pool, parsed and resolved once per state of the files.
    pub async fn roster(&self) -> Arc<Roster> {
        let tree = self.project.tree.snapshot();
        let rules = self.rules();
        let files = Self::files(&tree, &rules);
        let stamp = Self::stamp_of(&tree, &files, &rules);
        if let Some(r) = self.roster.peek_at(&(), stamp) {
            return r;
        }
        let root = PathBuf::from(&self.root);
        let exec = self.exec.clone();
        let read = self
            .exec
            .io(move || exec.par_io(files, |rel| (rel.as_str().to_owned(), read_file(&rel.to_fs(&root)))))
            .await;
        let roster = self.roster.fresh(&(), stamp, move || Roster::build(stamp, read)).await;
        *self.latest.write() = Some((roster.stamp, roster.briefs.clone()));
        roster
    }

    /// The roster once the file tree holds every change marked so far: what a command reads
    /// after it wrote or deleted a file.
    pub async fn fresh_roster(&self) -> Arc<Roster> {
        self.project.tree.refresh().await;
        self.roster().await
    }

    /// The briefs of the project's characters (`EditorCharacters::Briefs`).
    pub async fn briefs(&self) -> Arc<[CharacterBrief]> {
        self.roster().await.briefs.clone()
    }

    /// A refresh in the background, one at a time.
    fn kick(self: &Arc<Self>) {
        if self.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = self.clone();
        self.exec.spawn(async move {
            me.roster().await;
            me.refreshing.store(false, Ordering::Release);
        });
    }

    fn lookup_rev(self: &Arc<Self>) -> Rev {
        let stamp = self.current_stamp();
        let latest = self.latest.read().as_ref().map(|(rev, _)| *rev);
        match latest {
            Some(rev) if rev == stamp => rev,
            Some(rev) => {
                self.kick();
                rev
            }
            None => {
                self.kick();
                stamp.mix(FROM_TREE)
            }
        }
    }

    fn lookup_briefs(&self) -> Arc<[CharacterBrief]> {
        if let Some((_, briefs)) = self.latest.read().as_ref() {
            return briefs.clone();
        }
        // until the first roster: what the file tree says, as gz-nq's own fallback
        let tree = self.project.tree.snapshot();
        let rules = self.rules();
        let briefs: Vec<CharacterBrief> = Self::files(&tree, &rules)
            .into_iter()
            .map(|rel| {
                let template = tree.entry(&rel).is_some_and(|e| {
                    matches!(e.kind, EntryKind::File { asset: Some(AssetKind::CharacterTemplate), .. })
                });
                CharacterBrief {
                    id: id_from_name(rel.stem()),
                    template,
                    name: String::new(),
                    file: rel.as_str().to_owned(),
                }
            })
            .collect();
        Arc::from(briefs)
    }

    // ---- what validation reads ------------------------------------------------------------------

    /// What validation reads of the linked game now; [`Checker::unlinked`] without one
    /// (`EditorGameContent::Mounted`).
    pub async fn checker(&self, editor: &Editor) -> Checker {
        let Ok(link) = GameLink::of(editor) else { return Checker::unlinked() };
        let Ok((files, rev)) = link.snapshot().await else { return Checker::unlinked() };
        let pickers = match NqService::of(editor) {
            Ok(nq) => Some(nq.pickers(editor).await),
            Err(_) => None,
        };
        let trade = match self.presets.peek_at(&(), rev) {
            Some(t) => t,
            None => {
                let f = files.clone();
                let text = self
                    .exec
                    .io(move || {
                        f.read(TRADE_PRESETS).ok().map(|b| gz_base::text::cp1251_to_utf8(&b).into_owned())
                    })
                    .await;
                self.presets
                    .fresh(&(), rev, move || text.map(|t| trade_categories(&t)).unwrap_or_default())
                    .await
            }
        };
        let damage = match self.damage.peek_at(&(), rev) {
            Some(d) => d,
            None => {
                let ltx = link.configs().await.ok();
                self.damage
                    .fresh(&(), rev, move || ltx.map(|l| stalker_damage_bones(&l)).unwrap_or_default())
                    .await
            }
        };
        let cache = {
            let mut b = self.bones.lock();
            if b.0 != rev {
                *b = (rev, Arc::new(Mutex::new(FxHashMap::default())));
            }
            b.1.clone()
        };
        Checker {
            linked: true,
            pickers,
            trade_categories: trade,
            models: Some(GameModels { files, damage, cache }),
        }
    }

    /// The absolute path of a project-relative file of the roster.
    #[must_use]
    pub fn abs(&self, file: &str) -> PathBuf {
        let p = Path::new(file);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            PathBuf::from(&self.root).join(file.replace('/', "\\"))
        }
    }
}
