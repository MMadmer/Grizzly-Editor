//! The module's level layers of the open scene - the spawn layer, the collision and visual
//! overlays, the level cut - as `level.export` runs them (`gz-levelbuild` registers the command),
//! the bake of the whole scene the level build and the module build run, and the `E201` check.

use std::path::PathBuf;
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::Finding;
use gz_build::BuildChecks;
use gz_editor::fsx::WriteGuard;
use gz_editor::{CmdError, Editor, ProjectSession, Reply, Value, json};
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

/// What an export of a level layer takes.
#[derive(Clone, Debug, Default)]
pub struct Layer {
    /// The level the layer is for; `None`: the open scene's.
    pub level: Option<String>,
    /// The objects it is made of, by name; `None`: the selection.
    pub objects: Option<Vec<String>>,
    /// Every object of the scene instead.
    pub all: bool,
    /// The spawn layer: only the spawn points of this section.
    pub section: Option<String>,
    /// The spawn layer: `add`, `remove` or both (left out).
    pub mode: Option<String>,
    /// The collision overlay: the sector its faces are in.
    pub sector: i64,
    /// The level cut: also hide the game's visuals that only touch a box.
    pub overlap: bool,
    /// The level cut: the boxes grown by this many centimetres.
    pub grow_cm: i64,
}

