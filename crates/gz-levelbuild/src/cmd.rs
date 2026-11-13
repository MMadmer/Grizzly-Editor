//! `level_build`: the Level toolbar's builds and runs (`UITopBarForm::DrawLevel`), each a job.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, JobCtx, JobError, ProjectSession, Registry, Reply, json};
use gz_entities::Settings;
use gz_fmt_object::ObjectFlags;
use gz_game::GameLink;
use gz_project::DeleteContext;
use gz_scene::objects::PointKind;
use gz_scene::{ObjClass, ObjData, SceneService, SceneState};
use gz_store::Snap;

use crate::compile;
use crate::files;
use crate::graph::{self, GraphPoint, LevelGameGraph};
use crate::libs::Engine;
use crate::nav::LevelNav;
use crate::prj;
use crate::process::{self, Ended};
use crate::service::{self, Built, LevelBuild, Running};
use crate::spawn::{self, Element, Way};

const AREA: &str = "gz-levelbuild";
/// The job of a build; one at a time.
pub const JOB_BUILD: &str = "level_build";
/// The job that holds the level compiler or the game while it runs, and its group (the Level
/// tab greys the builds while either group runs).
pub const JOB_RUN: &str = "level_run";
const GROUP_RUN: &str = "game";
const BUSY: &str = "A compiler or the game is running.";
/// The game's command line for a scene of the editor (`ClickPlayPC`), after the root's own
/// `-fsltx` (`PlayRoot::fsltx_args`).
const PLAY_PARAMS: &str =
    "-editor_scene -start server(editor/single/alife/new) client(localhost) -nointro -noprefetch";

/// The commands, as the Level tab's toolbar and Build menu name them (`gz-ui-level`'s
/// `LEVEL_ACTIONS` and `BUILD_ROWS`).
pub const COMMANDS: [&str; 17] = [
    "build_cform",
    "build_aimap",
    "build_game_graph",
    "build_all",
    "play",
    "play_game",
    "play_clean",
    "play_stop",
    "reload_configs",
    "make_all",
    "make_game",
    "make_details",
    "make_hom",
    "make_som",
    "make_aimap",
    "compile_geometry",
    "compile_details",
];

/// Files by name, as a build writes them into the level folder.
type NamedFiles = Vec<(String, Vec<u8>)>;

/// Registers the commands of [`COMMANDS`], `level_rebuild_status` and `level_build` (the same
/// commands by `action`, with `status`).
pub fn register(reg: &mut Registry) {
    for name in COMMANDS {
        reg.add(AREA, name, move |ed, a, o| async move { level_build(&ed, name, &a, o).await });
    }
    // a compile on its own: a level folder of the project (`level=`, else the open scene's)
    reg.add(AREA, "level_compile", |ed, a, o| async move { level_build(&ed, "compile", &a, o).await });
    reg.add(AREA, "level_rebuild_status", |ed, _a, _o| async move { rebuild_status(&ed) });
    reg.add(AREA, "scene_validate", |ed, _a, _o| async move { scene_validate(&ed).await });
    reg.add(AREA, "scene_export_obj", |ed, a, _o| async move { scene_export_obj(&ed, &a).await });
    reg.add(AREA, "level_build", |ed, a, o| async move {
        let action = a.str("action").unwrap_or("status").to_owned();
        level_build(&ed, &action, &a, o).await
    });
}

/// `{collision, ai_map, game_graph}`: what the scene's edits left for the level builders.
fn rebuild_status(editor: &Arc<Editor>) -> Reply {
    let Some(session) = editor.project_opt() else {
        return Ok(json!({ "collision": false, "ai_map": false, "game_graph": false }));
    };
    let r = match session.services.get::<SceneService>() {
        Some(svc) => LevelBuild::of(&session).rebuild(&svc.store.snapshot()),
        None => service::Rebuild::default(),
    };
    Ok(json!({ "collision": r.cform, "ai_map": r.ai_map, "game_graph": r.game_graph, "text": r.text() }))
}

