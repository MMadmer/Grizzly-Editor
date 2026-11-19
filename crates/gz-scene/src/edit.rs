//! The scene's edits that go through a file or make a new scene: Copy and Paste through the
//! project's clipboard file, Duplicate, saving a set of objects as a `.group` asset and placing
//! one, New Scene.
//!
//! The file work runs on the IO pool, the text on the CPU pool; the change is one `SceneCmd`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::{Origin, RelPath};
use gz_editor::{CmdError, Editor};

use crate::apply::{ReplaceKind, SceneCmd, SceneReply};
use crate::format::group::{self, Bundle};
use crate::math::Vec3;
use crate::model::{ObjData, ObjId, SceneState};
use crate::objects::Loaded;
use crate::service::SceneService;

/// What a paste added.
#[derive(Clone, Debug, Default)]
pub struct Added {
    pub objects: usize,
    pub warnings: Vec<String>,
}

fn no_scene(s: &SceneState) -> Result<(), CmdError> {
    if s.valid { Ok(()) } else { Err(CmdError::new("no_scene", "no scene is open")) }
}

/// The project's clipboard: a `.group` text in its `temp\`.
#[must_use]
pub fn clipboard_file(svc: &SceneService) -> PathBuf {
    svc.root.join("temp").join("clipboard.group")
}

/// The objects of `ids` with their groups' members as a bundle (in the scene's order).
#[must_use]
pub fn bundle_of(state: &SceneState, ids: &[ObjId]) -> Bundle {
    let all = group::with_members(state, ids);
    // a copy of what the level holds is no object of the level: it is the scene's own, whole,
    // and taken out of nothing
    let objects: Vec<crate::model::Object> = all
        .iter()
        .filter_map(|id| state.get(*id))
        .map(|o| {
            let mut copy = (**o).clone();
            copy.marks.base = false;
            copy.marks.removed = false;
            copy
        })
        .collect();
    let mut groups = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        if let ObjData::Group(g) = &o.data {
            let idx: Vec<usize> = g.members.iter().filter_map(|m| all.iter().position(|x| x == m)).collect();
            groups.push((i, idx));
        }
    }
    Bundle { objects, groups }
}

/// The objects a command means: the named ones, else the selection.
pub fn ids_of(state: &SceneState, names: Option<&[String]>) -> Result<Vec<ObjId>, CmdError> {
    match names {
        Some(list) if !list.is_empty() => {
            let mut ids = Vec::with_capacity(list.len());
            let mut missing = Vec::new();
            for n in list {
                match state.find_any(n) {
                    Some(o) => ids.push(o.id),
                    None => missing.push(n.clone()),
                }
            }
            if !missing.is_empty() {
                return Err(CmdError::not_found(format!("no object is named {}", missing.join(", "))));
            }
            Ok(ids)
        }
        _ => Ok(state.selected_in_order()),
    }
}

/// Copy: the objects (with their groups' members) as a `.group` text in the project's clipboard
/// file. Answers how many objects it holds.
pub async fn copy(editor: &Arc<Editor>, svc: &SceneService, ids: Vec<ObjId>) -> Result<usize, CmdError> {
    let snap = svc.store.snapshot();
    no_scene(&snap)?;
    if ids.is_empty() {
        return Err(CmdError::invalid("nothing is selected to copy"));
    }
    let settings = crate::game::settings(editor).await;
    let all = group::with_members(&snap, &ids);
    let n = all.len();
    let text = editor
        .exec
        .cpu(move || {
            group::group_text(
                &snap,
                &all,
                Vec3::ZERO,
                settings.as_deref().map(|s| s as &(dyn gz_entities::Settings + Sync)),
            )
        })
        .await
        .map_err(CmdError::invalid)?;
    let file = clipboard_file(svc);
    editor
        .exec
        .io(move || {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            gz_editor::fsx::write_atomic(&file, text.as_bytes())
        })
        .await
        .map_err(|e| CmdError::io(format!("the clipboard could not be written: {e}")))?;
    Ok(n)
}