/// Fills `gz-build`'s bake slot with the mirror pass of the open scene, and `gz-world`'s
/// `SceneStarts` slot with the scene's Player Starts (`E201` counts them for the scene's level).
/// Called once at start.
pub fn install(editor: &Arc<Editor>) {
    BuildChecks::of(editor).set_bake(|ed, session| -> BoxFuture<'static, Vec<Finding>> {
        Box::pin(async move { bake_note(&ed, &session).await })
    });
    gz_world::SceneStarts::of(editor).provide(|ed| {
        let session = ed.project_opt()?;
        let svc = session.services.get::<SceneService>()?;
        let snap = svc.store.snapshot();
        let level = resolve_level(None, &snap.meta.level_name()).filter(|_| snap.valid)?;
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

/// The open project, the scene's snapshot and the level an export is for.
struct Ctx {
    session: Arc<ProjectSession>,
    /// What every export's writes pass: nothing under the linked game.
    guard: WriteGuard,
    svc: Arc<SceneService>,
    snap: Arc<Snap<SceneState>>,
    level: String,
}

fn ctx(editor: &Arc<Editor>, l: &Layer) -> Result<Ctx, CmdError> {
    let session = editor.project()?;
    let svc = SceneService::of(editor)?;
    let mut snap = svc.store.snapshot();
    if !snap.valid {
        return Err(CmdError::not_found("No scene is open."));
    }
    let level = resolve_level(l.level.as_deref(), &snap.meta.level_name())
        .ok_or_else(|| CmdError::invalid("no level name - open a scene or pass 'level'"))?;
    // the objects named stand for the selection the exports read
    if let Some(names) = &l.objects {
        let mut state = snap.state.clone();
        state.selection = gz_store::imbl::OrdSet::new();
        for n in names {
            let o = state
                .find_relaxed(n)
                .ok_or_else(|| CmdError::not_found(format!("there is no object '{n}' in the scene")))?;
            state.selection.insert(o.id);
        }
        snap = Arc::new(Snap { rev: snap.rev, edit: snap.edit, state });
    }
    let guard = gz_project::Project::of(editor).map(|(_, p)| p.write_guard(None)).unwrap_or_default();
    Ok(Ctx { session, guard, svc, snap, level })
}

fn log_lines(editor: &Editor, found: &[Finding]) {
    for f in found {
        editor.log.finding(f);
    }
}

/// The library objects the scene's objects name, loaded on the IO pool.
async fn library(editor: &Arc<Editor>, svc: &Arc<SceneService>, scene: &SceneState) -> Arc<Library> {
    svc.library_for(editor, scene).await
}

fn lookup(lib: Arc<Library>) -> impl Fn(&str) -> Option<Arc<LibraryObject>> + Sync {
    move |name: &str| lib.cached(name)
}

/// One spawn pass: gathered, baked on the CPU pool, written on the IO pool.
struct SpawnRun {
    bake: SpawnBake,
    path: Option<PathBuf>,
    /// What the pass wrote into the level's `overlay.things`, when it took the whole scene.
    things: crate::things::Counts,
    /// What the file could not say as the scene has it, a sentence each.
    left_out: Vec<String>,
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
    let radio = gz_scene::game::radio_cx(editor).await;
    let game = match GameLink::of(editor) {
        Ok(link) => link.files().await.ok(),
        Err(_) => None,
    };
    // whether the game loads a module's piece of a level: its static objects then go there
    let game_root = gz_project::Project::of(editor).map(|(_, p)| p.game_root()).unwrap_or_default();
    let statics_in_piece = !game_root.is_empty()
        && editor
            .exec
            .io(move || {
                gz_game::version::game_states(
                    std::path::Path::new(&game_root),
                    &gz_game::version::LINKED_GAME,
                    gz_game::version::words::LEVEL_PIECES,
                )
            })
            .await;
    // the level is the game's: the scene lies over it, or the game holds its files
    let of_game = c.snap.meta.base.is_set()
        || game.as_ref().is_some_and(|g| g.exists(&format!("levels/{}/level", c.level.to_lowercase())));
    let root = c.session.root.clone();
    let sdk = gz_project::library_root(editor).join("gamedata").join("meshes");
    let (snap, level) = (c.snap.clone(), c.level.clone());
    let st = settings.clone();
    let library = c.session.services.get::<gz_scene::SceneService>().map(|svc| svc.library.clone());
    let gathered: Gathered = editor
        .exec
        .io(move || {
            let s = st.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync));
            let sources =
                gather::ModelSources { game: game.as_deref(), sdk_meshes: &sdk, library: library.as_deref() };
            gather::spawn_inputs(&root, &level, &snap, selected_only, s, &sources)
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
                radio: Some(&radio),
                old_text: &g2.old_text,
                statics_in_piece,
            };
            spawn::bake(&input)
        })
        .await
        .map_err(|e| e.0)?;
    let (root, level) = (c.session.root.clone(), c.level.clone());
    let stale = gathered.stale_custom_data;
    let guard = c.guard.clone();
    let (bake, written) = editor
        .exec
        .io(move || {
            let r = gather::apply_spawn(&root, &level, mirror, &stale, &bake, &guard);
            (bake, r)
        })
        .await;
    log_lines(editor, &bake.log);
    let (path, failed) = written?;
    log_lines(editor, &failed);
    if let Some(p) = &path {
        editor.log.info(format!("[XMS] spawn layer: {} op(s) -> {}", bake.result.exported, p.display()))
    }
    // what the level takes from the module as text, the file the game puts beside the level's
    // own: over a level of the game everything the module adds there, on a level of the module's
    // own what the level's own files cannot carry
    let mut things = crate::things::Counts::default();
    let mut left_out = Vec::new();
    if mirror {
        let over = if of_game { crate::things::Over::Game } else { crate::things::Over::Own };
        let made = crate::things::text(&c.snap, over);
        if let Some(t) = &made {
            things = t.counts;
            left_out.clone_from(&t.warnings);
        }
        let text = made.filter(|t| t.counts.total() > 0).map(|t| t.text);
        let (root, level, guard) = (c.session.root.clone(), c.level.clone(), c.guard.clone());
        let holds = things.total();
        let kept = editor
            .exec
            .io(move || -> Result<bool, String> {
                let at = crate::things::path(&root, &level);
                // a file somebody wrote is theirs: the build neither replaces nor removes it
                let theirs =
                    std::fs::read(&at).is_ok_and(|b| !crate::things::is_builds(&String::from_utf8_lossy(&b)));
                if theirs {
                    return Ok(true);
                }
                match text {
                    Some(text) => {
                        gather::write_level_file(
                            &root,
                            &level,
                            "overlay.things",
                            text.as_bytes(),
                            "overlay.things",
                            &guard,
                        )?;
                    }
                    None if at.is_file() => {
                        guard.check(&at).map_err(|e| e.to_string())?;
                        std::fs::remove_file(&at).map_err(|e| {
                            format!("overlay.things could not be removed: {}", gz_base::io_reason(&e))
                        })?;
                    }
                    None => {}
                }
                Ok(false)
            })
            .await?;
        if kept {
            left_out.push(format!(
                "levels\\{}\\overlay.things was not made by the level build and is left as it is{}. Remove the file to let the build make it.",
                c.level,
                if holds > 0 { format!(": {holds} thing(s) of the scene are not written into it") } else { String::new() }
            ));
            things = crate::things::Counts::default();
        }
        if things.total() > 0 {
            let said: Vec<String> =
                things.named().iter().filter(|(_, n)| *n > 0).map(|(k, n)| format!("{n} {k}")).collect();
            editor.log.info(format!(
                "[XMS] level things: {} -> levels\\{}\\overlay.things",
                said.join(", "),
                c.level
            ));
        }
    }
    Ok(SpawnRun { bake, path, things, left_out })
}