async fn level_build(editor: &Arc<Editor>, action: &str, a: &Args, _o: Origin) -> Reply {
    let session = editor.project()?;
    let lb = LevelBuild::of(&session);
    let busy = lb.running().is_some() || editor.jobs.running_in(JOB_BUILD).is_some();
    match action {
        "status" => status(editor, &session),
        "stop" | "play_stop" => {
            let mut stopped = false;
            if let Some((_, id)) = *lb.running.lock() {
                stopped |= editor.jobs.cancel(id);
            }
            // a compile or the game that has not marked itself yet is the group's job
            for group in [JOB_BUILD, GROUP_RUN] {
                if let Some(job) = editor.jobs.running_in(group) {
                    stopped |= editor.jobs.cancel(job.id);
                }
            }
            Ok(json!({ "stopped": stopped }))
        }
        "reload_configs" => {
            if busy {
                return Err(CmdError::busy(BUSY));
            }
            let link = GameLink::of(editor).map_err(|e| CmdError::invalid(e.to_string()))?;
            link.refresh().await.map_err(|e| CmdError::invalid(e.to_string()))?;
            editor.log.info("* The game configs are read again.");
            Ok(json!({ "reloaded": true }))
        }
        "play_clean" | "play_root" => {
            if busy {
                return Err(CmdError::busy(BUSY));
            }
            let place = play_place(editor, &session, a).await?;
            let game = PathBuf::from(linked_game(editor)?);
            let archived = longest_game_path(editor).await;
            let params = place.params.clone();
            let root = editor
                .exec
                .io(move || {
                    place.guard.check(&place.root).map_err(|e| e.to_string())?;
                    crate::play::prepare(&crate::play::PlayInput {
                        game: &game,
                        root: &place.root,
                        context: &place.context,
                        archived: &archived,
                        level: None,
                    })
                })
                .await
                .map_err(CmdError::invalid)?;
            if action == "play_root" {
                return Ok(root_answer(&root));
            }
            start_game(editor, &session, &root, &params)
        }
        "compile_geometry" | "compile_details" => {
            if busy {
                return Err(CmdError::busy(BUSY));
            }
            let svc = SceneService::of(editor)?;
            let level = svc.store.snapshot().meta.level.level_path.to_string();
            let parts = if action == "compile_geometry" {
                vec![compile::Part::Level]
            } else {
                vec![compile::Part::Details]
            };
            compile_command(editor, &session, &level, Some(parts), a).await
        }
        "compile" => {
            if busy {
                return Err(CmdError::busy(BUSY));
            }
            let level = match a.text("level").filter(|l| !l.trim().is_empty()) {
                Some(l) => l.to_owned(),
                None => SceneService::of(editor)?.store.snapshot().meta.level.level_path.to_string(),
            };
            compile_command(editor, &session, &level, None, a).await
        }
        "build_cform" | "build_aimap" | "build_game_graph" | "build_ggraph" | "build_all" | "play"
        | "play_game" | "make_all" | "make_game" | "make_details" | "make_hom" | "make_som"
        | "make_aimap" => {
            if busy {
                return Err(CmdError::busy(BUSY));
            }
            let act = action.to_owned();
            // Play's "Verify Space Restrictors" (the Level toolbar's Play menu)
            let verify = a.flag("verify_restrictors");
            // the root is checked before anything is built or written
            let place = if matches!(action, "play" | "play_game") {
                Some(play_place(editor, &session, a).await?)
            } else {
                None
            };
            let (ed, s) = (editor.clone(), session.clone());
            let title = match action {
                "build_cform" => "Building the collision",
                "build_aimap" => "Building the AI map",
                "build_game_graph" | "build_ggraph" => "Building the game graph",
                "build_all" | "make_all" => "Building the level",
                "make_game" => "Making the game files",
                "make_details" => "Making the details",
                "make_hom" => "Making the HOM",
                "make_som" => "Making the SOM",
                "make_aimap" => "Exporting the AI map",
                _ => "Building the scene for play",
            };
            let id = editor
                .jobs
                .start(JOB_BUILD, Some(JOB_BUILD), title, move |ctx| async move {
                    run_action(&ed, &s, &act, verify, place, &ctx).await
                })
                .map_err(|b| CmdError::busy(format!("{} is still running", b.title)))?;
            let info = editor
                .jobs
                .wait(id)
                .await
                .ok_or_else(|| CmdError::internal("the build job was forgotten"))?;
            match info.error {
                Some(e) => Err(CmdError::invalid(e)),
                None => Ok(info.result.map(|r| (*r).clone()).unwrap_or(json!({}))),
            }
        }
        other => Err(CmdError::invalid(format!("unknown action '{other}'"))),
    }
}

fn status(editor: &Arc<Editor>, session: &Arc<ProjectSession>) -> Reply {
    let lb = LevelBuild::of(session);
    let rebuild = match session.services.get::<SceneService>() {
        Some(svc) => lb.rebuild(&svc.store.snapshot()),
        None => service::Rebuild::default(),
    };
    Ok(json!({
        "rebuild": rebuild,
        "needs_rebuild": rebuild.any(),
        "rebuild_text": rebuild.text(),
        "running": lb.running().map(Running::name),
        "building": editor.jobs.running_in(JOB_BUILD).map(|j| j.stage),
    }))
}

/// The project's `gamedata`: an open project remounts `$game_levels$` and `$game_spawn$` there
/// (the C++ `EditorProject` mounts), so a level build lands in the project, never in the editor
/// or the game.
fn game_out(editor: &Editor) -> PathBuf {
    match editor.project_opt() {
        Some(s) => s.root.join("gamedata"),
        None => editor.paths.root.join("gamedata"),
    }
}

/// What every file the level build writes passes: nothing under the linked game.
fn write_guard(editor: &Editor) -> gz_editor::fsx::WriteGuard {
    gz_project::Project::of(editor).map(|(_, p)| p.write_guard(None)).unwrap_or_default()
}

/// `$game_levels$`.
fn levels_root(editor: &Editor) -> PathBuf {
    game_out(editor).join("levels")
}

/// `$game_levels$\<level>\`, or the sentence that refuses the level path.
fn level_dir(editor: &Editor, level: &str) -> Result<PathBuf, JobError> {
    crate::folder::level_folder(&levels_root(editor), level).map_err(JobError::new)
}

/// The level compilers on `level`'s folder of the project as the `level_run` job (`parts`: the
/// given ones, else the level and, when the build wrote `build.details`, the details; `details=`
/// decides that). Answers the compile's report when it ends, or at once with `wait=false`.
async fn compile_command(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    level: &str,
    parts: Option<Vec<compile::Part>>,
    a: &Args,
) -> Reply {
    if level.trim().is_empty() {
        return Err(CmdError::invalid(
            "No level to compile: name it (level=) or open a scene with a level path.",
        ));
    }
    let dir = crate::folder::level_folder(&levels_root(editor), level).map_err(CmdError::invalid)?;
    let (d2, guard, details) = (dir.clone(), write_guard(editor), a.bool("details"));
    let parts = editor
        .exec
        .io(move || {
            guard.check(&d2).map_err(|e| e.to_string())?;
            if !d2.join("build.prj").is_file() {
                return Err(format!("{} holds no build.prj: Make All writes it first.", d2.display()));
            }
            let parts = parts.unwrap_or_else(|| compile::parts_of(&d2, details));
            if parts.contains(&compile::Part::Details) && !d2.join("build.details").is_file() {
                return Err(format!(
                    "{} holds no build.details: Make Details writes it first.",
                    d2.display()
                ));
            }
            if parts == [compile::Part::Details] && !d2.join("build.cform").is_file() {
                return Err(format!("{} holds no build.cform: compile the level first.", d2.display()));
            }
            Ok(parts)
        })
        .await
        .map_err(CmdError::invalid)?;
    let flags = a.text("params").unwrap_or_default().trim().to_owned();
    let names: Vec<&str> = parts.iter().map(|p| p.name()).collect();
    editor.log.info(format!(
        "* Compile {level} ({}){}.",
        names.join(", "),
        if flags.is_empty() { String::new() } else { format!(" {flags}") }
    ));
    let req = compile::Request { level: level.to_owned(), dir, parts: parts.clone(), flags };
    let lb = LevelBuild::of(session);
    let ed = editor.clone();
    let id = editor
        .jobs
        .start(JOB_RUN, Some(GROUP_RUN), format!("Compiling {level}"), move |ctx| async move {
            *lb.running.lock() = Some((Running::Compile, ctx.id));
            let r = compile::run(&ed, req, &ctx).await;
            *lb.running.lock() = None;
            r
        })
        .map_err(|_| CmdError::busy(BUSY))?;
    let started = json!({ "started": Running::Compile.name(), "level": level, "parts": names, "job": id.0 });
    if a.bool("wait") == Some(false) {
        return Ok(started);
    }
    let info =
        editor.jobs.wait(id).await.ok_or_else(|| CmdError::internal("the compile job was forgotten"))?;
    match info.error {
        Some(e) => Err(CmdError::invalid(e)),
        None => {
            let mut out = started;
            out["report"] = info.result.map(|r| (*r).clone()).unwrap_or(json!({}));
            Ok(out)
        }
    }
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
        "* Run {line} (the installed engine {}, started in place in the game root {}).",
        exe.display(),
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
                    ed.log.error(format!("! {e}"));
                    Err(JobError::new(e))
                }
            }
        })
        .map_err(|_| CmdError::busy(BUSY))?;
    *lb.running.lock() = Some((Running::Game, id));
    Ok(json!({
        "started": Running::Game.name(),
        "command": line,
        "exe":root.exe.display().to_string().replace('\\', "/"),
    }))
}