/// Reads a `.group` text of a file into a bundle with the linked game's configs.
pub async fn read_group_file(
    editor: &Arc<Editor>,
    file: PathBuf,
    rel: RelPath,
) -> Result<(Bundle, Vec<String>), CmdError> {
    let shown = file.display().to_string();
    let text = editor
        .exec
        .io(move || std::fs::read_to_string(&file))
        .await
        .map_err(|e| CmdError::not_found(format!("{shown} could not be read ({e})")))?;
    let settings = crate::game::settings(editor).await;
    editor
        .exec
        .cpu(move || {
            group::read_group(
                &text,
                &rel,
                settings.as_deref().map(|s| s as &(dyn gz_entities::Settings + Sync)),
            )
            .map(|(b, problems)| (b, problems.iter().map(gz_base::Problem::line_text).collect()))
        })
        .await
        .map_err(|r| CmdError::new(r.code, r.problem.line_text()))
}

/// Paste: the clipboard's objects added under free names, one undo step.
pub async fn paste(
    editor: &Arc<Editor>,
    svc: &SceneService,
    offset: Vec3,
    origin: Origin,
) -> Result<Added, CmdError> {
    no_scene(&svc.store.snapshot())?;
    let file = clipboard_file(svc);
    if !editor
        .exec
        .io({
            let f = file.clone();
            move || f.is_file()
        })
        .await
    {
        return Err(CmdError::invalid("the clipboard is empty: copy something first"));
    }
    let (bundle, warnings) = read_group_file(
        editor,
        file,
        RelPath::new("temp/clipboard.group").unwrap_or_else(|_| RelPath::root()),
    )
    .await?;
    for w in &warnings {
        editor.log.warn(w);
    }
    let reply = svc
        .store
        .send(SceneCmd::Paste { bundle: Box::new(bundle), offset, label: "Paste".into() }, origin)
        .await?;
    Ok(Added { objects: if let SceneReply::Count(n) = reply { n } else { 0 }, warnings })
}

/// Duplicate: a copy of the objects added beside them (moved by `offset`), one undo step.
pub async fn duplicate(
    svc: &SceneService,
    ids: Vec<ObjId>,
    offset: Vec3,
    origin: Origin,
) -> Result<Added, CmdError> {
    let snap = svc.store.snapshot();
    no_scene(&snap)?;
    if ids.is_empty() {
        return Err(CmdError::invalid("nothing is selected to duplicate"));
    }
    let bundle = bundle_of(&snap, &ids);
    let reply = svc
        .store
        .send(SceneCmd::Paste { bundle: Box::new(bundle), offset, label: "Duplicate".into() }, origin)
        .await?;
    Ok(Added { objects: if let SceneReply::Count(n) = reply { n } else { 0 }, warnings: Vec::new() })
}

/// The objects saved as a `.group` asset of the project, positions about `pivot`.
pub async fn save_group(
    editor: &Arc<Editor>,
    svc: &SceneService,
    ids: Vec<ObjId>,
    pivot: Vec3,
    file: PathBuf,
) -> Result<usize, CmdError> {
    let snap = svc.store.snapshot();
    no_scene(&snap)?;
    writable(editor, &file)?;
    let settings = crate::game::settings(editor).await;
    let all = group::with_members(&snap, &ids);
    let n = all.len();
    let text = editor
        .exec
        .cpu(move || {
            group::group_text(
                &snap,
                &all,
                pivot,
                settings.as_deref().map(|s| s as &(dyn gz_entities::Settings + Sync)),
            )
        })
        .await
        .map_err(CmdError::invalid)?;
    let ed = editor.clone();
    editor
        .exec
        .io(move || -> std::io::Result<()> {
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir)?;
            }
            gz_editor::fsx::write_atomic(&file, text.as_bytes())?;
            if let Ok((_, project)) = gz_project::Project::of(&ed) {
                project.tree.changes().note_written_fs(&file, text.as_bytes());
            }
            Ok(())
        })
        .await
        .map_err(|e| CmdError::io(format!("the group could not be written: {e}")))?;
    Ok(n)
}