/// The spawn layer of the selection (or the objects named, or the whole scene), merged into the
/// level's `.xspawn`.
pub async fn export_spawn(editor: &Arc<Editor>, l: &Layer) -> Reply {
    let c = ctx(editor, l)?;
    let run = run_spawn(editor, &c, !l.all, l.section.clone(), l.mode.clone(), false)
        .await
        .map_err(|e| CmdError::new("failed", e))?;
    let r = &run.bake.result;
    Ok(json!({
        "exported": r.exported,
        "skipped": r.skipped,
        "scaled": r.scaled,
        "invisible": r.invisible,
        "starts": r.starts,
        "paths": r.paths,
        "orders": r.orders,
        "radios": r.radios,
        // the bake's "Base objects removed" row and its missing-model warning
        "missing": r.missing,
        "removed": r.removed,
        "path": run.path.as_deref().map(json_path).unwrap_or_default(),
        "names": r.names,
        "warnings": r.warnings,
    }))
}

/// The collision overlay (`levels\<level>\overlay.xcform`) of the selection's static objects.
pub async fn export_collision(editor: &Arc<Editor>, l: &Layer) -> Reply {
    let c = ctx(editor, l)?;
    let sector = l.sector;
    let selected_only = !l.all;
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
        .map_err(|e| CmdError::new("failed", e.0))?;
    let bytes = made.form.write();
    let (root, level, guard) = (c.session.root.clone(), c.level.clone(), c.guard.clone());
    let path = editor
        .exec
        .io(move || {
            gather::write_level_file(&root, &level, "overlay.xcform", &bytes, "overlay.xcform", &guard)
        })
        .await
        .map_err(|e| CmdError::new("io", e))?;
    editor.log.info(format!(
        "[XMS] collision overlay: {} object(s), {} tri(s) -> {}",
        made.objects_used,
        made.form.tris.len(),
        path.display()
    ));
    Ok(
        json!({ "path": json_path(&path), "vertices": made.form.verts.len(), "tris": made.form.tris.len(), "materials": made.form.materials.len() }),
    )
}

/// The level cut: the selection's boxes hide the game's geometry and visuals there.
pub async fn export_cut(editor: &Arc<Editor>, l: &Layer) -> Reply {
    let c = ctx(editor, l)?;
    let selected_only = !l.all;
    let overlap = l.overlap;
    let grow = l.grow_cm as f32 / 100.0;
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
        .map_err(|e| CmdError::new("failed", e.0))?;
    let bytes = made.form.write();
    let total = made.form.cuts.len();
    let (root, level, guard) = (c.session.root.clone(), c.level.clone(), c.guard.clone());
    let ltx = made.ltx.clone();
    let path = editor
        .exec
        .io(move || -> Result<PathBuf, String> {
            let p =
                gather::write_level_file(&root, &level, "overlay.xcform", &bytes, "overlay.xcform", &guard)?;
            gather::write_level_file(
                &root,
                &level,
                "overlay_visuals.ltx",
                ltx.as_bytes(),
                "overlay_visuals.ltx",
                &guard,
            )
            .map_err(|_| "cuts written, but overlay_visuals.ltx is not writable".to_owned())?;
            Ok(p)
        })
        .await
        .map_err(|e| CmdError::new("io", e))?;
    editor.log.info(format!(
        "[XMS] level cut: {} new volume(s), {total} total -> {} (+ hide entries in overlay_visuals.ltx)",
        made.added,
        path.display()
    ));
    Ok(json!({ "cuts": made.added, "overlap": overlap, "path": json_path(&path) }))
}

/// Who built an exported model, as its description says it: the editor and its version. Never
/// the computer or the user - the file ships with the module, and the same export is the same
/// bytes on every machine.
const BUILD_NAME: &str = concat!("Grizzly Editor ", env!("CARGO_PKG_VERSION"));

