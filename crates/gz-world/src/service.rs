//! What the world area keeps while a project is open: the linked game's world, read once per
//! game revision, and the readers of the Player Starts and the problems.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gz_base::{RelPath, Rev};
use gz_editor::{CmdError, Editor, Executors};
use gz_game::GameLink;
use gz_mod::ManifestService;
use gz_nq::{NqService, PickerType, Transition, WorldLookup};
use gz_project::Project;
use gz_query::Query;
use parking_lot::RwLock;
use rustc_hash::FxHashMap;

use crate::game::{GameWorld, read_all_spawn};
use crate::model::{StartKit, World};
use crate::starts::{Start, read_starts};
use crate::validate::validate;

/// The game file the world is read from.
pub const ALL_SPAWN: &str = "spawns\\all.spawn";

type SceneStartsFn = Arc<dyn Fn(&Editor) -> Option<(String, Vec<Start>)> + Send + Sync>;

/// The slot the scene area fills: the open scene's level and the Player Starts placed in it,
/// which stand in for that level's baked file (the bake mirrors the scene into it). Until it is
/// filled, only what the module's baked files hold counts. Lives in `editor.services`.
#[derive(Default)]
pub struct SceneStarts {
    hook: RwLock<Option<SceneStartsFn>>,
}

impl SceneStarts {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    /// `f` answers `(level, starts)` of the open scene, `None` without one.
    pub fn provide(&self, f: impl Fn(&Editor) -> Option<(String, Vec<Start>)> + Send + Sync + 'static) {
        *self.hook.write() = Some(Arc::new(f));
    }

    #[must_use]
    pub fn get(&self, editor: &Editor) -> Option<(String, Vec<Start>)> {
        let hook = self.hook.read().clone()?;
        hook(editor)
    }
}

/// The world area of the open project (`session.services`). Made on first use, which a project
/// open asks for (it fills `gz_nq`'s `WorldLookup`).
pub struct WorldService {
    exec: Executors,
    root: String,
    project: Arc<Project>,
    link: Arc<GameLink>,
    manifest: Arc<ManifestService>,
    game: Query<(), GameWorld>,
    /// The newest game world, for the readers that cannot wait.
    latest: RwLock<Option<(Rev, Arc<GameWorld>)>>,
    refreshing: AtomicBool,
}

/// What `gz_nq` reads of the world: the level changers and the patrols (`EditorWorld::
/// Transitions`, `GamePatrols`).
struct Lookup(Arc<WorldService>);

impl WorldLookup for Lookup {
    fn rev(&self) -> Rev {
        self.0.lookup_rev()
    }

