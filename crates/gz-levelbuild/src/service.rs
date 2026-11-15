//! What the level build keeps while a project is open: the record of the level's builds and what
//! it says about the scene there is now, the problems of the last build, the graphs it made, the
//! process that runs.
//!
//! The status is a [`Query`] keyed on the scene's revision and the record's: the status bar and
//! `level.status` read it, nothing polls it, and it is computed on the CPU pool once per change.

use std::path::PathBuf;

use std::sync::{Arc, OnceLock};

use gz_base::{Rev, RevCounter, Severity};
use gz_editor::{Editor, JobId, ProjectSession};
use gz_fmt_level::LevelGraph;
use gz_query::{Answer, Query};
use gz_scene::{SceneService, SceneState};
use parking_lot::Mutex;
use serde::Serialize;

use crate::fingerprint::{self, HashCache, Objects};
use crate::graph::LevelGameGraph;
use crate::state::BuildState;
use crate::steps::{Options, Quality, Step};

/// What holds the `level_run` job: the level compiler in the editor's process, or the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Running {
    Compile,
    Game,
}

impl Running {
    /// The name `level.status` answers in `running`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Compile => "compiler",
            Self::Game => "game",
        }
    }
}

/// Where a step stands for the scene there is now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    /// Built from what the scene holds now.
    Fresh,
    /// Built, but from something that has changed since.
    Stale,
    /// Never built for this level (or its record is gone).
    Never,
    /// Its last run failed.
    Failed,
    /// The scene has nothing this step makes (no detail objects), or the level is one of the
    /// linked game's and the step compiles it (the module adds to the game's level).
    NotNeeded,
}

/// One step's state, with why it is not fresh.
#[derive(Clone, Debug, Serialize)]
pub struct StepStatus {
    pub step: Step,
    pub title: &'static str,
    pub state: StepState,
    /// What changed, why it failed, or what it lacks; empty when fresh.
    pub why: String,
    /// When it last ran, local time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<Quality>,
}

/// The level build's state for one scene version.
#[derive(Clone, Debug, Serialize)]
pub struct Status {
    /// The level the scene builds (Level Settings' level path).
    pub level: String,
    pub steps: Vec<StepStatus>,
    /// The level is one of the linked game's: the module adds to it, nothing is compiled.
    pub stock: bool,
    /// On a stock level: the compile steps an earlier build ran there, whose files replace the
    /// game's own in every game that mounts the module (`level.clean` removes them).
    pub built_over: Vec<Step>,
}

impl Status {
    /// The steps a build without arguments would run.
    #[must_use]
    pub fn to_build(&self) -> Vec<Step> {
        self.steps
            .iter()
            .filter(|s| matches!(s.state, StepState::Stale | StepState::Never | StepState::Failed))
            .map(|s| s.step)
            .collect()
    }

    /// The status bar's words: the steps to build, by name.
    #[must_use]
    pub fn text(&self) -> String {
        self.to_build().iter().map(|s| s.title()).collect::<Vec<_>>().join(" \u{b7} ")
    }

    #[must_use]
    pub fn get(&self, step: Step) -> Option<&StepStatus> {
        self.steps.iter().find(|s| s.step == step)
    }
}

/// A finding of a build: what is wrong, on which object, from which step.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LevelProblem {
    pub step: Step,
    pub code: &'static str,
    pub severity: Severity,
    pub text: String,
    /// The scene object it is about.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// Where in the level, in metres.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<[f32; 3]>,
}

/// The levels of the linked game, read for the game's revision, and whether it loads pieces of a
/// level.
#[derive(Default)]
struct GameLevels {
    rev: Option<Rev>,
    levels: Arc<Vec<String>>,
    pieces: bool,
    reading: bool,
}

/// The record of the level the scene builds, as read from its folder.
#[derive(Default)]
struct RecordSlot {
    level: String,
    state: Option<Arc<BuildState>>,
    loading: bool,
    /// Read for the level once (found or not).
    loaded: bool,
}

