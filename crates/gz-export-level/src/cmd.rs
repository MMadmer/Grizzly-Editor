//! The exports over MCP (the wire is `EditorModScene::McpExport*`), the build's bake of the open
//! scene and the `E201` check.

use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::Origin;
use gz_build::BuildChecks;
use gz_editor::{Args, ArgsExt, CmdError, Editor, ProjectSession, Registry, Reply, json};
use gz_game::GameLink;
use gz_scene::library::{Library, LibraryObject};
use gz_scene::{Object, SceneService, SceneState};
use gz_store::Snap;

use crate::gather::{self, Gathered};
use crate::names::{json_path, resolve_level};
use crate::overlay;
use crate::service::ExportService;
use crate::spawn::{self, SpawnBake, SpawnInput};
use crate::starts;

const AREA: &str = "gz-export-level";

/// Registers `mod_export_spawn`, `mod_export_xcform`, `mod_export_cut`, `mod_export_ogf_probe`.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "mod_export_spawn", |ed, a, o| async move { export_spawn(&ed, &a, o).await });
    reg.add(AREA, "mod_export_xcform", |ed, a, _o| async move { export_xcform(&ed, &a).await });
    reg.add(AREA, "mod_export_cut", |ed, a, _o| async move { export_cut(&ed, &a).await });
    reg.add(AREA, "mod_export_ogf_probe", |ed, a, _o| async move { export_ogf(&ed, &a).await });
}

/// Fills `gz-build`'s bake slot with the mirror pass of the open scene, and `gz-world`'s
/// `SceneStarts` slot with the scene's Player Starts (`E201` counts them for the scene's level).
/// Called once at start.
pub fn install(editor: &Arc<Editor>) {
    BuildChecks::of(editor).set_bake(|ed, session| -> BoxFuture<'static, String> {
        Box::pin(async move { bake_note(&ed, &session).await })
    });
    gz_world::SceneStarts::of(editor).provide(|ed| {
        let session = ed.project_opt()?;
        let svc = session.services.get::<SceneService>()?;
        let snap = svc.store.snapshot();
        let level = resolve_level(None, &snap.meta.name()).filter(|_| snap.valid)?;
        let found = starts::scene_starts(&snap, &level);
        Some((level, found))
    });
    // the quest pickers read the open scene's restrictors, story ids and way paths
    gz_project::ProjectHooks::of(editor).on_open(|ed, _session| {
        if let Ok(nq) = gz_nq::NqService::of(ed) {
            nq.set_scene(Arc::new(crate::lookup::LiveScene(Arc::downgrade(ed))));
        }
    });
}

/// `ArgBool`: a flag is `true` only when it says so; anything else, absent included, is
/// `default` only when absent.
fn arg_bool(a: &Args, key: &str, default: bool) -> bool {
    if a.has(key) { a.bool(key).unwrap_or(false) } else { default }
}

/// `ArgInt`: `atoi` of what follows the key.
fn arg_int(a: &Args, key: &str, default: i64) -> i64 {
    if a.has(key) { a.i64(key).unwrap_or(0) } else { default }
}

/// The open project, the scene's snapshot and the level an export is for.
struct Ctx {
    session: Arc<ProjectSession>,
    svc: Arc<SceneService>,
    snap: Arc<Snap<SceneState>>,
    level: String,
}

fn ctx(editor: &Arc<Editor>, a: &Args) -> Result<Ctx, CmdError> {
    let session = editor.project()?;
    let svc = SceneService::of(editor)?;
    let snap = svc.store.snapshot();
    if !snap.valid {
        return Err(CmdError::invalid("no scene"));
    }
    let level = resolve_level(a.str("level"), &snap.meta.name())
        .ok_or_else(|| CmdError::invalid("no level name - open a scene or pass 'level'"))?;
    Ok(Ctx { session, svc, snap, level })
}

fn log_lines(editor: &Editor, lines: &[String]) {
    for l in lines {
        if l.starts_with('!') { editor.log.warn(l) } else { editor.log.info(l) }
    }
}