/// The visual overlay: each static object of the selection as a model in world space, with its
/// registry entry.
pub async fn export_visuals(editor: &Arc<Editor>, l: &Layer) -> Reply {
    let c = ctx(editor, l)?;
    let selected_only = !l.all;
    let lib = library(editor, &c.svc, &c.snap).await;
    let dir = gather::level_dir(&c.session.root, &c.level);
    let ltx_path = dir.join("overlay_visuals.ltx");
    let lp = ltx_path.clone();
    let registry_text = editor
        .exec
        .io(move || std::fs::read(&lp).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default())
        .await;
    let snap = c.snap.clone();
    // the game's materials: a surface whose material the game lacks is not exported
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
            overlay::visuals(&snap, selected_only, &look, BUILD_NAME, materials)
        })
        .await;
    let visuals_dir = dir.join("visuals");
    let made = match made {
        Ok(m) => m,
        Err(e) => return Err(CmdError::new("failed", e.0)),
    };
    log_lines(editor, &made.log);
    let vd = visuals_dir.clone();
    let files: Vec<(String, PathBuf, Vec<u8>)> = made
        .files
        .iter()
        .map(|f| (f.name.clone(), vd.join(format!("{}.ogf", f.name)), f.bytes.clone()))
        .collect();
    let exec = editor.exec.clone();
    let guard = c.guard.clone();
    let written: Vec<(String, bool)> = editor
        .exec
        .io(move || {
            guard.check(&vd).map_err(|e| e.to_string())?;
            let _ = std::fs::create_dir_all(&vd);
            Ok::<_, String>(exec.par_io(files, |(n, p, b)| (n, gz_editor::fsx::write_atomic(&p, &b).is_ok())))
        })
        .await
        .map_err(|e| CmdError::new("io", e))?;
    let names: Vec<String> = written.iter().filter(|(_, ok)| *ok).map(|(n, _)| n.clone()).collect();
    let skipped_failed = made.skipped_failed + u32::try_from(written.len() - names.len()).unwrap_or(0);
    if names.is_empty() {
        return Err(CmdError::new(
            "failed",
            "Nothing was exported: the selection holds only skeleton, dynamic or unresolvable objects.",
        ));
    }
    let ltx = overlay::registry(&registry_text, &names);
    let guard = c.guard.clone();
    editor
        .exec
        .io(move || {
            guard.check(&ltx_path).map_err(|e| e.to_string())?;
            gz_editor::fsx::write_atomic(&ltx_path, ltx.as_bytes())
                .map_err(|_| "visuals exported but can't write overlay_visuals.ltx".to_owned())
        })
        .await
        .map_err(|e| CmdError::new("io", e))?;
    editor.log.info(format!("[XMS] visual overlay: {} ogf(s) -> {}", names.len(), visuals_dir.display()));
    Ok(json!({
        "exported": names.len(),
        "skipped_dynamic": made.skipped_dynamic,
        "skipped_failed": skipped_failed,
        "dir": json_path(&visuals_dir),
        "ltx": "overlay_visuals.ltx",
        "names": names,
    }))
}

/// The spawn layer of the whole scene `snap` (the level build's step): the level's `.xspawn` made
/// again from what the scene holds. Answers what was baked, or why it was not.
pub async fn bake_layer(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    snap: Arc<Snap<SceneState>>,
) -> Result<Value, String> {
    let svc = session.services.get::<SceneService>().ok_or("no scene")?;
    let level =
        resolve_level(None, &snap.meta.level_name()).ok_or("the scene has no name yet: save it first")?;
    let guard =
        session.services.get::<gz_project::Project>().map(|p| p.write_guard(None)).unwrap_or_default();
    let c = Ctx { session: session.clone(), guard, svc, snap, level: level.clone() };
    let run = run_spawn(editor, &c, false, None, None, true).await?;
    let r = &run.bake.result;
    let mut warnings = r.warnings.clone();
    warnings.extend(run.left_out.iter().cloned());
    Ok(json!({
        "level": level,
        "exported": r.exported,
        "missing": r.missing,
        "invisible": r.invisible,
        "removed": r.removed,
        "warnings": warnings,
        "path": run.path.as_deref().map(json_path).unwrap_or_default(),
        "things": run.things.named().iter().map(|(k, n)| ((*k).to_owned(), json!(n))).collect::<gz_editor::Args>(),
    }))
}

