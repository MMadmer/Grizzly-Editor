//! The scene's edits that read or write a file on the way: Cut, Copy, Paste and Duplicate
//! through the clipboard (`CommandCut` / `CommandCopy` / `CommandPaste`, `UIMainForm::LevelEdit`),
//! Open Selection and Save Selection As (`CommandLoadSelection` / `CommandSaveSelection`), a
//! level part loaded again or unloaded (`CommandEnableTarget`), New Scene (`CommandClear`).
//!
//! The file work runs on the IO pool, the parse on the CPU pool; the change is one `SceneCmd`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{CmdError, Editor};

use crate::apply::{ReplaceKind, SceneCmd, SceneReply};
use crate::class::ObjClass;
use crate::io::{self, SyncSettings};
use crate::model::SceneState;
use crate::selection::{self, Selection};
use crate::service::SceneService;

/// What a paste or an open added.
#[derive(Clone, Debug, Default)]
pub struct Added {
    pub objects: usize,
    pub warnings: Vec<String>,
}

fn no_scene(s: &SceneState) -> Result<(), CmdError> {
    if s.valid { Ok(()) } else { Err(CmdError::new("no_scene", "no scene")) }
}

/// `$temp$` of the open project: where a copy's selection file goes.
fn temp_dir(svc: &SceneService) -> PathBuf {
    svc.root.join("temp")
}

/// `EScene::SaveSelection(class, file)`.
pub async fn save_selection(
    editor: &Arc<Editor>,
    svc: &SceneService,
    class: Option<ObjClass>,
    file: PathBuf,
) -> Result<(), CmdError> {
    let snap = svc.store.snapshot();
    no_scene(&snap)?;
    let bytes = editor
        .exec
        .cpu(move || selection::write_selection(&snap, class))
        .await
        .map_err(|e| CmdError::internal(e.to_string()))?;
    editor
        .exec
        .io(move || {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&file, &bytes)
        })
        .await
        .map_err(|e| CmdError::io(format!("the selection could not be written: {e}")))
}

/// Reads a selection file and parses it with the linked game's configs.
async fn read_selection_file(editor: &Arc<Editor>, file: PathBuf) -> Result<(Selection, Vec<String>), CmdError> {
    let shown = file.display().to_string();
    let bytes = editor
        .exec
        .io(move || std::fs::read(&file))
        .await
        .map_err(|e| CmdError::not_found(format!("the selection file could not be read: {shown} ({e})")))?;
    let settings = crate::game::settings(editor).await;
    editor
        .exec
        .cpu(move || {
            let st: SyncSettings<'_> = settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync));
            selection::read_selection(&bytes, st.map(|s| s as &dyn gz_entities::Settings))
        })
        .await
        .map_err(|e| CmdError::invalid(e.to_string()))
}

/// `EScene::LoadSelection(file)`: its objects added under free names, one undo step.
pub async fn load_selection(
    editor: &Arc<Editor>,
    svc: &SceneService,
    file: PathBuf,
    label: &str,
    origin: Origin,
) -> Result<Added, CmdError> {
    no_scene(&svc.store.snapshot())?;
    let (sel, warnings) = read_selection_file(editor, file).await?;
    for w in &warnings {
        editor.log.warn(w);
    }
    let reply = svc.store.send(SceneCmd::Paste { selection: Box::new(sel), label: label.to_owned() }, origin).await?;
    let objects = match reply {
        SceneReply::Count(n) => n,
        _ => 0,
    };
    Ok(Added { objects, warnings })
}

/// `CopySelection(class)`: the selection file in the project's `temp\`, its record on the
/// clipboard. Answers the file.
pub async fn copy(editor: &Arc<Editor>, svc: &SceneService, class: ObjClass) -> Result<PathBuf, CmdError> {
    let dir = temp_dir(svc);
    let file = editor
        .exec
        .io(move || selection::new_clip_file(&dir))
        .await
        .map_err(|e| CmdError::io(format!("the clipboard file could not be made: {e}")))?;
    save_selection(editor, svc, Some(class), file.clone()).await?;
    let record = selection::clip_record(Some(class), &file);
    editor.exec.io(move || selection::clipboard_set(&record)).await.map_err(CmdError::io)?;
    Ok(file)
}

