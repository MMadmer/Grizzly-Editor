//! The disk side of the exports: what they read before the CPU work, and how what they made is
//! written. Blocking - called on the IO pool. Only the project is written; the linked game and
//! the editor's own meshes are only read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::{Finding, Findings};
use gz_editor::fsx::WriteGuard;
use gz_entities::Settings;
use gz_scene::objects::PointKind;
use gz_scene::{ObjClass, ObjData, SceneState};
use gz_vfs::GameFiles;
use rayon::prelude::*;

use crate::names::config_visual;
use crate::spawn::{SpawnBake, spawn_visual};
use crate::visuals::{Found, SpawnVisual, Visuals};

/// What a spawn bake needs from the disk.
#[derive(Clone, Debug, Default)]
pub struct Gathered {
    pub visuals: Visuals,
    /// The `.xspawn` there is now (empty without one).
    pub old_text: String,
    /// `spawn\custom_data\<level>_*.ltx`: what a mirror pass deletes first.
    pub stale_custom_data: Vec<PathBuf>,
}

/// `<project>\gamedata\meshes\<visual>.ogf`.
#[must_use]
pub fn staged_path(root: &Path, visual: &str) -> PathBuf {
    root.join("gamedata").join("meshes").join(format!("{visual}.ogf"))
}

/// `<project>\spawn\<level>.xspawn`.
#[must_use]
pub fn xspawn_path(root: &Path, level: &str) -> PathBuf {
    root.join("spawn").join(format!("{level}.xspawn"))
}

/// `<project>\levels\<level>`.
#[must_use]
pub fn level_dir(root: &Path, level: &str) -> PathBuf {
    root.join("levels").join(level)
}

fn read_shared(p: &Path) -> Option<Arc<[u8]>> {
    std::fs::read(p).ok().map(Arc::from)
}

/// Reads what [`crate::spawn::bake`] needs: where each model is (in parallel), the file on disk,
/// the custom data a mirror pass replaces. `sdk_meshes` is the editor's own `gamedata\meshes`.
#[must_use]
pub fn spawn_inputs(
    root: &Path,
    level: &str,
    scene: &SceneState,
    selected_only: bool,
    settings: Option<&(dyn Settings + Sync)>,
    game: Option<&GameFiles>,
    sdk_meshes: &Path,
) -> Gathered {
    let selected = |id| !selected_only || scene.is_selected(id);
    let mut props: Vec<String> = Vec::new();
    let mut prop_raw: HashMap<String, PathBuf> = HashMap::new();
    for o in scene.of_class(ObjClass::SceneObject) {
        if let ObjData::SceneObject(d) = &o.data
            && o.marks.authored
            && selected(o.id)
            && !d.reference.is_empty()
        {
            props.push(d.reference.to_string());
        }
    }
    props.sort();
    props.dedup();
    let mut spawn_names: Vec<String> = Vec::new();
    for o in scene.of_class(ObjClass::SpawnPoint) {
        if let ObjData::Spawn(pt) = &o.data
            && o.marks.authored
            && selected(o.id)
            && pt.kind == PointKind::Spawn
            && let Some(e) = &pt.entity
        {
            let v = spawn_visual(e, settings);
            if !v.is_empty() {
                spawn_names.push(v);
            }
        }
    }
    spawn_names.sort();
    spawn_names.dedup();

    let in_game = |visual: &str| game.is_some_and(|g| g.exists(&format!("meshes\\{visual}.ogf")));
    let prop_found: Vec<(String, Found)> = props
        .par_iter()
        .map(|reference| {
            let visual = config_visual(reference);
            let found = if staged_path(root, &visual).is_file() {
                Found::Staged
            } else {
                let phys = root.join(reference.replace('/', "\\"));
                if phys.is_file() {
                    Found::Project(phys)
                } else if in_game(&visual) {
                    Found::Game
                } else {
                    Found::Missing
                }
            };
            (reference.clone(), found)
        })
        .collect();
    for (reference, found) in &prop_found {
        if let Found::Project(p) = found {
            prop_raw.insert(config_visual(reference), p.clone());
        }
    }
    let spawns: Vec<(String, SpawnVisual)> = spawn_names
        .par_iter()
        .map(|v| {
            let staged = staged_path(root, v);
            let info = if staged.is_file() {
                SpawnVisual { found: Found::Staged, bytes: read_shared(&staged) }
            } else if in_game(v) {
                let bytes = game.and_then(|g| g.read(&format!("meshes\\{v}.ogf")).ok()).map(Arc::from);
                SpawnVisual { found: Found::Game, bytes }
            } else {
                let sdk = sdk_meshes.join(format!("{v}.ogf"));
                if sdk.is_file() {
                    SpawnVisual { found: Found::Sdk(sdk.clone()), bytes: read_shared(&sdk) }
                } else {
                    // a prop of this bake may stage it: its bytes are the prop's file
                    SpawnVisual { found: Found::Missing, bytes: prop_raw.get(v).and_then(|p| read_shared(p)) }
                }
            };
            (v.clone(), info)
        })
        .collect();

    let old_text = std::fs::read(xspawn_path(root, level))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let mut stale = Vec::new();
    let prefix = format!("{level}_").to_lowercase();
    if let Ok(rd) = std::fs::read_dir(root.join("spawn").join("custom_data")) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.starts_with(&prefix) && name.ends_with(".ltx") && e.path().is_file() {
                stale.push(e.path());
            }
        }
    }
    stale.sort();
    Gathered {
        visuals: Visuals { props: prop_found.into_iter().collect(), spawns: spawns.into_iter().collect() },
        old_text,
        stale_custom_data: stale,
    }
}

