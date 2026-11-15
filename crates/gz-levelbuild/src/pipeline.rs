//! The level build as one job: the steps a request needs, in order, each recorded in
//! `build.state` as it ends, so a Stop or a failure keeps the finished ones fresh.
//!
//! Before the first step runs, what a later step would refuse and can be seen at once is checked
//! ([`preflight`]): the build refuses in seconds with every such problem on its object, instead
//! of failing after the long steps. An error ends the build at once, like Unreal's: the finished
//! steps stay, and the job ends failed naming the step that failed and the steps not run. A
//! warning never stops it. Stop ends the step in progress within a moment: every long loop of the
//! steps asks the job's cancel flag ([`crate::Stop`]).
//!
//! On a level the linked game has (a stock level), the compile steps are not needed for the
//! module unless they are asked for by name or forced: Build Level builds the spawn layer, and
//! what an earlier build left in the level's folder is a warning ([`crate::stock`]).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use gz_base::{Finding, Severity};
use gz_editor::{Editor, JobCtx, JobError, ProjectSession, json};
use gz_entities::Settings;
use gz_fmt_object::ObjectFlags;
use gz_game::GameLink;
use gz_scene::objects::PointKind;
use gz_scene::{ObjClass, ObjData, SceneService, SceneState};
use gz_store::Snap;
use serde_json::Value;

use crate::fingerprint::{self, Objects};
use crate::graph::{self, GraphPoint};
use crate::libs::Engine;
use crate::nav::LevelNav;
use crate::prj;
use crate::service::{self, LevelBuild, LevelProblem, StepState};
use crate::spawn::{self, Element, Way};
use crate::state::{BuildState, Generated, StepRecord};
use crate::steps::{Options, Step, with_needs};

/// A step's error as the job's: a stopped one is the job's Stop.
fn job_err(e: crate::BuildError) -> JobError {
    if e.is_stopped() {
        JobError { message: crate::STOPPED.to_owned(), cancelled: true }
    } else {
        JobError::new(e.0)
    }
}

/// Files by name, as a step writes them into the level folder.
type NamedFiles = Vec<(String, Vec<u8>)>;

/// What a build is asked to do.
#[derive(Clone, Debug, Default)]
pub struct Request {
    /// The steps to consider, with what they need; `None`: every step.
    pub steps: Option<Vec<Step>>,
    /// One step and what it needs.
    pub only: Option<Step>,
    /// Run the steps asked for even when they are fresh.
    pub force: bool,
    pub opts: Options,
    /// Play's check of the space restrictors in the spawn.
    pub verify_restrictors: bool,
}

/// What a build reads: the snapshot, the library objects it names, the engine libraries, the
/// game's configs.
pub(crate) struct Inputs {
    pub snap: Arc<Snap<SceneState>>,
    pub lib: Arc<gz_scene::library::Library>,
    pub engine: Arc<Engine>,
    pub settings: Option<Arc<gz_scene::game::GameSettings>>,
}

pub(crate) async fn inputs(editor: &Arc<Editor>, ctx: &JobCtx) -> Result<Inputs, JobError> {
    let svc = SceneService::of(editor).map_err(|e| JobError::new(e.message))?;
    let snap = svc.store.snapshot();
    if !snap.valid {
        return Err(JobError::new("No scene is open."));
    }
    ctx.stage("Loading the library objects");
    // a library of the build's own: every step reads the models as they were loaded here
    let lib = svc.library_for(editor, &snap).await;
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
    Ok(Inputs { snap, lib, engine, settings })
}

pub(crate) fn sync_settings(s: &Option<Arc<gz_scene::game::GameSettings>>) -> Option<&(dyn Settings + Sync)> {
    s.as_deref().map(|g| g as &(dyn Settings + Sync))
}

