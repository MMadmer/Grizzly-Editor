//! The scene's commands: every change of [`SceneState`] is one of them.

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::SmolStr;
use gz_store::{Apply, CmdError, State};

use crate::class::ObjClass;
use crate::math::{Vec3, normal_align, rotation_matrix};
use crate::model::{ObjData, ObjId, Object, SceneState, co};
use crate::objects::{GroupData, XmsOrder};
use crate::snap::SnapAction;
use crate::tools::{BulkData, LevelOptions, ToolPart};

/// One edit of a transform, with the class rules of `SetPosition`, `SetRotation`, `SetScale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TransformEdit {
    Position(Vec3),
    Rotation(Vec3),
    Scale(Vec3),
    /// `MoveTo`: a position, and with `up` the rotation that stands the object on it.
    MoveTo {
        position: Vec3,
        up: Option<Vec3>,
    },
    /// A landing computed elsewhere (the drop to ground).
    Land {
        position: Vec3,
        rotation: Option<Vec3>,
    },
}

/// How a selection command combines with the selection there is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectMode {
    /// Clear everything first.
    Set,
    Add,
    Remove,
    Toggle,
}

/// What replaces the whole scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplaceKind {
    /// A load: history starts over, clean.
    Loaded,
    /// A recovery snapshot: history starts over, dirty.
    Recovered,
}

/// Every change of the scene.
#[derive(Clone, Debug)]
pub enum SceneCmd {
    /// A loaded (or new) scene takes the place of the open one.
    Replace(Box<SceneState>, ReplaceKind),
    /// `AppendObject(obj, true)`: the object is the author's and becomes the selection of its
    /// class (`SelectObjects(false, cls)` first). The name must be free.
    Place(Box<Object>),
    /// Removes objects (a group's members are removed with it).
    Delete(Vec<ObjId>),
    /// `RemoveSelection(OBJCLASS_DUMMY)`: every selected object not inside a group.
    DeleteSelected,
    Rename {
        id: ObjId,
        name: SmolStr,
    },
    /// Transforms, one undo step. `members` moves a group's objects with it (the Details
    /// panel's `NumSet*`); the MCP setters move the group alone, as the C++ ones do.
    Transform {
        items: Vec<(ObjId, TransformEdit)>,
        members: bool,
        label: String,
        merge: Option<SmolStr>,
    },
    Select {
        ids: Vec<ObjId>,
        mode: SelectMode,
    },
    /// `SelectObjects(flag, cls)`: `None` is every class. Only visible objects are selected.
    SelectClass {
        class: Option<ObjClass>,
        select: bool,
    },
    /// The current target class.
    SetTarget(ObjClass),
    /// `Show`: a hidden object is deselected.
    Show {
        ids: Vec<ObjId>,
        visible: bool,
    },
    /// A tool's eye or its read-only switch.
    ToolFlags {
        class: ObjClass,
        visible: Option<bool>,
        readonly: Option<bool>,
    },
    /// `GroupObjects`: the selected visible objects not in a group, into a new group.
    Group {
        name: SmolStr,
    },
    /// `UngroupObjects` of every selected group.
    Ungroup,
    /// Objects computed from a snapshot with `serial` (a property set, a visual, a bone list).
    ReplaceObjects {
        objects: Vec<Object>,
        serial: u64,
        label: String,
    },
    /// The level options and the tools' settings (Level Settings).
    SetWorld {
        level: Box<LevelOptions>,
        tools: Vec<ToolPart>,
        serial: u64,
        label: String,
    },
    /// `flAuthorPlaced` on the selection; answers how many changed.
    MarkAuthored(bool),
    /// `flBaseRemoved` on the selected spawns that are not authored; answers changed, skipped.
    MarkRemoved(bool),
    /// A stalker's standing order.
    SetOrder {
        id: ObjId,
        order: XmsOrder,
    },
    /// The Snap Objects buttons and switches.
    Snap {
        action: Option<SnapAction>,
        use_list: Option<bool>,
        pick: Option<bool>,
    },
    /// New bulk data (an AI map tile, details, wallmarks).
    SetBulk(Box<BulkData>),
    /// The scene is on disk as `file`.
    Saved {
        file: PathBuf,
    },
    /// Marks the content changed without an undo step (a restored recovery snapshot).
    Touch,
    /// The unsaved changes are given up (a close that discards them): the scene no longer counts
    /// as unsaved work. What closes next drops it with the project.
    Forget,
    /// An edit of the sector, portal, way, shape, light and fog volume tools.
    ObjTool(Box<crate::objtools::ToolEdit>),
    /// An edit of the AI map, detail objects and wallmark tools.
    Bulk(Box<crate::bulk::BulkEdit>),
    /// `RemoveSelection(classfilter)`: the selected objects of a class (`None`: of every class)
    /// that are not inside a group; one undo step under `label` (Delete, Cut).
    RemoveSelection {
        class: Option<ObjClass>,
        label: String,
    },
    /// `LoadSelection`: a selection file's objects added (Paste, Open Selection); one undo step.
    Paste {
        selection: Box<crate::selection::Selection>,
        label: String,
    },
    /// `MultiRenameObjects` of a class: each selected object takes the next free name made from
    /// its reference.
    MultiRename(ObjClass),
    /// A level part loaded again or unloaded (Scene Targets' Enable): the class's objects and
    /// tool data replaced by `part`, or cleared with `None`.
    LevelPart {
        class: ObjClass,
        enabled: bool,
        part: Option<Box<crate::io::PartLoad>>,
    },
    /// `CSpawnPoint::AttachObject` (a shape of the scene becomes the point's own) or, with no
    /// object, `DetachObject` (the point's shape goes back into the scene). One undo step.
    SpawnAttach {
        spawn: ObjId,
        object: Option<ObjId>,
    },
    /// `CGroupObject::UpdateReference(true)` for each group (the Group tool's Reload
    /// References): its objects made again from the library `.group` read into `loaded`, the
    /// group where it stood. With as many objects as before, a member marked unique stays as it
    /// was and the others take their old names. The sectors are not touched (the C++ saves and
    /// restores them around the reload). One undo step.
    ReloadGroups {
        groups: Vec<(ObjId, Box<crate::objects::Loaded>)>,
    },
    /// The Group tool's add (`TUI_ControlGroupAdd`): a library group read into `loaded`, under
    /// its name, set where `at` says (`SetReference` on the object `DefaultAddObject` made),
    /// selected. One undo step.
    PlaceGroup {
        loaded: Box<crate::objects::Loaded>,
        at: crate::math::Transform,
    },
    /// The compiler's error list the viewport draws: no edit of the scene.
    DebugDraw(std::sync::Arc<crate::debug::DebugDraw>),
    /// A scene object's motion and where it puts the object (the Motion rows of Details).
    Motion {
        id: ObjId,
        motion: Option<std::sync::Arc<crate::model::MotionRaw>>,
        flags: u32,
        pose: Option<(Vec3, Vec3)>,
        label: String,
        merge: Option<SmolStr>,
    },
}