/// The library objects the scene's objects name, loaded on the IO pool.
async fn library(editor: &Arc<Editor>, svc: &Arc<SceneService>, scene: &SceneState) -> Arc<Library> {
    svc.prefetch_library(editor, scene).await;
    svc.library.clone()
}

fn lookup(lib: Arc<Library>) -> impl Fn(&str) -> Option<Arc<LibraryObject>> + Sync {
    move |name: &str| lib.cached(name)
}

/// One spawn pass: gathered, baked on the CPU pool, written on the IO pool.
struct SpawnRun {
    bake: SpawnBake,
    path: Option<PathBuf>,
}

async fn run_spawn(
    editor: &Arc<Editor>,
    c: &Ctx,
    selected_only: bool,
    section: Option<String>,
    mode: Option<String>,
    mirror: bool,
) -> Result<SpawnRun, String> {
    let settings = gz_scene::game::settings(editor).await;
    let patrols = ExportService::game_patrols(editor).await;
    let game = match GameLink::of(editor) {
        Ok(link) => link.files().await.ok(),
        Err(_) => None,
    };
    let root = c.session.root.clone();
    let sdk = editor.paths.root.join("gamedata").join("meshes");
    let (snap, level) = (c.snap.clone(), c.level.clone());
    let st = settings.clone();
    let gathered: Gathered = editor
        .exec
        .io(move || {
            let s = st.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync));
            gather::spawn_inputs(&root, &level, &snap, selected_only, s, game.as_deref(), &sdk)
        })
        .await;
    let (snap, level) = (c.snap.clone(), c.level.clone());
    let g2 = gathered.clone();
    let bake = editor
        .exec
        .cpu(move || {
            let input = SpawnInput {
                scene: &snap,
                level,
                selected_only,
                section,
                mode,
                mirror,
                visuals: &g2.visuals,
                settings: settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync)),
                game_patrols: &patrols,
                old_text: &g2.old_text,
            };
            spawn::bake(&input)
        })
        .await
        .map_err(|e| e.0)?;
    let (root, level) = (c.session.root.clone(), c.level.clone());
    let stale = gathered.stale_custom_data;
    let (bake, written) = editor
        .exec
        .io(move || {
            let r = gather::apply_spawn(&root, &level, mirror, &stale, &bake);
            (bake, r)
        })
        .await;
    log_lines(editor, &bake.log);
    let (path, failed) = written?;
    log_lines(editor, &failed);
    if let Some(p) = &path {
        editor.log.info(format!("* [XMS] spawn layer: {} op(s) -> {}", bake.result.exported, p.display()))
    }
    Ok(SpawnRun { bake, path })
}

async fn export_spawn(editor: &Arc<Editor>, a: &Args, _o: Origin) -> Reply {
    let c = ctx(editor, a)?;
    let selected_only = arg_bool(a, "selected_only", true);
    let run = run_spawn(
        editor,
        &c,
        selected_only,
        a.text("section").map(str::to_owned),
        a.text("mode").map(str::to_owned),
        false,
    )
    .await
    .map_err(CmdError::invalid)?;
    let r = &run.bake.result;
    Ok(json!({
        "exported": r.exported,
        "skipped": r.skipped,
        "scaled": r.scaled,
        "invisible": r.invisible,
        "starts": r.starts,
        "paths": r.paths,
        "orders": r.orders,
        "path": run.path.as_deref().map(json_path).unwrap_or_default(),
        "names": r.names,
        "warnings": r.warnings,
    }))
}