/// The graphs the last build made, by the fingerprint they were made for.
#[derive(Default)]
pub(crate) struct Graphs {
    pub level: Option<(u64, Arc<LevelGraph>)>,
    pub game: Option<(u64, Arc<LevelGameGraph>)>,
}

/// The level build of the open project.
#[derive(Default)]
pub struct LevelBuild {
    /// The compile or the game that runs, and its job.
    pub(crate) running: Mutex<Option<(Running, JobId)>>,
    record: Mutex<RecordSlot>,
    record_rev: RevCounter,
    pub(crate) hashes: HashCache,
    status: OnceLock<Query<(), Status>>,
    pub(crate) problems: Mutex<Vec<LevelProblem>>,
    pub(crate) graphs: Mutex<Graphs>,
    game_levels: Mutex<GameLevels>,
    /// A read of the game's levels ended: the status follows it.
    levels_rev: RevCounter,
}

/// The project's `gamedata\levels`: a level build lands in the project, never in the editor or
/// the game.
#[must_use]
pub fn levels_root(session: &ProjectSession) -> PathBuf {
    session.root.join("gamedata").join("levels")
}

/// The options a build without arguments uses: the level's own settings, no compiler flags.
#[must_use]
pub fn default_options(editor: &Editor) -> Options {
    let game = gz_project::Project::of(editor).map(|(_, p)| p.game_root()).unwrap_or_default();
    Options {
        quality: Quality::Standard,
        flags: String::new(),
        game,
        library: 0,
        stock: false,
        game_level: false,
        pieces: false,
    }
}

impl LevelBuild {
    #[must_use]
    pub fn of(session: &ProjectSession) -> Arc<Self> {
        session.services.get_or_init(Self::default)
    }

    /// The compile or the game that runs.
    #[must_use]
    pub fn running(&self) -> Option<Running> {
        self.running.lock().map(|(p, _)| p)
    }

    /// The levels of the linked game (its game graph, `gz-world`'s read of `all.spawn`) without
    /// waiting: `None` until the first read for the game's revision ended, which this starts.
    pub fn game_levels_now(&self, editor: &Arc<Editor>) -> Option<Arc<Vec<String>>> {
        let rev = gz_project::Project::of(editor).ok().map(|(_, p)| p.game_rev());
        let mut slot = self.game_levels.lock();
        if slot.rev.is_some() && slot.rev == rev {
            return Some(slot.levels.clone());
        }
        if !slot.reading {
            slot.reading = true;
            let (ed, session) = (editor.clone(), editor.project_opt());
            editor.exec.spawn(async move {
                let Some(session) = session else { return };
                let lb = Self::of(&session);
                let (levels, pieces) = read_game_levels(&ed).await;
                let mut slot = lb.game_levels.lock();
                *slot = GameLevels { rev, levels: Arc::new(levels), pieces, reading: false };
                drop(slot);
                lb.levels_rev.bump();
                ed.exec.wake().ping();
            });
        }
        // the levels of the game read before, while the new ones are read
        slot.rev.is_some().then(|| slot.levels.clone())
    }

    /// The levels of the linked game, read now when they have not been for its revision.
    pub async fn game_levels(&self, editor: &Arc<Editor>) -> Arc<Vec<String>> {
        let rev = gz_project::Project::of(editor).ok().map(|(_, p)| p.game_rev());
        {
            let slot = self.game_levels.lock();
            if slot.rev.is_some() && slot.rev == rev {
                return slot.levels.clone();
            }
        }
        let (levels, pieces) = read_game_levels(editor).await;
        let levels = Arc::new(levels);
        let mut slot = self.game_levels.lock();
        *slot = GameLevels { rev, levels: levels.clone(), pieces, reading: false };
        drop(slot);
        self.levels_rev.bump();
        levels
    }

    /// Whether the linked game loads pieces of a level, as read with its levels
    /// ([`Self::game_levels`]).
    pub async fn game_pieces(&self, editor: &Arc<Editor>) -> bool {
        let _ = self.game_levels(editor).await;
        self.game_levels.lock().pieces
    }

    /// [`Self::game_pieces`] as last read, asking for nothing.
    fn game_pieces_now(&self) -> bool {
        self.game_levels.lock().pieces
    }