/// The linked game's folder, or the sentence that says the project has none.
fn linked_game(editor: &Editor) -> Result<String, CmdError> {
    let (_, project) = gz_project::Project::of(editor)?;
    let root = project.game_root();
    if root.trim().is_empty() {
        return Err(CmdError::invalid(gz_project::NOT_LINKED));
    }
    Ok(root)
}

/// Where Play runs the game: its root (`root=`, else [`crate::play_root::default_root`]), what
/// the root checks compare with, the project's write guard and what the caller adds to the
/// command line (`params=`).
struct PlayPlace {
    root: PathBuf,
    params: String,
    guard: gz_editor::fsx::WriteGuard,
    context: Arc<DeleteContext>,
}

/// What a Play root is checked against: the deletion's guards of this machine
/// ([`gz_project::system_guards`]), the open project and every listed one.
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
/// (and again by `prepare`): an engine that does not run in place is refused here.
async fn play_place(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    a: &Args,
) -> Result<PlayPlace, CmdError> {
    let root = match a.text("root").filter(|r| !r.trim().is_empty()) {
        Some(r) => PathBuf::from(r),
        // an editor with a home of its own (-home, GRIZZLY_HOME) plays under it
        None => {
            let home = (editor.paths.home != editor.paths.root).then_some(editor.paths.home.as_path());
            crate::play_root::default_root(&session.root, home)
        }
    };
    let game = PathBuf::from(linked_game(editor)?);
    let context = Arc::new(play_context(editor).await);
    let (r, cx) = (root.clone(), context.clone());
    editor
        .exec
        .io(move || {
            crate::play::ready_engine(&game, &crate::play::LINKED_GAME)?;
            crate::play_root::check_root(&cx, &r)
        })
        .await
        .map_err(CmdError::invalid)?;
    Ok(PlayPlace {
        root,
        params: a.text("params").unwrap_or_default().to_owned(),
        guard: write_guard(editor),
        context,
    })
}

/// `play_root`'s answer: the root, what it was, the installed engine it runs.
fn root_answer(r: &crate::play::PlayRoot) -> serde_json::Value {
    json!({
        "root": r.root.display().to_string().replace('\\', "/"),
        "was": r.was,
        "exe": r.exe.file_name().map(|n| n.to_string_lossy().into_owned()),
        "exe_path": r.exe.display().to_string().replace('\\', "/"),
        "path_limit": r.path_limit,
        "walked": r.walked,
        "walk_ms": r.walk_ms,
    })
}

/// What a build reads: the snapshot, the library objects it names, the engine libraries, the
/// game's configs.
struct Inputs {
    snap: Arc<Snap<SceneState>>,
    lib: Arc<gz_scene::library::Library>,
    engine: Arc<Engine>,
    settings: Option<Arc<gz_scene::game::GameSettings>>,
}

async fn inputs(editor: &Arc<Editor>, ctx: &JobCtx) -> Result<Inputs, JobError> {
    let svc = SceneService::of(editor).map_err(|e| JobError::new(e.message))?;
    let snap = svc.store.snapshot();
    if !snap.valid {
        return Err(JobError::new("no scene"));
    }
    ctx.stage("Loading the library objects");
    svc.prefetch_library(editor, &snap).await;
    ctx.check()?;
    ctx.stage("Reading the engine libraries");
    let read = |rel: &'static str| {
        let ed = editor.clone();
        async move { gz_game::read_game_or_bundled(&ed, rel).await.ok().map(|(b, _)| b) }
    };
    let (sh, xr, gm) = (read("shaders.xr").await, read("shaders_xrlc.xr").await, read("gamemtl.xr").await);
    let engine =
        Arc::new(editor.exec.cpu(move || Engine::read(sh.as_deref(), xr.as_deref(), gm.as_deref())).await);
    let settings = gz_scene::game::settings(editor).await;
    Ok(Inputs { snap, lib: svc.library.clone(), engine, settings })
}

fn sync_settings(s: &Option<Arc<gz_scene::game::GameSettings>>) -> Option<&(dyn Settings + Sync)> {
    s.as_deref().map(|g| g as &(dyn Settings + Sync))
}