/// What a command answers.
#[derive(Clone, Debug, PartialEq)]
pub enum SceneReply {
    None,
    Id(ObjId),
    Count(usize),
    Counts(usize, usize),
}

fn missing(id: ObjId) -> CmdError {
    CmdError::not_found(format!("object {} is not in the scene", id.0))
}

/// The class rules of a position change; a spawn carries its shape, a way path its points, a
/// sound source its parameters.
fn set_position(o: &mut Object, p: Vec3) {
    match &mut o.data {
        ObjData::SoundSrc(s) => s.position = p,
        ObjData::Way(w) => w.move_to(p),
        ObjData::Spawn(s) => {
            o.xform.position = p;
            if let Some(a) = &mut s.attached {
                Arc::make_mut(a).xform.position = p;
            }
        }
        _ => o.xform.position = p,
    }
}

fn set_rotation(o: &mut Object, r: Vec3) {
    o.xform.rotation = r;
    if let ObjData::Spawn(s) = &mut o.data
        && let Some(a) = &mut s.attached
    {
        Arc::make_mut(a).xform.rotation = r;
    }
}

fn set_scale(o: &mut Object, v: Vec3) {
    match &mut o.data {
        ObjData::Glow(g) => {
            // `CGlow::SetScale`: the component that changed is the radius
            let s = o.xform.scale;
            let mut r = g.radius;
            for a in 0..3 {
                if (s[a] - v[a]).abs() > 1e-6 {
                    r = v[a];
                }
            }
            g.radius = r;
            o.xform.scale = Vec3::splat(r);
        }
        ObjData::Shape(sh) => o.xform.scale = sh.constrain_scale(v),
        ObjData::Spawn(s) => {
            o.xform.scale = v;
            if let Some(a) = &mut s.attached {
                let a = Arc::make_mut(a);
                a.xform.scale = match &a.data {
                    ObjData::Shape(sh) => sh.constrain_scale(v),
                    _ => v,
                };
            }
        }
        _ => o.xform.scale = v,
    }
}

fn apply_edit(o: &mut Object, e: TransformEdit) {
    match e {
        TransformEdit::Position(p) => set_position(o, p),
        TransformEdit::Rotation(r) => set_rotation(o, r),
        TransformEdit::Scale(s) => set_scale(o, s),
        TransformEdit::MoveTo { position, up } => {
            if let Some(up) = up {
                let k = rotation_matrix(o.rotation()).row(2);
                set_rotation(o, normal_align(up, k));
            }
            set_position(o, position);
        }
        TransformEdit::Land { position, rotation } => {
            if let Some(r) = rotation {
                set_rotation(o, r);
            }
            set_position(o, position);
        }
    }
}

impl SceneState {
    fn bump(&mut self) {
        self.serial += 1;
    }

    /// A read-only part's objects are not changed (`flForceReadonly`, the Outliner's text).
    fn check_editable(&self, id: ObjId) -> Result<(), CmdError> {
        match self.get(id) {
            Some(o) if !self.tools.part(o.class).editable() => {
                Err(CmdError::invalid(format!("The object '{}' is read only.", o.name)))
            }
            _ => Ok(()),
        }
    }

    fn edit_object(&mut self, id: ObjId, f: impl FnOnce(&mut Object)) -> Result<(), CmdError> {
        let mut o = (**self.get(id).ok_or_else(|| missing(id))?).clone();
        f(&mut o);
        self.replace(o);
        Ok(())
    }

    /// One transform, with a group's members carried when `members` (`NumSetPosition` & co).
    fn transform(&mut self, id: ObjId, e: TransformEdit, members: bool) -> Result<(), CmdError> {
        let before = (**self.get(id).ok_or_else(|| missing(id))?).clone();
        let mut after = before.clone();
        apply_edit(&mut after, e);
        if members && let ObjData::Group(g) = &before.data {
            let prev_inv = gz_fmt_object::Mat4::invert_43(&before.xform.matrix());
            let now = after.xform.matrix();
            let (dr, ds) =
                (after.xform.rotation - before.xform.rotation, after.xform.scale - before.xform.scale);
            for m in g.members.clone() {
                self.edit_object(m, |mo| {
                    match e {
                        TransformEdit::Rotation(_) => set_rotation(mo, mo.rotation() + dr),
                        TransformEdit::Scale(_) => set_scale(mo, mo.scale() + ds),
                        _ => {}
                    }
                    let p = now.transform_tiny(prev_inv.transform_tiny(mo.position()));
                    set_position(mo, p);
                })?;
            }
        }
        // `UpdateTransform`: Auto Key keys a selected motion's object where it now stands
        if after.flags.co & (co::MOTION | crate::objmotion::AUTO_KEY)
            == co::MOTION | crate::objmotion::AUTO_KEY
            && self.selection.contains(&id)
            && let Some(raw) = after.motion.clone()
            && let Some(mut m) = raw.model()
        {
            let t = crate::objmotion::params(&m, &raw).t_current;
            crate::objmotion::create_key(&mut m, t, after.position(), after.rotation());
            after.motion = Some(Arc::new(crate::model::MotionRaw::from_model(&m, t)));
        }
        self.replace(after);
        Ok(())
    }