    /// The record of `level` when it has been read; starts the read the first time a level is
    /// asked about. `None` while it is read.
    pub(crate) fn record(
        &self,
        editor: &Arc<Editor>,
        session: &ProjectSession,
        level: &str,
    ) -> Option<Option<Arc<BuildState>>> {
        let mut slot = self.record.lock();
        if slot.level != level {
            *slot = RecordSlot { level: level.to_owned(), ..RecordSlot::default() };
        }
        if slot.loaded {
            return Some(slot.state.clone());
        }
        if !slot.loading {
            slot.loading = true;
            let dir = crate::folder::level_folder(&levels_root(session), level).ok();
            let (ed, lb, lv) = (editor.clone(), Self::of(session), level.to_owned());
            editor.exec.spawn_io(move || {
                let state = dir.and_then(|d| BuildState::load(&d)).filter(|s| s.level == lv).map(Arc::new);
                let mut slot = lb.record.lock();
                if slot.level == lv {
                    slot.state = state;
                    slot.loading = false;
                    slot.loaded = true;
                }
                drop(slot);
                lb.record_rev.bump();
                ed.exec.wake().ping();
            });
        }
        None
    }

    /// The record of `level`, read now when it has not been.
    pub(crate) async fn record_now(
        &self,
        editor: &Arc<Editor>,
        session: &ProjectSession,
        level: &str,
    ) -> Option<Arc<BuildState>> {
        if let Some(r) = self.record(editor, session, level) {
            return r;
        }
        // the read `record` started may still be on its way: read here too and keep the first
        let dir = crate::folder::level_folder(&levels_root(session), level).ok();
        let lv = level.to_owned();
        let state = editor
            .exec
            .io(move || dir.and_then(|d| BuildState::load(&d)).filter(|s| s.level == lv).map(Arc::new))
            .await;
        let mut slot = self.record.lock();
        if slot.level != level {
            return state;
        }
        if !slot.loaded {
            slot.state = state;
            slot.loading = false;
            slot.loaded = true;
            let out = slot.state.clone();
            drop(slot);
            self.record_rev.bump();
            return out;
        }
        slot.state.clone()
    }

    /// A build wrote a new record.
    pub(crate) fn set_record(&self, state: Arc<BuildState>) {
        let mut slot = self.record.lock();
        *slot = RecordSlot { level: state.level.clone(), state: Some(state), loading: false, loaded: true };
        drop(slot);
        self.record_rev.bump();
    }

    /// The last status worked out, asking for nothing: what an action's state reads.
    #[must_use]
    pub fn status_last(editor: &Editor) -> Option<Arc<Status>> {
        let session = editor.project_opt()?;
        Self::of(&session).status.get()?.peek(&())
    }

    /// The status of the open scene without waiting: fresh, the last one while a new one is
    /// worked out, or nothing yet. What the status bar draws.
    #[must_use]
    pub fn status_now(editor: &Arc<Editor>) -> Option<Arc<Status>> {
        let session = editor.project_opt()?;
        let svc = session.services.get::<SceneService>()?;
        let snap = svc.store.snapshot();
        if !snap.valid || snap.objects.is_empty() {
            return None;
        }
        let lb = Self::of(&session);
        let level = snap.meta.level.level_path.to_string();
        let record = lb.record(editor, &session, &level)?;
        let levels = lb.game_levels_now(editor)?;
        let mut opts = default_options(editor);
        opts.stock = crate::stock::is_stock(&levels, &level);
        opts.game_level = opts.stock;
        opts.pieces = lb.game_pieces_now();
        let lib = svc.library.clone();
        let rev = svc
            .store
            .rev()
            .mix(lb.record_rev.get())
            .mix(lb.levels_rev.get())
            .mix(Rev(fingerprint::text_hash(&opts.game)))
            .mix(lib.rev());
        let query = lb.status.get_or_init(|| Query::new(editor.exec.clone()));
        let lb2 = lb.clone();
        let made = move || {
            let opts = Options { library: fingerprint::library_stamp(&snap, &lib), ..opts };
            status_of(&snap, &lb2.hashes, record.as_deref(), &opts)
        };
        match query.get(&(), rev, made) {
            Answer::Fresh(s) | Answer::Stale(s) => Some(s),
            Answer::Pending => None,
        }
    }

