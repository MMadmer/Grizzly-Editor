//! The level build as one job: the steps a request needs, in order, each recorded in
//! `build.state` as it ends, so a Stop or a failure keeps the finished ones fresh. A step that
//! fails holds back only the steps that need it: the others still run, and the job ends failed
//! naming what failed, what was built and what was left for later.

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

/// Runs the build: plans the steps, runs each, records each. Answers what ran.
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
    let library = editor.exec.io(move || fingerprint::library_stamp(&snap, &lib)).await;
    let req = &Request { opts: Options { library, ..req.opts.clone() }, ..req.clone() };
    let level = inp.snap.meta.level.level_path.to_string();
    if level.trim().is_empty() {
        return Err(JobError::new("The scene has no level path: set it in Level Settings."));
    }
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
    let not_needed: Vec<Step> = asked.iter().copied().filter(|s| !service::needed(&work, *s)).collect();
    let plan: Vec<Step> = asked
        .iter()
        .copied()
        .filter(|s| service::needed(&work, *s) && (forced.contains(s) || stale(s)))
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
    };
    // what a step that has nothing to make made before goes, so the game reads none of it
    for s in &not_needed {
        if cx.record.steps.remove(s).is_some() {
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
        return Ok(json!({ "level": level, "ran": [], "fresh": true }));
    }
    editor.log.info(format!(
        "Building {level}: {}.",
        plan.iter().map(|s| s.title()).collect::<Vec<_>>().join(", ")
    ));
    let mut ran: Vec<Value> = Vec::new();
    let mut failed: Vec<(Step, String)> = Vec::new();
    let mut skipped: Vec<Step> = Vec::new();
    let mut objects = objects;
    let total = plan.len();
    for (k, step) in plan.iter().copied().enumerate() {
        if let Err(stop) = ctx.check() {
            finish_problems(&lb, &plan[..k], cx.problems);
            return Err(stop.into());
        }
        // a step that needs one that failed waits for it; the others go on
        let held = |n: &Step| failed.iter().any(|(f, _)| f == n) || skipped.contains(n);
        if let Some(blocker) = with_needs(&[step]).into_iter().find(|n| *n != step && held(n)) {
            editor.log.warn(format!("{} is left for later: it needs {}.", step.title(), blocker.title()));
            skipped.push(step);
            continue;
        }
        ctx.stage(format!("{} ({} of {total})", step.stage(), k + 1));
        ctx.fraction(k as f32 / total as f32);
        let begun = Instant::now();
        let out = run_step(&mut cx, step).await;
        let seconds = begun.elapsed().as_secs_f64();
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
                        library: if step.classes().contains(&gz_scene::ObjClass::SceneObject) {
                            req.opts.library
                        } else {
                            0
                        },
                        failed: None,
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
                finish_problems(&lb, &plan[..=k], cx.problems);
                return Err(e);
            }
            Err(e) => {
                let fingerprint = cx.fps.get(&step).copied().unwrap_or(0);
                cx.record.steps.insert(
                    step,
                    StepRecord {
                        fingerprint,
                        time: crate::state::now_text(),
                        seconds,
                        quality: req.opts.quality,
                        flags: req.opts.flags.clone(),
                        library: if step.classes().contains(&gz_scene::ObjClass::SceneObject) {
                            req.opts.library
                        } else {
                            0
                        },
                        failed: Some(e.message.clone()),
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
                failed.push((step, e.message));
            }
        }
    }
    ctx.fraction(1.0);
    let problems = cx.problems.len();
    finish_problems(&lb, &plan, cx.problems);
    if !failed.is_empty() {
        let mut text: Vec<String> =
            failed.iter().map(|(s, m)| format!("{} failed: {m}", s.title())).collect();
        if !ran.is_empty() {
            let built: Vec<&str> = ran.iter().filter_map(|r| r["title"].as_str()).collect();
            text.push(format!("Built: {}.", built.join(", ")));
        }
        if !skipped.is_empty() {
            let left: Vec<&str> = skipped.iter().map(|s| s.title()).collect();
            text.push(format!("Left for later: {}.", left.join(", ")));
        }
        return Err(JobError::new(text.join(" ")));
    }
    editor.log.info(format!("{level} is built."));
    Ok(json!({ "level": level, "ran": ran, "problems": problems }))
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
    let (lib, engine, settings) = (cx.inp.lib.clone(), cx.inp.engine.clone(), cx.inp.settings.clone());
    let (made, log) = cx
        .editor
        .exec
        .cpu(move || -> crate::Result<(NamedFiles, Vec<Finding>)> {
            let look = crate::files::lookup(lib);
            let mut out = vec![("level.ltx".to_owned(), crate::files::level_ltx(&scene, &look))];
            let g = crate::files::game_files(&scene, &look, &engine, sync_settings(&settings))?;
            out.extend(g.files.into_iter().map(|(n, b)| (n.to_owned(), b)));
            out.extend(g.meshes);
            Ok((out, g.log))
        })
        .await
        .map_err(|e| JobError::new(e.0))?;
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
                let model =
                    gz_scene::bulk::collide::CollideModel::build(&work, &lib, &names, xrlc.xrlc.as_ref());
                let keep_going = |f: f32, total: usize| {
                    c.stage(format!("Generating the AI map: {total} nodes"));
                    c.fraction(f);
                    !c.cancel.is_cancelled()
                };
                gz_scene::bulk::cmd::generate_map(&work, &lib, &model, &keep_going)
                    .ok_or_else(|| "The scene has no static objects for the AI map to lie on.".to_owned())
            })
            .await
            .map_err(JobError::new)?;
        cx.ctx.check()?;
        if made.nodes == 0 {
            return Err(JobError::new(
                "No AI map node could be made: no spawn point stands on ground an NPC can walk, and neither does the middle of the level. Place a graph point on the ground.",
            ));
        }
        // the map goes into the scene as one undo step, so the author sees what the game walks on
        let svc = SceneService::of(cx.editor).map_err(|e| JobError::new(e.message))?;
        let mut edit =
            gz_scene::bulk::BulkEdit::new("Generate AI Map", cx.work.serial).with_ai(made.part.clone());
        edit.kind = gz_scene::bulk::Kind::Step;
        match svc.store.send(gz_scene::SceneCmd::Bulk(Box::new(edit)), gz_base::Origin::Mcp).await {
            Ok(_) => {}
            Err(e) if e.code == "busy" => {
                return Err(JobError::new("The scene changed while the AI map was generated: build again."));
            }
            Err(e) => return Err(JobError::new(e.message)),
        }
        cx.work.bulk.ai_map = Some(Arc::new(made.part));
        cx.work.serial += 1;
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
    let built = cx
        .editor
        .exec
        .cpu(move || {
            let nav = LevelNav::new(&lg);
            let points = graph_points(&work, sync_settings(&settings));
            if points.is_empty() {
                return Err(crate::BuildError::new(
                    "The level has no graph point: place at least one on the AI map.",
                ));
            }
            graph::build_game_graph(&nav, &points, &level, lg.header.guid, guid())
        })
        .await
        .map_err(|e| JobError::new(e.0))?;
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
    let built = cx
        .editor
        .exec
        .cpu(move || {
            let nav = LevelNav::new(&lg);
            let st = sync_settings(&settings);
            let mut elements = Vec::new();
            for o in work.of_class(ObjClass::SpawnPoint) {
                if let ObjData::Spawn(pt) = &o.data
                    && pt.kind == PointKind::Spawn
                    && let Ok(Some((e, _))) = crate::files::export_entity(o, st)
                {
                    elements.push(Element { entity: e });
                }
            }
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
            spawn::build_spawn(&level, &nav, &play_graph, elements, &ways, guid(), verify)
        })
        .await
        .map_err(|e| JobError::new(e.0))?;
    log_lines(cx.editor, &built.log);
    let spawns = cx.session.root.join("gamedata").join("spawns");
    write_files(cx.editor, spawns, vec![("editor.spawn".to_owned(), built.bytes)]).await?;
    Ok(Made { files: vec!["gamedata/spawns/editor.spawn".to_owned()], ..Made::default() })
}