    /// `GroupObjects` with `UpdatePivot(0, false)`.
    fn group_selection(&mut self, name: SmolStr) -> Result<ObjId, CmdError> {
        let ids: Vec<ObjId> = self
            .selected_in_order()
            .into_iter()
            .filter(|id| {
                self.get(*id).is_some_and(|o| {
                    o.flags.visible
                        && o.flags.co & co::IN_GROUP == 0
                        && o.owner.is_none()
                        && o.class != ObjClass::Sector
                        && o.class != ObjClass::Group
                })
            })
            .collect();
        if ids.is_empty() {
            return Err(CmdError::invalid("Group can't be created."));
        }
        let pivot = ids
            .iter()
            .filter_map(|id| self.get(*id))
            .find(|o| o.class == ObjClass::SceneObject)
            .or_else(|| self.get(ids[0]))
            .map(|o| (o.position(), o.rotation()));
        let mut g = Object::new(
            ObjClass::Group,
            &name,
            ObjData::Group(GroupData { reference: SmolStr::default(), members: ids.clone() }),
        );
        if let Some((p, r)) = pivot {
            g.xform.position = p;
            g.xform.rotation = r;
        }
        g.marks.authored = true;
        let gid = self.insert(g);
        for id in &ids {
            self.edit_object(*id, |o| {
                o.owner = Some(gid);
                o.flags.co |= co::IN_GROUP;
                o.flags.co &= !co::IN_GROUP_UNIQUE;
            })?;
            self.selection.remove(id);
        }
        Ok(gid)
    }

    fn ungroup_selection(&mut self) -> usize {
        let groups: Vec<ObjId> = self
            .selected_in_order()
            .into_iter()
            .filter(|id| self.get(*id).is_some_and(|o| o.class == ObjClass::Group))
            .collect();
        for gid in &groups {
            let Some(g) = self.get(*gid).cloned() else { continue };
            if let ObjData::Group(gd) = &g.data {
                for m in &gd.members {
                    let _ = self.edit_object(*m, |o| {
                        o.owner = None;
                        o.flags.co &= !co::IN_GROUP;
                    });
                    if self.get(*m).is_some_and(|o| o.flags.visible) {
                        self.selection.insert(*m);
                    }
                }
            }
            self.remove(*gid);
        }
        groups.len()
    }

    /// A group as read, its objects carried from where the file had it to `at`
    /// (`NumSetPosition`, `NumSetRotation`, `NumSetScale`).
    fn insert_group_at(
        &mut self,
        loaded: crate::objects::Loaded,
        at: crate::math::Transform,
    ) -> Result<ObjId, CmdError> {
        let gid = crate::io::insert_loaded(self, loaded);
        for e in [
            TransformEdit::Position(at.position),
            TransformEdit::Rotation(at.rotation),
            TransformEdit::Scale(at.scale),
        ] {
            self.transform(gid, e, true)?;
        }
        Ok(gid)
    }

    /// One group of `ReloadGroups`.
    fn reload_group(&mut self, id: ObjId, mut loaded: crate::objects::Loaded) -> Result<(), CmdError> {
        let old = (**self.get(id).ok_or_else(|| missing(id))?).clone();
        let ObjData::Group(og) = &old.data else {
            return Err(CmdError::invalid(format!("'{}' is not a group.", old.name)));
        };
        let before: Vec<Object> =
            og.members.iter().filter_map(|m| self.get(*m)).map(|o| (**o).clone()).collect();
        let selected = self.is_selected(id);
        for m in &og.members {
            self.remove(*m);
        }
        self.remove(id);
        loaded.obj.name = old.name.clone();
        if let ObjData::Group(g) = &mut loaded.obj.data {
            g.reference = og.reference.clone();
        }
        let same = loaded.members.len() == before.len();
        for (m, b) in loaded.members.iter_mut().zip(before.iter().chain(std::iter::repeat(&old))) {
            m.obj.flags.co &= !co::IN_GROUP_UNIQUE;
            if same && b.flags.co & co::IN_GROUP_UNIQUE == 0 {
                m.obj.name = b.name.clone();
            }
        }
        let gid = self.insert_group_at(loaded, old.xform)?;
        // a unique member is kept as it was, in the place of the one the file made
        if same && let Some(ObjData::Group(ng)) = self.get(gid).map(|g| g.data.clone()) {
            for (slot, mut b) in ng.members.iter().zip(before) {
                if b.flags.co & co::IN_GROUP_UNIQUE == 0 {
                    continue;
                }
                b.id = *slot;
                b.owner = Some(gid);
                self.replace(b);
            }
        }
        if selected {
            self.selection.insert(gid);
        }
        Ok(())
    }

    /// Removes an object; a group takes its members along, a member leaves its group. A scene
    /// object's meshes leave the sectors that hold them (`ESceneSectorTool::OnObjectRemove`).
    fn delete(&mut self, id: ObjId) {
        if let Some(o) = self.get(id).cloned()
            && let ObjData::Group(g) = &o.data
        {
            for m in &g.members {
                self.forget_in_sectors(*m);
                self.remove(*m);
            }
        }
        self.forget_in_sectors(id);
        self.remove(id);
    }

    /// `CSector::DelMesh` for every mesh of a scene object: a sector left without a mesh goes
    /// too ("Sector has no meshes and will be removed.").
    fn forget_in_sectors(&mut self, id: ObjId) {
        let Some(o) = self.get(id) else { return };
        if o.class != ObjClass::SceneObject {
            return;
        }
        let name = o.name.clone();
        let mut emptied = Vec::new();
        for sid in self.by_class[ObjClass::Sector.index()].clone() {
            let Some(sec) = self.get(sid).cloned() else { continue };
            let ObjData::Sector(d) = &sec.data else { continue };
            if !d.items.iter().any(|(obj, _)| obj.eq_ignore_ascii_case(&name)) {
                continue;
            }
            let mut d2 = d.clone();
            d2.items.retain(|(obj, _)| !obj.eq_ignore_ascii_case(&name));
            if d2.items.is_empty() {
                emptied.push(sid);
            }
            let mut s2 = (*sec).clone();
            s2.data = ObjData::Sector(d2);
            self.replace(s2);
        }
        for sid in emptied {
            self.remove(sid);
        }
    }

