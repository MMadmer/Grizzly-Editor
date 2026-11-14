//! The `level` area: building the open scene's level (`level.build`, `level.status`,
//! `level.problems`, `level.validate`, `level.clean`), running it (`level.play`), its module
//! layers and the OBJ (`level.export`) and the compiler on a folder of its own (`level.compile`).

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{CmdError, Editor, JobError, JsonSchema, ProjectSession, Registry, Reply, Spec, Value, json};
use gz_game::GameLink;
use gz_project::DeleteContext;
use gz_scene::SceneService;
use serde::Deserialize;

use crate::compile;
use crate::pipeline::{self, Request, write_guard};
use crate::process::{self, Ended};
use crate::service::{self, LevelBuild, Running};
use crate::steps::{Options, Quality, Step};

/// The job of a build; one at a time.
pub const JOB_BUILD: &str = "level_build";
/// The job that holds the level compiler on a folder or the game while it runs, and its group.
pub const JOB_RUN: &str = "level_run";
const GROUP_RUN: &str = "game";
/// The group of the module's builds and packaging (`gz-build`): Play's module build shares the
/// builds' row, so it waits for theirs.
const MODULE_BUILDS: &str = "build";
/// The game's command line for a scene, after the root's own `-fsltx`.
const PLAY_PARAMS: &str =
    "-editor_scene -start server(editor/single/alife/new) client(localhost) -nointro -noprefetch";

pub fn register(reg: &mut Registry) {
    reg.area(
        "level",
        "Building and running the open scene's level: build, status, problems, play, export, compile",
    );
    reg.cmd(
        Spec::write(
            "level.build",
            "Builds the level as one job: what a step would refuse is checked first, then every stale step in order; an error ends it there, Stop ends it now. On a level the linked game has, only what the module needs.",
        )
        .example("quality=draft"),
        |ed, p: BuildParams, o| async move { build(ed, p, o).await },
    );
    reg.cmd(
        Spec::read("level.status", "Says for every step of the level build whether it is fresh, stale and why, never built or failed."),
        |ed, p: StatusParams, _o| async move { status(ed, p).await },
    );
    reg.cmd_sync(
        Spec::write(
            "level.problems",
            "What the last builds found, on the objects they are about; clear=true empties the list.",
        ),
        problems,
    );
    reg.cmd(
        Spec::read("level.validate", "Checks the scene the way a build does and answers the problems."),
        |ed, p: ValidateParams, _o| async move { validate(ed, p).await },
    );
    reg.cmd(
        Spec::write(
            "level.play",
            "Plays the scene in the linked game where it is installed: on a level the game has, the module built into Play's root and a new game started over it (nothing compiled); on a new level, what is stale built first. Stops it; says whether it runs.",
        )
        .example("action=start"),
        |ed, p: PlayParams, o| async move { play(ed, p, o).await },
    );
    reg.cmd(
        Spec::destroy(
            "level.clean",
            "Removes from the folder of a level the linked game has the files a level build wrote there, which would replace the game's own; dry_run=true names them.",
        )
        .example("dry_run=true"),
        |ed, p: CleanParams, _o| async move { clean(ed, p).await },
    );
    reg.cmd(
        Spec::write(
            "level.export",
            "Exports a module layer of the selection (spawn, collision, visuals, cut) or the static geometry as OBJ.",
        )
        .example("kind=spawn objects=stalker_guard_01,box_02"),
        |ed, p: ExportParams, _o| async move { export(ed, p).await },
    );
    reg.cmd(
        Spec::write(
            "level.compile",
            "Runs the level compiler on a folder that holds a build.prj, with or without a project.",
        )
        .example("folder=D:/Levels/l01_escape nosun=true"),
        |ed, p: CompileParams, _o| async move { compile_folder(ed, p).await },
    );
}

/// A build or a compile is running.
fn busy(editor: &Editor) -> Option<CmdError> {
    let job = editor
        .jobs
        .running_in(JOB_BUILD)
        .or_else(|| editor.jobs.running_in(GROUP_RUN))
        .or_else(|| editor.jobs.running_in(MODULE_BUILDS))?;
    Some(
        CmdError::busy(format!(
            "{} is still running: stop it (job.stop job={}) or wait for it.",
            job.title, job.id.0
        ))
        .with_data(json!({ "job": job.id.0 })),
    )
}

/// The scene a `level.*` command works on: the open one, or the project's, opened first.
async fn need_scene(editor: &Arc<Editor>) -> Result<(Arc<ProjectSession>, Arc<SceneService>), CmdError> {
    let session = editor.project()?;
    let svc = gz_scene::need_scene(editor).await?;
    Ok((session, svc))
}

/// What a job answered, as a command's answer.
async fn answer_job(editor: &Arc<Editor>, id: gz_editor::JobId, wait: Option<bool>) -> Reply {
    if wait == Some(false) {
        return Ok(json!({ "job": id.0 }));
    }
    let info = editor.jobs.wait(id).await.ok_or_else(|| CmdError::internal("the job is gone"))?;
    match info.state {
        gz_editor::JobState::Done => {
            let mut v = info.result.map_or(json!({}), |r| (*r).clone());
            v["job"] = json!(id.0);
            Ok(v)
        }
        gz_editor::JobState::Cancelled => {
            Err(CmdError::new("failed", "Stopped: the steps that ended before are kept.")
                .with_data(json!({ "job": id.0, "cancelled": true })))
        }
        _ => Err(CmdError::new("failed", info.error.unwrap_or_else(|| "The build failed.".into()))
            .with_data(json!({ "job": id.0 }))),
    }
}

