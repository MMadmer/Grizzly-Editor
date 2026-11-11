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
    /// An edit of the sector, portal, way, shape, light and fog volume tools.
    ObjTool(Box<crate::objtools::ToolEdit>),
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

    /// Removes an object; a group takes its members along, a member leaves its group.
    fn delete(&mut self, id: ObjId) {
        if let Some(o) = self.get(id).cloned()
            && let ObjData::Group(g) = &o.data
        {
            for m in &g.members {
                self.remove(*m);
            }
        }
        self.remove(id);
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
                let sel: Vec<ObjId> = self.selected_in_order();
                let count = sel.len();
                for id in sel {
                    let Some(o) = self.get(id) else { continue };
                    if o.flags.co & co::IN_GROUP != 0 || !self.tools.part(o.class).editable() {
                        continue;
                    }
                    // `CWayObject::OnSelectionRemove`: in Points mode a way loses its selected
                    // points and goes only when none are left
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
                self.bump();
                cx.undoable("Delete");
                Ok(SceneReply::Count(count))
            }
            SceneCmd::Rename { id, name } => {
                let name = SmolStr::new(name.to_lowercase());
                if self.find_any(&name).is_some_and(|o| o.id != id) {
                    return Err(CmdError::invalid("new name already used"));
                }
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
            SceneCmd::ObjTool(edit) => crate::objtools::apply(self, *edit, cx),
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