/// The open scene mirrored into its level's `.xspawn` before a module build: a note for the
/// layer, a warning per thing left out, an error when the bake failed. Empty without an open
/// scene.
async fn bake_note(editor: &Arc<Editor>, session: &Arc<ProjectSession>) -> Vec<Finding> {
    // the level of the scene that is open, as it is now - saved or not
    let open = session
        .services
        .get::<SceneService>()
        .and_then(|svc| resolve_level(None, &svc.store.snapshot().meta.level_name()));
    let leftovers = leftover_layers(editor, session, open).await;
    let Some(svc) = session.services.get::<SceneService>() else { return leftovers };
    let snap = svc.store.snapshot();
    let Some(level) = resolve_level(None, &snap.meta.level_name()) else { return leftovers };
    if !snap.valid {
        return leftovers;
    }
    let guard =
        session.services.get::<gz_project::Project>().map(|p| p.write_guard(None)).unwrap_or_default();
    let c = Ctx { session: session.clone(), guard, svc, snap, level: level.clone() };
    let mut note = Vec::new();
    match run_spawn(editor, &c, false, None, None, true).await {
        Ok(run) => {
            let r = &run.bake.result;
            note.push(Finding::info(if r.exported > 0 {
                format!("Scene baked: {} object(s) -> spawn\\{level}.xspawn", r.exported)
            } else {
                format!("Scene baked: empty, spawn\\{level}.xspawn cleared")
            }));
            // what taking the level's own spawn points out means for a save, where the author
            // meets the build's result
            if r.removed > 0 {
                note.push(Finding::info(format!(
                    "{} spawn point(s) of {level} are taken out for the module's mode: a new game started with the module has none of them, and a save made in it keeps them out for a game without the module too.",
                    r.removed
                )));
            }
            // every object left out or unseen, every left-out path and order, by name: the note is
            // what the build shows
            note.extend(r.warnings.iter().map(Finding::warning));
        }
        Err(e) => note.push(Finding::error(format!("The scene of '{level}' could not be baked: {e}"))),
    }
    note.extend(leftovers);
    note
}

/// The module layers a level build makes in `levels\<level>` and beside it.
const LAYER_FILES: [&str; 4] = ["overlay.things", "overlay.xcform", "overlay.cut", "overlay.aiedit"];

/// The levels the project holds module layers of that no scene of the project is of: a scene that
/// lay over one was moved to another level (its `base` changed) or renamed, and what its builds
/// left still ships. A warning each.
///
/// Only what a level build left is a leftover: the level's `gamedata\levels\<level>\build.state`
/// says a build made its layers. Layers made by hand - a project without a scene, a level beside
/// the scene's - have no such record and are told nothing. `open` is the level of the scene that
/// is open, as it is now: its files may still say another. And while a scene of the project does
/// not read, nobody can say which level it lies over: nothing is called a leftover then.
async fn leftover_layers(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    open: Option<String>,
) -> Vec<Finding> {
    let Some(project) = session.services.get::<gz_project::Project>() else { return Vec::new() };
    let root = session.root.clone();
    let scenes: Vec<PathBuf> = project
        .tree
        .snapshot()
        .files_with_extension(gz_scene::format::EXT)
        .iter()
        .map(|r| r.to_fs(&root))
        .collect();
    if scenes.is_empty() {
        return Vec::new();
    }
    editor
        .exec
        .io(move || {
            let mut of_scenes: Vec<String> = open.iter().map(|l| l.to_lowercase()).collect();
            for s in &scenes {
                match gz_scene::format::level_of_readable_file(s) {
                    Some(level) => of_scenes.push(level.to_lowercase()),
                    None => return Vec::new(),
                }
            }
            let mut found = Vec::new();
            let Ok(dir) = std::fs::read_dir(root.join("levels")) else { return found };
            let mut levels: Vec<String> = dir
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            levels.sort();
            for level in levels {
                if of_scenes.contains(&level.to_lowercase()) {
                    continue;
                }
                let at = root.join("levels").join(&level);
                let mut holds: Vec<&str> = LAYER_FILES.iter().copied().filter(|f| at.join(f).is_file()).collect();
                if at.join("pieces").is_dir() {
                    holds.push("pieces");
                }
                // made by a level build, or by hand: the build's record of the level tells
                let built = root.join("gamedata").join("levels").join(&level).join("build.state").is_file();
                if holds.is_empty() || !built {
                    continue;
                }
                found.push(Finding::warning(format!(
                    "levels\\{level} holds layers a level build made ({}) and no scene of the project is of that level: a scene that lay over it was moved to another level or renamed, and the module still ships them. Remove the folder, spawn\\{level}.xspawn and gamedata\\levels\\{level} when they are leftovers.",
                    holds.join(", ")
                )));
            }
            found
        })
        .await
}