/// The graph points of the scene (`load_graph_points`).
fn graph_points(scene: &SceneState, settings: Option<&(dyn Settings + Sync)>) -> Vec<GraphPoint> {
    let mut out = Vec::new();
    for o in scene.of_class(ObjClass::SpawnPoint) {
        let Ok(Some((e, _))) = files::export_entity(o, settings) else { continue };
        if e.class_info().map(|c| c.class) != Some(gz_entities::Class::GraphPoint) {
            continue;
        }
        let Some(gp) = e.object().and_then(|ob| ob.graph_point()) else { continue };
        out.push(GraphPoint {
            name: o.name.to_string(),
            position: o.position(),
            locations: gp.locations,
            connection_level: gp.connection_level_name.to_string(),
            connection_point: gp.connection_point_name.to_string(),
        });
    }
    out
}

fn guid() -> [u8; 16] {
    let t =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut g = [0u8; 16];
    g.copy_from_slice(&(t ^ (u128::from(std::process::id()) << 64)).to_le_bytes());
    g
}

/// `EScene::BuildCForm`.
async fn ensure_cform(editor: &Arc<Editor>, lb: &Arc<LevelBuild>, inp: &Inputs) -> Result<(), JobError> {
    let key = service::cform_key(&inp.snap);
    if lb.state.lock().cform.as_ref().is_some_and(|b| b.key == key) {
        return Ok(());
    }
    let (snap, lib, engine) = (inp.snap.clone(), inp.lib.clone(), inp.engine.clone());
    let form = editor.exec.cpu(move || files::collision_form(&snap, &files::lookup(lib), &engine)).await;
    let Some(form) = form else {
        editor.log.warn("! CForm is empty!");
        return Err(JobError::new("CFrom is empty!\nSee log."));
    };
    lb.state.lock().cform = Some(Built { key, value: Arc::new(form) });
    Ok(())
}

/// `EScene::BuildAIMap`: also forgets the game graph.
async fn ensure_ai(
    editor: &Arc<Editor>,
    lb: &Arc<LevelBuild>,
    inp: &Inputs,
) -> Result<Arc<gz_fmt_level::LevelGraph>, JobError> {
    let key = service::ai_key(&inp.snap);
    if let Some(b) = lb.state.lock().level_graph.as_ref().filter(|b| b.key == key) {
        return Ok(b.value.clone());
    }
    let snap = inp.snap.clone();
    let lg = editor
        .exec
        .cpu(move || match files::ai_nodes(&snap) {
            Some((nodes, params)) => graph::level_graph(&nodes, &params, guid()),
            None => Err(crate::BuildError::new("! AIMap is empty!")),
        })
        .await
        .map_err(|e| {
            editor.log.warn(&e.0);
            JobError::new("AIMap is empty!\nSee log.")
        })?;
    let lg = Arc::new(lg);
    let mut st = lb.state.lock();
    st.level_graph = Some(Built { key, value: lg.clone() });
    st.game_graph = None;
    Ok(lg)
}

/// `EScene::BuildGameGraph`.
async fn ensure_graph(
    editor: &Arc<Editor>,
    lb: &Arc<LevelBuild>,
    inp: &Inputs,
) -> Result<Arc<LevelGameGraph>, JobError> {
    let lg = ensure_ai(editor, lb, inp).await?;
    let key = service::graph_key(&inp.snap);
    if let Some(b) = lb.state.lock().game_graph.as_ref().filter(|b| b.key == key) {
        return Ok(b.value.clone());
    }
    let (snap, settings) = (inp.snap.clone(), inp.settings.clone());
    let level = inp.snap.meta.level.level_path.to_string();
    let built = editor
        .exec
        .cpu(move || {
            let nav = LevelNav::new(&lg);
            let points = graph_points(&snap, sync_settings(&settings));
            graph::build_game_graph(&nav, &points, &level, lg.header.guid, guid())
        })
        .await;
    let g = match built {
        Ok(g) => g,
        Err(e) => {
            editor.log.warn(&e.0);
            return Err(JobError::new("GameGraph is empty!\nSee log."));
        }
    };
    for l in &g.log {
        editor.log.info(l);
    }
    let g = Arc::new(g);
    lb.state.lock().game_graph = Some(Built { key, value: g.clone() });
    Ok(g)
}

async fn write_files(
    editor: &Arc<Editor>,
    dir: PathBuf,
    files: Vec<(String, Vec<u8>)>,
) -> Result<(), JobError> {
    let guard = write_guard(editor);
    editor
        .exec
        .io(move || -> Result<(), String> {
            guard.check(&dir).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to prepare level folders: {e}"))?;
            for (name, bytes) in files {
                let p = dir.join(&name);
                if let Some(parent) = p.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                gz_editor::fsx::write_atomic(&p, &bytes)
                    .map_err(|e| format!("Can't write {}: {e}", p.display()))?;
            }
            Ok(())
        })
        .await
        .map_err(JobError::new)
}

/// The LOD pictures the scene's multiple-usage objects name, read from the game's textures.
async fn lod_pictures(editor: &Arc<Editor>, inp: &Inputs) -> HashMap<String, (Vec<u8>, u32, u32)> {
    let mut names: Vec<String> = Vec::new();
    for o in inp.snap.of_class(ObjClass::SceneObject) {
        let ObjData::SceneObject(d) = &o.data else { continue };
        let Some(lo) = inp.lib.cached(&d.reference) else { continue };
        if lo.object.flags.contains(ObjectFlags::USING_LOD)
            && lo.object.flags.contains(ObjectFlags::MULTIPLE_USAGE)
        {
            let n = prj::lod_texture_name(&lo.name);
            names.push(format!("{n}_nm"));
            names.push(n);
        }
    }
    names.sort();
    names.dedup();
    let mut out = HashMap::new();
    for n in names {
        let rel = format!("textures\\{n}.dds");
        let Ok((bytes, _)) = gz_game::read_game_or_bundled(editor, &rel).await else { continue };
        let decoded = editor.exec.cpu(move || gz_fmt_tex::read_dds_level(&bytes, 0, 0).ok()).await;
        if let Some(img) = decoded {
            out.insert(n, (img.rgba, img.width, img.height));
        }
    }
    out
}