/// `build.prj` and the files beside it: the static geometry with its lights, glows and visibility
/// cells - the cells completed by the build - the LOD atlas, the HOM and SOM, the wallmarks.
async fn step_geometry(cx: &mut StepCx<'_>) -> Result<Made, JobError> {
    cx.ctx.stage("Completing the visibility cells");
    let (work, lib) = (Arc::new(cx.work.clone()), cx.inp.lib.clone());
    let c = cx.ctx.clone();
    let completion = cx
        .editor
        .exec
        .cpu(move || gz_scene::objtools::zones::complete(&work, &lib, &|| c.cancel.is_cancelled()))
        .await
        .map_err(|_| JobError { message: "Stopped.".to_owned(), cancelled: true })?;
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
    cx.ctx.stage("Reading the LOD textures");
    let lods = Arc::new(lod_pictures(cx.editor, &scene, &cx.inp.lib).await);
    cx.ctx.check()?;
    cx.ctx.stage("Building the geometry");
    let (lib, engine) = (cx.inp.lib.clone(), cx.inp.engine.clone());
    let s2 = scene.clone();
    let (made, log) = cx
        .editor
        .exec
        .cpu(move || -> crate::Result<(NamedFiles, Vec<Finding>)> {
            let look = crate::files::lookup(lib);
            let lod_src = |n: &str| lods.get(n).cloned();
            let c = prj::compile(&s2, &look, &engine, &lod_src)?;
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
        .map_err(|e| JobError::new(e.0))?;
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
    let req = crate::compile::Request {
        level: cx.level.clone(),
        dir: cx.dir.clone(),
        parts: vec![part],
        flags: cx.req.opts.flags.clone(),
    };
    let step = if part == crate::compile::Part::Level { Step::Light } else { Step::DetailLight };
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