// ---- level.build ----------------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BuildParams {
    /// The steps to build, each with what it needs; left out: every step that is stale.
    #[serde(default)]
    steps: Option<Vec<Step>>,
    /// One step and what it needs.
    #[serde(default)]
    only: Option<Step>,
    /// Build the steps asked for even when they are fresh.
    #[serde(default)]
    force: bool,
    /// How long the compiler spends on the light; left out: standard (the level's own settings).
    #[serde(default)]
    quality: Option<Quality>,
    /// The compiler: no sun light.
    #[serde(default)]
    nosun: bool,
    /// The compiler: no progressive meshes.
    #[serde(default)]
    noise: bool,
    /// The compiler: smoothing by angle, the faces' smoothing groups ignored.
    #[serde(default)]
    nosmg: bool,
    /// The compiler: the geometry is not optimised first.
    #[serde(default)]
    no_optimize: bool,
    /// The compiler: invalid faces do not stop it.
    #[serde(default)]
    no_invalid_faces: bool,
    /// The compiler: point lights fall off linearly.
    #[serde(default)]
    linear: bool,
    /// Wait for the build to end (the default); false answers the job at once.
    #[serde(default)]
    wait: Option<bool>,
}

/// The compiler's command line of the switches a request turned on.
fn compiler_flags(on: [(bool, &str); 6]) -> String {
    on.iter().filter(|(b, _)| *b).map(|(_, f)| *f).collect::<Vec<_>>().join(" ")
}

async fn build(ed: Arc<Editor>, p: BuildParams, _o: Origin) -> Reply {
    let (session, _svc) = need_scene(&ed).await?;
    if let Some(b) = busy(&ed) {
        return Err(b);
    }
    let mut opts = service::default_options(&ed);
    opts.quality = p.quality.unwrap_or_default();
    opts.flags = compiler_flags([
        (p.nosun, "-nosun"),
        (p.noise, "-noise"),
        (p.nosmg, "-nosmg"),
        (p.no_optimize, "-no_optimize"),
        (p.no_invalid_faces, "-no_invalidefaces"),
        (p.linear, "-att"),
    ]);
    let req = Request { steps: p.steps, only: p.only, force: p.force, opts, verify_restrictors: false };
    let id = start_build(&ed, &session, req, "Building the level")?;
    answer_job(&ed, id, p.wait).await
}

fn start_build(
    ed: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    req: Request,
    title: &str,
) -> Result<gz_editor::JobId, CmdError> {
    let (e2, s2) = (ed.clone(), session.clone());
    ed.jobs
        .start(JOB_BUILD, Some(JOB_BUILD), title, move |ctx| async move {
            pipeline::run(&e2, &s2, &req, &ctx).await
        })
        .map_err(|b| {
            CmdError::busy(format!("{} is still running", b.title)).with_data(json!({ "job": b.id.0 }))
        })
}

// ---- level.status, level.problems, level.validate --------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StatusParams {}