    /// The status of the open scene, computed now when it is not.
    pub async fn status(
        editor: &Arc<Editor>,
        session: &Arc<ProjectSession>,
        scene: Arc<gz_store::Snap<SceneState>>,
        rev: Rev,
    ) -> Arc<Status> {
        let lb = Self::of(session);
        let level = scene.meta.level.level_path.to_string();
        let record = lb.record_now(editor, session, &level).await;
        let levels = lb.game_levels(editor).await;
        let mut opts = default_options(editor);
        opts.stock = crate::stock::is_stock(&levels, &level);
        opts.game_level = opts.stock;
        opts.pieces = lb.game_pieces(editor).await;
        let lib = session.services.get::<SceneService>().map(|svc| svc.library.clone());
        let rev = rev
            .mix(lb.record_rev.get())
            .mix(lb.levels_rev.get())
            .mix(Rev(fingerprint::text_hash(&opts.game)))
            .mix(lib.as_ref().map_or(Rev::ZERO, |l| l.rev()));
        let query = lb.status.get_or_init(|| Query::new(editor.exec.clone()));
        let lb2 = lb.clone();
        query
            .fresh(&(), rev, move || {
                let library = lib.map_or(0, |l| fingerprint::library_stamp(&scene, &l));
                status_of(&scene, &lb2.hashes, record.as_deref(), &Options { library, ..opts })
            })
            .await
    }
}

/// The levels of the linked game's game graph, as `gz-world` reads its `all.spawn` (cached per
/// game revision there), none when no game is linked or it cannot be read; and whether the game
/// loads pieces of a level (its engine's version resources say `level-pieces`).
async fn read_game_levels(editor: &Arc<Editor>) -> (Vec<String>, bool) {
    let levels = match gz_world::WorldService::of(editor).await {
        Ok(w) => w.game().await.levels.clone(),
        Err(_) => Vec::new(),
    };
    let game = gz_project::Project::of(editor).map(|(_, p)| p.game_root()).unwrap_or_default();
    let pieces = !game.is_empty()
        && editor
            .exec
            .io(move || {
                crate::play::game_states(
                    std::path::Path::new(&game),
                    &crate::play::LINKED_GAME,
                    crate::play::PIECES,
                )
            })
            .await;
    (levels, pieces)
}

/// Whether the scene has what a step makes: detail objects for the details, a static object of
/// the module's own for the piece.
#[must_use]
pub fn needed(scene: &SceneState, step: Step) -> bool {
    match step {
        Step::Details | Step::DetailLight => {
            scene.bulk.details.as_ref().is_some_and(|d| !d.model.slots.is_empty())
        }
        Step::Piece => has_authored_static(scene),
        _ => true,
    }
}

/// Whether the scene places a static object of the module's own (marked authored).
#[must_use]
pub fn has_authored_static(scene: &SceneState) -> bool {
    scene.of_class(gz_scene::ObjClass::SceneObject).any(|o| {
        o.marks.authored
            && matches!(&o.data, gz_scene::ObjData::SceneObject(d) if !d.visual_mode() && !d.reference.is_empty())
    })
}

/// Why a step has nothing to make for this scene and these options, in a sentence; `None`: it has.
#[must_use]
pub fn not_needed_why(scene: &SceneState, step: Step, opts: &Options) -> Option<String> {
    match step {
        Step::Details | Step::DetailLight if !needed(scene, step) => {
            Some("The scene has no detail objects.".to_owned())
        }
        Step::Piece if !opts.game_level => Some(
            "The level is the module's own: it is built whole, and a piece is what a module adds to a level of the linked game."
                .to_owned(),
        ),
        Step::Piece if !needed(scene, step) => {
            Some("The scene has no static object of the module's own on the game's level.".to_owned())
        }
        Step::Piece if !opts.pieces => Some("The linked game does not load pieces of a level.".to_owned()),
        _ => None,
    }
}