    /// `RemoveSelection(classfilter)`: what goes, as `DeleteSelected` takes it.
    fn remove_selection(&mut self, class: Option<ObjClass>) -> usize {
        let sel: Vec<ObjId> = self
            .selected_in_order()
            .into_iter()
            .filter(|id| class.is_none_or(|c| self.get(*id).is_some_and(|o| o.class == c)))
            .collect();
        let count = sel.len();
        for id in sel {
            let Some(o) = self.get(id) else { continue };
            if o.flags.co & co::IN_GROUP != 0 || !self.tools.part(o.class).editable() {
                continue;
            }
            // `CWayObject::OnSelectionRemove`: in Points mode a way loses its selected points
            // and goes only when none are left
            if self.tools.obj.way_points
                && let ObjData::Way(w) = &o.data
            {
                let mut w2 = w.clone();
                if crate::objtools::way::remove_selected_points(&mut w2) == 0 && !w2.points.is_empty() {
                    continue;
                }
                if !w2.points.is_empty() {
                    let mut n = (**o).clone();
                    n.data = ObjData::Way(w2);
                    self.replace(n);
                    continue;
                }
            }
            self.delete(id);
        }
        count
    }

    /// `LoadSelection`'s appends: names made free with `GenObjectName`, the tools that are not
    /// enabled or are read-only left out, a bulk tool's data replaced. Returns the objects added.
    fn paste(&mut self, sel: crate::selection::Selection) -> usize {
        use crate::selection::ToolSelection;
        // `SelectObjects(false)`
        self.selection.clear();
        let mut n = 0;
        for l in sel.legacy {
            // `EScene::OnLoadSelectionAppendObject`: the author's, and selected
            let id = crate::io::insert_loaded(self, l);
            if let Some(o) = self.get(id).cloned() {
                let mut o = (*o).clone();
                o.marks.authored = true;
                self.replace(o);
            }
            self.selection.insert(id);
            n += 1;
        }
        for (class, part) in sel.tools {
            let t = self.tools.part(class);
            if !t.enabled || !t.editable() {
                continue;
            }
            match part {
                // oracle: `ESceneCustomOTool::OnLoadSelectionAppendObject` neither selects the
                // object nor marks it the author's: the copy keeps the flags it was copied with
                ToolSelection::Objects(objs) => {
                    for l in objs {
                        crate::io::insert_loaded(self, l);
                        n += 1;
                    }
                }
                ToolSelection::Bulk(bytes) => {
                    let _ = self.bulk.set_stream(class, &bytes);
                }
            }
        }
        n
    }

    /// `ESceneCustomOTool::MultiRenameObjects`.
    fn multi_rename(&mut self, class: ObjClass) -> usize {
        let mut n = 0;
        for id in self.by_class[class.index()].clone() {
            if !self.selection.contains(&id) {
                continue;
            }
            let Some(o) = self.get(id).cloned() else { continue };
            // oracle: the object's own name counts as used, so a name equal to its reference
            // becomes `<ref>_00`
            let name = self.gen_name(o.ref_name());
            if !o.name.eq_ignore_ascii_case(&name) {
                let mut o2 = (*o).clone();
                o2.name = SmolStr::new(name);
                self.replace(o2);
                n += 1;
            }
        }
        n
    }
}

impl State for SceneState {
    type Cmd = SceneCmd;
    type Reply = SceneReply;