/// A group of the library (`rawdata\groups\<reference>.group`, the project's first, then the
/// editor's), written as the X-Ray SDK writes one.
pub async fn library_group(
    editor: &Arc<Editor>,
    svc: &SceneService,
    reference: &str,
) -> Result<Loaded, CmdError> {
    let (root, lib) = (svc.root.clone(), gz_project::library_root(editor));
    let settings = crate::game::settings(editor).await;
    let r = reference.to_owned();
    let (loaded, warnings) = editor
        .exec
        .io(move || {
            library_group_at(
                &root,
                Some(&lib),
                &r,
                settings.as_deref().map(|s| s as &(dyn gz_entities::Settings + Sync)),
            )
        })
        .await?;
    for w in warnings {
        editor.log.warn(w);
    }
    Ok(loaded)
}

/// [`library_group`] blocking, with the folders it looks in.
fn library_group_at(
    root: &Path,
    editor_root: Option<&Path>,
    reference: &str,
    settings: Option<&(dyn gz_entities::Settings + Sync)>,
) -> Result<(Loaded, Vec<String>), CmdError> {
    let rel = format!("{}.group", reference.replace('/', "\\").trim_end_matches(".group"));
    let body = std::iter::once(root)
        .chain(editor_root)
        .map(|r| r.join("rawdata").join("groups").join(&rel))
        .find_map(|f| std::fs::read(f).ok())
        .ok_or_else(|| {
            CmdError::not_found(format!("there is no group {rel} in the project or the library"))
        })?;
    crate::sdk::read_library_group(&body, reference, settings).map_err(CmdError::invalid)
}

/// A group whose `.group` is `reference`: the project's asset, else a group of the library.
pub async fn group_asset(
    editor: &Arc<Editor>,
    svc: &SceneService,
    reference: &str,
) -> Result<Loaded, CmdError> {
    let (root, lib) = (svc.root.clone(), gz_project::library_root(editor));
    let settings = crate::game::settings(editor).await;
    let r = reference.to_owned();
    let (loaded, warnings) = editor
        .exec
        .io(move || {
            group_asset_at(
                &root,
                Some(&lib),
                &r,
                settings.as_deref().map(|s| s as &(dyn gz_entities::Settings + Sync)),
            )
        })
        .await?;
    for w in &warnings {
        editor.log.warn(w);
    }
    Ok(loaded)
}

/// [`group_asset`] blocking. A project asset that holds one group gives that group and its
/// members; one that holds no group gives every object it holds as a member of a group at its
/// pivot.
pub(crate) fn group_asset_at(
    root: &Path,
    editor_root: Option<&Path>,
    reference: &str,
    settings: Option<&(dyn gz_entities::Settings + Sync)>,
) -> Result<(Loaded, Vec<String>), CmdError> {
    let rel = RelPath::new(&reference.replace('\\', "/")).ok();
    let file = rel.as_ref().map(|r| r.to_fs(root)).filter(|f| f.is_file());
    let (Some(file), Some(rel)) = (file, rel) else {
        return library_group_at(root, editor_root, reference, settings);
    };
    let text = std::fs::read_to_string(&file)
        .map_err(|e| CmdError::not_found(format!("{} could not be read ({e})", file.display())))?;
    let (bundle, problems) =
        group::read_group(&text, &rel, settings).map_err(|r| CmdError::new(r.code, r.problem.line_text()))?;
    let warnings = problems.iter().map(gz_base::Problem::line_text).collect();
    let (obj, members) = match bundle.groups.as_slice() {
        [(gi, members)] => (
            bundle
                .objects
                .get(*gi)
                .cloned()
                .ok_or_else(|| CmdError::invalid(format!("{reference} holds no group")))?,
            members.iter().filter_map(|i| bundle.objects.get(*i).cloned()).collect::<Vec<_>>(),
        ),
        [] => (
            crate::model::Object::new(
                crate::class::ObjClass::Group,
                "group",
                ObjData::Group(crate::objects::GroupData::default()),
            ),
            bundle.objects,
        ),
        _ => return Err(CmdError::invalid(format!("{reference} holds no single group"))),
    };
    let members = members.into_iter().map(|obj| Loaded { obj, members: Vec::new() }).collect();
    Ok((Loaded { obj, members }, warnings))
}