fn write_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    gz_editor::fsx::write_atomic(path, bytes)
}

/// Writes a spawn bake: the stale custom data of a mirror pass removed, the models staged, the
/// custom data files, then the `.xspawn` (or its removal, when a mirror pass found nothing).
/// Returns the file and the log lines of what failed on the way.
pub fn apply_spawn(
    root: &Path,
    level: &str,
    mirror: bool,
    stale: &[PathBuf],
    bake: &SpawnBake,
    guard: &WriteGuard,
) -> Result<(Option<PathBuf>, Vec<Finding>), String> {
    let cd_dir = root.join("spawn").join("custom_data");
    // every folder the pass writes, where it really is: never one of the linked game's
    let mut folders = vec![root.join("spawn"), cd_dir.clone()];
    folders
        .extend(bake.stage.iter().filter_map(|(_, v)| staged_path(root, v).parent().map(Path::to_path_buf)));
    folders.dedup();
    for f in &folders {
        guard.check(f).map_err(|e| e.to_string())?;
    }
    let mut log = Vec::new();
    if mirror {
        // stale logic files would outlive their ops in a mirror pass
        for p in stale {
            let _ = std::fs::remove_file(p);
        }
    }
    for (src, visual) in &bake.stage {
        let dst = staged_path(root, visual);
        let copied = dst.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok())
            && std::fs::copy(src, &dst).is_ok();
        if !copied {
            log.warn(format!(
                "[XMS] spawn export: can't stage '{}' -> gamedata\\meshes\\{visual}.ogf",
                src.display()
            ));
        }
    }
    for (file, text) in &bake.custom_data {
        if write_file(&cd_dir.join(file), text.as_bytes()).is_err() {
            let op =
                file.strip_suffix(".ltx").and_then(|f| f.strip_prefix(&format!("{level}_"))).unwrap_or(file);
            log.warn(format!("[XMS] spawn export: can't write custom_data for '{op}'"));
        }
    }
    let path = xspawn_path(root, level);
    match &bake.text {
        Some(text) => {
            let dir = root.join("spawn");
            if std::fs::create_dir_all(&dir).is_err() || !dir.is_dir() {
                return Err("can't create the spawn folder".to_owned());
            }
            gz_editor::fsx::write_atomic(&path, text.as_bytes())
                .map_err(|_| "can't write the .xspawn file".to_owned())?;
            Ok((Some(path), log))
        }
        None => {
            let _ = std::fs::remove_file(&path);
            Ok((None, log))
        }
    }
}

/// Writes a file of the level's overlay folder, the folder made first. `what` names the file
/// in the error (`overlay.xcform`).
pub fn write_level_file(
    root: &Path,
    level: &str,
    rel: &str,
    bytes: &[u8],
    what: &str,
    guard: &WriteGuard,
) -> Result<PathBuf, String> {
    let dir = level_dir(root, level);
    guard.check(&dir).map_err(|e| e.to_string())?;
    if std::fs::create_dir_all(&dir).is_err() || !dir.is_dir() {
        return Err("can't create the overlay folder".to_owned());
    }
    let path = dir.join(rel);
    write_file(&path, bytes).map_err(|_| format!("can't write {what}"))?;
    Ok(path)
}