    fn apply(&mut self, cmd: SceneCmd, cx: &mut Apply) -> Result<SceneReply, CmdError> {
        match cmd {
            SceneCmd::Replace(state, kind) => {
                let serial = self.serial + 1;
                *self = *state;
                self.serial = serial;
                // a level bound is seen only while Edit Level Bound is on
                crate::objtools::show_level_bounds(self, self.tools.obj.edit_level_bound);
                cx.new_document();
                // a restored snapshot is unsaved work: it is followed by `Touch`
                let _ = kind;
                Ok(SceneReply::None)
            }
            SceneCmd::Touch => {
                cx.content_changed();
                Ok(SceneReply::None)
            }
            SceneCmd::Place(obj) => {
                if self.name_used(&obj.name) {
                    return Err(CmdError::invalid("object name already used"));
                }
                if !self.tools.part(obj.class).editable() {
                    return Err(CmdError::invalid(format!("The object '{}' is read only.", obj.name)));
                }
                let class = obj.class;
                for id in self.by_class[class.index()].clone() {
                    self.selection.remove(&id);
                }
                let mut obj = *obj;
                obj.marks.authored = true;
                let id = self.insert(obj);
                self.selection.insert(id);
                self.tools.target = class;
                self.bump();
                cx.undoable("Add Object");
                Ok(SceneReply::Id(id))
            }
            SceneCmd::Delete(ids) => {
                let mut n = 0;
                for id in ids {
                    if self.get(id).is_some()
                        && self.tools.part(self.get(id).map_or(ObjClass::Group, |o| o.class)).editable()
                    {
                        self.delete(id);
                        n += 1;
                    }
                }
                if n > 0 {
                    self.bump();
                }
                cx.undoable("Delete");
                Ok(SceneReply::Count(n))
            }
            SceneCmd::DeleteSelected => {
                let count = self.remove_selection(None);
                self.bump();
                cx.undoable("Delete");
                Ok(SceneReply::Count(count))
            }
            SceneCmd::Rename { id, name } => {
                let name = SmolStr::new(name.to_lowercase());
                if self.find_any(&name).is_some_and(|o| o.id != id) {
                    return Err(CmdError::invalid("new name already used"));
                }
                self.check_editable(id)?;
                self.edit_object(id, |o| o.name = name)?;
                self.bump();
                cx.undoable("Rename");
                Ok(SceneReply::None)
            }
            SceneCmd::Transform { items, members, label, merge } => {
                for (id, e) in &items {
                    if !match e {
                        TransformEdit::Position(v) | TransformEdit::Rotation(v) | TransformEdit::Scale(v) => {
                            v.is_finite()
                        }
                        TransformEdit::MoveTo { position, .. } | TransformEdit::Land { position, .. } => {
                            position.is_finite()
                        }
                    } {
                        return Err(CmdError::invalid("non-finite component"));
                    }
                    self.check_editable(*id)?;
                    self.transform(*id, *e, members)?;
                }
                if items.is_empty() {
                    cx.unchanged();
                    return Ok(SceneReply::Count(0));
                }
                self.bump();
                match merge {
                    Some(key) => cx.undoable_merged(label, key),
                    None => cx.undoable(label),
                }
                Ok(SceneReply::Count(items.len()))
            }
            SceneCmd::Select { ids, mode } => {
                if mode == SelectMode::Set {
                    self.selection.clear();
                }
                let mut n = 0;
                for id in ids {
                    let Some(o) = self.get(id) else { continue };
                    if !o.flags.visible {
                        continue;
                    }
                    n += 1;
                    match mode {
                        SelectMode::Set | SelectMode::Add => {
                            self.selection.insert(id);
                        }
                        SelectMode::Remove => {
                            self.selection.remove(&id);
                        }
                        SelectMode::Toggle => {
                            if self.selection.remove(&id).is_none() {
                                self.selection.insert(id);
                            }
                        }
                    }
                }
                Ok(SceneReply::Count(n))
            }
            SceneCmd::SelectClass { class, select } => {
                let classes: Vec<ObjClass> = class.map_or_else(|| ObjClass::ALL.to_vec(), |c| vec![c]);
                for c in classes {
                    for id in self.by_class[c.index()].clone() {
                        let visible = self.get(id).is_some_and(|o| o.flags.visible);
                        if select && visible {
                            self.selection.insert(id);
                        } else if !select && visible {
                            self.selection.remove(&id);
                        }
                    }
                }
                Ok(SceneReply::None)
            }
            SceneCmd::SetTarget(class) => {
                // `ESceneShapeTool::OnActivate` / `OnDeactivate`: Edit Level Bound goes off
                let before = self.tools.target;
                if before != class
                    && (before == ObjClass::Shape || class == ObjClass::Shape)
                    && self.tools.obj.edit_level_bound
                {
                    self.tools.obj.edit_level_bound = false;
                    crate::objtools::show_level_bounds(self, false);
                }
                self.tools.target = class;
                Ok(SceneReply::None)
            }
            SceneCmd::Show { ids, visible } => {
                let mut n = 0;
                for id in ids {
                    if self.get(id).is_some_and(|o| o.flags.visible != visible) {
                        self.edit_object(id, |o| o.flags.visible = visible)?;
                        n += 1;
                    }
                    if !visible {
                        self.selection.remove(&id);
                    }
                }
                Ok(SceneReply::Count(n))
            }
            SceneCmd::ToolFlags { class, visible, readonly } => {
                let p = self.tools.part_mut(class);
                let mut changed = 0;
                if let Some(v) = visible
                    && p.visible != v
                {
                    p.visible = v;
                    changed += 1;
                }
                if let Some(r) = readonly
                    && p.readonly != r
                {
                    p.readonly = r;
                    changed += 1;
                }
                Ok(SceneReply::Count(changed))
            }
            SceneCmd::Group { name } => {
                let name =
                    if self.name_used(&name) { SmolStr::new(self.gen_name(Some(&name))) } else { name };
                let id = self.group_selection(name)?;
                self.bump();
                cx.undoable("Group");
                Ok(SceneReply::Id(id))
            }
            SceneCmd::Ungroup => {
                let n = self.ungroup_selection();
                if n == 0 {
                    return Err(CmdError::invalid("Nothing selected."));
                }
                self.bump();
                cx.undoable("Ungroup");
                Ok(SceneReply::Count(n))
            }
            SceneCmd::ReplaceObjects { objects, serial, label } => {
                if serial != self.serial {
                    return Err(CmdError::busy("the scene changed meanwhile; ask again"));
                }
                let n = objects.len();
                for o in objects {
                    if self.get(o.id).is_none() {
                        return Err(missing(o.id));
                    }
                    self.check_editable(o.id)?;
                    if self.find_any(&o.name).is_some_and(|x| x.id != o.id) {
                        return Err(CmdError::invalid("the name is used by another object"));
                    }
                    self.replace(o);
                }
                self.bump();
                cx.undoable(label);
                Ok(SceneReply::Count(n))
            }
            SceneCmd::SetWorld { level, tools, serial, label } => {
                if serial != self.serial {
                    return Err(CmdError::busy("the scene changed meanwhile; ask again"));
                }
                self.meta.level = *level;
                for t in tools {
                    let c = t.class;
                    *self.tools.part_mut(c) = t;
                }
                self.bump();
                cx.undoable(label);
                Ok(SceneReply::None)
            }
            SceneCmd::MarkAuthored(on) => {
                let mut n = 0;
                for id in self.selected_in_order() {
                    if self.get(id).is_some_and(|o| o.marks.authored != on) {
                        self.edit_object(id, |o| o.marks.authored = on)?;
                        n += 1;
                    }
                }
                if n > 0 {
                    self.bump();
                    cx.undoable("Mark Authored");
                } else {
                    cx.unchanged();
                }
                Ok(SceneReply::Count(n))
            }
            SceneCmd::MarkRemoved(on) => {
                let (mut n, mut own) = (0, 0);
                for id in self.selected_in_order() {
                    let Some(o) = self.get(id) else { continue };
                    if o.class != ObjClass::SpawnPoint {
                        continue;
                    }
                    if o.marks.authored {
                        own += 1;
                        continue;
                    }
                    if o.marks.removed != on {
                        self.edit_object(id, |o| o.marks.removed = on)?;
                        n += 1;
                    }
                }
                if n > 0 {
                    self.bump();
                    cx.undoable("Mark Removed");
                } else {
                    cx.unchanged();
                }
                Ok(SceneReply::Counts(n, own))
            }
            SceneCmd::SetOrder { id, order } => {
                self.edit_object(id, |o| {
                    if let ObjData::Spawn(s) = &mut o.data {
                        s.order = order;
                    }
                })?;
                self.bump();
                cx.undoable("Order");
                Ok(SceneReply::None)
            }
            SceneCmd::Snap { action, use_list, pick } => {
                if let Some(u) = use_list {
                    self.tools.snap_use = u;
                }
                if let Some(p) = pick {
                    self.tools.snap_pick = p;
                }
                if let Some(a) = action
                    && crate::snap::apply_snap(self, a)
                    && a != SnapAction::Select
                {
                    self.bump();
                    cx.undoable("Snap List");
                }
                Ok(SceneReply::Count(self.tools.snap.len()))
            }
            SceneCmd::SetBulk(bulk) => {
                self.bulk = *bulk;
                self.bump();
                cx.undoable("Edit Bulk Data");
                Ok(SceneReply::None)
            }
            SceneCmd::Saved { file } => {
                self.meta.file = Some(file);
                cx.mark_saved();
                Ok(SceneReply::None)
            }
            SceneCmd::Forget => {
                cx.mark_saved();
                Ok(SceneReply::None)
            }
            SceneCmd::ObjTool(edit) => crate::objtools::apply(self, *edit, cx),
            SceneCmd::Bulk(edit) => crate::bulk::apply(self, *edit, cx),
            SceneCmd::RemoveSelection { class, label } => {
                let n = self.remove_selection(class);
                self.bump();
                cx.undoable(label);
                Ok(SceneReply::Count(n))
            }
            SceneCmd::Paste { selection, label } => {
                let n = self.paste(*selection);
                self.bump();
                cx.undoable(label);
                Ok(SceneReply::Count(n))
            }
            SceneCmd::MultiRename(class) => {
                let n = self.multi_rename(class);
                if n == 0 {
                    cx.unchanged();
                    return Ok(SceneReply::Count(0));
                }
                self.bump();
                cx.undoable("Rename Objects");
                Ok(SceneReply::Count(n))
            }
            SceneCmd::LevelPart { class, enabled, part } => {
                for id in self.by_class[class.index()].clone() {
                    self.remove(id);
                }
                match class {
                    ObjClass::AiMap => self.bulk.ai_map = None,
                    ObjClass::DetailObjects => self.bulk.details = None,
                    ObjClass::Wallmark => self.bulk.wallmarks = None,
                    _ => {}
                }
                let keep = (self.tools.part(class).visible, self.tools.part(class).readonly);
                *self.tools.part_mut(class) = crate::tools::ToolPart::new(class);
                if let Some(p) = part {
                    let p = *p;
                    *self.tools.part_mut(class) = p.tool;
                    if let Some(b) = &p.bulk {
                        let _ = self.bulk.set_stream(class, b);
                    }
                    for o in p.objects {
                        crate::io::insert_loaded(self, o);
                    }
                }
                let t = self.tools.part_mut(class);
                t.enabled = enabled;
                (t.visible, t.readonly) = keep;
                self.bump();
                // the part's file is what it was: published, nothing of it unsaved
                let _ = cx;
                Ok(SceneReply::None)
            }
            SceneCmd::ReloadGroups { groups } => {
                for (id, _) in &groups {
                    self.check_editable(*id)?;
                }
                let n = groups.len();
                for (id, loaded) in groups {
                    self.reload_group(id, *loaded)?;
                }
                self.bump();
                cx.undoable("Reload References");
                Ok(SceneReply::Count(n))
            }
            SceneCmd::PlaceGroup { loaded, at } => {
                if !self.tools.part(ObjClass::Group).editable() {
                    return Err(CmdError::invalid(format!("The object '{}' is read only.", loaded.obj.name)));
                }
                if self.name_used(&loaded.obj.name) {
                    return Err(CmdError::invalid("object name already used"));
                }
                let mut l = *loaded;
                l.obj.marks.authored = true;
                // the first setup: nothing of a new group is unique
                for m in &mut l.members {
                    m.obj.flags.co &= !co::IN_GROUP_UNIQUE;
                    m.obj.marks.authored = true;
                }
                for id in self.by_class[ObjClass::Group.index()].clone() {
                    self.selection.remove(&id);
                }
                let gid = self.insert_group_at(l, at)?;
                self.selection.insert(gid);
                self.tools.target = ObjClass::Group;
                self.bump();
                cx.undoable("Add Object");
                Ok(SceneReply::Id(gid))
            }
            SceneCmd::SpawnAttach { spawn, object } => {
                let sp = (**self.get(spawn).ok_or_else(|| missing(spawn))?).clone();
                let ObjData::Spawn(sd) = &sp.data else {
                    return Err(CmdError::invalid("Attach impossible."));
                };
                self.check_editable(spawn)?;
                let mut sd = (**sd).clone();
                // the point's own shape goes back into the scene first (`DetachObject`)
                if let Some(old) = sd.attached.take() {
                    let mut back = (*old).clone();
                    // oracle: the C++ appends the point's own shape (`EnsureShape` made it with no
                    // name) as it is - a nameless object nothing can address; it is named as the
                    // Shape tool names a new one
                    let base = if back.name.is_empty() { "shape" } else { back.name.as_str() };
                    back.name = SmolStr::new(self.gen_name(Some(base)));
                    back.owner = None;
                    let id = self.insert(back);
                    if object.is_none() {
                        self.selection.insert(id);
                    }
                }
                let mut n = sp.clone();
                if let Some(oid) = object {
                    let shape = (**self.get(oid).ok_or_else(|| missing(oid))?).clone();
                    // `bAllowed`: a shape, onto an entity that has one
                    let has_shape = sd
                        .entity
                        .as_ref()
                        .and_then(|e| e.entity())
                        .and_then(|e| e.class_info())
                        .is_some_and(|c| c.class.has_shape());
                    if shape.class != ObjClass::Shape || !has_shape {
                        return Err(CmdError::invalid("Attach impossible."));
                    }
                    self.check_editable(oid)?;
                    self.remove(oid);
                    n.xform = shape.xform;
                    sd.attached = Some(Arc::new(shape));
                }
                n.data = ObjData::Spawn(Box::new(sd));
                self.replace(n);
                self.bump();
                cx.undoable(if object.is_some() { "Attach" } else { "Detach" });
                Ok(SceneReply::None)
            }
            SceneCmd::DebugDraw(d) => {
                self.tools.debug = d;
                Ok(SceneReply::None)
            }
            SceneCmd::Motion { id, motion, flags, pose, label, merge } => {
                self.edit_object(id, |o| {
                    o.motion = motion;
                    o.flags.co = flags & !(co::AUTHOR_PLACED | co::BASE_REMOVED);
                    if let Some((p, r)) = pose {
                        set_position(o, p);
                        set_rotation(o, r);
                    }
                })?;
                self.bump();
                match merge {
                    Some(key) => cx.undoable_merged(label, key),
                    None => cx.undoable(label),
                }
                Ok(SceneReply::None)
            }
        }
    }