/// The detail objects' textures by name, as RGBA pixels: `textures\<name>.dds` of the editor's own
/// `gamedata` - what `CreateMergedTexture` reads from `$game_textures$` on disk, which no project
/// mount and no game link moves. The linked game's are larger and would not fit the atlas.
async fn textures(editor: &Arc<Editor>, names: Vec<String>) -> HashMap<String, (Vec<u8>, u32, u32)> {
    let bundled = gz_game::Bundled::of(editor);
    let mut out = HashMap::new();
    for n in names {
        let rel = format!("textures\\{n}.dds");
        let Some(bytes) = bundled.read(&rel).await else { continue };
        let decoded = editor.exec.cpu(move || gz_fmt_tex::read_dds_level(&bytes, 0, 0).ok()).await;
        if let Some(img) = decoded {
            out.insert(n, (img.rgba, img.width, img.height));
        }
    }
    out
}

/// The LOD textures the scene's multiple-usage objects need (`lod\lod_x.dds`, `..._nm.dds`):
/// the path of each under the editor's `gamedata\textures` (the C++ `$game_textures$`) and
/// whether the linked game or that folder has it.
async fn lod_texture_files(editor: &Arc<Editor>, inp: &Inputs) -> HashMap<String, (String, bool)> {
    let mut names: Vec<String> = Vec::new();
    for o in inp.snap.of_class(ObjClass::SceneObject) {
        let ObjData::SceneObject(d) = &o.data else { continue };
        if d.visual_mode() {
            continue;
        }
        let Some(lo) = inp.lib.cached(&d.reference) else { continue };
        if crate::validate::is_mu_static(&lo) {
            names.extend(crate::validate::lod_texture_files(&lo.name));
        }
    }
    names.sort();
    names.dedup();
    let game = match GameLink::of(editor) {
        Ok(link) => link.files().await.ok(),
        Err(_) => None,
    };
    let bundled = gz_game::Bundled::of(editor);
    let mut out = HashMap::new();
    for n in names {
        let rel = format!("textures\\{n}");
        let path = bundled.path(&rel);
        let in_game = game.as_ref().is_some_and(|f| f.exists(&rel));
        let p2 = path.clone();
        let there = in_game || editor.exec.io(move || p2.is_file()).await;
        out.insert(n, (path.display().to_string(), there));
    }
    out
}

/// `SceneBuilder::Compile`: the build project and every file beside it.
/// `Scene->Validate` before a build: its lines go to the log, and a scene that fails is not
/// built (`VERIFY_COMPILE(.., "Validation failed.", "Invalid scene.")`).
async fn validate_scene(
    editor: &Arc<Editor>,
    inp: &Inputs,
    tests: crate::validate::Tests,
    ctx: &JobCtx,
) -> Result<(), JobError> {
    ctx.stage("Validating the scene");
    let lods = if tests.full { lod_texture_files(editor, inp).await } else { HashMap::new() };
    let (snap, lib, engine) = (inp.snap.clone(), inp.lib.clone(), inp.engine.clone());
    let checked = editor
        .exec
        .cpu(move || {
            let texture = |n: &str| lods.get(n).cloned().unwrap_or_else(|| (format!("textures\\{n}"), false));
            crate::validate::validate(&snap, &lib, &engine, tests, &texture)
        })
        .await;
    let (lines, ok) = match checked {
        Ok(l) => (l, true),
        Err(l) => (l, false),
    };
    for line in &lines {
        if line.starts_with('*') || line.starts_with('!') {
            editor.log.warn(line);
        } else {
            editor.log.info(line);
        }
    }
    if ok { Ok(()) } else { Err(JobError::new("ERROR: Validation failed. Invalid scene.")) }
}