async fn status(ed: Arc<Editor>, _p: StatusParams) -> Reply {
    let (session, svc) = need_scene(&ed).await?;
    let snap = svc.store.snapshot();
    let rev = svc.store.rev();
    let st = LevelBuild::status(&ed, &session, snap.clone(), rev).await;
    // a step whose files are gone from the level folder is not fresh, whatever its record says
    let dir = pipeline::level_dir(&session, &st.level).ok();
    let checks: Vec<(Step, Vec<String>)> =
        Step::ORDER.iter().map(|s| (*s, s.files().iter().map(|f| (*f).to_owned()).collect())).collect();
    let spawn = session.root.join("gamedata").join("spawns").join("editor.spawn");
    let missing: Vec<(Step, String)> = ed
        .exec
        .io(move || {
            let mut out = Vec::new();
            if let Some(dir) = dir {
                for (s, files) in checks {
                    if let Some(f) = files.iter().find(|f| !dir.join(f).is_file()) {
                        out.push((s, f.clone()));
                    }
                }
            }
            if !spawn.is_file() {
                out.push((Step::Spawn, "gamedata\\spawns\\editor.spawn".to_owned()));
            }
            out
        })
        .await;
    let lb = LevelBuild::of(&session);
    let mut steps: Vec<Value> = Vec::new();
    let mut to_build: Vec<&str> = Vec::new();
    for s in &st.steps {
        let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
        if s.state == service::StepState::Fresh
            && let Some((_, f)) = missing.iter().find(|(m, _)| *m == s.step)
        {
            v["state"] = json!("stale");
            v["why"] = json!(format!("Its file {f} is gone from the level folder."));
        }
        if matches!(v["state"].as_str(), Some("stale" | "never" | "failed")) {
            to_build.push(s.step.name());
        }
        steps.push(v);
    }
    let running = lb.running().map(Running::name);
    let building = ed
        .jobs
        .running_in(JOB_BUILD)
        .map(|j| json!({ "job": j.id.0, "stage": j.stage, "fraction": j.fraction }));
    // a stock level: what an earlier build left there that the game would take for its own
    let replaces: Vec<String> = match (st.stock, pipeline::level_dir(&session, &st.level).ok()) {
        (true, Some(dir)) => {
            let record = lb.record_now(&ed, &session, &st.level).await;
            ed.exec
                .io(move || {
                    record
                        .map(|r| crate::stock::replacing(&crate::stock::written(&r, &dir)))
                        .unwrap_or_default()
                })
                .await
        }
        _ => Vec::new(),
    };
    Ok(json!({
        "level": st.level,
        "steps": steps,
        "stale": to_build,
        "fresh": to_build.is_empty(),
        "stock": st.stock,
        "replaces": replaces,
        "problems": lb.problems.lock().len(),
        "running": running,
        "building": building,
    }))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ProblemsParams {
    /// Empty the list (and the viewport's drawing of the compiler's errors).
    #[serde(default)]
    clear: bool,
    /// Only the problems of this step.
    #[serde(default)]
    step: Option<Step>,
    /// At most this many (100).
    #[serde(default)]
    limit: Option<usize>,
    /// Skip this many first.
    #[serde(default)]
    offset: Option<usize>,
}

fn problems(ed: &Editor, p: &ProblemsParams, _o: Origin) -> Reply {
    let Some(session) = ed.project_opt() else { return Ok(json!({ "problems": [], "total": 0 })) };
    let lb = LevelBuild::of(&session);
    if p.clear {
        let n = std::mem::take(&mut *lb.problems.lock()).len();
        if let Some(svc) = session.services.get::<SceneService>() {
            let store = svc.store.clone();
            ed.exec.spawn(async move {
                let _ = store
                    .send(
                        gz_scene::SceneCmd::DebugDraw(Arc::new(gz_scene::debug::DebugDraw::default())),
                        Origin::Mcp,
                    )
                    .await;
            });
        }
        return Ok(json!({ "cleared": n }));
    }
    let all = lb.problems.lock();
    let list: Vec<&service::LevelProblem> =
        all.iter().filter(|x| p.step.is_none_or(|s| x.step == s)).collect();
    let total = list.len();
    let (offset, limit) = (p.offset.unwrap_or(0), p.limit.unwrap_or(100));
    let page: Vec<Value> = list
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|x| serde_json::to_value(x).unwrap_or(Value::Null))
        .collect();
    Ok(json!({ "problems": page, "total": total }))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ValidateParams {
    /// Wait for the check to end (the default); false answers the job at once.
    #[serde(default)]
    wait: Option<bool>,
}

async fn validate(ed: Arc<Editor>, p: ValidateParams) -> Reply {
    let (session, _svc) = need_scene(&ed).await?;
    let (e2, s2) = (ed.clone(), session.clone());
    let id = ed
        .jobs
        .start("scene_validate", Some(JOB_BUILD), "Checking the scene", move |ctx| async move {
            let inp = pipeline::inputs(&e2, &ctx).await?;
            let lib = inp.lib.clone();
            let c = ctx.clone();
            let work = inp.snap.clone();
            let completed = e2
                .exec
                .cpu(move || gz_scene::objtools::zones::complete(&work, &lib, &|| c.cancel.is_cancelled()))
                .await
                .map_err(|_| JobError { message: "Stopped.".to_owned(), cancelled: true })?;
            let mut found = Vec::new();
            let ok = pipeline::validate_scene(
                &e2,
                Arc::new(completed.state),
                &inp,
                crate::validate::Tests::BUILD,
                Step::Geometry,
                &mut found,
                &ctx,
            )
            .await
            .is_ok();
            let lb = LevelBuild::of(&s2);
            let list: Vec<Value> =
                found.iter().map(|x| serde_json::to_value(x).unwrap_or(Value::Null)).collect();
            {
                let mut all = lb.problems.lock();
                all.retain(|x| x.code != "validation");
                all.extend(found);
            }
            Ok(json!({ "valid": ok, "problems": list }))
        })
        .map_err(|b| {
            CmdError::busy(format!("{} is still running", b.title)).with_data(json!({ "job": b.id.0 }))
        })?;
    answer_job(&ed, id, p.wait).await
}

// ---- level.play -------------------------------------------------------------------------------------

/// What `level.play` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PlayAction {
    /// Builds what is stale and starts the game on the level.
    #[default]
    Start,
    /// Stops the game or the compiler that runs.
    Stop,
    /// Whether the game runs.
    Status,
    /// Makes Play's game root without starting the game, and says what it is.
    Root,
    /// Reads the linked game's configs again (system.ltx and what it includes).
    ReloadConfigs,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlayParams {
    /// What to do; left out: start.
    #[serde(default)]
    action: PlayAction,
    /// start: the game as it is, without the scene and without building anything.
    #[serde(default)]
    clean: bool,
    /// start: build what is stale first (the default); false plays what was built last.
    #[serde(default)]
    build: Option<bool>,
    /// start, root: Play's game root; left out: the default one.
    #[serde(default)]
    root: Option<String>,
    /// start: more of the game's command line.
    #[serde(default)]
    params: Option<String>,
    /// start: the spawn checks every space restrictor.
    #[serde(default)]
    verify_restrictors: bool,
    /// start: the quality a stale level is built at; left out: the quality it was built at last.
    #[serde(default)]
    quality: Option<Quality>,
    /// start: everything but the game itself - the root made, the module or the level put in it -
    /// and the command line it would run answered.
    #[serde(default)]
    dry_run: bool,
}

async fn play(ed: Arc<Editor>, p: PlayParams, _o: Origin) -> Reply {
    let session = ed.project()?;
    let lb = LevelBuild::of(&session);
    match p.action {
        PlayAction::Status => Ok(json!({
            "running": lb.running().map(Running::name),
            "building": ed.jobs.running_in(JOB_BUILD).map(|j| j.stage),
        })),
        PlayAction::Stop => {
            let mut stopped = false;
            if let Some((_, id)) = *lb.running.lock() {
                stopped |= ed.jobs.cancel(id);
            }
            for group in [JOB_BUILD, GROUP_RUN] {
                if let Some(job) = ed.jobs.running_in(group) {
                    stopped |= ed.jobs.cancel(job.id);
                }
            }
            if let Some(job) = ed.jobs.running_in(MODULE_BUILDS).filter(|j| j.kind == JOB_PLAY_MODULE) {
                stopped |= ed.jobs.cancel(job.id);
            }
            Ok(json!({ "stopped": stopped }))
        }
        PlayAction::ReloadConfigs => {
            if let Some(b) = busy(&ed) {
                return Err(b);
            }
            let link = GameLink::of(&ed).map_err(|e| CmdError::new("refused", e.to_string()))?;
            link.refresh().await.map_err(|e| CmdError::new("failed", e.to_string()))?;
            ed.log.info("The linked game's configs are read again.");
            Ok(json!({ "reloaded": true }))
        }
        PlayAction::Root => {
            if let Some(b) = busy(&ed) {
                return Err(b);
            }
            let place = play_place(&ed, &session, &p).await?;
            let root = prepare_root(&ed, &place, None, None).await?;
            Ok(root_answer(&root))
        }
        PlayAction::Start if p.clean => {
            if let Some(b) = busy(&ed) {
                return Err(b);
            }
            let place = play_place(&ed, &session, &p).await?;
            let root = prepare_root(&ed, &place, None, None).await?;
            if p.dry_run {
                return Ok(dry_answer(&root, &place.params));
            }
            start_game(&ed, &session, &root, &place.params)
        }
        PlayAction::Start => {
            let (session, svc) = need_scene(&ed).await?;
            if let Some(b) = busy(&ed) {
                return Err(b);
            }
            // the root is checked before anything is built or written
            let place = play_place(&ed, &session, &p).await?;
            // a level the linked game has plays the module: nothing of the level is compiled
            let level = svc.store.snapshot().meta.level.level_path.to_string();
            if crate::stock::is_stock(&lb.game_levels(&ed).await, &level) {
                return play_module(&ed, &session, &svc, place, &p, &level).await;
            }
            if p.build != Some(false) {
                let snap = svc.store.snapshot();
                let level = snap.meta.level.level_path.to_string();
                let last = lb.record_now(&ed, &session, &level).await;
                let mut opts: Options = service::default_options(&ed);
                // a level built at draft quality, or with compiler flags, plays as it was built
                let light = last.as_ref().and_then(|r| r.steps.get(&Step::Light));
                opts.quality = p.quality.unwrap_or_else(|| light.map_or(Quality::Standard, |r| r.quality));
                opts.flags = light.map(|r| r.flags.clone()).unwrap_or_default();
                let steps: Vec<Step> = Step::ORDER.into_iter().filter(|s| *s != Step::SpawnLayer).collect();
                let req = Request {
                    steps: Some(steps),
                    only: None,
                    force: false,
                    opts,
                    verify_restrictors: p.verify_restrictors,
                };
                let id = start_build(&ed, &session, req, "Building the level for play")?;
                answer_job(&ed, id, None).await?;
            }
            let snap = svc.store.snapshot();
            let level = snap.meta.level.level_path.to_string();
            let dir = pipeline::level_dir(&session, &level).map_err(|e| CmdError::new("refused", e))?;
            let spawn = session.root.join("gamedata").join("spawns").join("editor.spawn");
            let (d2, s2) = (dir.clone(), spawn.clone());
            let built = ed
                .exec
                .io(move || {
                    ["level", "level.ltx", "level.geom", "level.cform", "level.ai"]
                        .iter()
                        .all(|f| d2.join(f).is_file())
                        && s2.is_file()
                })
                .await;
            if !built {
                return Err(CmdError::new(
                    "refused",
                    format!("The level {level} is not built: run level.build first."),
                ));
            }
            let root = prepare_root(&ed, &place, Some((level.clone(), dir.clone(), spawn)), None).await?;
            let params = format!("{PLAY_PARAMS} {}", place.params);
            if p.dry_run {
                let mut v = dry_answer(&root, params.trim_end());
                v["way"] = json!("level");
                v["level"] = json!(level);
                return Ok(v);
            }
            let mut started = start_game(&ed, &session, &root, params.trim_end())?;
            started["root"] = json!(root.root.display().to_string().replace('\\', "/"));
            started["way"] = json!("level");
            started["level"] = json!(level);
            Ok(started)
        }
    }
}

/// Where Play runs the game: its root, what the root checks compare with, the project's write
/// guard and what the caller adds to the command line.
struct PlayPlace {
    root: PathBuf,
    params: String,
    guard: gz_editor::fsx::WriteGuard,
    context: Arc<DeleteContext>,
    game: PathBuf,
}

/// The linked game's folder, or the sentence that says the project has none.
fn linked_game(editor: &Editor) -> Result<String, CmdError> {
    let (_, project) = gz_project::Project::of(editor)?;
    let root = project.game_root();
    if root.trim().is_empty() {
        return Err(CmdError::new("refused", gz_project::NOT_LINKED));
    }
    Ok(root)
}

/// What a Play root is checked against: the deletion's guards of this machine, the open project
/// and every listed one.
async fn play_context(editor: &Arc<Editor>) -> DeleteContext {
    let listed = gz_project::Projects::of(editor).list(editor).await.entries();
    let open = gz_project::Project::of(editor).ok().map(|(_, p)| (p.root().to_owned(), p.game_root()));
    let (exe, root, home) =
        (editor.paths.exe_dir.clone(), editor.paths.root.clone(), editor.paths.home.clone());
    editor
        .exec
        .io(move || {
            let guards =
                gz_project::system_guards(&exe, &root, &home, open.as_ref().map(|o| o.1.as_str()), &listed);
            DeleteContext { listed, open: open.map(|o| o.0), guards }
        })
        .await
}

/// The root of a Play and the linked game's engine, checked before anything is built or written
/// (and again when the root is made): an engine that does not run in place is refused here.
async fn play_place(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    p: &PlayParams,
) -> Result<PlayPlace, CmdError> {
    let root = match p.root.as_deref().filter(|r| !r.trim().is_empty()) {
        Some(r) => PathBuf::from(r),
        // an editor with a home of its own (-home, GRIZZLY_HOME) plays under it
        None => {
            let home = (editor.paths.home != editor.paths.root).then_some(editor.paths.home.as_path());
            crate::play_root::default_root(&session.root, home)
        }
    };
    let game = PathBuf::from(linked_game(editor)?);
    let context = Arc::new(play_context(editor).await);
    let (r, cx, g) = (root.clone(), context.clone(), game.clone());
    editor
        .exec
        .io(move || {
            crate::play::ready_engine(&g, &crate::play::LINKED_GAME)?;
            crate::play_root::check_root(&cx, &r)
        })
        .await
        .map_err(|e| CmdError::new("refused", e))?;
    Ok(PlayPlace {
        root,
        params: p.params.clone().unwrap_or_default(),
        guard: write_guard(editor),
        context,
        game,
    })
}

/// The longest path of the linked game's files, relative to its gamedata.
async fn longest_game_path(editor: &Arc<Editor>) -> String {
    let files = match GameLink::of(editor) {
        Ok(link) => link.files().await.ok(),
        Err(_) => None,
    };
    files
        .and_then(|f| f.iter().map(|r| r.path).max_by_key(|p| p.chars().count()).map(str::to_owned))
        .unwrap_or_default()
}

/// Play's root made (or refreshed) and, with a level, the level staged into it; with a module,
/// the root's marker naming it before the module build writes it.
async fn prepare_root(
    editor: &Arc<Editor>,
    place: &PlayPlace,
    level: Option<(String, PathBuf, PathBuf)>,
    module: Option<String>,
) -> Result<crate::play::PlayRoot, CmdError> {
    let archived = longest_game_path(editor).await;
    let (root, guard, context, game) =
        (place.root.clone(), place.guard.clone(), place.context.clone(), place.game.clone());
    editor
        .exec
        .io(move || {
            guard.check(&root).map_err(|e| e.to_string())?;
            let r = crate::play::prepare(&crate::play::PlayInput {
                game: &game,
                root: &root,
                context: &context,
                archived: &archived,
                level: level.as_ref().map(|(l, d, _)| (l.as_str(), d.as_path())),
                module: module.as_deref(),
            })?;
            if let Some((l, d, spawn)) = &level {
                crate::play::stage_level(&r.root, l, d, spawn)?;
            }
            Ok::<_, String>(r)
        })
        .await
        .map_err(|e| CmdError::new("refused", e))
}

/// Starts the installed engine where it is, in the root on the root's `fsgame.ltx` and `params`,
/// as the `level_run` job; answers once it started.
fn start_game(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    root: &crate::play::PlayRoot,
    params: &str,
) -> Reply {
    let mut args = root.fsltx_args();
    args.extend(process::arguments(params));
    let (exe, cwd) = (root.exe.clone(), root.root.clone());
    let line = process::command_line(&exe, &args);
    editor.log.info(format!(
        "Run {line} (the installed engine, started in place in the game root {}).",
        cwd.display()
    ));
    let lb = LevelBuild::of(session);
    let (ed, lb2) = (editor.clone(), lb.clone());
    let id = editor
        .jobs
        .start(JOB_RUN, Some(GROUP_RUN), "Running the game", move |ctx| async move {
            ctx.stage("Running the game");
            let c2 = ctx.clone();
            let log = ed.log.clone();
            let ended = process::run(&exe, &args, &cwd, &ctx.cancel, move |l| {
                c2.stage(l.to_owned());
                log.info(l);
            })
            .await;
            *lb2.running.lock() = None;
            match ended {
                Ok(Ended::Exited(code)) => Ok(json!({ "exit_code": code })),
                Ok(Ended::Stopped) => Err(JobError { message: "Stopped.".to_owned(), cancelled: true }),
                Err(e) => {
                    ed.log.error(&e);
                    Err(JobError::new(e))
                }
            }
        })
        .map_err(|b| {
            CmdError::busy(format!("{} is still running", b.title)).with_data(json!({ "job": b.id.0 }))
        })?;
    *lb.running.lock() = Some((Running::Game, id));
    Ok(
        json!({ "started": Running::Game.name(), "job": id.0, "command": line, "exe": root.exe.display().to_string().replace('\\', "/") }),
    )
}

/// What a Play that starts nothing (`dry_run`) answers: the root and the command line it would run.
fn dry_answer(root: &crate::play::PlayRoot, params: &str) -> Value {
    let mut args = root.fsltx_args();
    args.extend(process::arguments(params));
    let line = process::command_line(&root.exe, &args);
    let mut v = root_answer(root);
    v["command"] = json!(line);
    v["params"] = json!(params);
    v["started"] = Value::Null;
    v
}

/// The job of Play's module build; in the group of the module builds, which share its row.
pub const JOB_PLAY_MODULE: &str = "play_module";

/// Play on a level the linked game has: the module built into Play's root the way a module build
/// builds it, Play's own Player Start in its copy when the scene has none, the new game's mode the
/// module's, and the game started on a new game over the module.
async fn play_module(
    ed: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    svc: &Arc<SceneService>,
    place: PlayPlace,
    p: &PlayParams,
    level: &str,
) -> Reply {
    let manifest = gz_mod::ManifestService::of_session(ed, session).await?;
    manifest.sync().await?;
    let m = manifest.snapshot().manifest().cloned().ok_or_else(|| {
        CmdError::new("refused", "The project has no mod.ltx: Play builds the module from it.")
    })?;
    let mode =
        crate::module_play::new_game_mode(&m.target, &m.mode).map_err(|e| CmdError::new("refused", e))?;
    let note = format!("Playing the module over {level}: no compile needed.");
    ed.log.info(&note);
    // what an earlier build left in the level's folder ships with the module and replaces the game's own
    let lb = LevelBuild::of(session);
    let replaces: Vec<String> =
        match (lb.record_now(ed, session, level).await, pipeline::level_dir(session, level)) {
            (Some(r), Ok(dir)) => {
                ed.exec.io(move || crate::stock::replacing(&crate::stock::written(&r, &dir))).await
            }
            _ => Vec::new(),
        };
    {
        let mut all = lb.problems.lock();
        all.retain(|x| x.code != "replaces_stock");
        if let Some(p) = crate::stock::problem(level, &replaces) {
            ed.log.warn(&p.text);
            all.push(p);
        }
    }
    // the game would load them over its own level - an AI map its cross table does not know - and stop
    if !replaces.is_empty() {
        let named: Vec<&str> = replaces.iter().map(String::as_str).take(5).collect();
        let more = if replaces.len() > named.len() { ", ..." } else { "" };
        return Err(CmdError::new(
            "refused",
            format!(
                "The module would replace {} of {level}'s own files ({}{more}): the game loads them over its own level and stops there. Remove them first: level.clean, or Build > Advanced > Remove Build Files...",
                replaces.len(),
                named.join(", ")
            ),
        )
        .with_data(json!({ "replaces": replaces })));
    }
    let (e2, s2, svc2, id, lv, build) =
        (ed.clone(), session.clone(), svc.clone(), m.id.clone(), level.to_owned(), p.build != Some(false));
    let place = Arc::new(place);
    let pl = place.clone();
    let mode2 = mode.clone();
    let job = ed
        .jobs
        .start(JOB_PLAY_MODULE, Some(MODULE_BUILDS), "Building the module for play", move |ctx| async move {
            ctx.stage("Preparing the game root");
            let root = prepare_root(&e2, &pl, None, Some(id.clone())).await.map_err(|e| JobError::new(e.message))?;
            ctx.check()?;
            let target = root.root.display().to_string();
            let mut files = Value::Null;
            if build {
                let req = gz_build::BuildRequest {
                    kind: gz_build::BuildKind::ToFolder,
                    target: target.clone(),
                    clean: true,
                    remember_target: false,
                };
                let r = gz_build::run_build(&e2, &s2, &req, &ctx).await;
                if r.cancelled {
                    return Err(JobError { message: crate::STOPPED.to_owned(), cancelled: true });
                }
                if !r.ok {
                    return Err(JobError::new(format!("The module could not be built for play: {}", r.err)));
                }
                files = json!(r.files);
            }
            let module_dir = root.root.join("modules").join(&id);
            let present = { let d = module_dir.clone(); e2.exec.io(move || d.join(gz_mod::FILE).is_file()).await };
            if !present {
                return Err(JobError::new(format!(
                    "Play's root holds no module {id} yet: play it once with build (the default)."
                )));
            }
            ctx.check()?;
            // the actor's start: the module's, or Play's own where the camera looks
            let snap = svc2.store.snapshot();
            let scene_level = gz_export_level::names::resolve_level(None, &snap.meta.name()).unwrap_or_else(|| lv.clone());
            let has_start = !gz_export_level::starts::scene_starts(&snap, &scene_level).is_empty();
            let start = if has_start {
                json!({ "placed": false, "source": "scene" })
            } else {
                ctx.stage("Placing the Player Start where the camera looks");
                svc2.prefetch_library(&e2, &snap).await;
                let (lib, camera, lv2) = (svc2.library.clone(), gz_editor::ViewCamera::of(&e2).get(), lv.clone());
                let (start, how) =
                    e2.exec.cpu(move || crate::module_play::camera_start(&snap, &lib, camera.as_ref(), &lv2)).await;
                let (r2, id2, st2) = (root.root.clone(), id.clone(), start.clone());
                let dropped = e2.exec.io(move || crate::play::place_start(&r2, &id2, &st2)).await.map_err(JobError::new)?;
                json!({ "placed": true, "source": how, "position": start.position, "direction": start.direction, "replaced": dropped })
            };
            let (r3, game, m3) = (root.root.clone(), pl.game.clone(), mode2.clone());
            e2.exec
                .io(move || crate::play::write_game_options(&r3, &game, m3.as_deref()))
                .await
                .map_err(JobError::new)?;
            Ok(json!({
                "root": target.replace('\\', "/"),
                "exe": root.exe.display().to_string().replace('\\', "/"),
                "module": id,
                "module_dir": module_dir.display().to_string().replace('\\', "/"),
                "files": files,
                "start": start,
            }))
        })
        .map_err(|b| CmdError::busy(format!("{} is still running", b.title)).with_data(json!({ "job": b.id.0 })))?;
    let built = answer_job(ed, job, None).await?;
    let root = crate::play::PlayRoot {
        root: place.root.clone(),
        exe: PathBuf::from(built["exe"].as_str().unwrap_or_default().replace('/', "\\")),
        path_limit: 0,
        was: "play_root",
        walked: 0,
        walk_ms: 0,
        linked: 0,
        copied: 0,
    };
    let params = format!("{} {}", crate::module_play::PARAMS, place.params);
    let mut out = if p.dry_run {
        dry_answer(&root, params.trim_end())
    } else {
        let mut v = start_game(ed, session, &root, params.trim_end())?;
        v["root"] = json!(root.root.display().to_string().replace('\\', "/"));
        v
    };
    for k in ["module", "module_dir", "files", "start"] {
        out[k] = built[k].clone();
    }
    out["way"] = json!("module");
    out["level"] = json!(level);
    out["mode"] = json!(mode);
    out["note"] = json!(note);
    out["replaces"] = json!(replaces);
    Ok(out)
}

/// `action=root`'s answer: the root, what it was, the installed engine it runs.
fn root_answer(r: &crate::play::PlayRoot) -> Value {
    json!({
        "root": r.root.display().to_string().replace('\\', "/"),
        "was": r.was,
        "exe": r.exe.file_name().map(|n| n.to_string_lossy().into_owned()),
        "exe_path": r.exe.display().to_string().replace('\\', "/"),
        "path_limit": r.path_limit,
        "walked": r.walked,
        "walk_ms": r.walk_ms,
        "linked": r.linked,
        "copied": r.copied,
    })
}

// ---- level.clean ------------------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CleanParams {
    /// Name the files and remove nothing.
    #[serde(default)]
    dry_run: bool,
}