async fn export_xcform(editor: &Arc<Editor>, a: &Args) -> Reply {
    let c = ctx(editor, a)?;
    let sector = arg_int(a, "sector", 0);
    let selected_only = arg_bool(a, "selected_only", true);
    let lib = library(editor, &c.svc, &c.snap).await;
    let (root, level) = (c.session.root.clone(), c.level.clone());
    let existing = editor
        .exec
        .io(move || {
            std::fs::read(gather::level_dir(&root, &level).join("overlay.xcform")).unwrap_or_default()
        })
        .await;
    let snap = c.snap.clone();
    let made = editor
        .exec
        .cpu(move || {
            let look = lookup(lib);
            overlay::collision(&snap, selected_only, sector, &look, &existing)
        })
        .await
        .map_err(|e| CmdError::invalid(e.0))?;
    let bytes = made.form.write();
    let (root, level) = (c.session.root.clone(), c.level.clone());
    let path = editor
        .exec
        .io(move || gather::write_level_file(&root, &level, "overlay.xcform", &bytes, "overlay.xcform"))
        .await
        .map_err(CmdError::invalid)?;
    editor.log.info(format!(
        "* [XMS] collision overlay: {} object(s), {} tri(s) -> {}",
        made.objects_used,
        made.form.tris.len(),
        path.display()
    ));
    Ok(
        json!({ "path": json_path(&path), "vertices": made.form.verts.len(), "tris": made.form.tris.len(), "materials": made.form.materials.len() }),
    )
}

async fn export_cut(editor: &Arc<Editor>, a: &Args) -> Reply {
    let c = ctx(editor, a)?;
    let selected_only = arg_bool(a, "selected_only", true);
    let overlap = arg_bool(a, "overlap", false);
    // centimetres
    let grow = arg_int(a, "grow", 0) as f32 / 100.0;
    let lib = library(editor, &c.svc, &c.snap).await;
    let (root, level) = (c.session.root.clone(), c.level.clone());
    let (xcform, ltx) = editor
        .exec
        .io(move || {
            let dir = gather::level_dir(&root, &level);
            let x = std::fs::read(dir.join("overlay.xcform")).unwrap_or_default();
            let l = std::fs::read(dir.join("overlay_visuals.ltx"))
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .unwrap_or_default();
            (x, l)
        })
        .await;
    let snap = c.snap.clone();
    let made = editor
        .exec
        .cpu(move || {
            let bounds = |o: &Object| {
                gz_scene::objects::world_box(&snap, o, &|r: &str| lib.cached(r).map(|l| l.bounds))
            };
            overlay::cut(&snap, selected_only, overlap, grow, &bounds, &xcform, &ltx)
        })
        .await
        .map_err(|e| CmdError::invalid(e.0))?;
    let bytes = made.form.write();
    let total = made.form.cuts.len();
    let (root, level) = (c.session.root.clone(), c.level.clone());
    let ltx = made.ltx.clone();
    let path = editor
        .exec
        .io(move || -> Result<PathBuf, String> {
            let p = gather::write_level_file(&root, &level, "overlay.xcform", &bytes, "overlay.xcform")?;
            gather::write_level_file(
                &root,
                &level,
                "overlay_visuals.ltx",
                ltx.as_bytes(),
                "overlay_visuals.ltx",
            )
            .map_err(|_| "cuts written, but overlay_visuals.ltx is not writable".to_owned())?;
            Ok(p)
        })
        .await
        .map_err(CmdError::invalid)?;
    editor.log.info(format!(
        "* [XMS] level cut: {} new volume(s), {total} total -> {} (+ hide entries in overlay_visuals.ltx)",
        made.added,
        path.display()
    ));
    Ok(json!({ "cuts": made.added, "overlap": overlap, "path": json_path(&path) }))
}

/// `\\computer\user`, the build name the C++ stamps into an exported model.
fn build_name() -> String {
    let pc = std::env::var("COMPUTERNAME").unwrap_or_default();
    let user = std::env::var("USERNAME").unwrap_or_default();
    format!("\\\\{pc}\\{user}")
}