/// `CutSelection(class)`: the copy, then the selection of the class removed as one undo step.
pub async fn cut(editor: &Arc<Editor>, svc: &SceneService, class: ObjClass, origin: Origin) -> Result<(PathBuf, usize), CmdError> {
    let file = copy(editor, svc, class).await?;
    let reply =
        svc.store.send(SceneCmd::RemoveSelection { class: Some(class), label: "Cut".into() }, origin).await?;
    let n = match reply {
        SceneReply::Count(n) => n,
        _ => 0,
    };
    Ok((file, n))
}

/// `PasteSelection`: the selection file the clipboard names.
pub async fn paste(editor: &Arc<Editor>, svc: &SceneService, origin: Origin) -> Result<Added, CmdError> {
    let record = editor.exec.io(selection::clipboard_get).await.map_err(CmdError::invalid)?;
    let file = selection::clip_file(&record).ok_or_else(|| CmdError::invalid("No data in clipboard"))?;
    load_selection(editor, svc, file, "Paste", origin).await
}

/// Duplicate: a copy pasted where it was (`UIMainForm::LevelEdit` "duplicate").
pub async fn duplicate(editor: &Arc<Editor>, svc: &SceneService, class: ObjClass, origin: Origin) -> Result<Added, CmdError> {
    let file = copy(editor, svc, class).await?;
    load_selection(editor, svc, file, "Duplicate", origin).await
}

/// `CommandEnableTarget`: on reads the class's part of the scene's file again, off unloads it.
/// A part is read only for a scene that has a file (`m_LastFileName`).
pub async fn set_part_enabled(
    editor: &Arc<Editor>,
    svc: &SceneService,
    class: ObjClass,
    enabled: bool,
    origin: Origin,
) -> Result<(), CmdError> {
    let snap = svc.store.snapshot();
    no_scene(&snap)?;
    let part = match (enabled, snap.meta.file.clone()) {
        (true, Some(scene)) => {
            let guid = snap.meta.guid;
            let files = editor
                .exec
                .io(move || {
                    let mut out = Vec::new();
                    for i in 0.. {
                        if i > 0 && !class.uses_ltx() {
                            break;
                        }
                        let p = io::part_path(&scene, class, i);
                        match std::fs::read(&p) {
                            Ok(b) => out.push((p, b)),
                            Err(_) => break,
                        }
                    }
                    out
                })
                .await;
            let settings = crate::game::settings(editor).await;
            let (part, warnings) = editor
                .exec
                .cpu(move || {
                    let st: SyncSettings<'_> =
                        settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync));
                    io::parse_part(class, &files, guid, st)
                })
                .await
                .map_err(|e| CmdError::invalid(e.to_string()))?;
            for w in warnings {
                editor.log.warn(w);
            }
            Some(Box::new(part))
        }
        _ => None,
    };
    svc.store.send(SceneCmd::LevelPart { class, enabled, part }, origin).await?;
    // `ExecCommand(COMMAND_CHANGE_TARGET, OBJCLASS_SCENEOBJECT)` after an unload
    if !enabled {
        svc.store.send(SceneCmd::SetTarget(ObjClass::SceneObject), origin).await?;
    }
    Ok(())
}

/// `CommandClear`: an empty scene with no file, its history new. The level options and every
/// tool's settings are the defaults again; a part that was not loaded stays so.
pub async fn new_scene(editor: &Arc<Editor>, svc: &SceneService, origin: Origin) -> Result<(), CmdError> {
    let snap = svc.store.snapshot();
    let mut fresh = SceneState::default();
    fresh.meta.guid = io::new_guid(svc.root.as_os_str().len() as u64);
    for c in ObjClass::ALL {
        let now = snap.tools.part(c);
        let t = fresh.tools.part_mut(c);
        t.enabled = now.enabled;
        t.visible = now.visible;
        t.readonly = now.readonly;
    }
    svc.store.send(SceneCmd::Replace(Box::new(fresh), ReplaceKind::Loaded), origin).await?;
    let rec = svc.recovery.clone();
    editor.exec.io(move || rec.reset()).await;
    Ok(())
}

/// The file a name means in the project's `$maps$`: absolute as given, `.level` added.
#[must_use]
pub fn maps_file(svc: &SceneService, name: &str, ext: &str) -> PathBuf {
    let mut p = PathBuf::from(name.replace('/', "\\"));
    if p.extension().is_none() {
        p.set_extension(ext);
    }
    if p.is_absolute() { p } else { svc.maps_dir().join(p) }
}

/// Whether a path lies in the project's `$maps$`: the C++ then keeps it relative.
#[must_use]
pub fn under_maps(svc: &SceneService, file: &Path) -> bool {
    file.starts_with(svc.maps_dir())
}