/// The files a level build wrote into the folder of a level the linked game has, removed for good
/// (every one the record names, or the names its steps make) with the compiler's stage
/// folders. The record keeps only the steps that are no compile.
async fn clean(ed: Arc<Editor>, p: CleanParams) -> Reply {
    let (session, svc) = need_scene(&ed).await?;
    if let Some(b) = busy(&ed) {
        return Err(b);
    }
    let level = svc.store.snapshot().meta.level.level_path.to_string();
    let lb = LevelBuild::of(&session);
    if !crate::stock::is_stock(&lb.game_levels(&ed).await, &level) {
        return Err(CmdError::new(
            "refused",
            format!(
                "{level} is no level of the linked game: what its build wrote is the level the module brings."
            ),
        ));
    }
    let dir = pipeline::level_dir(&session, &level).map_err(|e| CmdError::new("refused", e))?;
    let Some(record) = lb.record_now(&ed, &session, &level).await else {
        return Ok(json!({ "level": level, "files": [], "removed": [] }));
    };
    let (r2, d2) = (record.clone(), dir.clone());
    let files = ed.exec.io(move || crate::stock::written(&r2, &d2)).await;
    let replaces = crate::stock::replacing(&files);
    if p.dry_run {
        return Ok(json!({ "level": level, "files": files, "replaces": replaces, "removed": [] }));
    }
    let guard = write_guard(&ed);
    // the spawn step's own file outside the level folder (Play's single-level spawn) goes with it
    let outside: Vec<String> = record
        .steps
        .iter()
        .filter(|(s, _)| s.compiles())
        .flat_map(|(_, r)| r.files.iter())
        .map(|f| f.replace('/', "\\"))
        .filter(|f| f.eq_ignore_ascii_case(r"gamedata\spawns\editor.spawn"))
        .collect();
    let (d3, names, root3) = (dir.clone(), files.clone(), session.root.clone());
    let (removed, kept) = ed
        .exec
        .io(move || {
            let (mut removed, mut kept) = (Vec::new(), Vec::new());
            let there: Vec<(String, std::path::PathBuf)> = names
                .into_iter()
                .map(|n| {
                    let at = d3.join(n.replace('/', "\\"));
                    (n, at)
                })
                .chain(outside.into_iter().map(|f| {
                    let at = root3.join(&f);
                    (f, at)
                }))
                .filter(|(_, at)| at.is_file())
                .collect();
            for (n, at) in there {
                match guard
                    .check(&at)
                    .map_err(|e| e.to_string())
                    .and_then(|()| std::fs::remove_file(&at).map_err(|e| gz_base::io_reason(&e)))
                {
                    Ok(()) => removed.push(n),
                    Err(why) => kept.push(json!({ "file": n, "why": why })),
                }
            }
            for stage in [".lc_new", ".lc_old"] {
                let at = d3.join(stage);
                if at.is_dir() && guard.check(&at).is_ok() {
                    gz_build::install::remove_tree(&at);
                }
            }
            (removed, kept)
        })
        .await;
    // the record keeps what was not compiled here (the spawn layer), and every file the removal
    // left: it is still the build's, so the warning and Play's refusal stay until it goes
    let left: Vec<String> = kept
        .iter()
        .filter_map(|k| k.get("file").and_then(Value::as_str).map(|f| f.replace('/', "\\")))
        .collect();
    let mut r = (*record).clone();
    r.steps.retain(|s, rec| {
        if !s.compiles() {
            return true;
        }
        if rec.files.is_empty() {
            return !left.is_empty();
        }
        rec.files.retain(|f| left.iter().any(|l| l.eq_ignore_ascii_case(&f.replace('/', "\\"))));
        !rec.files.is_empty()
    });
    let r = Arc::new(r);
    lb.set_record(r.clone());
    let (d4, guard) = (dir.clone(), write_guard(&ed));
    let saved =
        ed.exec.io(move || guard.check(&d4).map_err(std::io::Error::other).and_then(|()| r.save(&d4))).await;
    if let Err(e) = saved {
        ed.log.warn(format!("The build's record could not be written into {}: {e}", dir.display()));
    }
    if kept.is_empty() {
        lb.problems.lock().retain(|x| x.code != "replaces_stock");
    }
    ed.log.info(format!("Removed {} build files from {level}.", removed.len()));
    if !kept.is_empty() {
        ed.log
            .warn(format!("{} build files of {level} could not be removed and stay on record.", kept.len()));
    }
    Ok(json!({ "level": level, "files": files, "removed": removed, "kept": kept }))
}