    /// Undo brings the content back; the selection of objects that still exist, the target
    /// class, the tools' eyes and switches and the file name stay.
    fn restored(&self, current: &Self) -> Self {
        let mut s = self.clone();
        s.selection = current.selection.iter().copied().filter(|id| s.objects.contains_key(id)).collect();
        s.tools.target = current.tools.target;
        s.tools.snap_use = current.tools.snap_use;
        s.tools.snap_pick = current.tools.snap_pick;
        for c in ObjClass::ALL {
            s.tools.part_mut(c).visible = current.tools.part(c).visible;
            s.tools.part_mut(c).readonly = current.tools.part(c).readonly;
        }
        s.meta.file = current.meta.file.clone();
        s.serial = current.serial + 1;
        s.tools.obj = current.tools.obj;
        // a level part loaded or unloaded since is not undone: the class is taken as it is now
        for c in ObjClass::ALL {
            if s.tools.part(c).enabled == current.tools.part(c).enabled {
                continue;
            }
            for id in s.by_class[c.index()].clone() {
                s.remove(id);
            }
            for o in current.of_class(c) {
                s.by_class[c.index()].insert(o.id);
                s.names.insert((o.class, o.name.clone()), o.id);
                s.objects.insert(o.id, o.clone());
            }
            *s.tools.part_mut(c) = current.tools.part(c).clone();
            match c {
                ObjClass::AiMap => s.bulk.ai_map.clone_from(&current.bulk.ai_map),
                ObjClass::DetailObjects => s.bulk.details.clone_from(&current.bulk.details),
                ObjClass::Wallmark => s.bulk.wallmarks.clone_from(&current.bulk.wallmarks),
                _ => {}
            }
        }
        s.next_id = s.next_id.max(current.next_id);
        s.tools.bulk = current.tools.bulk.clone();
        s.tools.debug = current.tools.debug.clone();
        let on = s.tools.obj.edit_level_bound;
        crate::objtools::show_level_bounds(&mut s, on);
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{GlowData, LightData};
    use gz_store::{Store, StoreOptions};
    use gz_task::{ExecOptions, Runtime};

    struct T {
        rt: Runtime,
        store: Store<SceneState>,
    }

    impl T {
        fn new(s: SceneState) -> Self {
            let rt = Runtime::start(ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 1 }).unwrap();
            let store = Store::spawn("scene", s, &rt.exec(), StoreOptions::default());
            Self { rt, store }
        }
        fn send(&self, cmd: SceneCmd) -> Result<SceneReply, CmdError> {
            self.rt.block_on(self.store.send(cmd, gz_base::Origin::Mcp))
        }
        fn state(&self) -> SceneState {
            self.store.snapshot().state.clone()
        }
    }