    fn transitions(&self) -> Vec<Transition> {
        self.0
            .latest()
            .map(|g| {
                g.transitions
                    .iter()
                    .map(|t| Transition {
                        name: t.name.clone(),
                        level: t.level.clone(),
                        to_level: t.to_level.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn patrols(&self) -> Vec<String> {
        self.0.latest().map(|g| g.patrols.clone()).unwrap_or_default()
    }

    fn ready(&self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
        let svc = self.0.clone();
        Box::pin(async move {
            svc.game().await;
        })
    }
}

impl WorldService {
    /// The world area of the open project, made on first use; `no active project` without one.
    pub async fn of(editor: &Editor) -> Result<Arc<Self>, CmdError> {
        let (session, project) = Project::of(editor)?;
        if let Some(found) = session.services.get::<Self>() {
            return Ok(found);
        }
        let manifest = ManifestService::of_session(editor, &session).await?;
        let link = GameLink::of(editor).map_err(|e| CmdError::internal(e.to_string()))?;
        let mut made = false;
        let svc = session.services.get_or_init(|| {
            made = true;
            Self {
                exec: editor.exec.clone(),
                root: project.root().to_owned(),
                project,
                link,
                manifest,
                game: Query::new(editor.exec.clone()),
                latest: RwLock::new(None),
                refreshing: AtomicBool::new(false),
            }
        });
        if made && let Ok(nq) = NqService::of(editor) {
            nq.set_world(Arc::new(Lookup(svc.clone())));
        }
        Ok(svc)
    }

    #[must_use]
    pub fn manifest(&self) -> &Arc<ManifestService> {
        &self.manifest
    }

    #[must_use]
    pub fn project(&self) -> &Arc<Project> {
        &self.project
    }

    /// The linked game's world, read once per game revision: `all.spawn` read on the IO pool,
    /// parsed on the CPU pool (`EnsureGame`).
    pub async fn game(&self) -> Arc<GameWorld> {
        let rev = self.project.game_rev();
        if let Some(g) = self.game.peek_at(&(), rev) {
            return g;
        }
        let world = self.read_game(rev).await;
        *self.latest.write() = Some((world.0, world.1.clone()));
        world.1
    }

    async fn read_game(&self, rev: Rev) -> (Rev, Arc<GameWorld>) {
        if self.project.game_root().is_empty() {
            return (rev, self.game.fresh(&(), rev, || GameWorld::failed("no game is linked")).await);
        }
        let (files, rev) = match self.link.snapshot().await {
            Ok(s) => s,
            Err(e) => {
                let text = e.to_string();
                return (rev, self.game.fresh(&(), rev, move || GameWorld::failed(text)).await);
            }
        };
        if let Some(g) = self.game.peek_at(&(), rev) {
            return (rev, g);
        }
        if !files.exists(ALL_SPAWN) {
            return (
                rev,
                self.game
                    .fresh(&(), rev, || GameWorld::failed("the linked game has no spawns\\all.spawn"))
                    .await,
            );
        }
        let bytes = self.exec.io(move || files.read(ALL_SPAWN)).await;
        let Ok(bytes) = bytes else {
            return (
                rev,
                self.game.fresh(&(), rev, || GameWorld::failed("spawns\\all.spawn could not be read")).await,
            );
        };
        let configs = self.link.configs().await.ok();
        let world =
            self.game.fresh(&(), rev, move || read_all_spawn(Arc::from(bytes), configs.as_deref())).await;
        (rev, world)
    }

    /// The newest game world read, without waiting.
    #[must_use]
    pub fn latest(&self) -> Option<Arc<GameWorld>> {
        self.latest.read().as_ref().map(|(_, g)| g.clone())
    }

    /// A read in the background, one at a time.
    fn kick(self: &Arc<Self>) {
        if self.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = self.clone();
        self.exec.spawn(async move {
            me.game().await;
            me.refreshing.store(false, Ordering::Release);
        });
    }

    fn lookup_rev(self: &Arc<Self>) -> Rev {
        let now = self.project.game_rev();
        let latest = self.latest.read().as_ref().map(|(rev, _)| *rev);
        match latest {
            Some(rev) if rev == now => rev,
            other => {
                self.kick();
                other.unwrap_or(Rev::ZERO)
            }
        }
    }

    /// What the player reads of a level, from the game's `game_levels.ltx` through its string
    /// table; a level with none is missing here.
    pub async fn captions(&self, editor: &Editor) -> FxHashMap<String, String> {
        let Ok(nq) = NqService::of(editor) else { return FxHashMap::default() };
        let pickers = nq.pickers(editor).await;
        pickers
            .entries(PickerType::Level)
            .iter()
            .filter(|e| !e.name.is_empty())
            .map(|e| (e.id.clone(), e.name.clone()))
            .collect()
    }

    /// Every Player Start (`Starts`): the spawn layers' `op = start` entries, with the open
    /// scene's own starts standing in for its level's file.
    pub async fn starts(&self, editor: &Editor) -> Vec<Start> {
        let (scene_level, scene) = SceneStarts::of(editor).get(editor).unwrap_or_default();
        let tree = self.project.tree.snapshot();
        let files: Vec<RelPath> = match RelPath::new("spawn") {
            Ok(spawn) => tree
                .files_under(&spawn)
                .into_iter()
                .map(|(rel, _)| rel)
                .filter(|rel| rel.has_extension("xspawn"))
                .collect(),
            Err(_) => Vec::new(),
        };
        let root = PathBuf::from(&self.root);
        let exec = self.exec.clone();
        let found: Vec<Vec<Start>> = self
            .exec
            .io(move || {
                exec.par_io(files, |rel| {
                    std::fs::read(rel.to_fs(&root))
                        .map(|b| read_starts(&gz_base::text::decode_guess(&b), rel.as_str()))
                        .unwrap_or_default()
                })
            })
            .await;
        let mut out: Vec<Start> = found
            .into_iter()
            .flatten()
            .filter(|s| scene_level.is_empty() || !s.level.eq_ignore_ascii_case(&scene_level))
            .collect();
        out.extend(scene);
        out
    }

    /// The problems of a world and a kit in the open project (`Validate`), with the starts they
    /// were found among.
    pub async fn problems(&self, editor: &Editor, w: &World, k: &StartKit) -> (Vec<Start>, Vec<String>) {
        let starts = self.starts(editor).await;
        let game = self.game().await;
        // the item lookup is the pickers', and only a kit with items asks it
        let pickers = if k.declared && !k.items.is_empty() {
            match NqService::of(editor) {
                Ok(nq) => Some(nq.pickers(editor).await).filter(|p| p.available()),
                Err(_) => None,
            }
        } else {
            None
        };
        // an item the module brings itself is known too
        let own = if pickers.is_some() { Some(gz_mod::own_sections(editor).await) } else { None };
        let known = pickers.as_ref().map(|p| {
            move |id: &str| p.known(PickerType::Item, id) || own.as_ref().is_some_and(|o| o.has(id))
        });
        let problems =
            validate(w, k, &starts, Some(&game), known.as_ref().map(|f| f as &dyn Fn(&str) -> bool));
        (starts, problems)
    }
}