// ---- level.export -----------------------------------------------------------------------------------

/// What `level.export` writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ExportKind {
    /// The module's spawn layer: the objects merged into `spawn\<level>.xspawn`.
    Spawn,
    /// The module's collision overlay: `levels\<level>\overlay.xcform`.
    Collision,
    /// The module's visual overlay: the static objects as models in `levels\<level>\visuals`.
    Visuals,
    /// The level cut: the objects' boxes hide the game's geometry there.
    Cut,
    /// The static geometry as a Wavefront OBJ with its MTL.
    Obj,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExportParams {
    /// What to write.
    kind: ExportKind,
    /// The objects it is made of; left out: the selection.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// Every object of the scene instead of the selection.
    #[serde(default)]
    all: bool,
    /// The level the layer is for; left out: the open scene's.
    #[serde(default)]
    level: Option<String>,
    /// spawn: only the spawn points of this section.
    #[serde(default)]
    section: Option<String>,
    /// spawn: `add`, `remove`, or both when left out.
    #[serde(default)]
    mode: Option<String>,
    /// collision: the sector its faces are in (0).
    #[serde(default)]
    sector: Option<i64>,
    /// cut: hide the game's visuals that only touch a box too.
    #[serde(default)]
    overlap: bool,
    /// cut: grow every box by this many centimetres.
    #[serde(default)]
    grow: Option<i64>,
    /// obj: the file, absolute or under the project's `import` folder.
    #[serde(default)]
    path: Option<String>,
}