    #[test]
    fn place_selects_and_marks_the_author() {
        let t = T::new(SceneState::default());
        let Ok(SceneReply::Id(id)) = t.send(SceneCmd::Place(Box::new(Object::new(
            ObjClass::Light,
            "l",
            ObjData::Light(LightData::default()),
        )))) else {
            panic!()
        };
        let s = t.state();
        assert!(s.is_selected(id));
        assert!(s.get(id).unwrap().marks.authored);
        assert!(
            t.send(SceneCmd::Place(Box::new(Object::new(
                ObjClass::Glow,
                "l",
                ObjData::Glow(GlowData::default())
            ))))
            .is_err()
        );
        assert!(t.store.is_dirty());
    }

    #[test]
    fn deleting_a_scene_object_takes_it_out_of_its_sectors() {
        // `ESceneSectorTool::OnObjectRemove`: the sector that held the object alone goes, the
        // other keeps the meshes of the object that stays
        use crate::objects::{SectorData, default_data};
        let mut s = SceneState::default();
        let a = s.insert(Object::new(ObjClass::SceneObject, "a", default_data(ObjClass::SceneObject)));
        s.insert(Object::new(ObjClass::SceneObject, "b", default_data(ObjClass::SceneObject)));
        let only_a = SectorData { items: vec![("a".into(), "m".into())], ..SectorData::default() };
        let both = SectorData {
            items: vec![("A".into(), "m".into()), ("b".into(), "m".into())],
            ..SectorData::default()
        };
        s.insert(Object::new(ObjClass::Sector, "s1", ObjData::Sector(only_a)));
        let s2 = s.insert(Object::new(ObjClass::Sector, "s2", ObjData::Sector(both)));
        s.selection.insert(a);
        let t = T::new(s);
        assert!(matches!(
            t.send(SceneCmd::RemoveSelection { class: Some(ObjClass::SceneObject), label: "Delete".into() }),
            Ok(SceneReply::Count(1))
        ));
        let s = t.state();
        assert!(s.find_relaxed("s1").is_none(), "the sector left without a mesh goes");
        let Some(ObjData::Sector(d)) = s.get(s2).map(|o| &o.data) else { panic!("s2 stays") };
        assert_eq!(d.items, vec![(SmolStr::new("b"), SmolStr::new("m"))]);
    }

    #[test]
    fn undo_restores_the_content_and_keeps_the_view() {
        let mut s = SceneState::default();
        let id = s.insert(Object::new(ObjClass::Light, "l", ObjData::Light(LightData::default())));
        let t = T::new(s);
        t.send(SceneCmd::Transform {
            items: vec![(id, TransformEdit::Position(Vec3::X))],
            members: false,
            label: "Move".into(),
            merge: None,
        })
        .unwrap();
        t.send(SceneCmd::Select { ids: vec![id], mode: SelectMode::Set }).unwrap();
        assert_eq!(t.state().get(id).unwrap().position(), Vec3::X);
        assert_eq!(t.rt.block_on(t.store.undo()).as_deref(), Some("Move"));
        let s = t.state();
        assert_eq!(s.get(id).unwrap().position(), Vec3::ZERO);
        assert!(s.is_selected(id));
        assert!(!t.store.is_dirty());
        t.rt.block_on(t.store.redo());
        assert_eq!(t.state().get(id).unwrap().position(), Vec3::X);
    }