/// Places every group a scene's files name with a `ref` and no `members`: its members made from
/// the group asset at the group's position, rotation and scale, named `<group>_<member>`. Answers
/// a note per group placed and an error per group whose asset cannot be read (that group is kept
/// as written), and how many were placed.
pub(crate) fn place_unplaced(
    state: &mut SceneState,
    unplaced: &[crate::format::Unplaced],
    root: &Path,
    editor_root: Option<&Path>,
    settings: Option<&(dyn gz_entities::Settings + Sync)>,
) -> (Vec<gz_base::Problem>, usize) {
    let mut problems = Vec::new();
    let mut placed = 0;
    for u in unplaced {
        let Some(group) = state.get(u.id).map(|g| (**g).clone()) else { continue };
        let ObjData::Group(gd) = &group.data else { continue };
        let reference = gd.reference.clone();
        let at = |p: gz_base::Problem| {
            p.in_file(u.file.clone()).at(u.span.line, u.span.column, u.span.end_line, u.span.end_column)
        };
        let (mut loaded, _) = match group_asset_at(root, editor_root, &reference, settings) {
            Ok(l) => l,
            Err(e) => {
                problems.push(at(gz_base::Problem::error(
                    "S022",
                    format!("the group '{}' is placed from {reference}, which cannot be read: {}; it is kept as written", group.name, e.message),
                )));
                continue;
            }
        };
        let n = loaded.members.len();
        for m in &mut loaded.members {
            m.obj.name = format!("{}_{}", group.name, m.obj.name).into();
            m.obj.marks = group.marks;
        }
        let mut g = group.clone();
        g.xform = loaded.obj.xform;
        g.data =
            ObjData::Group(crate::objects::GroupData { reference: reference.clone(), members: Vec::new() });
        loaded.obj = g;
        state.remove(u.id);
        if let Err(e) = state.insert_group_at(loaded, group.xform) {
            problems.push(at(gz_base::Problem::error(
                "S022",
                format!("the group '{}' could not be placed: {}", group.name, e.message),
            )));
            continue;
        }
        placed += 1;
        problems.push(at(gz_base::Problem::note(
            "S023",
            format!("the group '{}' was placed from {reference}: {n} objects (the scene is unsaved until it is saved)", group.name),
        )));
    }
    (problems, placed)
}

/// New Scene: an empty scene with no file, its history new; the kinds' eyes and locks stay.
pub async fn new_scene(
    editor: &Arc<Editor>,
    svc: &SceneService,
    over: Option<(&str, &Arc<SceneState>)>,
    origin: Origin,
) -> Result<(), CmdError> {
    let _turn = svc.turn().await;
    let snap = svc.store.snapshot();
    // over a level of the game the scene starts with the level's own objects
    let mut fresh = over.map_or_else(SceneState::default, |(name, level)| crate::base::over(level, name));
    for c in crate::class::ObjClass::ALL {
        let now = snap.tools.part(c);
        let t = fresh.tools.part_mut(c);
        t.visible = now.visible;
        t.locked = now.locked;
    }
    svc.store.send(SceneCmd::Replace(Box::new(fresh), ReplaceKind::Loaded), origin).await?;
    svc.forget_files();
    let rec = svc.recovery.clone();
    editor.exec.io(move || rec.reset()).await;
    Ok(())
}