async fn compile_level(
    editor: &Arc<Editor>,
    inp: &Inputs,
    ctx: &JobCtx,
) -> Result<serde_json::Value, JobError> {
    validate_scene(editor, inp, crate::validate::Tests::BUILD, ctx).await?;
    let level = inp.snap.meta.level.level_path.to_string();
    if level.is_empty() {
        return Err(JobError::new("ERROR: Failed to prepare level path "));
    }
    let dir = level_dir(editor, &level)?;
    ctx.stage("Preparing the level folder");
    let (d2, base, guard) = (dir.clone(), levels_root(editor), write_guard(editor));
    editor
        .exec
        .io(move || {
            guard.check(&d2).map_err(|e| e.to_string())?;
            crate::folder::check_wipe(&base, &d2)?;
            if d2.is_dir() {
                gz_build::install::wipe_tree(&d2);
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(JobError::new)?;
    ctx.check()?;
    ctx.stage("Reading the LOD textures");
    let lods = Arc::new(lod_pictures(editor, inp).await);
    ctx.check()?;
    let detail_names = match &inp.snap.bulk.details {
        Some(d) => crate::details::used_textures(&d.model, &files::lookup(inp.lib.clone())),
        None => Vec::new(),
    };
    let detail_textures = Arc::new(textures(editor, detail_names).await);
    ctx.check()?;
    ctx.stage("Compiling the static geometry");
    let (snap, lib, engine, settings) =
        (inp.snap.clone(), inp.lib.clone(), inp.engine.clone(), inp.settings.clone());
    let made = editor
        .exec
        .cpu(move || -> crate::Result<(NamedFiles, Vec<String>)> {
            let look = files::lookup(lib);
            let lod_src = |n: &str| lods.get(n).cloned();
            let c = prj::compile(&snap, &look, &engine, &lod_src)?;
            let mut out: Vec<(String, Vec<u8>)> = vec![("build.prj".to_owned(), c.project.write())];
            if let Some((color, normal)) = c.lod_textures {
                out.push((format!("{}.dds", prj::LEVEL_LODS_TEX_NAME), color));
                out.push(("level_lods_nm.dds".to_owned(), normal));
            }
            out.push(("level.ltx".to_owned(), files::level_ltx(&snap, &look)));
            let g = files::game_files(&snap, &look, &engine, sync_settings(&settings))?;
            for (n, b) in g.files {
                out.push((n.to_owned(), b));
            }
            out.extend(g.meshes);
            let (tga, txt) = files::scene_stat(&c.stat);
            out.push(("level_stat.tga".to_owned(), tga));
            out.push(("level_stat.txt".to_owned(), txt));
            if let Some(h) = files::hom(&snap, &look) {
                out.push(("level.hom".to_owned(), h));
            }
            if let Some(s) = files::som(&snap, &look, &engine)? {
                out.push(("level.som".to_owned(), s));
            }
            let mut log = c.log;
            log.extend(g.log);
            match snap.bulk.details.as_ref().filter(|d| !d.model.slots.is_empty()) {
                Some(d) => {
                    let tex = |n: &str| detail_textures.get(n).cloned();
                    let (file, dds) = crate::details::export(&d.model, &look, &tex).map_err(|e| {
                        crate::BuildError::new(format!("ERROR: Detail Objects export failed. {}", e.0))
                    })?;
                    out.push(("build.details".to_owned(), file));
                    out.push((format!("{}.dds", crate::details::DETAILS_TEX_NAME), dds));
                    log.push("Process Detail Objects - done.".to_owned());
                }
                None => log.push("Process Detail Objects - failed.".to_owned()),
            }
            let marks =
                snap.bulk.wallmarks.as_ref().map(|w| crate::wallmarks::read(&w.raw)).unwrap_or_default();
            if marks.is_empty() {
                log.push("Process Wallmarks - failed.".to_owned());
            } else {
                out.push(("level.wallmarks".to_owned(), crate::wallmarks::write(&marks)));
                log.push("Process Wallmarks - done.".to_owned());
            }
            match files::build_aimap(&snap) {
                Some(b) => {
                    out.push(("build.aimap".to_owned(), b));
                    log.push("Process AI Map - done.".to_owned());
                }
                None => log.push("Process AI Map - failed.".to_owned()),
            }
            Ok((out, log))
        })
        .await
        .map_err(|e| JobError::new(e.0))?;
    for l in &made.1 {
        if l.starts_with('!') { editor.log.warn(l) } else { editor.log.info(l) }
    }
    ctx.check()?;
    ctx.stage("Writing the level files");
    let names: Vec<String> = made.0.iter().map(|(n, _)| n.clone()).collect();
    write_files(editor, dir.clone(), made.0).await?;
    Ok(json!({ "level": level, "dir": dir.display().to_string().replace('\\', "/"), "files": names }))
}

/// `EScene::BuildForPCPlay`: the level must be compiled; the collision, the AI map and the game
/// graph are made when they are not there; `level.ai.temp` and `editor.spawn` are written.
async fn build_for_play(
    editor: &Arc<Editor>,
    lb: &Arc<LevelBuild>,
    inp: &Inputs,
    verify_restrictors: bool,
    ctx: &JobCtx,
) -> Result<Staged, JobError> {
    let level = inp.snap.meta.level.level_path.to_string();
    let dir = level_dir(editor, &level)?;
    let d2 = dir.clone();
    let built = editor
        .exec
        .io(move || ["level", "level.ltx", "level.geom", "level.cform"].iter().all(|f| d2.join(f).is_file()))
        .await;
    if !built {
        return Err(JobError::new("Level is not builded!"));
    }
    ctx.stage("Building the collision");
    ensure_cform(editor, lb, inp).await?;
    ctx.stage("Building the AI map");
    let lg = ensure_ai(editor, lb, inp).await?;
    ctx.stage("Building the game graph");
    let gg = ensure_graph(editor, lb, inp).await?;
    ctx.check()?;
    ctx.stage("Joining the linked game's graph");
    let base = linked_game_graph(editor).await?;
    let own = gg.graph.clone();
    let joined =
        editor.exec.cpu(move || graph::join_base(&base, &own)).await.map_err(|e| JobError::new(e.0))?;
    let play_graph = match joined {
        Some(g) => Arc::new(g),
        None => {
            editor.log.warn(format!(
                "* {level} is a level of the linked game: Play keeps the level's own game graph, and the game's scripts that name its vertices find none of them."
            ));
            Arc::new(gg.graph.clone())
        }
    };
    ctx.check()?;
    ctx.stage("Building the spawn");
    let (snap, settings) = (inp.snap.clone(), inp.settings.clone());
    let lv = level.clone();
    let lg2 = lg.clone();
    let built = editor
        .exec
        .cpu(move || {
            let nav = LevelNav::new(&lg2);
            let st = sync_settings(&settings);
            let mut elements = Vec::new();
            for o in snap.of_class(ObjClass::SpawnPoint) {
                if let ObjData::Spawn(pt) = &o.data
                    && pt.kind == PointKind::Spawn
                    && let Ok(Some((e, _))) = files::export_entity(o, st)
                {
                    elements.push(Element { entity: e });
                }
            }
            let ways: Vec<Way> = snap
                .of_class(ObjClass::Way)
                .filter_map(|o| match &o.data {
                    ObjData::Way(w) => Some(Way {
                        name: o.name.to_string(),
                        points: w.points.iter().map(|p| (p.name.to_string(), p.position, p.flags)).collect(),
                        links: w
                            .points
                            .iter()
                            .enumerate()
                            .flat_map(|(i, p)| p.links.iter().map(move |(t, pr)| (i as u32, *t, *pr)))
                            .collect(),
                    }),
                    _ => None,
                })
                .collect();
            spawn::build_spawn(&lv, &nav, &play_graph, elements, &ways, guid(), verify_restrictors)
        })
        .await
        .map_err(|e| {
            editor.log.warn(&e.0);
            JobError::new("! Failed build spawn! \nSee log.")
        })?;
    for l in &built.log {
        editor.log.info(l);
    }
    let ai_temp = lg.write().map_err(|e| JobError::new(e.to_string()))?;
    write_files(editor, dir.clone(), vec![("level.ai.temp".to_owned(), ai_temp.clone())])
        .await
        .map_err(|_| JobError::new("level.ai.temp can't save!\nSee log."))?;
    let spawns = game_out(editor).join("spawns");
    write_files(editor, spawns, vec![("editor.spawn".to_owned(), built.bytes.clone())]).await?;
    Ok(Staged { level, dir, ai: ai_temp, spawn: built.bytes })
}

/// The longest path of the linked game's files, relative to its gamedata: what the game indexes
/// under a game root's `gamedata` besides what the root holds itself.
async fn longest_game_path(editor: &Arc<Editor>) -> String {
    let files = match GameLink::of(editor) {
        Ok(link) => link.files().await.ok(),
        Err(_) => None,
    };
    files
        .and_then(|f| f.iter().map(|r| r.path).max_by_key(|p| p.chars().count()).map(str::to_owned))
        .unwrap_or_default()
}

/// The linked game's game graph: chunk 4 of its `spawns\all.spawn`, read and never written.
async fn linked_game_graph(editor: &Arc<Editor>) -> Result<gz_fmt_spawn::GameGraph, JobError> {
    let link = GameLink::of(editor).map_err(|e| JobError::new(e.to_string()))?;
    let files = link.files().await.map_err(|e| JobError::new(e.to_string()))?;
    let bytes =
        editor.exec.io(move || files.read("spawns\\all.spawn")).await.map_err(|e| {
            JobError::new(format!("The linked game's spawns\\all.spawn could not be read: {e}"))
        })?;
    editor
        .exec
        .cpu(move || gz_fmt_spawn::SpawnFile::parse(Arc::from(bytes)).map(|f| f.game_graph))
        .await
        .map_err(|e| JobError::new(format!("The linked game's spawns\\all.spawn could not be read: {e}")))?
        .ok_or_else(|| JobError::new("The linked game's spawns\\all.spawn holds no game graph."))
}

/// What Play stages into its game root: the level built, its AI map, the spawn.
struct Staged {
    level: String,
    dir: PathBuf,
    ai: Vec<u8>,
    spawn: Vec<u8>,
}

/// `MakeGame`, `MakeDetails`, `MakeHOM`, `MakeSOM`, `MakeAIMap`: one part of the level files,
/// into the level folder as it is.
async fn make_part(
    editor: &Arc<Editor>,
    inp: &Inputs,
    action: &str,
    ctx: &JobCtx,
) -> Result<serde_json::Value, JobError> {
    if action == "make_game" {
        validate_scene(editor, inp, crate::validate::Tests::MAKE_GAME, ctx).await?;
    }
    let level = inp.snap.meta.level.level_path.to_string();
    if level.is_empty() {
        return Err(JobError::new("ERROR: Failed to prepare level path. "));
    }
    let dir = level_dir(editor, &level)?;
    let detail_textures = if action == "make_details" {
        let names = match &inp.snap.bulk.details {
            Some(d) => crate::details::used_textures(&d.model, &files::lookup(inp.lib.clone())),
            None => Vec::new(),
        };
        Arc::new(textures(editor, names).await)
    } else {
        Arc::new(HashMap::new())
    };
    ctx.check()?;
    let (snap, lib, engine, settings, act) =
        (inp.snap.clone(), inp.lib.clone(), inp.engine.clone(), inp.settings.clone(), action.to_owned());
    let (made, log): (NamedFiles, Vec<String>) = editor
        .exec
        .cpu(move || -> crate::Result<(NamedFiles, Vec<String>)> {
            let look = files::lookup(lib);
            let mut log = Vec::new();
            let made = match act.as_str() {
                "make_game" => {
                    let mut out = vec![("level.ltx".to_owned(), files::level_ltx(&snap, &look))];
                    let g = files::game_files(&snap, &look, &engine, sync_settings(&settings))?;
                    out.extend(g.files.into_iter().map(|(n, b)| (n.to_owned(), b)));
                    out.extend(g.meshes);
                    log = g.log;
                    out
                }
                "make_details" => {
                    let d = snap
                        .bulk
                        .details
                        .as_ref()
                        .filter(|d| !d.model.slots.is_empty())
                        .ok_or_else(|| crate::BuildError::new("ERROR: Export failed. "))?;
                    let tex = |n: &str| detail_textures.get(n).cloned();
                    let (file, dds) = crate::details::export(&d.model, &look, &tex)?;
                    vec![
                        ("build.details".to_owned(), file),
                        (format!("{}.dds", crate::details::DETAILS_TEX_NAME), dds),
                    ]
                }
                "make_hom" => vec![(
                    "level.hom".to_owned(),
                    files::hom(&snap, &look)
                        .ok_or_else(|| crate::BuildError::new("ERROR: Failed to build HOM model. "))?,
                )],
                "make_som" => vec![(
                    "level.som".to_owned(),
                    files::som(&snap, &look, &engine)?
                        .ok_or_else(|| crate::BuildError::new("ERROR: Failed to build SOM model. "))?,
                )],
                _ => vec![(
                    "build.aimap".to_owned(),
                    files::build_aimap(&snap)
                        .ok_or_else(|| crate::BuildError::new("ERROR: Failed to build AI-Map. "))?,
                )],
            };
            Ok((made, log))
        })
        .await
        .map_err(|e| JobError::new(e.0))?;
    for l in &log {
        if l.starts_with('!') { editor.log.warn(l) } else { editor.log.info(l) }
    }
    let names: Vec<String> = made.iter().map(|(n, _)| n.clone()).collect();
    write_files(editor, dir, made).await?;
    Ok(json!({ "level": level, "files": names }))
}

async fn run_action(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    action: &str,
    verify_restrictors: bool,
    place: Option<PlayPlace>,
    ctx: &JobCtx,
) -> Result<serde_json::Value, JobError> {
    let lb = LevelBuild::of(session);
    let inp = inputs(editor, ctx).await?;
    match action {
        "build_cform" => {
            ctx.stage("Building the collision");
            ensure_cform(editor, &lb, &inp).await?;
            let st = lb.state.lock();
            let (v, t) = st.cform.as_ref().map_or((0, 0), |b| (b.value.verts.len(), b.value.tris.len()));
            Ok(json!({ "vertices": v, "tris": t }))
        }
        "build_aimap" => {
            ctx.stage("Building the AI map");
            let lg = ensure_ai(editor, &lb, &inp).await?;
            Ok(json!({ "nodes": lg.nodes.len() }))
        }
        "build_game_graph" | "build_ggraph" => {
            ctx.stage("Building the game graph");
            let g = ensure_graph(editor, &lb, &inp).await?;
            Ok(json!({ "vertices": g.graph.vertices.len(), "edges": g.graph.edges.len() }))
        }
        "make_all" => {
            let r = compile_level(editor, &inp, ctx).await?;
            editor.log.info("Building OK.");
            Ok(json!({ "build": r }))
        }
        "make_game" | "make_details" | "make_hom" | "make_som" | "make_aimap" => {
            make_part(editor, &inp, action, ctx).await
        }
        "build_all" => {
            let r = compile_level(editor, &inp, ctx).await?;
            editor.log.info("Building OK.");
            // `RunXrLC` after the build: here the level compiler in the same job, so Stop stops it
            let level = inp.snap.meta.level.level_path.to_string();
            let dir = level_dir(editor, &level)?;
            let req =
                compile::Request { level, dir, parts: vec![compile::Part::Level], flags: String::new() };
            let report = compile::run(editor, req, ctx).await?;
            Ok(json!({ "build": r, "compile": report }))
        }
        _ => {
            let staged = build_for_play(editor, &lb, &inp, verify_restrictors, ctx).await?;
            let place = place.ok_or_else(|| JobError::new("Play has no game root"))?;
            ctx.stage("Preparing the game");
            let game = PathBuf::from(linked_game(editor).map_err(|e| JobError::new(e.message))?);
            let archived = longest_game_path(editor).await;
            let (root, guard, context) = (place.root.clone(), place.guard.clone(), place.context.clone());
            let ready = editor
                .exec
                .io(move || {
                    guard.check(&root).map_err(|e| e.to_string())?;
                    let r = crate::play::prepare(&crate::play::PlayInput {
                        game: &game,
                        root: &root,
                        context: &context,
                        archived: &archived,
                        level: Some((&staged.level, &staged.dir)),
                    })?;
                    crate::play::stage_level(&r.root, &staged.level, &staged.dir, &staged.ai, &staged.spawn)?;
                    Ok::<_, String>(r)
                })
                .await
                .map_err(JobError::new)?;
            let params = format!("{PLAY_PARAMS} {}", place.params);
            let started = start_game(editor, session, &ready, params.trim_end())
                .map_err(|e| JobError::new(e.message))?;
            Ok(json!({
                "run": started,
                "root": ready.root.display().to_string().replace('\\', "/"),
            }))
        }
    }
}

// ---- Scene > Validate, Scene > Export ... as OBJ ---------------------------------------------

/// `scene_validate`: `CommandValidateScene` - `EScene::Validate(true, ...)`, every test, as a job
/// of the build group. Answers `ok` and the lines it wrote to the log.
pub(crate) async fn scene_validate(editor: &Arc<Editor>) -> Reply {
    let ed = editor.clone();
    let id = editor
        .jobs
        .start("scene_validate", Some(JOB_BUILD), "Validating the scene", move |ctx| async move {
            let inp = inputs(&ed, &ctx).await?;
            let lods = lod_texture_files(&ed, &inp).await;
            let (snap, lib, engine) = (inp.snap.clone(), inp.lib.clone(), inp.engine.clone());
            let checked = ed
                .exec
                .cpu(move || {
                    let texture =
                        |n: &str| lods.get(n).cloned().unwrap_or_else(|| (format!("textures\\{n}"), false));
                    crate::validate::validate(&snap, &lib, &engine, crate::validate::Tests::BUILD, &texture)
                })
                .await;
            let (mut lines, ok) = match checked {
                Ok(l) => (l, true),
                Err(l) => (l, false),
            };
            lines.push(if ok { "Validation OK!" } else { "Validation FAILED!" }.to_owned());
            for line in &lines {
                if line.starts_with('*') || line.starts_with('!') {
                    ed.log.warn(line);
                } else {
                    ed.log.info(line);
                }
            }
            Ok(json!({ "ok": ok, "lines": lines }))
        })
        .map_err(|b| CmdError::busy(format!("{} is still running", b.title)))?;
    let info =
        editor.jobs.wait(id).await.ok_or_else(|| CmdError::internal("the validation job was forgotten"))?;
    match info.error {
        Some(e) => Err(CmdError::invalid(e)),
        None => Ok(info.result.map(|r| (*r).clone()).unwrap_or(json!({}))),
    }
}

/// `scene_export_obj`: `EScene::ExportObj(selected)` - the static geometry of the scene (the
/// selected objects with `selected=true`) as a Wavefront `.obj` and its `.mtl`, in metres x 100
/// with Z turned round, as `SaveBuildAsObject` writes them. `file=` names the `.obj` (absolute,
/// or under the project's `import\`).
pub(crate) async fn scene_export_obj(editor: &Arc<Editor>, a: &Args) -> Reply {
    let selected = a.flag("selected");
    let session = editor.project()?;
    let raw = PathBuf::from(a.need("file")?.replace('/', "\\"));
    let file =
        if raw.is_absolute() { raw } else { session.root.join("import").join(raw) }.with_extension("obj");
    let ed = editor.clone();
    let out = file.clone();
    let id = editor
        .jobs
        .start("scene_export_obj", Some(JOB_BUILD), "Exporting the scene as OBJ", move |ctx| async move {
            let inp = inputs(&ed, &ctx).await?;
            ctx.stage("Building the geometry");
            let stem = out.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let (snap, lib, engine) = (inp.snap.clone(), inp.lib.clone(), inp.engine.clone());
            let made = ed
                .exec
                .cpu(move || {
                    let look = files::lookup(lib);
                    prj::export_obj(&snap, &look, &engine, selected, &stem)
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
            Ok(json!({ "file": out.display().to_string().replace('\\', "/"), "objects": objects }))
        })
        .map_err(|b| CmdError::busy(format!("{} is still running", b.title)))?;
    let info =
        editor.jobs.wait(id).await.ok_or_else(|| CmdError::internal("the export job was forgotten"))?;
    match info.error {
        Some(e) => Err(CmdError::invalid(e)),
        None => Ok(info.result.map(|r| (*r).clone()).unwrap_or(json!({}))),
    }
}