/// The graph points of the scene.
fn graph_points(scene: &SceneState, settings: Option<&(dyn Settings + Sync)>) -> Vec<GraphPoint> {
    let mut out = Vec::new();
    for o in scene.of_class(ObjClass::SpawnPoint) {
        let Ok(Some((e, _))) = crate::files::export_entity(o, settings) else { continue };
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

pub(crate) fn guid() -> [u8; 16] {
    let t =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut g = [0u8; 16];
    g.copy_from_slice(&(t ^ (u128::from(std::process::id()) << 64)).to_le_bytes());
    g
}

/// What every file the level build writes passes: nothing under the linked game.
pub(crate) fn write_guard(editor: &Editor) -> gz_editor::fsx::WriteGuard {
    gz_project::Project::of(editor).map(|(_, p)| p.write_guard(None)).unwrap_or_default()
}

/// The level folder of `level` in the project, or the sentence that refuses the level path.
pub(crate) fn level_dir(session: &ProjectSession, level: &str) -> Result<PathBuf, String> {
    crate::folder::level_folder(&service::levels_root(session), level)
}

pub(crate) async fn write_files(
    editor: &Arc<Editor>,
    dir: PathBuf,
    files: NamedFiles,
) -> Result<(), JobError> {
    let guard = write_guard(editor);
    editor
        .exec
        .io(move || -> Result<(), String> {
            guard.check(&dir).map_err(|e| e.to_string())?;
            std::fs::create_dir_all(&dir).map_err(|e| format!("The level folder could not be made: {e}"))?;
            for (name, bytes) in files {
                let p = dir.join(&name);
                if let Some(parent) = p.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                gz_editor::fsx::write_atomic(&p, &bytes)
                    .map_err(|e| format!("{} could not be written: {e}", p.display()))?;
            }
            Ok(())
        })
        .await
        .map_err(JobError::new)
}

/// Removes files of the level folder an earlier build wrote and this one does not.
async fn remove_files(editor: &Arc<Editor>, dir: PathBuf, names: Vec<String>) {
    let guard = write_guard(editor);
    editor
        .exec
        .io(move || {
            if guard.check(&dir).is_err() {
                return;
            }
            for n in names {
                let _ = std::fs::remove_file(dir.join(n));
            }
        })
        .await;
}

/// The breakable parts' models of an earlier build (`meshes\brkbl#N.ogf`) this one did not write.
async fn stale_meshes(editor: &Arc<Editor>, dir: PathBuf, written: Vec<String>) {
    let d2 = dir.clone();
    let stale: Vec<String> = editor
        .exec
        .io(move || {
            let Ok(listing) = std::fs::read_dir(d2.join("meshes")) else { return Vec::new() };
            listing
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.to_ascii_lowercase().starts_with("brkbl#"))
                .map(|n| format!("meshes\\{n}"))
                .filter(|n| !written.iter().any(|w| w.eq_ignore_ascii_case(n)))
                .collect()
        })
        .await;
    if !stale.is_empty() {
        remove_files(editor, dir, stale).await;
    }
}

/// The LOD pictures the scene's multiple-usage objects name, read from the game's textures.
async fn lod_pictures(
    editor: &Arc<Editor>,
    scene: &SceneState,
    lib: &gz_scene::library::Library,
) -> HashMap<String, (Vec<u8>, u32, u32)> {
    let mut names: Vec<String> = Vec::new();
    for o in scene.of_class(ObjClass::SceneObject) {
        let ObjData::SceneObject(d) = &o.data else { continue };
        let Some(lo) = lib.cached(&d.reference) else { continue };
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

/// The detail meshes' textures by name, as RGBA pixels, from the editor's own `gamedata`: the
/// linked game's are larger and would not fit the details' atlas.
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

/// The LOD textures the scene's multiple-usage objects need: where each would be under the
/// editor's `gamedata\textures`, and whether the linked game or that folder has it.
pub(crate) async fn lod_texture_files(
    editor: &Arc<Editor>,
    scene: &SceneState,
    lib: &gz_scene::library::Library,
) -> HashMap<String, (String, bool)> {
    let mut names: Vec<String> = Vec::new();
    for o in scene.of_class(ObjClass::SceneObject) {
        let ObjData::SceneObject(d) = &o.data else { continue };
        if d.visual_mode() {
            continue;
        }
        let Some(lo) = lib.cached(&d.reference) else { continue };
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

/// The first quoted name of a line that is an object of the scene.
fn object_in(scene: &SceneState, line: &str) -> Option<String> {
    for quoted in line.split(['\'', '[', ']']).skip(1).step_by(2) {
        if scene.find_relaxed(quoted).is_some() {
            return Some(quoted.to_owned());
        }
    }
    None
}

/// A check's findings as problems: the errors and the warnings, each with the object its text names.
fn finding_problems(
    scene: &SceneState,
    step: Step,
    code: &'static str,
    found: &[Finding],
) -> Vec<LevelProblem> {
    found
        .iter()
        .filter(|f| matches!(f.severity, Severity::Error | Severity::Warning))
        .map(|f| LevelProblem {
            step,
            code,
            severity: f.severity,
            text: f.text.clone(),
            object: object_in(scene, &f.text),
            position: None,
        })
        .collect()
}

/// The scene's check before a build step: its findings go to the log and become problems; a scene that fails
/// is not built.
pub(crate) async fn validate_scene(
    editor: &Arc<Editor>,
    scene: Arc<SceneState>,
    inp: &Inputs,
    tests: crate::validate::Tests,
    step: Step,
    problems: &mut Vec<LevelProblem>,
    ctx: &JobCtx,
) -> Result<(), JobError> {
    ctx.stage("Checking the scene");
    let lods = if tests.full { lod_texture_files(editor, &scene, &inp.lib).await } else { HashMap::new() };
    let (s2, lib, engine) = (scene.clone(), inp.lib.clone(), inp.engine.clone());
    let checked = editor
        .exec
        .cpu(move || {
            let texture = |n: &str| lods.get(n).cloned().unwrap_or_else(|| (format!("textures\\{n}"), false));
            crate::validate::validate(&s2, &lib, &engine, tests, &texture)
        })
        .await;
    let (found, ok) = match checked {
        Ok(l) => (l, true),
        Err(l) => (l, false),
    };
    for f in &found {
        editor.log.finding(f);
    }
    problems.extend(finding_problems(&scene, step, "validation", &found));
    if ok {
        Ok(())
    } else {
        let first = problems.iter().find(|p| p.severity == Severity::Error).map(|p| p.text.clone());
        Err(JobError::new(match first {
            Some(t) => format!("The scene does not pass its checks: {t}"),
            None => "The scene does not pass its checks (see the problems).".to_owned(),
        }))
    }
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

/// What a step works with.
struct StepCx<'a> {
    editor: &'a Arc<Editor>,
    session: &'a Arc<ProjectSession>,
    inp: &'a Inputs,
    /// The scene the steps read; a step that generates the AI map changes it.
    work: SceneState,
    level: String,
    dir: PathBuf,
    req: &'a Request,
    fps: rustc_hash::FxHashMap<Step, u64>,
    record: BuildState,
    problems: Vec<LevelProblem>,
    ctx: &'a JobCtx,
    /// The steps whose scene check the preflight ran: they do not run it again.
    validated: Vec<Step>,
    /// The visibility cells completed by the preflight, for the scene of this serial.
    completed: Option<(u64, gz_scene::objtools::zones::Completion)>,
}

/// What a step made.
#[derive(Default)]
struct Made {
    files: Vec<String>,
    note: Value,
    /// The step changed the scene the later steps read.
    changed_scene: bool,
}

fn named(files: &NamedFiles) -> Vec<String> {
    files.iter().map(|(n, _)| n.clone()).collect()
}

/// Runs the build: plans the steps, checks what they need, runs each, records each. Answers
/// what ran; an error ends it at the step that failed.
pub(crate) async fn run(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    req: &Request,
    ctx: &JobCtx,
) -> Result<Value, JobError> {
    let lb = LevelBuild::of(session);
    let inp = inputs(editor, ctx).await?;
    // the models the scene's objects are made from are part of what the steps read
    let (snap, lib) = (inp.snap.clone(), inp.lib.clone());
    // the media assets as they are now: a build waits for the index, a status never does
    let media = match gz_media::Media::get(editor) {
        Some(m) => Some((m.index().await, m.root().to_path_buf())),
        None => None,
    };
    let library = editor.exec.io(move || fingerprint::library_stamp_known(&snap, &lib, media.as_ref())).await;
    let level = inp.snap.meta.level.level_path.to_string();
    if level.trim().is_empty() {
        return Err(JobError::new("The scene has no level path: set it in Level Settings."));
    }
    // a stock level's compile steps run only when they are asked for by name or forced
    let stock = crate::stock::is_stock(&lb.game_levels(editor).await, &level);
    let pieces = lb.game_pieces(editor).await;
    let explicit = req.force || req.only.is_some() || req.steps.is_some();
    let req = &Request {
        opts: Options { library, stock: stock && !explicit, game_level: stock, pieces, ..req.opts.clone() },
        ..req.clone()
    };
    let dir = level_dir(session, &level).map_err(JobError::new)?;
    let record = lb
        .record_now(editor, session, &level)
        .await
        .map(|r| (*r).clone())
        .unwrap_or_else(|| BuildState::new(&level));
    let work: SceneState = (*inp.snap).clone();
    let objects = lb.hashes.objects(&work);
    let fps = fingerprint::steps(&work, &objects, &req.opts);
    let status = service::status_with(&work, &objects, Some(&record), &req.opts);
    let asked: Vec<Step> = match (req.only, &req.steps) {
        (Some(s), _) => with_needs(&[s]),
        (None, Some(list)) => with_needs(list),
        (None, None) => Step::ORDER.to_vec(),
    };
    let forced: Vec<Step> = match (req.force, req.only) {
        (false, _) => Vec::new(),
        (true, Some(s)) => vec![s],
        (true, None) => asked.clone(),
    };
    let stale = |s: &Step| {
        status.get(*s).is_some_and(|x| !matches!(x.state, StepState::Fresh | StepState::NotNeeded))
    };
    // what has nothing to make (no detail objects): not what a stock level does not need
    let not_needed: Vec<Step> =
        asked.iter().copied().filter(|s| !service::is_needed(&work, *s, &req.opts)).collect();
    let plan: Vec<Step> = asked
        .iter()
        .copied()
        .filter(|s| service::is_needed(&work, *s, &req.opts) && (forced.contains(s) || stale(s)))
        .collect();
    let mut cx = StepCx {
        editor,
        session,
        inp: &inp,
        work,
        level: level.clone(),
        dir: dir.clone(),
        req,
        fps,
        record,
        problems: Vec::new(),
        ctx,
        validated: Vec::new(),
        completed: None,
    };
    // what a step that has nothing to make made before goes, so the game reads none of it
    for s in &not_needed {
        // a piece the linked game cannot load stays: it is the module's for the games that can
        if *s == Step::Piece && req.opts.game_level && service::needed(&cx.work, *s) {
            continue;
        }
        if let Some(rec) = cx.record.steps.remove(s) {
            if *s == Step::Piece {
                remove_piece(editor, session, &level, &rec.files).await;
                continue;
            }
            let names: Vec<String> = match s {
                Step::Details => {
                    vec!["build.details".into(), format!("{}.dds", crate::details::DETAILS_TEX_NAME)]
                }
                Step::DetailLight => vec!["level.details".into()],
                _ => Vec::new(),
            };
            remove_files(editor, dir.clone(), names).await;
        }
    }
    if plan.is_empty() {
        save_record(editor, &lb, &cx.record, &dir, &objects).await;
        let replaced = stock_problem(&mut cx, stock).await;
        finish_problems(&lb, &[], std::mem::take(&mut cx.problems));
        return Ok(json!({ "level": level, "ran": [], "fresh": true, "stock": stock, "replaces": replaced }));
    }
    // what a later step would refuse, before the first one runs
    ctx.stage("Checking what the steps need");
    if let Err(e) = preflight(&mut cx, &plan).await {
        finish_problems(&lb, &plan, std::mem::take(&mut cx.problems));
        return Err(e);
    }
    editor.log.info(format!(
        "Building {level}: {}.",
        plan.iter().map(|s| s.title()).collect::<Vec<_>>().join(", ")
    ));
    let mut ran: Vec<Value> = Vec::new();
    let mut objects = objects;
    let total = plan.len();
    for (k, step) in plan.iter().copied().enumerate() {
        if let Err(stop) = ctx.check() {
            crate::report::write(
                editor,
                &dir,
                &level,
                req.opts.quality,
                &ran,
                &cx.problems,
                crate::report::End::Stopped { step: None },
            )
            .await;
            finish_problems(&lb, &plan[..k], cx.problems);
            return Err(stop.into());
        }
        ctx.stage(format!("{} ({} of {total})", step.stage(), k + 1));
        ctx.fraction(k as f32 / total as f32);
        let begun = Instant::now();
        let out = run_step(&mut cx, step).await;
        let seconds = begun.elapsed().as_secs_f64();
        let library =
            if step.classes().contains(&gz_scene::ObjClass::SceneObject) { req.opts.library } else { 0 };
        match out {
            Ok(made) => {
                if made.changed_scene {
                    objects = lb.hashes.objects(&cx.work);
                    cx.fps = fingerprint::steps(&cx.work, &objects, &req.opts);
                }
                let fingerprint = cx.fps.get(&step).copied().unwrap_or(0);
                cx.record.steps.insert(
                    step,
                    StepRecord {
                        fingerprint,
                        time: crate::state::now_text(),
                        seconds,
                        quality: req.opts.quality,
                        flags: req.opts.flags.clone(),
                        library,
                        failed: None,
                        files: made.files.clone(),
                    },
                );
                save_record(editor, &lb, &cx.record, &dir, &objects).await;
                editor.log.info(format!("{} done in {seconds:.1} s.", step.title()));
                ran.push(json!({
                    "step": step.name(),
                    "title": step.title(),
                    "seconds": (seconds * 10.0).round() / 10.0,
                    "files": made.files,
                    "note": made.note,
                }));
            }
            Err(e) if e.cancelled => {
                editor.log.warn(format!("The build of {level} was stopped in {}.", step.title()));
                crate::report::write(
                    editor,
                    &dir,
                    &level,
                    req.opts.quality,
                    &ran,
                    &cx.problems,
                    crate::report::End::Stopped { step: Some(step) },
                )
                .await;
                finish_problems(&lb, &plan[..=k], cx.problems);
                return Err(e);
            }
            Err(e) => {
                // an error ends the build here: what ran stays, what comes after does not run
                let fingerprint = cx.fps.get(&step).copied().unwrap_or(0);
                cx.record.steps.insert(
                    step,
                    StepRecord {
                        fingerprint,
                        time: crate::state::now_text(),
                        seconds,
                        quality: req.opts.quality,
                        flags: req.opts.flags.clone(),
                        library,
                        failed: Some(e.message.clone()),
                        files: Vec::new(),
                    },
                );
                save_record(editor, &lb, &cx.record, &dir, &objects).await;
                cx.problems.push(LevelProblem {
                    step,
                    code: "failed",
                    severity: Severity::Error,
                    text: e.message.clone(),
                    object: None,
                    position: None,
                });
                editor.log.error(format!("{}: {}", step.title(), e.message));
                let not_run: Vec<&str> = plan[k + 1..].iter().map(|s| s.title()).collect();
                crate::report::write(
                    editor,
                    &dir,
                    &level,
                    req.opts.quality,
                    &ran,
                    &cx.problems,
                    crate::report::End::Failed { step, seconds, text: &e.message },
                )
                .await;
                finish_problems(&lb, &plan[..=k], cx.problems);
                return Err(JobError::new(ended_text(step, &e.message, &ran, &not_run)));
            }
        }
    }
    ctx.fraction(1.0);
    let replaced = stock_problem(&mut cx, stock).await;
    let problems = cx.problems.len();
    crate::report::write(
        editor,
        &dir,
        &level,
        req.opts.quality,
        &ran,
        &cx.problems,
        crate::report::End::Built,
    )
    .await;
    finish_problems(&lb, &plan, cx.problems);
    editor.log.info(format!("{level} is built."));
    Ok(json!({ "level": level, "ran": ran, "problems": problems, "stock": stock, "replaces": replaced }))
}

/// The sentence a build that met an error ends with: the step and why, what was built, what did
/// not run.
fn ended_text(step: Step, why: &str, ran: &[Value], not_run: &[&str]) -> String {
    let end = if why.ends_with(['.', '!', '?']) { "" } else { "." };
    let mut text = vec![format!("{} failed: {why}{end}", step.title())];
    let built: Vec<&str> = ran.iter().filter_map(|r| r["title"].as_str()).collect();
    if !built.is_empty() {
        text.push(format!("Built: {}.", built.join(", ")));
    }
    if !not_run.is_empty() {
        text.push(format!("Not run: {}.", not_run.join(", ")));
    }
    text.join(" ")
}

/// On a stock level: the files an earlier build left in the level's folder that would replace the
/// game's own, as a warning on the level. Answers their names.
async fn stock_problem(cx: &mut StepCx<'_>, stock: bool) -> Vec<String> {
    // the warning is said again by every build of the level, never twice
    LevelBuild::of(cx.session).problems.lock().retain(|p| p.code != "replaces_stock");
    if !stock {
        return Vec::new();
    }
    let (record, dir) = (cx.record.clone(), cx.dir.clone());
    let replaced =
        cx.editor.exec.io(move || crate::stock::replacing(&crate::stock::written(&record, &dir))).await;
    if let Some(p) = crate::stock::problem(&cx.level, &replaced) {
        cx.problems.push(p);
    }
    replaced
}

/// The spawn points the spawn step builds, as its elements.
fn spawn_elements(work: &SceneState, settings: Option<&(dyn Settings + Sync)>) -> Vec<Element> {
    let mut elements = Vec::new();
    for o in work.of_class(ObjClass::SpawnPoint) {
        if let ObjData::Spawn(pt) = &o.data
            && pt.kind == PointKind::Spawn
            && let Ok(Some((e, _))) = crate::files::export_entity(o, settings)
        {
            elements.push(Element { entity: e });
        }
    }
    elements
}

/// What a step of `plan` would refuse and can be seen now, each a problem on what it is about:
/// the compiler's options, the inputs no planned step makes, the spawn's points and the linked
/// game's `all.spawn`, the graph points, the ground of an AI map to generate, the scene's checks
/// (the visibility cells completed first, kept for the geometry step). `Err` names the first
/// error and how many there are; the steps never run then.
async fn preflight(cx: &mut StepCx<'_>, plan: &[Step]) -> Result<(), JobError> {
    let has = |s: Step| plan.contains(&s);
    let mut found: Vec<LevelProblem> = Vec::new();
    let refuse = |found: &mut Vec<LevelProblem>, step: Step, text: String, object: Option<String>| {
        found.push(LevelProblem {
            step,
            code: "preflight",
            severity: Severity::Error,
            text,
            object,
            position: None,
        });
    };
    if (has(Step::Light) || has(Step::DetailLight) || has(Step::Piece))
        && let Err(e) = gz_lc::Options::parse(cx.req.opts.flags.split_whitespace())
    {
        refuse(&mut found, Step::Light, e.to_string(), None);
    }
    // the inputs a step reads from the level folder when no planned step makes them
    let mut inputs: Vec<(Step, &'static str, &'static str)> = Vec::new();
    if has(Step::GameGraph) && !has(Step::AiMap) {
        inputs.push((Step::GameGraph, "level.ai", "build the AI map first"));
    }
    if has(Step::Spawn) && !has(Step::AiMap) {
        inputs.push((Step::Spawn, "level.ai", "build the AI map first"));
    }
    if has(Step::Spawn) && !has(Step::GameGraph) {
        inputs.push((Step::Spawn, "level.graph", "build the game graph first"));
    }
    if has(Step::Light) && !has(Step::Geometry) {
        inputs.push((Step::Light, "build.prj", "build the geometry first"));
    }
    if has(Step::DetailLight) && !has(Step::Details) {
        inputs.push((Step::DetailLight, "build.details", "build the details first"));
    }
    if has(Step::DetailLight) && !has(Step::Light) {
        inputs.push((Step::DetailLight, "build.cform", "build the light first"));
        inputs.push((Step::DetailLight, "build.lights", "build the light first"));
    }
    let dir = cx.dir.clone();
    let lb = LevelBuild::of(cx.session);
    let cached_ai = lb.graphs.lock().level.is_some();
    let missing: Vec<(Step, &'static str, &'static str)> = cx
        .editor
        .exec
        .io(move || inputs.into_iter().filter(|(_, f, _)| !dir.join(f).is_file()).collect())
        .await;
    for (step, file, how) in missing {
        if file == "level.ai" && cached_ai {
            continue;
        }
        refuse(&mut found, step, format!("The level has no {file}: {how}."), None);
    }
    // a terrain that cuts a level of the game needs a game that cuts it
    if has(Step::Piece) && cx.req.opts.game_level && service::cutting_terrain(&cx.work).is_some() {
        let cuts = service::game_states(cx.editor, service::LEVEL_CUTS).await;
        if let Some((name, text)) = cut_refusal(&cx.work, &cx.level, cuts) {
            refuse(&mut found, Step::Piece, text, Some(name));
        }
    }
    if has(Step::GameGraph) && graph_points(&cx.work, sync_settings(&cx.inp.settings)).is_empty() {
        refuse(
            &mut found,
            Step::GameGraph,
            "The level has no graph point: place at least one on the AI map.".into(),
            None,
        );
    }
    if has(Step::Spawn) {
        let linked = match GameLink::of(cx.editor) {
            Ok(link) => link.files().await.ok().is_some_and(|f| f.exists("spawns\\all.spawn")),
            Err(_) => false,
        };
        if !linked {
            refuse(
                &mut found,
                Step::Spawn,
                "The linked game has no spawns\\all.spawn: the spawn joins the level to the game's graph, so link a game first.".into(),
                None,
            );
        }
        let (work, settings) = (Arc::new(cx.work.clone()), cx.inp.settings.clone());
        let refused = cx
            .editor
            .exec
            .cpu(move || spawn::refusals(&spawn_elements(&work, sync_settings(&settings))))
            .await;
        for (text, object) in refused {
            refuse(&mut found, Step::Spawn, text, object);
        }
    }
    if has(Step::AiMap) && cx.work.bulk.ai_map.as_ref().is_none_or(|a| a.node_count() == 0) {
        let (work, lib) = (Arc::new(cx.work.clone()), cx.inp.lib.clone());
        let ground = cx
            .editor
            .exec
            .cpu(move || {
                let ids = gz_scene::bulk::cmd::ai_ground_ids(&work, &lib);
                !gz_scene::bulk::ground::names(&work, &ids).is_empty()
            })
            .await;
        if !ground {
            refuse(
                &mut found,
                Step::AiMap,
                "The scene has no static objects for the AI map to lie on.".into(),
                None,
            );
        }
    }
    cx.ctx.check()?;
    let mut failed_checks = false;
    if has(Step::Game) {
        let scene = Arc::new(cx.work.clone());
        let ok = validate_scene(
            cx.editor,
            scene,
            cx.inp,
            crate::validate::Tests::MAKE_GAME,
            Step::Game,
            &mut cx.problems,
            cx.ctx,
        )
        .await
        .is_ok();
        failed_checks |= !ok;
        cx.validated.push(Step::Game);
    }
    if has(Step::Geometry) {
        cx.ctx.stage("Completing the visibility cells");
        let (work, lib) = (Arc::new(cx.work.clone()), cx.inp.lib.clone());
        let c = cx.ctx.clone();
        let completion = cx
            .editor
            .exec
            .cpu(move || gz_scene::objtools::zones::complete(&work, &lib, &|| c.cancel.is_cancelled()))
            .await
            .map_err(|_| JobError { message: crate::STOPPED.to_owned(), cancelled: true })?;
        let mut scene = completion.state.clone();
        scene.meta.level = cx.req.opts.quality.apply(&scene.meta.level);
        let ok = validate_scene(
            cx.editor,
            Arc::new(scene),
            cx.inp,
            crate::validate::Tests::BUILD,
            Step::Geometry,
            &mut cx.problems,
            cx.ctx,
        )
        .await
        .is_ok();
        failed_checks |= !ok;
        cx.validated.push(Step::Geometry);
        cx.completed = Some((cx.work.serial, completion));
    }
    cx.ctx.check()?;
    let errors = found.len() + cx.problems.iter().filter(|p| p.severity == Severity::Error).count();
    if found.is_empty() && !failed_checks {
        return Ok(());
    }
    let first = found
        .first()
        .or_else(|| cx.problems.iter().find(|p| p.severity == Severity::Error))
        .map(|p| p.text.trim_end_matches('.').to_owned())
        .unwrap_or_else(|| "the scene does not pass its checks".to_owned());
    for p in &found {
        cx.editor.log.error(format!("{}: {}", p.step.title(), p.text));
    }
    cx.problems.extend(found);
    let more =
        if errors > 1 { format!(" ({} problems in all: see the problems)", errors) } else { String::new() };
    let steps: Vec<&str> = plan.iter().map(|s| s.title()).collect();
    Err(JobError::new(format!(
        "The build of {} did not start: {first}{more}. Not run: {}.",
        cx.level,
        steps.join(", ")
    )))
}

/// The problems of the steps that ran replace what those steps found before.
fn finish_problems(lb: &LevelBuild, ran: &[Step], found: Vec<LevelProblem>) {
    let mut all = lb.problems.lock();
    all.retain(|p| !ran.contains(&p.step));
    all.extend(found);
}

async fn save_record(
    editor: &Arc<Editor>,
    lb: &Arc<LevelBuild>,
    record: &BuildState,
    dir: &std::path::Path,
    objects: &Objects,
) {
    let mut r = record.clone();
    r.objects = objects.clone();
    let r = Arc::new(r);
    lb.set_record(r.clone());
    let (d2, guard) = (dir.to_path_buf(), write_guard(editor));
    let saved = editor
        .exec
        .io(move || {
            guard.check(&d2).map_err(std::io::Error::other)?;
            r.save(&d2)
        })
        .await;
    if let Err(e) = saved {
        editor.log.warn(format!("The build's record could not be written into {}: {e}", dir.display()));
    }
}

async fn run_step(cx: &mut StepCx<'_>, step: Step) -> Result<Made, JobError> {
    match step {
        Step::Game => step_game(cx).await,
        Step::SpawnLayer => step_spawn_layer(cx).await,
        Step::AiMap => step_ai_map(cx).await,
        Step::GameGraph => step_game_graph(cx).await,
        Step::Spawn => step_spawn(cx).await,
        Step::Geometry => step_geometry(cx).await,
        Step::Details => step_details(cx).await,
        Step::Light => step_compile(cx, crate::compile::Part::Level).await,
        Step::DetailLight => step_compile(cx, crate::compile::Part::Details).await,
        Step::Piece => step_piece(cx).await,
    }
}

fn log_lines(editor: &Editor, found: &[Finding]) {
    for f in found {
        editor.log.finding(f);
    }
}

/// `level.ltx` and the game files: the spawn packets, patrol paths, sounds, particles, fog.
async fn step_game(cx: &mut StepCx<'_>) -> Result<Made, JobError> {
    let scene = Arc::new(cx.work.clone());
    if !cx.validated.contains(&Step::Game) {
        validate_scene(
            cx.editor,
            scene.clone(),
            cx.inp,
            crate::validate::Tests::MAKE_GAME,
            Step::Game,
            &mut cx.problems,
            cx.ctx,
        )
        .await?;
    }
    let (lib, engine, settings) = (cx.inp.lib.clone(), cx.inp.engine.clone(), cx.inp.settings.clone());
    let c = cx.ctx.clone();
    let (made, log) = cx
        .editor
        .exec
        .cpu(move || -> crate::Result<(NamedFiles, Vec<Finding>)> {
            let look = crate::files::lookup(lib);
            let mut out = vec![("level.ltx".to_owned(), crate::files::level_ltx(&scene, &look))];
            let stop = || c.cancel.is_cancelled();
            let g = crate::files::game_files_until(&scene, &look, &engine, sync_settings(&settings), &stop)?;
            out.extend(g.files.into_iter().map(|(n, b)| (n.to_owned(), b)));
            out.extend(g.meshes);
            Ok((out, g.log))
        })
        .await
        .map_err(job_err)?;
    log_lines(cx.editor, &log);
    let names = named(&made);
    write_files(cx.editor, cx.dir.clone(), made).await?;
    stale_meshes(cx.editor, cx.dir.clone(), names.clone()).await;
    Ok(Made { files: names, ..Made::default() })
}

/// The spawn layer of the whole scene.
async fn step_spawn_layer(cx: &mut StepCx<'_>) -> Result<Made, JobError> {
    let snap = Arc::new(Snap { rev: cx.inp.snap.rev, edit: cx.inp.snap.edit, state: cx.work.clone() });
    let note = gz_export_level::bake_layer(cx.editor, cx.session, snap).await.map_err(JobError::new)?;
    for w in note.get("warnings").and_then(Value::as_array).into_iter().flatten() {
        if let Some(t) = w.as_str() {
            cx.problems.push(LevelProblem {
                step: Step::SpawnLayer,
                code: "spawn_layer",
                severity: Severity::Warning,
                text: t.to_owned(),
                object: object_in(&cx.work, t),
                position: None,
            });
        }
    }
    let path = note.get("path").and_then(Value::as_str).unwrap_or_default().to_owned();
    Ok(Made { files: if path.is_empty() { Vec::new() } else { vec![path] }, note, ..Made::default() })
}

/// The fingerprint of what an AI map is generated from (the ground, the bounds, the settings),
/// and of the nodes alone.
fn ai_inputs(scene: &SceneState, objects: &Objects) -> (u64, u64) {
    let mut h = fingerprint::Stable::new();
    for c in [ObjClass::SceneObject, ObjClass::Shape] {
        for e in objects.class(c) {
            h.str(&e.name);
            h.word(e.hash.xform);
            h.word(e.hash.data);
        }
    }
    if let Some(a) = &scene.bulk.ai_map {
        h.str(&format!("{:?}{:?}", a.header.params, a.header.snap_objects));
    }
    h.str(&scene.tools.bulk.ignored.iter().map(|s| s.to_lowercase()).collect::<Vec<_>>().join("|"));
    (h.finish(), fingerprint::ai_nodes_hash(scene))
}

/// `level.ai`: the AI map as the game reads it. A scene without an AI map, or one whose map the
/// build generated and whose ground has moved since, gets the map generated over its ground first
/// (an undo step of the scene); a map the author edited by hand is kept as it is.
async fn step_ai_map(cx: &mut StepCx<'_>) -> Result<Made, JobError> {
    let lb = LevelBuild::of(cx.session);
    let objects = lb.hashes.objects(&cx.work);
    let (inputs, nodes) = ai_inputs(&cx.work, &objects);
    let empty = cx.work.bulk.ai_map.as_ref().is_none_or(|a| a.node_count() == 0);
    let generated_by_build = cx.record.ai_map.is_some_and(|g| g.nodes == nodes);
    let regenerate = empty || (generated_by_build && cx.record.ai_map.is_some_and(|g| g.inputs != inputs));
    let mut changed_scene = false;
    let mut note = json!({ "generated": false });
    if regenerate {
        cx.ctx.stage("Generating the AI map over the ground");
        let (work, lib, xrlc) = (Arc::new(cx.work.clone()), cx.inp.lib.clone(), cx.inp.engine.clone());
        let c = cx.ctx.clone();
        let made = cx
            .editor
            .exec
            .cpu(move || {
                let ids = gz_scene::bulk::cmd::ai_ground_ids(&work, &lib);
                let names = gz_scene::bulk::ground::names(&work, &ids);
                if names.is_empty() {
                    return Err("The scene has no static objects for the AI map to lie on.".to_owned());
                }
                let stop = || c.cancel.is_cancelled();
                let model = gz_scene::bulk::collide::CollideModel::build_until(
                    &work,
                    &lib,
                    &names,
                    xrlc.xrlc.as_ref(),
                    &stop,
                )
                .ok_or_else(|| crate::STOPPED.to_owned())?;
                let keep_going = |f: f32, total: usize| {
                    c.stage(format!("Generating the AI map: {total} nodes"));
                    c.fraction(f);
                    !c.cancel.is_cancelled()
                };
                gz_scene::bulk::cmd::generate_map(&work, &lib, &model, &keep_going)
                    .ok_or_else(|| "The scene has no static objects for the AI map to lie on.".to_owned())
            })
            .await
            .map_err(|e| {
                if e == crate::STOPPED { JobError { message: e, cancelled: true } } else { JobError::new(e) }
            })?;
        cx.ctx.check()?;
        if made.nodes == 0 {
            return Err(JobError::new(
                "No AI map node could be made: no spawn point stands on ground an NPC can walk, and neither does the middle of the level. Place a graph point on the ground.",
            ));
        }
        // the map goes into the scene as one undo step, so the author sees what the game walks on
        let svc = SceneService::of(cx.editor).map_err(|e| JobError::new(e.message))?;
        let send = |serial: u64| {
            let mut edit =
                gz_scene::bulk::BulkEdit::new("Generate AI Map", serial).with_ai(made.part.clone());
            edit.kind = gz_scene::bulk::Kind::Step;
            svc.store.send(gz_scene::SceneCmd::Bulk(Box::new(edit)), gz_base::Origin::Mcp)
        };
        let mut serial = cx.work.serial;
        match send(serial).await {
            Ok(_) => {}
            Err(e) if e.code == "busy" => {
                // the scene moved meanwhile: the map still stands when what it lies on did not move (a
                // selection, the file's name); anything else is a scene the map was not made for
                let now = svc.store.snapshot();
                let (inputs_now, nodes_now) = ai_inputs(&now, &lb.hashes.objects(&now));
                if inputs_now != inputs || nodes_now != ai_inputs(&cx.work, &objects).1 {
                    return Err(JobError::new(
                        "The scene changed while the AI map was generated: build again.",
                    ));
                }
                serial = now.serial;
                send(serial).await.map_err(|_| {
                    JobError::new("The scene changed while the AI map was generated: build again.")
                })?;
            }
            Err(e) => return Err(JobError::new(e.message)),
        }
        cx.work.bulk.ai_map = Some(Arc::new(made.part));
        cx.work.serial = serial + 1;
        changed_scene = true;
        let (_, nodes_now) = ai_inputs(&cx.work, &objects);
        cx.record.ai_map = Some(Generated { inputs, nodes: nodes_now });
        note =
            json!({ "generated": true, "nodes": made.nodes, "seeds": made.seeds, "stopped": made.stopped });
        cx.editor.log.info(format!("The AI map was generated over the ground: {} nodes.", made.nodes));
    } else if !generated_by_build {
        note = json!({ "generated": false, "kept": "The AI map was made by hand: it is kept. scene.ai_map action=generate makes it again over the ground." });
    }
    let Some((nodes, params)) = crate::files::ai_nodes(&cx.work) else {
        return Err(JobError::new("The AI map is empty."));
    };
    let lg = cx
        .editor
        .exec
        .cpu(move || graph::level_graph(&nodes, &params, guid()))
        .await
        .map_err(|e| JobError::new(e.0))?;
    let bytes = lg.write().map_err(|e| JobError::new(e.to_string()))?;
    write_files(cx.editor, cx.dir.clone(), vec![("level.ai".to_owned(), bytes)]).await?;
    // the fingerprint after a generation is that of the scene with the new map
    if changed_scene {
        let objs = lb.hashes.objects(&cx.work);
        cx.fps = fingerprint::steps(&cx.work, &objs, &cx.req.opts);
    }
    let fp = cx.fps.get(&Step::AiMap).copied().unwrap_or(0);
    lb.graphs.lock().level = Some((fp, Arc::new(lg)));
    Ok(Made { files: vec!["level.ai".to_owned()], note, changed_scene })
}

/// The level graph: the one the AI map step made in this session, or the level folder's.
async fn level_graph(cx: &StepCx<'_>) -> Result<Arc<gz_fmt_level::LevelGraph>, JobError> {
    let lb = LevelBuild::of(cx.session);
    let fp = cx.fps.get(&Step::AiMap).copied().unwrap_or(0);
    if let Some((f, g)) = lb.graphs.lock().level.as_ref()
        && *f == fp
    {
        return Ok(g.clone());
    }
    let path = cx.dir.join("level.ai");
    let bytes = cx
        .editor
        .exec
        .io(move || std::fs::read(&path))
        .await
        .map_err(|_| JobError::new("The level has no level.ai: build the AI map first."))?;
    let g = cx
        .editor
        .exec
        .cpu(move || gz_fmt_level::LevelGraph::read(&bytes))
        .await
        .map_err(|e| JobError::new(format!("level.ai could not be read: {e}")))?;
    let g = Arc::new(g);
    lb.graphs.lock().level = Some((fp, g.clone()));
    Ok(g)
}

/// `level.graph`: the graph points on the AI map, joined by their paths.
async fn step_game_graph(cx: &mut StepCx<'_>) -> Result<Made, JobError> {
    let lg = level_graph(cx).await?;
    let (work, settings, level) = (Arc::new(cx.work.clone()), cx.inp.settings.clone(), cx.level.clone());
    let c = cx.ctx.clone();
    let built = cx
        .editor
        .exec
        .cpu(move || {
            let stop = || c.cancel.is_cancelled();
            let nav = LevelNav::new(&lg);
            let points = graph_points(&work, sync_settings(&settings));
            if points.is_empty() {
                return Err(crate::BuildError::new(
                    "The level has no graph point: place at least one on the AI map.",
                ));
            }
            graph::build_game_graph_until(&nav, &points, &level, lg.header.guid, guid(), &stop)
        })
        .await
        .map_err(job_err)?;
    log_lines(cx.editor, &built.log);
    let bytes = built.graph.to_bytes();
    write_files(cx.editor, cx.dir.clone(), vec![("level.graph".to_owned(), bytes)]).await?;
    let lb = LevelBuild::of(cx.session);
    let fp = cx.fps.get(&Step::GameGraph).copied().unwrap_or(0);
    let vertices = built.graph.vertices.len();
    lb.graphs.lock().game = Some((fp, Arc::new(built)));
    Ok(Made {
        files: vec!["level.graph".to_owned()],
        note: json!({ "vertices": vertices }),
        ..Made::default()
    })
}

/// The level's game graph: the one the game graph step made in this session, or `level.graph`.
async fn game_graph(cx: &StepCx<'_>) -> Result<gz_fmt_spawn::GameGraph, JobError> {
    let lb = LevelBuild::of(cx.session);
    let fp = cx.fps.get(&Step::GameGraph).copied().unwrap_or(0);
    if let Some((f, g)) = lb.graphs.lock().game.as_ref()
        && *f == fp
    {
        return Ok(g.graph.clone());
    }
    let path = cx.dir.join("level.graph");
    let bytes = cx
        .editor
        .exec
        .io(move || std::fs::read(&path))
        .await
        .map_err(|_| JobError::new("The level has no level.graph: build the game graph first."))?;
    cx.editor
        .exec
        .cpu(move || gz_fmt_spawn::GameGraph::parse_file(&Arc::from(bytes)))
        .await
        .map_err(|e| JobError::new(format!("level.graph could not be read: {e}")))
}

/// `gamedata\spawns\editor.spawn`: the scene's spawn points on the level and the game graph
/// joined with the linked game's, what Play runs.
async fn step_spawn(cx: &mut StepCx<'_>) -> Result<Made, JobError> {
    let lg = level_graph(cx).await?;
    let own = game_graph(cx).await?;
    cx.ctx.stage("Joining the linked game's graph");
    let base = linked_game_graph(cx.editor).await?;
    let own2 = own.clone();
    let joined =
        cx.editor.exec.cpu(move || graph::join_base(&base, &own2)).await.map_err(|e| JobError::new(e.0))?;
    let level = cx.level.clone();
    let play_graph = match joined {
        Some(g) => Arc::new(g),
        None => {
            cx.problems.push(LevelProblem {
                step: Step::Spawn,
                code: "game_level",
                severity: Severity::Note,
                text: format!("{level} is a level of the linked game: the spawn keeps the level's own game graph, and the game's scripts that name its vertices find none of them."),
                object: None,
                position: None,
            });
            Arc::new(own)
        }
    };
    cx.ctx.stage("Building the spawn");
    let (work, settings, verify) =
        (Arc::new(cx.work.clone()), cx.inp.settings.clone(), cx.req.verify_restrictors);
    let c = cx.ctx.clone();
    let built = cx
        .editor
        .exec
        .cpu(move || {
            let stop = || c.cancel.is_cancelled();
            let nav = LevelNav::new(&lg);
            let elements = spawn_elements(&work, sync_settings(&settings));
            let ways: Vec<Way> = work
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
            spawn::build_spawn_until(&level, &nav, &play_graph, elements, &ways, guid(), verify, &stop)
        })
        .await
        .map_err(job_err)?;
    log_lines(cx.editor, &built.log);
    let spawns = cx.session.root.join("gamedata").join("spawns");
    write_files(cx.editor, spawns, vec![("editor.spawn".to_owned(), built.bytes)]).await?;
    Ok(Made { files: vec!["gamedata/spawns/editor.spawn".to_owned()], ..Made::default() })
}

/// `build.prj` and the files beside it: the static geometry with its lights, glows and visibility
/// cells - the cells completed by the build - the LOD atlas, the HOM and SOM, the wallmarks.
async fn step_geometry(cx: &mut StepCx<'_>) -> Result<Made, JobError> {
    // the preflight's completion, when the scene is still the one it completed
    let completion = match cx.completed.take() {
        Some((serial, done)) if serial == cx.work.serial => done,
        _ => {
            cx.validated.retain(|s| *s != Step::Geometry);
            cx.ctx.stage("Completing the visibility cells");
            let (work, lib) = (Arc::new(cx.work.clone()), cx.inp.lib.clone());
            let c = cx.ctx.clone();
            cx.editor
                .exec
                .cpu(move || gz_scene::objtools::zones::complete(&work, &lib, &|| c.cancel.is_cancelled()))
                .await
                .map_err(|_| JobError { message: crate::STOPPED.to_owned(), cancelled: true })?
        }
    };
    for (note, cells) in &completion.notes {
        cx.problems.push(LevelProblem {
            step: Step::Geometry,
            code: "zones",
            severity: note.severity,
            text: note.text.clone(),
            object: cells.first().cloned(),
            position: None,
        });
    }
    let mut scene = completion.state;
    scene.meta.level = cx.req.opts.quality.apply(&scene.meta.level);
    let scene = Arc::new(scene);
    if !cx.validated.contains(&Step::Geometry) {
        validate_scene(
            cx.editor,
            scene.clone(),
            cx.inp,
            crate::validate::Tests::BUILD,
            Step::Geometry,
            &mut cx.problems,
            cx.ctx,
        )
        .await?;
    }
    cx.ctx.stage("Reading the LOD textures");
    let lods = Arc::new(lod_pictures(cx.editor, &scene, &cx.inp.lib).await);
    cx.ctx.check()?;
    cx.ctx.stage("Building the geometry");
    let (lib, engine) = (cx.inp.lib.clone(), cx.inp.engine.clone());
    let s2 = scene.clone();
    let job = cx.ctx.clone();
    let (made, log) = cx
        .editor
        .exec
        .cpu(move || -> crate::Result<(NamedFiles, Vec<Finding>)> {
            let look = crate::files::lookup(lib);
            let lod_src = |n: &str| lods.get(n).cloned();
            let stop = || job.cancel.is_cancelled();
            let c = prj::compile_until(&s2, &look, &engine, &lod_src, &stop)?;
            let mut out: NamedFiles = vec![("build.prj".to_owned(), c.project.write())];
            if let Some((color, normal)) = c.lod_textures {
                out.push((format!("{}.dds", prj::LEVEL_LODS_TEX_NAME), color));
                out.push(("level_lods_nm.dds".to_owned(), normal));
            }
            let (tga, txt) = crate::files::scene_stat(&c.stat);
            out.push(("level_stat.tga".to_owned(), tga));
            out.push(("level_stat.txt".to_owned(), txt));
            if let Some(h) = crate::files::hom(&s2, &look) {
                out.push(("level.hom".to_owned(), h));
            }
            if let Some(s) = crate::files::som(&s2, &look, &engine)? {
                out.push(("level.som".to_owned(), s));
            }
            let marks =
                s2.bulk.wallmarks.as_ref().map(|w| crate::wallmarks::read(&w.raw)).unwrap_or_default();
            if !marks.is_empty() {
                out.push(("level.wallmarks".to_owned(), crate::wallmarks::write(&marks)));
            }
            Ok((out, c.log))
        })
        .await
        .map_err(job_err)?;
    log_lines(cx.editor, &log);
    let names = named(&made);
    let gone: Vec<String> = [
        format!("{}.dds", prj::LEVEL_LODS_TEX_NAME),
        "level_lods_nm.dds".to_owned(),
        "level.hom".to_owned(),
        "level.som".to_owned(),
        "level.wallmarks".to_owned(),
    ]
    .into_iter()
    .filter(|n| !names.contains(n))
    .collect();
    write_files(cx.editor, cx.dir.clone(), made).await?;
    remove_files(cx.editor, cx.dir.clone(), gone).await;
    let note = json!({ "default_cell_meshes": completion.default_meshes, "portals_computed": completion.portals, "quality": cx.req.opts.quality.name() });
    Ok(Made { files: names, note, ..Made::default() })
}

/// `build.details` and its texture, from the painted slots.
async fn step_details(cx: &mut StepCx<'_>) -> Result<Made, JobError> {
    let Some(d) = cx.work.bulk.details.clone().filter(|d| !d.model.slots.is_empty()) else {
        return Ok(Made::default());
    };
    let names = crate::details::used_textures(&d.model, &crate::files::lookup(cx.inp.lib.clone()));
    let tex = Arc::new(textures(cx.editor, names).await);
    let lib = cx.inp.lib.clone();
    let (file, dds) = cx
        .editor
        .exec
        .cpu(move || {
            let look = crate::files::lookup(lib);
            let t = |n: &str| tex.get(n).cloned();
            crate::details::export(&d.model, &look, &t)
        })
        .await
        .map_err(|e| JobError::new(format!("The details could not be written: {}", e.0)))?;
    let files =
        vec![("build.details".to_owned(), file), (format!("{}.dds", crate::details::DETAILS_TEX_NAME), dds)];
    let names = named(&files);
    write_files(cx.editor, cx.dir.clone(), files).await?;
    Ok(Made { files: names, ..Made::default() })
}

/// The level compiler (geometry and light) or the detail compiler on the level folder.
async fn step_compile(cx: &mut StepCx<'_>, part: crate::compile::Part) -> Result<Made, JobError> {
    let level_part = part == crate::compile::Part::Level;
    // the level's streaming cells go into the module's level folder; with none, the cells an
    // earlier build left there go
    let pieces = if level_part {
        let dir = pieces_dir(cx.session, &cx.level);
        let (guard, d2) = (write_guard(cx.editor), dir.clone());
        cx.editor
            .exec
            .io(move || guard.check(&d2).map_err(|e| e.to_string()))
            .await
            .map_err(JobError::new)?;
        Some(dir)
    } else {
        None
    };
    let (cells, no_cells) = if level_part { service::cells_of(&cx.work, &cx.req.opts) } else { (0, None) };
    if let Some(why) = no_cells {
        cx.problems.push(LevelProblem {
            step: Step::Light,
            code: "no_cells",
            severity: Severity::Warning,
            text: why.to_owned(),
            object: None,
            position: None,
        });
    }
    let req = crate::compile::Request {
        level: cx.level.clone(),
        dir: cx.dir.clone(),
        parts: vec![part],
        flags: cx.req.opts.flags.clone(),
        pieces,
        piece: false,
        cells,
    };
    let step = if level_part { Step::Light } else { Step::DetailLight };
    let lb = LevelBuild::of(cx.session);
    *lb.running.lock() = Some((service::Running::Compile, cx.ctx.id));
    let out = crate::compile::run(cx.editor, req, cx.ctx).await;
    *lb.running.lock() = None;
    // the faces the compiler refused, drawn in the viewport and listed
    if step == Step::Light {
        show_errors(cx).await;
    }
    let report = out?;
    for part in report.get("parts").and_then(Value::as_array).into_iter().flatten() {
        for w in part.get("warnings").and_then(Value::as_array).into_iter().flatten() {
            if let Some(t) = w.as_str() {
                cx.problems.push(LevelProblem {
                    step,
                    code: "compiler",
                    severity: Severity::Warning,
                    text: t.trim().to_owned(),
                    object: None,
                    position: None,
                });
            }
        }
        let invalid = part.get("invalid_faces").and_then(Value::as_u64).unwrap_or(0);
        if invalid > 0 {
            cx.problems.push(LevelProblem {
                step,
                code: "invalid_faces",
                severity: Severity::Warning,
                text: format!(
                    "The compiler kept {invalid} invalid faces; the viewport shows where they are."
                ),
                object: None,
                position: None,
            });
        }
    }
    let files: Vec<String> = report
        .get("parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|p| p.get("files").and_then(Value::as_array).cloned().unwrap_or_default())
        .filter_map(|f| f.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect();
    Ok(Made { files, note: report, ..Made::default() })
}

/// The module's folder of a level's pieces: `<project>\levels\<level>\pieces`, beside the
/// level's other module files (`overlay.xcform`, `overlay_visuals.ltx`).
pub(crate) fn pieces_dir(session: &ProjectSession, level: &str) -> PathBuf {
    gz_export_level::gather::level_dir(&session.root, level).join("pieces")
}

/// Where a piece's build works: `<project>\temp\piece_<level>` - its build project, its
/// compiler's scratch - never beside the piece, which ships.
pub(crate) fn piece_scratch(session: &ProjectSession, level: &str) -> PathBuf {
    session.root.join("temp").join(format!("piece_{level}"))
}

/// The files a module's piece build wrote, project-relative, `/` between the names.
fn piece_files(level: &str, report: &Value) -> Vec<String> {
    let mut files = Vec::new();
    for part in report.get("parts").and_then(Value::as_array).into_iter().flatten() {
        for p in part.get("pieces").and_then(Value::as_array).into_iter().flatten() {
            let name = p.get("name").and_then(Value::as_str).unwrap_or_default();
            for f in p.get("files").and_then(Value::as_array).into_iter().flatten() {
                if let Some(n) = f.get("name").and_then(Value::as_str) {
                    files.push(format!("levels/{level}/pieces/{name}/{}", n.replace('\\', "/")));
                }
            }
        }
    }
    files
}

/// A module's piece of a level of the game: the static objects the module places compiled and lit
/// beside the rest of the scene (its occluders) into `levels\<level>\pieces\main`, their collision
/// into the module's `overlay.xcform` with the cut list it holds kept. The build project and the
/// compiler's scratch stay under the project's `temp`.
async fn step_piece(cx: &mut StepCx<'_>) -> Result<Made, JobError> {
    let scratch = piece_scratch(cx.session, &cx.level);
    let pieces = pieces_dir(cx.session, &cx.level);
    let (root, level) = (cx.session.root.clone(), cx.level.clone());
    let mut scene = cx.work.clone();
    scene.meta.level = cx.req.opts.quality.apply(&scene.meta.level);
    let scene = Arc::new(scene);
    cx.ctx.stage("Building the piece's geometry");
    let (lib, engine, s2) = (cx.inp.lib.clone(), cx.inp.engine.clone(), scene.clone());
    let job = cx.ctx.clone();
    let (existing, overlay_visuals) = {
        let dir = gz_export_level::gather::level_dir(&root, &level);
        cx.editor
            .exec
            .io(move || {
                let listed = std::fs::read(dir.join("overlay_visuals.ltx")).is_ok_and(|t| {
                    String::from_utf8_lossy(&t).lines().any(|l| l.trim_start().starts_with("[v_"))
                });
                (std::fs::read(dir.join("overlay.xcform")).unwrap_or_default(), listed)
            })
            .await
    };
    // the piece draws every static object of the module's own; one exported as an overlay visual
    // before is drawn by both
    if overlay_visuals {
        cx.problems.push(LevelProblem {
            step: Step::Piece,
            code: "overlay_visuals",
            severity: Severity::Warning,
            text: format!(
                "levels\\{level}\\overlay_visuals.ltx lists exported visuals: an object exported there is in the level piece too and is drawn twice. Remove its entry and its file there."
            ),
            object: None,
            position: None,
        });
    }
    let (project, log, collision, edits) = cx
        .editor
        .exec
        .cpu(move || -> crate::Result<(Vec<u8>, Vec<Finding>, gz_fmt_level::XcForm, LevelEdits)> {
            let look = crate::files::lookup(lib.clone());
            let stop = || job.cancel.is_cancelled();
            let c = prj::compile_piece_until(&s2, &look, &engine, &stop)?;
            let sector =
                if existing.is_empty() { 0 } else { gz_fmt_level::XcForm::read_back(&existing).0.sector };
            // on a level of the game a terrain is always the module's own
            let form = gz_export_level::overlay::collision_of(
                &s2,
                &|o| (o.marks.authored || o.class == ObjClass::Terrain) && !o.marks.removed,
                i64::from(sector),
                &look,
                &existing,
                "The module's static objects on the level have no triangle to collide with.",
            )
            .map_err(|e| crate::BuildError::new(e.0))?;
            Ok((c.project.write(), c.log, form.form, LevelEdits::of(&s2, &lib)))
        })
        .await
        .map_err(job_err)?;
    log_lines(cx.editor, &log);
    cx.ctx.check()?;
    if !edits.ai_edits.is_empty() && !service::game_states(cx.editor, service::AI_EDITS).await {
        cx.problems.push(LevelProblem {
            step: Step::Piece,
            code: "no_ai_edits",
            severity: Severity::Warning,
            text: format!(
                "The linked game does not move a level's AI nodes (it does not state the ability {}): under the terrains its creatures walk the old heights of {level}.",
                service::AI_EDITS
            ),
            object: None,
            position: None,
        });
    }
    write_files(cx.editor, scratch.clone(), vec![("build.prj".to_owned(), project)]).await?;
    let (guard, d2) = (write_guard(cx.editor), pieces.clone());
    cx.editor.exec.io(move || guard.check(&d2).map_err(|e| e.to_string())).await.map_err(JobError::new)?;
    let req = crate::compile::Request {
        level: cx.level.clone(),
        dir: scratch,
        parts: vec![crate::compile::Part::Level],
        flags: cx.req.opts.flags.clone(),
        pieces: Some(pieces),
        piece: true,
        cells: 0,
    };
    let lb = LevelBuild::of(cx.session);
    *lb.running.lock() = Some((service::Running::Compile, cx.ctx.id));
    let out = crate::compile::run(cx.editor, req, cx.ctx).await;
    *lb.running.lock() = None;
    let report = out?;
    for part in report.get("parts").and_then(Value::as_array).into_iter().flatten() {
        for w in part.get("warnings").and_then(Value::as_array).into_iter().flatten() {
            if let Some(t) = w.as_str() {
                cx.problems.push(LevelProblem {
                    step: Step::Piece,
                    code: "compiler",
                    severity: Severity::Warning,
                    text: t.trim().to_owned(),
                    object: None,
                    position: None,
                });
            }
        }
    }
    // the collision once the piece is in: the overlay and the picture describe the same objects
    let bytes = collision.write();
    let (guard, root2, level2) = (write_guard(cx.editor), root.clone(), level.clone());
    cx.editor
        .exec
        .io(move || {
            gz_export_level::gather::write_level_file(
                &root2,
                &level2,
                "overlay.xcform",
                &bytes,
                "overlay.xcform",
                &guard,
            )
        })
        .await
        .map_err(JobError::new)?;
    let mut files = piece_files(&level, &report);
    files.push(format!("levels/{level}/overlay.xcform"));
    // the ground the terrains replace and the AI nodes under them, beside the collision
    let (guard, root2, level2, e2) = (write_guard(cx.editor), root.clone(), level.clone(), edits.clone());
    let written =
        cx.editor.exec.io(move || e2.write(&root2, &level2, &guard)).await.map_err(JobError::new)?;
    files.extend(written.iter().map(|n| format!("levels/{level}/{n}")));
    let cuts = edits.note();
    if !edits.regions.is_empty() {
        cx.editor.log.info(format!("{}: {}.", Step::Piece.title(), edits.sentence()));
    }
    let note = json!({ "parts": report.get("parts").cloned().unwrap_or(Value::Null), "lighter": report.get("lighter").cloned().unwrap_or(Value::Null), "collision": { "triangles": collision.tris.len(), "cuts": collision.cuts.len() }, "level_cuts": cuts });
    Ok(Made { files, note, ..Made::default() })
}

/// What a module's piece takes away of a level of the game and changes under it: the regions of
/// `overlay.cut`, what each cuts, and the edits of `overlay.aiedit` (`docs/plan/LEVEL_DESIGN.md`
/// 2.6, the game's `XMS_ARCHITECTURE.md` 8.4).
#[derive(Clone, Debug, Default)]
pub(crate) struct LevelEdits {
    pub regions: Vec<gz_terrain::Region>,
    /// The library references the regions cut, every one once.
    pub references: Vec<String>,
    pub ai_edits: Vec<gz_terrain::AiEdit>,
}

/// The files of a level of the game a piece build writes beside `overlay.xcform`.
pub(crate) const CUT_FILE: &str = "overlay.cut";
pub(crate) const AI_EDIT_FILE: &str = "overlay.aiedit";

impl LevelEdits {
    /// The scene's cutting terrains over the library objects loaded now.
    pub fn of(scene: &SceneState, lib: &gz_scene::library::Library) -> Self {
        let cuts = gz_scene::objects::terrain::cuts(scene, lib);
        let mut references: Vec<String> = Vec::new();
        for r in cuts.iter().flat_map(|c| c.references.iter()) {
            if !references.iter().any(|x| x.eq_ignore_ascii_case(r)) {
                references.push(r.to_string());
            }
        }
        let ai_edits =
            if cuts.is_empty() { Vec::new() } else { gz_scene::objects::terrain::ai_edits(scene, lib) };
        Self { regions: cuts.into_iter().map(|c| c.region).collect(), references, ai_edits }
    }

    /// Writes the two files of the module's level folder, and removes each that has nothing to
    /// say; answers the names written. Blocking.
    pub fn write(
        &self,
        root: &std::path::Path,
        level: &str,
        guard: &gz_editor::fsx::WriteGuard,
    ) -> Result<Vec<String>, String> {
        let mut written = Vec::new();
        for (name, bytes) in [
            (CUT_FILE, (!self.regions.is_empty()).then(|| gz_terrain::write_cuts(&self.regions))),
            (AI_EDIT_FILE, (!self.ai_edits.is_empty()).then(|| gz_terrain::write_ai_edits(&self.ai_edits))),
        ] {
            match bytes {
                Some(b) => {
                    gz_export_level::gather::write_level_file(root, level, name, &b, name, guard)?;
                    written.push(name.to_owned());
                }
                None => remove_level_file(root, level, name, guard),
            }
        }
        // a hole in the ground with nothing in it must never be seen: such a piece never streams
        if !self.regions.is_empty() {
            let ltx = gz_export_level::gather::level_dir(root, level)
                .join("pieces")
                .join(gz_lc::PIECE_NAME)
                .join("piece.ltx");
            if let Ok(text) = std::fs::read_to_string(&ltx) {
                guard.check(&ltx).map_err(|e| e.to_string())?;
                gz_editor::fsx::write_atomic(&ltx, not_streamed(&text).as_bytes())
                    .map_err(|e| format!("{} could not be written: {e}", ltx.display()))?;
            }
        }
        Ok(written)
    }

    /// The piece step's note of them: `build.report` and the build's answer count them.
    pub fn note(&self) -> Value {
        let closed = self.ai_edits.iter().filter(|e| e.closed).count();
        json!({
            "regions": self.regions.len(),
            "references": self.references.len(),
            "ai_edits": self.ai_edits.len(),
            "closed": closed,
        })
    }

    /// A sentence of them for the log.
    pub fn sentence(&self) -> String {
        let n = |k: usize, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
        format!(
            "{} cutting {}, {}",
            n(self.regions.len(), "region", "regions"),
            n(self.references.len(), "model of the level", "models of the level"),
            n(self.ai_edits.len(), "AI node moved", "AI nodes moved"),
        )
    }
}

/// `piece.ltx`'s text with `stream = false`.
fn not_streamed(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut seen = false;
    for line in text.split_inclusive('\n') {
        let key = line.split(['=', ';']).next().unwrap_or_default().trim();
        if key.eq_ignore_ascii_case("stream") {
            let end = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            out.push_str("stream  = false");
            out.push_str(end);
            seen = true;
        } else {
            out.push_str(line);
        }
    }
    if !seen {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push_str("\r\n");
        }
        out.push_str("stream  = false\r\n");
    }
    out
}

/// Removes a file of the module's level folder a piece build wrote, when the guard lets it.
fn remove_level_file(root: &std::path::Path, level: &str, name: &str, guard: &gz_editor::fsx::WriteGuard) {
    let dir = gz_export_level::gather::level_dir(root, level);
    if guard.check(&dir).is_ok() {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

/// Why a piece with a cutting terrain cannot be built for the linked game: a game that does not
/// state `level-cuts` would draw the terrain over the ground it does not take away. The terrain's
/// name and the sentence.
pub(crate) fn cut_refusal(scene: &SceneState, level: &str, game_cuts: bool) -> Option<(String, String)> {
    if game_cuts {
        return None;
    }
    let name = service::cutting_terrain(scene)?;
    let text = format!(
        "The terrain '{name}' cuts the ground of {level}, and the linked game cannot cut a level's ground: it does not state the ability {}. Turn the terrain's cut off, or link a game that states it.",
        service::LEVEL_CUTS
    );
    Some((name, text))
}

/// What a module's piece build left, when the scene has no piece any more: the piece goes, and so
/// do the triangles it wrote into `overlay.xcform` (the cut list stays; a file of nothing else
/// goes).
async fn remove_piece(editor: &Arc<Editor>, session: &ProjectSession, level: &str, files: &[String]) {
    let dir = pieces_dir(session, level).join(gz_lc::PIECE_NAME);
    let module = gz_export_level::gather::level_dir(&session.root, level);
    let xcform = files.iter().any(|f| f.to_ascii_lowercase().ends_with("overlay.xcform"));
    // the cut and the AI edits are the piece's alone
    let edits: Vec<&'static str> = [CUT_FILE, AI_EDIT_FILE]
        .into_iter()
        .filter(|n| files.iter().any(|f| f.to_ascii_lowercase().ends_with(n)))
        .collect();
    let guard = write_guard(editor);
    let level = level.to_owned();
    let root = session.root.clone();
    editor
        .exec
        .io(move || {
            if guard.check(&dir).is_err() || guard.check(&module).is_err() {
                return;
            }
            if dir.is_dir() {
                let _ = std::fs::remove_dir_all(&dir);
            }
            for name in edits {
                remove_level_file(&root, &level, name, &guard);
            }
            let path = module.join("overlay.xcform");
            if xcform && let Ok(bytes) = std::fs::read(&path) {
                let (mut form, _) = gz_fmt_level::XcForm::read_back(&bytes);
                if form.cuts.is_empty() {
                    let _ = std::fs::remove_file(&path);
                } else {
                    form.verts = Vec::new().into();
                    form.tris = Vec::new().into();
                    form.materials.clear();
                    let _ = gz_export_level::gather::write_level_file(
                        &root,
                        &level,
                        "overlay.xcform",
                        &form.write(),
                        "overlay.xcform",
                        &guard,
                    );
                }
            }
        })
        .await;
}

/// `build.err` of the level folder into the viewport's error drawing, each refused face a problem
/// at its place.
async fn show_errors(cx: &mut StepCx<'_>) {
    let path = cx.dir.join("build.err");
    let Some(bytes) = cx.editor.exec.io(move || std::fs::read(&path).ok()).await else { return };
    let Ok(d) = gz_scene::debug::read_compiler_error(&bytes) else { return };
    for f in d.wire_faces.iter().take(100) {
        let c = (f.p[0] + f.p[1] + f.p[2]) / 3.0;
        cx.problems.push(LevelProblem {
            step: Step::Light,
            code: "invalid_face",
            severity: Severity::Warning,
            text: "A face the compiler refused: too small or with its corners in a line.".to_owned(),
            object: None,
            position: Some([c.x, c.y, c.z]),
        });
    }
    if let Ok(svc) = SceneService::of(cx.editor) {
        let _ = svc.store.send(gz_scene::SceneCmd::DebugDraw(Arc::new(d)), gz_base::Origin::Mcp).await;
    }
}

#[cfg(test)]
mod tests {
    use gz_base::Findings;

    use super::*;

    #[test]
    fn an_error_ends_the_build_naming_the_step_what_was_built_and_what_did_not_run() {
        let ran = [json!({ "title": "Game Files" }), json!({ "title": "AI Map" })];
        assert_eq!(
            ended_text(Step::Spawn, "The level has no actor spawn point.", &ran, &["Geometry", "Light"]),
            "Spawn failed: The level has no actor spawn point. Built: Game Files, AI Map. Not run: Geometry, Light."
        );
        assert_eq!(
            ended_text(Step::Light, "the compile broke", &[], &[]),
            "Light failed: the compile broke."
        );
        let stopped = job_err(crate::BuildError::stopped());
        assert!(stopped.cancelled && stopped.message == "Stopped.");
        assert!(!job_err(crate::BuildError::new("x")).cancelled);
    }

    /// A module build ships the level's pieces - the module's content - and never the piece
    /// build's scratch under `temp`, nor the level build's own files.
    #[test]
    fn a_module_ships_its_pieces_and_never_their_scratch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for f in [
            "mod.ltx",
            "levels\\l01_escape\\pieces\\main\\piece",
            "levels\\l01_escape\\pieces\\main\\piece.geom",
            "levels\\l01_escape\\pieces\\main\\piece.ltx",
            "levels\\l01_escape\\pieces\\main\\lmap#1_1.dds",
            "levels\\l01_escape\\pieces\\cell_0_-1\\piece",
            "levels\\l01_escape\\overlay.xcform",
            "levels\\l01_escape\\overlay.cut",
            "levels\\l01_escape\\overlay.aiedit",
            "temp\\piece_l01_escape\\build.prj",
            "temp\\piece_l01_escape\\build.err",
            "gamedata\\levels\\l01_escape\\build.state",
        ] {
            let p = root.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x").unwrap();
        }
        let rt =
            gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 2 })
                .unwrap();
        let walk = gz_build::BuildWalk::scan(
            &gz_project::FileTree::scan(root),
            &[],
            gz_build::WalkScope::Module,
            &rt.exec(),
        );
        let files: Vec<&str> = walk.files.iter().map(|f| f.rel.as_str()).collect();
        for want in [
            "levels/l01_escape/pieces/main/piece",
            "levels/l01_escape/pieces/main/piece.geom",
            "levels/l01_escape/pieces/main/piece.ltx",
            "levels/l01_escape/pieces/main/lmap#1_1.dds",
            "levels/l01_escape/pieces/cell_0_-1/piece",
            "levels/l01_escape/overlay.xcform",
            "levels/l01_escape/overlay.cut",
            "levels/l01_escape/overlay.aiedit",
        ] {
            assert!(files.contains(&want), "{want} does not ship: {files:?}");
        }
        // the cut and the AI edits are the module's: no file of the level that replaces the game's
        let mut record = crate::state::BuildState::new("l01_escape");
        record.steps.insert(
            Step::Piece,
            StepRecord {
                fingerprint: 1,
                time: String::new(),
                seconds: 0.0,
                quality: crate::steps::Quality::default(),
                flags: String::new(),
                library: 0,
                failed: None,
                files: vec![
                    "levels/l01_escape/overlay.cut".into(),
                    "levels/l01_escape/overlay.aiedit".into(),
                ],
            },
        );
        let level = root.join("levels").join("l01_escape");
        assert!(crate::stock::written(&record, &level).is_empty());
        assert!(files.iter().all(|f| !f.starts_with("temp/") && !f.contains("build.")), "{files:?}");
        // where the steps put them
        let session_root = std::path::Path::new("P:\\proj");
        let pieces = gz_export_level::gather::level_dir(session_root, "l01_escape").join("pieces");
        assert_eq!(pieces, session_root.join("levels").join("l01_escape").join("pieces"));
    }

    /// The cut and the AI edits of a cutting terrain, written beside the piece and read back the
    /// way the game reads them; a piece with a cut never streams; nothing to say, no file.
    #[test]
    fn a_cutting_terrain_writes_the_cut_and_the_ai_edits_of_the_level() {
        let mut s = crate::prj::terrain_fixture::scene(true);
        let nodes = [
            gz_fmt_level::AiMapNode::new(glam::Vec3::new(0.0, 0.0, 0.0), glam::Vec3::Y),
            gz_fmt_level::AiMapNode::new(glam::Vec3::new(4.0, 0.0, 0.0), glam::Vec3::Y),
            gz_fmt_level::AiMapNode::new(glam::Vec3::new(20.0, 0.0, 0.0), glam::Vec3::Y),
        ];
        let header = gz_fmt_level::SceneAiMap {
            bbox: gz_fmt_level::Aabb::new(glam::Vec3::splat(-50.0), glam::Vec3::splat(50.0)),
            ..gz_fmt_level::SceneAiMap::default()
        };
        s.bulk.ai_map = Some(Arc::new(gz_scene::tools::AiMapPart::from_nodes(header, &nodes, None)));
        let lib = crate::prj::terrain_fixture::library(&s);
        let edits = LevelEdits::of(&s, &lib);
        assert_eq!(edits.regions.len(), 1);
        assert_eq!(edits.references, ["ground\\flat"]);
        assert_eq!(edits.ai_edits.len(), 2, "the node beside the terrain is no edit");
        assert_eq!(edits.note(), json!({ "regions": 1, "references": 1, "ai_edits": 2, "closed": 0 }));
        assert_eq!(edits.sentence(), "1 region cutting 1 model of the level, 2 AI nodes moved");

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let main = root.join("levels").join("l01_escape").join("pieces").join("main");
        std::fs::create_dir_all(&main).unwrap();
        std::fs::write(
            main.join("piece.ltx"),
            gz_lc::piece_ltx(&gz_fmt_level::Aabb::new(glam::Vec3::ZERO, glam::Vec3::ONE)),
        )
        .unwrap();
        let guard = gz_editor::fsx::WriteGuard::default();
        assert_eq!(edits.write(root, "l01_escape", &guard).unwrap(), [CUT_FILE, AI_EDIT_FILE]);
        let level = root.join("levels").join("l01_escape");
        let regions = gz_terrain::read_cuts(&std::fs::read(level.join(CUT_FILE)).unwrap()).unwrap();
        assert_eq!(regions, edits.regions);
        assert_eq!((regions[0].columns, regions[0].rows), (11, 11));
        assert!(regions[0].heights.iter().all(|h| h.abs() < 1e-4), "the old ground's heights");
        let read = gz_terrain::read_ai_edits(&std::fs::read(level.join(AI_EDIT_FILE)).unwrap()).unwrap();
        assert_eq!(read, edits.ai_edits);
        assert!((read[0].new_y - 2.0).abs() < 1e-3 && read[0].old_y == 0.0);
        let ltx = std::fs::read_to_string(main.join("piece.ltx")).unwrap();
        assert!(ltx.contains("stream  = false\r\n") && !ltx.contains("true"), "{ltx}");
        // the terrain's cut off: both files go
        assert!(LevelEdits::default().write(root, "l01_escape", &guard).unwrap().is_empty());
        assert!(!level.join(CUT_FILE).exists() && !level.join(AI_EDIT_FILE).exists());
    }

    #[test]
    fn a_game_that_cannot_cut_refuses_a_cutting_terrain_by_name() {
        let s = crate::prj::terrain_fixture::scene(true);
        let (name, text) = cut_refusal(&s, "l01_escape", false).unwrap();
        assert_eq!(name, "Yard");
        assert_eq!(
            text,
            "The terrain 'Yard' cuts the ground of l01_escape, and the linked game cannot cut a level's ground: it does not state the ability level-cuts. Turn the terrain's cut off, or link a game that states it."
        );
        assert!(cut_refusal(&s, "l01_escape", true).is_none());
        assert!(cut_refusal(&crate::prj::terrain_fixture::scene(false), "l01_escape", false).is_none());
        // a manifest without the key gets it
        assert_eq!(not_streamed("[piece]\r\nversion = 1"), "[piece]\r\nversion = 1\r\nstream  = false\r\n");
    }

    #[test]
    fn only_errors_and_warnings_become_problems_and_none_starts_with_a_mark() {
        let mut found = Vec::new();
        found.error("The level has no spawn point.");
        found.warn("The level has no HOM objects.");
        found.info("The scene failed its checks.");
        let problems = finding_problems(&SceneState::default(), Step::Geometry, "validation", &found);
        assert_eq!(
            problems.iter().map(|p| p.severity).collect::<Vec<_>>(),
            [Severity::Error, Severity::Warning]
        );
        assert!(problems.iter().all(|p| !p.text.starts_with(['!', '~', '*']) && p.code == "validation"));
    }
}