async fn export(ed: Arc<Editor>, p: ExportParams) -> Reply {
    need_scene(&ed).await?;
    let layer = gz_export_level::Layer {
        level: p.level.clone(),
        objects: p.objects.clone(),
        all: p.all,
        section: p.section.clone(),
        mode: p.mode.clone(),
        sector: p.sector.unwrap_or(0),
        overlap: p.overlap,
        grow_cm: p.grow.unwrap_or(0),
    };
    match p.kind {
        ExportKind::Spawn => gz_export_level::export_spawn(&ed, &layer).await,
        ExportKind::Collision => gz_export_level::export_collision(&ed, &layer).await,
        ExportKind::Visuals => gz_export_level::export_visuals(&ed, &layer).await,
        ExportKind::Cut => gz_export_level::export_cut(&ed, &layer).await,
        ExportKind::Obj => export_obj(&ed, &p).await,
    }
}

/// The static geometry of the scene (or of the objects named, or the selection) as a Wavefront
/// `.obj` and its `.mtl`.
async fn export_obj(editor: &Arc<Editor>, p: &ExportParams) -> Reply {
    let session = editor.project()?;
    let raw = PathBuf::from(
        p.path
            .clone()
            .ok_or_else(|| CmdError::invalid("kind=obj needs 'path': the .obj file"))?
            .replace('/', "\\"),
    );
    let file =
        if raw.is_absolute() { raw } else { session.root.join("import").join(raw) }.with_extension("obj");
    let selected = !p.all;
    let names = p.objects.clone();
    let ed = editor.clone();
    let out = file.clone();
    let id = editor
        .jobs
        .start("scene_export_obj", Some(JOB_BUILD), "Exporting the scene as OBJ", move |ctx| async move {
            let inp = pipeline::inputs(&ed, &ctx).await?;
            ctx.stage("Building the geometry");
            let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let (lib, engine) = (inp.lib.clone(), inp.engine.clone());
            let mut scene = (*inp.snap).clone();
            if let Some(names) = &names {
                scene.selection = gz_store::imbl::OrdSet::new();
                for n in names {
                    let o = scene
                        .find_relaxed(n)
                        .ok_or_else(|| JobError::new(format!("there is no object '{n}' in the scene")))?;
                    scene.selection.insert(o.id);
                }
            }
            let made = ed
                .exec
                .cpu(move || {
                    let look = crate::files::lookup(lib);
                    crate::prj::export_obj(&scene, &look, &engine, selected, &stem)
                })
                .await
                .map_err(|e| JobError::new(e.0))?;
            ctx.check()?;
            ctx.stage("Writing the files");
            let (obj, mtl) = (out.clone(), out.with_extension("mtl"));
            let objects = made.objects;
            let guard = write_guard(&ed);
            ed.exec
                .io(move || -> std::io::Result<()> {
                    guard.check(&obj).map_err(std::io::Error::other)?;
                    if let Some(d) = obj.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    std::fs::write(&mtl, made.mtl.as_bytes())?;
                    std::fs::write(&obj, made.obj.as_bytes())
                })
                .await
                .map_err(|e| JobError::new(format!("the OBJ could not be written: {e}")))?;
            Ok(json!({ "path": out.display().to_string().replace('\\', "/"), "objects": objects }))
        })
        .map_err(|b| {
            CmdError::busy(format!("{} is still running", b.title)).with_data(json!({ "job": b.id.0 }))
        })?;
    answer_job(editor, id, None).await
}