/// A file the scene's commands are about to write: never under the linked game's folder (the
/// one guard of every writer, `gz_editor::fsx::WriteGuard`).
pub fn writable(editor: &Editor, path: &Path) -> Result<(), CmdError> {
    let guard = gz_project::Project::of(editor).map(|(_, p)| p.write_guard(None)).unwrap_or_default();
    guard.check(path).map_err(|e| CmdError::invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::class::ObjClass;
    use crate::format::{self, Texts};
    use crate::model::Object;
    use crate::objects::SceneObjectData;

    fn hut(root: &Path) {
        let mut s = SceneState::default();
        for (name, p) in [("wall", Vec3::new(1.0, 0.0, 0.0)), ("roof", Vec3::new(0.0, 2.0, 0.0))] {
            let mut o = Object::new(
                ObjClass::SceneObject,
                name,
                ObjData::SceneObject(SceneObjectData::library(r"props\box")),
            );
            o.xform.position = p;
            s.insert(o);
        }
        let ids: Vec<ObjId> = s.objects.values().map(|o| o.id).collect();
        let text = group::group_text(&s, &ids, Vec3::ZERO, None).unwrap();
        std::fs::create_dir_all(root.join("prefabs")).unwrap();
        std::fs::write(root.join("prefabs").join("hut.group"), text).unwrap();
    }

    fn scene_with(group_ref: &str) -> Texts {
        Texts {
            scene: "return { scene = 2 }\n".into(),
            files: vec![(
                "groups.lua".into(),
                format!(
                    "return {{\n  groups = {{\n    {{ name = \"hut\", ref = \"{group_ref}\", pos = {{ 10, 0, 0 }}, authored = true }},\n  }},\n}}\n"
                ),
            )],
            layers: Vec::new(),
        }
    }

    fn read(t: &Texts) -> format::Read {
        let main = RelPath::new("test.scene").unwrap();
        let dir = RelPath::new("test").unwrap();
        format::read(t, &main, &dir, None).unwrap()
    }

    #[test]
    fn a_group_without_members_is_placed_from_its_asset() {
        let root = std::env::temp_dir().join(format!("gz-place-group-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        hut(&root);
        let mut r = read(&scene_with("prefabs/hut.group"));
        assert_eq!(r.unplaced.len(), 1);
        let (notes, placed) = place_unplaced(&mut r.state, &r.unplaced, &root, None, None);
        assert_eq!(placed, 1);
        let note = notes.iter().find(|p| p.code == "S023").expect("a note per group");
        assert!(note.message.contains("2 objects") && note.line == Some(3), "{note:?}");
        let wall = r.state.find(ObjClass::SceneObject, "hut_wall").expect("named <group>_<member>");
        assert_eq!(wall.xform.position, Vec3::new(11.0, 0.0, 0.0));
        assert!(wall.marks.authored && wall.owner.is_some());
        let g = r.state.find(ObjClass::Group, "hut").unwrap();
        let ObjData::Group(gd) = &g.data else { panic!() };
        assert_eq!(gd.members.len(), 2);
        assert_eq!(gd.reference, "prefabs/hut.group");
        // the next save writes the members and the group's list of them
        let plan = format::write(&r.state, Some(&r.files), None).unwrap();
        let names: Vec<&str> = plan.files.iter().map(|f| f.file.as_str()).collect();
        assert!(names.contains(&"objects.lua") && names.contains(&"groups.lua"), "{names:?}");
        // a group whose asset is not there is kept as written, with an error at its line
        let mut r = read(&scene_with("prefabs/gone.group"));
        let (notes, placed) = place_unplaced(&mut r.state, &r.unplaced, &root, None, None);
        assert_eq!(placed, 0);
        let e = notes.iter().find(|p| p.code == "S022").expect("an error");
        assert_eq!((e.severity, e.line), (gz_base::Severity::Error, Some(3)));
        assert!(r.state.find(ObjClass::Group, "hut").is_some());
        assert!(r.state.of_class(ObjClass::SceneObject).next().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_group_with_members_is_read_as_it_is() {
        let t = Texts {
            scene: "return { scene = 2 }\n".into(),
            files: vec![(
                "groups.lua".into(),
                "return { groups = { { name = \"hut\", ref = \"prefabs/hut.group\", members = {} } } }\n"
                    .into(),
            )],
            layers: Vec::new(),
        };
        assert!(read(&t).unplaced.is_empty());
    }
}