async fn export_ogf(editor: &Arc<Editor>, a: &Args) -> Reply {
    let c = ctx(editor, a)?;
    let selected_only = arg_bool(a, "selected_only", true);
    let lib = library(editor, &c.svc, &c.snap).await;
    let dir = gather::level_dir(&c.session.root, &c.level);
    let ltx_path = dir.join("overlay_visuals.ltx");
    let lp = ltx_path.clone();
    let registry_text = editor
        .exec
        .io(move || std::fs::read(&lp).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default())
        .await;
    let snap = c.snap.clone();
    // the game's materials: a surface whose material the game lacks is not exported (C++
    // ExportObjectOGF against GameMaterialLibrary)
    let materials = match gz_game::read_game_or_bundled(editor, "gamemtl.xr").await {
        Ok((bytes, _)) => {
            editor.exec.cpu(move || gz_fmt_xr::GameMaterials::read(&bytes).ok().map(Arc::new)).await
        }
        Err(_) => None,
    };
    let made = editor
        .exec
        .cpu(move || {
            let look = lookup(lib);
            overlay::visuals(&snap, selected_only, &look, &build_name(), materials)
        })
        .await;
    let visuals_dir = dir.join("visuals");
    let made = match made {
        Ok(m) => m,
        Err(e) => return Err(CmdError::invalid(e.0)),
    };
    log_lines(editor, &made.log);
    let vd = visuals_dir.clone();
    let files: Vec<(String, PathBuf, Vec<u8>)> = made
        .files
        .iter()
        .map(|f| (f.name.clone(), vd.join(format!("{}.ogf", f.name)), f.bytes.clone()))
        .collect();
    let exec = editor.exec.clone();
    let written: Vec<(String, bool)> = editor
        .exec
        .io(move || {
            let _ = std::fs::create_dir_all(&vd);
            exec.par_io(files, |(n, p, b)| (n, gz_editor::fsx::write_atomic(&p, &b).is_ok()))
        })
        .await;
    let names: Vec<String> = written.iter().filter(|(_, ok)| *ok).map(|(n, _)| n.clone()).collect();
    let skipped_failed = made.skipped_failed + u32::try_from(written.len() - names.len()).unwrap_or(0);
    if names.is_empty() {
        return Err(CmdError::invalid(
            "nothing exported - only skeleton/dynamic or unresolvable objects in the selection",
        ));
    }
    let ltx = overlay::registry(&registry_text, &names);
    editor
        .exec
        .io(move || gz_editor::fsx::write_atomic(&ltx_path, ltx.as_bytes()))
        .await
        .map_err(|_| CmdError::invalid("visuals exported but can't write overlay_visuals.ltx"))?;
    editor.log.info(format!("* [XMS] visual overlay: {} ogf(s) -> {}", names.len(), visuals_dir.display()));
    Ok(json!({
        "exported": names.len(),
        "skipped_dynamic": made.skipped_dynamic,
        "skipped_failed": skipped_failed,
        "dir": json_path(&visuals_dir),
        "ltx": "overlay_visuals.ltx",
        "names": names,
    }))
}

/// `BakeCurrentSceneSpawn`: the open scene mirrored into its level's `.xspawn` before a build,
/// as a note - a line for the layer and a `WARNING: ` line per thing left out. Empty without an
/// open scene.
async fn bake_note(editor: &Arc<Editor>, session: &Arc<ProjectSession>) -> String {
    let Some(svc) = session.services.get::<SceneService>() else { return String::new() };
    let snap = svc.store.snapshot();
    let Some(level) = resolve_level(None, &snap.meta.name()) else { return String::new() };
    if !snap.valid {
        return String::new();
    }
    let c = Ctx { session: session.clone(), svc, snap, level: level.clone() };
    let (mut note, warnings) = match run_spawn(editor, &c, false, None, None, true).await {
        Ok(run) => {
            let r = &run.bake.result;
            let mut n = if r.exported > 0 {
                format!("Scene baked: {} object(s) -> spawn\\{level}.xspawn", r.exported)
            } else {
                format!("Scene baked: empty, spawn\\{level}.xspawn cleared")
            };
            if r.missing > 0 {
                n.push_str(&format!(
                    "\nWARNING: {} object(s) NOT baked - their model exists neither in the game nor in the project (see the log)",
                    r.missing
                ));
            }
            if r.invisible > 0 {
                n.push_str(&format!(
                    "\nWARNING: {} object(s) keep a model the engine draws nothing for - they work but stay invisible (see the log)",
                    r.invisible
                ));
            }
            (n, r.warnings.clone())
        }
        Err(e) => (format!("Scene bake FAILED for '{level}': {e}"), Vec::new()),
    };
    // every left-out path and order, by name: the note is what the build shows
    for w in warnings {
        note.push_str("\nWARNING: ");
        note.push_str(&w);
    }
    note
}