// ---- level.compile ----------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CompileParams {
    /// The level folder: absolute, or under the open project.
    folder: String,
    /// The detail compiler too, when the folder holds build.details.
    #[serde(default)]
    details: Option<bool>,
    /// Only the detail compiler.
    #[serde(default)]
    only_details: bool,
    /// No sun light.
    #[serde(default)]
    nosun: bool,
    /// No progressive meshes.
    #[serde(default)]
    noise: bool,
    /// Smoothing by angle, the faces' smoothing groups ignored.
    #[serde(default)]
    nosmg: bool,
    /// The geometry is not optimised first.
    #[serde(default)]
    no_optimize: bool,
    /// Invalid faces do not stop the compile.
    #[serde(default)]
    no_invalid_faces: bool,
    /// Point lights fall off linearly.
    #[serde(default)]
    linear: bool,
    /// The compile on a pool of its own of this many threads (the same result on any number).
    #[serde(default)]
    threads: Option<usize>,
    /// Wait for the compile to end (the default); false answers the job at once.
    #[serde(default)]
    wait: Option<bool>,
}

impl CompileParams {
    fn flags(&self) -> String {
        let mut f = compiler_flags([
            (self.nosun, "-nosun"),
            (self.noise, "-noise"),
            (self.nosmg, "-nosmg"),
            (self.no_optimize, "-no_optimize"),
            (self.no_invalid_faces, "-no_invalidefaces"),
            (self.linear, "-att"),
        ]);
        if let Some(n) = self.threads {
            f.push_str(&format!("{}-threads={n}", if f.is_empty() { "" } else { " " }));
        }
        f
    }
}