    #[test]
    fn groups_carry_their_members_only_when_asked() {
        let mut s = SceneState::default();
        let a = s.insert(Object::new(ObjClass::Light, "a", ObjData::Light(LightData::default())));
        s.selection.insert(a);
        let t = T::new(s);
        let Ok(SceneReply::Id(g)) = t.send(SceneCmd::Group { name: "00".into() }) else { panic!() };
        assert_eq!(t.state().get(a).unwrap().owner, Some(g));
        t.send(SceneCmd::Transform {
            items: vec![(g, TransformEdit::Position(Vec3::Y))],
            members: false,
            label: "Move".into(),
            merge: None,
        })
        .unwrap();
        assert_eq!(t.state().get(a).unwrap().position(), Vec3::ZERO);
        t.send(SceneCmd::Transform {
            items: vec![(g, TransformEdit::Position(Vec3::Y * 2.0))],
            members: true,
            label: "Move".into(),
            merge: None,
        })
        .unwrap();
        assert!((t.state().get(a).unwrap().position() - Vec3::Y).length() < 1e-5);
        t.send(SceneCmd::Select { ids: vec![g], mode: SelectMode::Set }).unwrap();
        t.send(SceneCmd::DeleteSelected).unwrap();
        let s = t.state();
        assert!(s.get(a).is_none() && s.get(g).is_none());
    }

    /// A library group as read: two lights about the file's origin.
    fn library_group(unique: bool) -> crate::objects::Loaded {
        let member = |name: &str, x: f32| {
            let mut o = Object::new(ObjClass::Light, name, ObjData::Light(LightData::default()));
            o.xform.position = Vec3::new(x, 0.0, 0.0);
            o.flags.co |= co::IN_GROUP | if unique { co::IN_GROUP_UNIQUE } else { 0 };
            crate::objects::Loaded { obj: o, members: Vec::new() }
        };
        crate::objects::Loaded {
            obj: Object::new(
                ObjClass::Group,
                "lib",
                ObjData::Group(GroupData { reference: "props\\pair".into(), members: Vec::new() }),
            ),
            members: vec![member("l0", -1.0), member("l1", 1.0)],
        }
    }

    #[test]
    fn a_placed_group_stands_where_it_was_put_and_is_selected() {
        let t = T::new(SceneState::default());
        let at = crate::math::Transform::at(Vec3::new(10.0, 0.0, 0.0));
        let Ok(SceneReply::Id(g)) =
            t.send(SceneCmd::PlaceGroup { loaded: Box::new(library_group(true)), at })
        else {
            panic!()
        };
        let s = t.state();
        let ObjData::Group(gd) = &s.get(g).unwrap().data else { panic!() };
        let xs: Vec<f32> = gd.members.iter().map(|m| s.get(*m).unwrap().position().x).collect();
        assert!((xs[0] - 9.0).abs() < 1e-5 && (xs[1] - 11.0).abs() < 1e-5, "{xs:?}");
        // the first setup: nothing unique, everything the author's
        assert!(gd.members.iter().all(|m| s.get(*m).unwrap().flags.co & co::IN_GROUP_UNIQUE == 0));
        assert!(s.get(g).unwrap().marks.authored && s.is_selected(g));
    }

    #[test]
    fn a_reload_keeps_the_group_its_names_and_its_unique_members() {
        let t = T::new(SceneState::default());
        let at = crate::math::Transform::at(Vec3::new(5.0, 0.0, 0.0));
        let Ok(SceneReply::Id(g)) =
            t.send(SceneCmd::PlaceGroup { loaded: Box::new(library_group(false)), at })
        else {
            panic!()
        };
        // the author renames one member and marks the other unique, moved off its place
        let s = t.state();
        let ObjData::Group(gd) = &s.get(g).unwrap().data else { panic!() };
        let (m0, m1) = (gd.members[0], gd.members[1]);
        let mut a = (**s.get(m0).unwrap()).clone();
        a.name = "renamed".into();
        let mut b = (**s.get(m1).unwrap()).clone();
        b.flags.co |= co::IN_GROUP_UNIQUE;
        b.xform.position = Vec3::new(100.0, 0.0, 0.0);
        t.send(SceneCmd::ReplaceObjects { objects: vec![a, b], serial: s.serial, label: "Edit".into() })
            .unwrap();
        let group_name = s.get(g).unwrap().name.clone();
        t.send(SceneCmd::ReloadGroups { groups: vec![(g, Box::new(library_group(false)))] }).unwrap();
        let s = t.state();
        let ng = s.find(ObjClass::Group, &group_name).expect("the group keeps its name");
        assert!((ng.position().x - 5.0).abs() < 1e-5);
        let ObjData::Group(gd) = &ng.data else { panic!() };
        let members: Vec<_> = gd.members.iter().map(|m| s.get(*m).unwrap().clone()).collect();
        assert_eq!(members[0].name.as_str(), "renamed");
        assert!((members[0].position().x - 4.0).abs() < 1e-5);
        assert!((members[1].position().x - 100.0).abs() < 1e-5, "the unique member stays as it was");
        assert!(members.iter().all(|m| m.owner == Some(ng.id)));
        // one step back is the scene before the reload
        assert_eq!(t.rt.block_on(t.store.undo()).as_deref(), Some("Reload References"));
        assert_eq!(t.state().get(m0).unwrap().name.as_str(), "renamed");
    }

    #[test]
    fn a_stale_replace_is_refused_and_changes_nothing() {
        let mut s = SceneState::default();
        let id = s.insert(Object::new(ObjClass::Light, "l", ObjData::Light(LightData::default())));
        let t = T::new(s);
        let snap = t.state();
        let o = (**snap.get(id).unwrap()).clone();
        t.send(SceneCmd::Rename { id, name: "m".into() }).unwrap();
        assert!(
            t.send(SceneCmd::ReplaceObjects { objects: vec![o], serial: snap.serial, label: "x".into() })
                .is_err()
        );
        assert_eq!(t.state().get(id).unwrap().name, "m");
    }
}