/// Whether a step has something to make ([`not_needed_why`]).
#[must_use]
pub fn is_needed(scene: &SceneState, step: Step, opts: &Options) -> bool {
    not_needed_why(scene, step, opts).is_none()
}

/// Every step's state for one scene version and the record of the level.
#[must_use]
pub fn status_of(
    scene: &SceneState,
    hashes: &HashCache,
    record: Option<&BuildState>,
    opts: &Options,
) -> Status {
    let objects = hashes.objects(scene);
    status_with(scene, &objects, record, opts)
}

pub(crate) fn status_with(
    scene: &SceneState,
    objects: &Objects,
    record: Option<&BuildState>,
    opts: &Options,
) -> Status {
    let fps = fingerprint::steps(scene, objects, opts);
    let mut steps: Vec<StepStatus> = Vec::with_capacity(Step::ORDER.len());
    for step in Step::ORDER {
        let rec = record.and_then(|r| r.steps.get(&step));
        let mut st = StepStatus {
            step,
            title: step.title(),
            state: StepState::Fresh,
            why: String::new(),
            time: rec.map(|r| r.time.clone()),
            seconds: rec.map(|r| r.seconds),
            quality: rec.map(|r| r.quality),
        };
        if let Some(why) = not_needed_why(scene, step, opts) {
            st.state = StepState::NotNeeded;
            st.why = why;
            steps.push(st);
            continue;
        }
        if opts.stock && step.compiles() {
            st.state = StepState::NotNeeded;
            st.why = format!(
                "{} is a level of the linked game: the module adds to it, and the level is not compiled.",
                scene.meta.level.level_path
            );
            steps.push(st);
            continue;
        }
        match rec {
            None => {
                st.state = StepState::Never;
                st.why = "Never built.".to_owned();
            }
            Some(r) if r.failed.is_some() => {
                st.state = StepState::Failed;
                st.why = r.failed.clone().unwrap_or_default();
            }
            Some(r) if Some(&r.fingerprint) == fps.get(&step) => {}
            Some(r) => {
                st.state = StepState::Stale;
                st.why = why_stale(scene, objects, record, opts, step, r, &steps);
            }
        }
        steps.push(st);
    }
    let built_over: Vec<Step> = if opts.stock {
        record.map(|r| r.steps.keys().copied().filter(|s| s.compiles()).collect()).unwrap_or_default()
    } else {
        Vec::new()
    };
    Status { level: scene.meta.level.level_path.to_string(), steps, stock: opts.stock, built_over }
}