async fn compile_folder(ed: Arc<Editor>, p: CompileParams) -> Reply {
    if let Some(b) = busy(&ed) {
        return Err(b);
    }
    let raw = PathBuf::from(p.folder.replace('/', "\\"));
    let dir = if raw.is_absolute() {
        raw
    } else {
        ed.project_root()
            .ok_or_else(|| CmdError::invalid("folder: an absolute path, or one inside the open project"))?
            .join(raw)
    };
    let guard = write_guard(&ed);
    let (d2, details, only) = (dir.clone(), p.details, p.only_details);
    let parts = ed
        .exec
        .io(move || {
            guard.check(&d2).map_err(|e| e.to_string())?;
            if !d2.join("build.prj").is_file() {
                return Err(format!("{} holds no build.prj.", d2.display()));
            }
            let parts = if only { vec![compile::Part::Details] } else { compile::parts_of(&d2, details) };
            if parts.contains(&compile::Part::Details) && !d2.join("build.details").is_file() {
                return Err(format!("{} holds no build.details.", d2.display()));
            }
            if parts == [compile::Part::Details] && !d2.join("build.cform").is_file() {
                return Err(format!("{} holds no build.cform: compile the level first.", d2.display()));
            }
            Ok(parts)
        })
        .await
        .map_err(|e| CmdError::new("refused", e))?;
    let level = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let flags = p.flags();
    let names: Vec<&str> = parts.iter().map(|x| x.name()).collect();
    ed.log.info(format!(
        "Compile {} ({}){}.",
        dir.display(),
        names.join(", "),
        if flags.is_empty() { String::new() } else { format!(" {flags}") }
    ));
    let req = compile::Request { level: level.clone(), dir: dir.clone(), parts, flags };
    let e2 = ed.clone();
    let id = ed
        .jobs
        .start(JOB_RUN, Some(GROUP_RUN), format!("Compiling {level}"), move |ctx| async move {
            compile::run(&e2, req, &ctx).await
        })
        .map_err(|b| {
            CmdError::busy(format!("{} is still running", b.title)).with_data(json!({ "job": b.id.0 }))
        })?;
    answer_job(&ed, id, p.wait).await
}