/// Why a stale step is stale, in a sentence.
fn why_stale(
    scene: &SceneState,
    objects: &Objects,
    record: Option<&BuildState>,
    opts: &Options,
    step: Step,
    rec: &crate::state::StepRecord,
    before: &[StepStatus],
) -> String {
    if let Some(n) =
        step.needs().iter().find_map(|n| before.iter().find(|s| s.step == *n && s.state != StepState::Fresh))
    {
        return format!("{} is not fresh.", n.title);
    }
    if (rec.quality != opts.quality || rec.flags != opts.flags)
        && matches!(step, Step::Geometry | Step::Light | Step::Piece)
    {
        let then = Options { quality: rec.quality, flags: rec.flags.clone(), ..opts.clone() };
        let fps = fingerprint::steps(scene, objects, &then);
        if Some(rec.fingerprint) == fps.get(&step).copied() {
            let mut how = Vec::new();
            if rec.quality != opts.quality {
                how.push(format!("at {} quality", capital(rec.quality.name())));
            }
            if rec.flags != opts.flags {
                how.push(format!("with the compiler's {}", rec.flags));
            }
            return format!("Built {}.", how.join(" and "));
        }
    }
    if rec.library != opts.library && step.classes().contains(&gz_scene::ObjClass::SceneObject) {
        let then = Options { library: rec.library, ..opts.clone() };
        if Some(rec.fingerprint) == fingerprint::steps(scene, objects, &then).get(&step).copied() {
            return "A model the level's objects are made from changed.".to_owned();
        }
    }
    let moved = record.map(|r| fingerprint::changes(step, &r.objects, objects)).unwrap_or_default();
    if !moved.is_empty() {
        return format!("{}.", capital(&moved));
    }
    match step {
        Step::AiMap => "The AI map or its settings changed.".to_owned(),
        Step::Details => "The detail objects changed.".to_owned(),
        Step::Geometry => "The level's build settings, lights or wallmarks changed.".to_owned(),
        Step::Game => "The level's settings changed.".to_owned(),
        Step::Spawn => "The linked game changed.".to_owned(),
        Step::Piece => "The level's build settings, lights or the linked game changed.".to_owned(),
        Step::Light => "The level's streaming cells changed.".to_owned(),
        _ => "What it is made of changed.".to_owned(),
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::state::StepRecord;
    use gz_scene::objects::{GlowData, LightData};
    use gz_scene::{ObjClass, ObjData, Object};

    fn scene() -> (SceneState, gz_scene::ObjId) {
        let mut s = SceneState::default();
        let lamp = s.insert(Object::new(ObjClass::Light, "lamp", ObjData::Light(LightData::default())));
        s.insert(Object::new(ObjClass::Glow, "glow", ObjData::Glow(GlowData::default())));
        (s, lamp)
    }

    /// The record of a build of everything `scene` holds now.
    fn built(s: &SceneState, cache: &HashCache, opts: &Options) -> BuildState {
        let objects = cache.objects(s);
        let fps = fingerprint::steps(s, &objects, opts);
        let mut r = BuildState::new("lvl");
        for step in Step::ORDER.into_iter().filter(|st| needed(s, *st)) {
            r.steps.insert(
                step,
                StepRecord {
                    fingerprint: fps[&step],
                    time: String::new(),
                    seconds: 0.0,
                    quality: opts.quality,
                    flags: opts.flags.clone(),
                    library: if step.classes().contains(&ObjClass::SceneObject) { opts.library } else { 0 },
                    failed: None,
                    files: Vec::new(),
                },
            );
        }
        r.objects = objects;
        r
    }

    #[test]
    fn a_moved_object_makes_stale_exactly_the_steps_that_read_it() {
        let (mut s, lamp) = scene();
        let (cache, opts) = (HashCache::default(), Options::default());
        let record = built(&s, &cache, &opts);
        let st = status_of(&s, &cache, Some(&record), &opts);
        assert!(st.to_build().is_empty(), "{:?}", st.to_build());
        assert_eq!(st.get(Step::Details).unwrap().state, StepState::NotNeeded);
        // the lamp moves: the geometry reads lights, the light reads the geometry, the spawn layer
        // reads everything; the game files, the AI map and the graphs read no light
        let mut o = (**s.get(lamp).unwrap()).clone();
        o.xform.position.x += 1.0;
        s.replace(o);
        let st = status_of(&s, &cache, Some(&record), &opts);
        assert_eq!(st.to_build(), [Step::SpawnLayer, Step::Geometry, Step::Light]);
        assert_eq!(st.get(Step::Geometry).unwrap().why, "1 object moved.");
        assert_eq!(st.get(Step::Light).unwrap().why, "Geometry is not fresh.");
        assert_eq!(st.get(Step::Game).unwrap().state, StepState::Fresh);
    }

    #[test]
    fn the_status_survives_the_record_being_written_and_read() {
        let (s, _) = scene();
        let (cache, opts) = (HashCache::default(), Options::default());
        let record = built(&s, &cache, &opts);
        let again = BuildState::read(&record.write()).unwrap();
        // a new session: no hash is cached, every one is worked out again from the content
        let fresh = HashCache::default();
        assert!(status_of(&s, &fresh, Some(&again), &opts).to_build().is_empty());
        // without a record everything is to build but the details and the piece (the scene has neither)
        assert_eq!(status_of(&s, &fresh, None, &opts).to_build().len(), Step::ORDER.len() - 3);
    }

    #[test]
    fn a_stop_keeps_what_ended_before_it() {
        // what the pipeline records when Stop comes after the quick steps: those steps' records,
        // and none for the step that was stopped
        let (s, _) = scene();
        let (cache, opts) = (HashCache::default(), Options::default());
        let mut record = built(&s, &cache, &opts);
        record.steps.remove(&Step::Light);
        record.steps.remove(&Step::DetailLight);
        let st = status_of(&s, &cache, Some(&record), &opts);
        assert_eq!(st.get(Step::Geometry).unwrap().state, StepState::Fresh);
        assert_eq!(st.get(Step::Light).unwrap().state, StepState::Never);
        assert_eq!(st.to_build(), [Step::Light]);
    }

    #[test]
    fn a_draft_build_says_so_and_a_failed_step_is_never_fresh() {
        let (s, _) = scene();
        let cache = HashCache::default();
        let draft = Options { quality: Quality::Draft, ..Options::default() };
        let mut record = built(&s, &cache, &draft);
        let st = status_of(&s, &cache, Some(&record), &Options::default());
        assert_eq!(st.get(Step::Geometry).unwrap().why, "Built at Draft quality.");
        let noise = Options { flags: "-noise".into(), ..Options::default() };
        let record2 = built(&s, &cache, &noise);
        let st = status_of(&s, &cache, Some(&record2), &Options::default());
        assert_eq!(st.get(Step::Light).unwrap().why, "Built with the compiler's -noise.");
        assert_eq!(st.get(Step::Geometry).unwrap().state, StepState::Fresh);
        assert_eq!(st.get(Step::Game).unwrap().state, StepState::Fresh);
        record.steps.get_mut(&Step::Game).unwrap().failed = Some("The scene has no spawn point.".into());
        let st = status_of(&s, &cache, Some(&record), &draft);
        assert_eq!(st.get(Step::Game).unwrap().state, StepState::Failed);
        assert_eq!(st.get(Step::Game).unwrap().why, "The scene has no spawn point.");
    }

    #[test]
    fn a_stock_level_needs_no_compile_and_names_what_a_build_left() {
        let (s, _) = scene();
        let cache = HashCache::default();
        let stock = Options { stock: true, ..Options::default() };
        // never built: the module needs its spawn layer and nothing else
        let st = status_of(&s, &cache, None, &stock);
        assert_eq!(st.to_build(), [Step::SpawnLayer]);
        assert!(st.stock && st.built_over.is_empty());
        let light = st.get(Step::Light).unwrap();
        assert_eq!(light.state, StepState::NotNeeded);
        assert!(light.why.contains("is a level of the linked game"), "{}", light.why);
        assert_eq!(st.text(), "Spawn Layer");
        // an earlier build compiled it here: still not needed, and the steps it ran are named
        let mut record = built(&s, &cache, &Options::default());
        record.steps.remove(&Step::SpawnLayer);
        let st = status_of(&s, &cache, Some(&record), &stock);
        assert_eq!(st.to_build(), [Step::SpawnLayer]);
        assert!(st.built_over.contains(&Step::Geometry) && !st.built_over.contains(&Step::SpawnLayer));
        // the same level built as a level of the module's own: every step by its record
        assert!(status_of(&s, &cache, Some(&record), &Options::default()).built_over.is_empty());
    }

    /// A scene object of the module's own (marked authored) or of the game's level.
    fn house(s: &mut SceneState, name: &str, authored: bool) -> gz_scene::ObjId {
        let data =
            gz_scene::objects::SceneObjectData { reference: "buildings\\house".into(), ..Default::default() };
        let mut o = Object::new(ObjClass::SceneObject, name, ObjData::SceneObject(data));
        o.marks.authored = authored;
        s.insert(o)
    }

    #[test]
    fn the_piece_is_built_on_a_game_level_with_objects_of_the_modules_own() {
        let (mut s, _) = scene();
        let cache = HashCache::default();
        let game = Options { stock: true, game_level: true, pieces: true, ..Options::default() };
        // the game's level without an object of the module's own: nothing to make
        let base = house(&mut s, "base_house", false);
        let st = status_of(&s, &cache, None, &game);
        let piece = st.get(Step::Piece).unwrap();
        assert_eq!(piece.state, StepState::NotNeeded);
        assert!(piece.why.contains("no static object of the module's own"), "{}", piece.why);
        assert_eq!(st.to_build(), [Step::SpawnLayer]);
        // one authored house: the piece joins the spawn layer, the compile steps stay not needed
        let mine = house(&mut s, "my_house", true);
        let st = status_of(&s, &cache, None, &game);
        assert_eq!(st.to_build(), [Step::SpawnLayer, Step::Piece]);
        assert_eq!(st.get(Step::Light).unwrap().state, StepState::NotNeeded);
        assert_eq!(st.get(Step::Piece).unwrap().state, StepState::Never);
        // a level of the module's own never has one, nor a game that does not load pieces
        let own = Options { pieces: true, ..Options::default() };
        let piece = status_of(&s, &cache, None, &own).get(Step::Piece).cloned().unwrap();
        assert!(piece.state == StepState::NotNeeded && piece.why.contains("module's own"), "{}", piece.why);
        let old = Options { pieces: false, ..game.clone() };
        let piece = status_of(&s, &cache, None, &old).get(Step::Piece).cloned().unwrap();
        assert!(
            piece.state == StepState::NotNeeded && piece.why.contains("does not load pieces"),
            "{}",
            piece.why
        );
        // built: fresh; what it is made of moving - the authored house, the game's house, a lamp -
        // makes it stale, and only the steps that read that
        let record = built(&s, &cache, &game);
        assert!(status_of(&s, &cache, Some(&record), &game).to_build().is_empty());
        for id in [mine, base] {
            let mut t = s.clone();
            let mut o = (**t.get(id).unwrap()).clone();
            o.xform.position.x += 1.0;
            t.replace(o);
            let st = status_of(&t, &cache, Some(&record), &game);
            assert_eq!(st.to_build(), [Step::SpawnLayer, Step::Piece], "{}", st.text());
            assert_eq!(st.get(Step::Piece).unwrap().why, "1 object moved.");
        }
        // the build settings, the quality and the linked game are part of it
        let mut t = s.clone();
        t.meta.level.build.lm_pixels_per_meter = 3.0;
        assert_eq!(status_of(&t, &cache, Some(&record), &game).to_build(), [Step::Piece]);
        let draft = Options { quality: Quality::Draft, ..game.clone() };
        let st = status_of(&s, &cache, Some(&record), &draft);
        assert_eq!(st.get(Step::Piece).unwrap().why, "Built at Standard quality.");
        let other = Options { game: "D:\\Other Game".into(), ..game.clone() };
        assert_eq!(status_of(&s, &cache, Some(&record), &other).to_build(), [Step::Piece]);
    }

    #[test]
    fn the_cells_are_part_of_the_light_and_none_change_nothing() {
        let (mut s, _) = scene();
        let cache = HashCache::default();
        let opts = Options::default();
        let record = built(&s, &cache, &opts);
        // no cells: the light's fingerprint of a level that does not stream is as it was
        assert!(status_of(&s, &cache, Some(&record), &opts).to_build().is_empty());
        s.meta.level.cell_size = 32;
        let st = status_of(&s, &cache, Some(&record), &opts);
        assert_eq!(st.to_build(), [Step::Light]);
        assert_eq!(st.get(Step::Light).unwrap().why, "The level's streaming cells changed.");
    }

    #[test]
    fn a_changed_model_makes_the_steps_that_read_objects_stale() {
        let (s, _) = scene();
        let cache = HashCache::default();
        let then = Options { library: 1, ..Options::default() };
        let record = built(&s, &cache, &then);
        // the same models: nothing to build
        assert!(status_of(&s, &cache, Some(&record), &then).to_build().is_empty());
        // a model's file changed since
        let now = Options { library: 2, ..Options::default() };
        let st = status_of(&s, &cache, Some(&record), &now);
        let geometry = st.get(Step::Geometry).unwrap();
        assert_eq!(geometry.state, StepState::Stale);
        assert_eq!(geometry.why, "A model the level's objects are made from changed.");
        // what is built on those steps follows, saying which step it waits for
        assert_eq!(st.get(Step::Light).unwrap().why, "Geometry is not fresh.");
    }
}
