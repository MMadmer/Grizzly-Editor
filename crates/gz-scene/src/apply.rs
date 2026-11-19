//! The scene's commands: every change of [`SceneState`] is one of them.

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::SmolStr;
use gz_store::{Apply, CmdError, State};

use crate::class::ObjClass;
use crate::math::{Vec3, normal_align, rotation_matrix};
use crate::model::{ObjData, ObjId, Object, SceneState, co};
use crate::objects::{GroupData, SpawnEntity, XmsOrder};
use crate::tools::{BulkData, LevelOptions, SceneMode, ToolPart};

/// One edit of a transform, with the class rules of the position, rotation and scale setters.
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

/// Moves an object by `d`: a way path with its points, a sound source with its sound, a spawn
/// point with its shape.
pub fn translate(o: &mut Object, d: Vec3) {
    if d == Vec3::ZERO {
        return;
    }
    let p = o.position() + d;
    set_position(o, p);
}

/// Every change of the scene.
#[derive(Clone, Debug)]
pub enum SceneCmd {
    /// A loaded (or new) scene takes the place of the open one.
    Replace(Box<SceneState>, ReplaceKind),
    /// A new object: the author's, and the selection. The name must be free.
    Place(Box<Object>),
    /// Removes objects (a group's members are removed with it).
    Delete(Vec<ObjId>),
    /// Every selected object that is not inside a group and not locked.
    DeleteSelected,
    Rename {
        id: ObjId,
        name: SmolStr,
    },
    /// Transforms, one undo step. `members` moves a group's objects with it (the Details
    /// panel's number rows); the MCP setters move the group alone.
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
    /// Selects (or deselects, or inverts the selection of) every object the viewport may pick,
    /// of the kinds `classes` names (`None`: of every kind).
    SelectAll {
        classes: Option<Vec<ObjClass>>,
        select: bool,
        invert: bool,
    },
    /// What the viewport's clicks do.
    SetMode(SceneMode),
    /// Place mode with a kind of thing to place (a painted layer's kind, or a cell's or a
    /// portal's, puts up that layer's mode instead).
    Arm(ObjClass),
    /// Objects' eyes: a hidden object is deselected. Kept with the scene, no undo step.
    Show {
        ids: Vec<ObjId>,
        visible: bool,
    },
    /// Objects' locks. Kept with the scene, no undo step.
    Lock {
        ids: Vec<ObjId>,
        locked: bool,
    },
    /// A kind's eye or lock.
    ClassView {
        class: ObjClass,
        visible: Option<bool>,
        locked: Option<bool>,
    },
    /// The selected visible objects not in a group, into a new group.
    Group {
        name: SmolStr,
    },
    /// Ungroups every selected group.
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
    /// Marks the selection as placed by the author; answers how many changed.
    MarkAuthored(bool),
    /// Marks the selected spawns that are not authored as removed from the base; answers changed, skipped.
    MarkRemoved(bool),
    /// A stalker's standing order.
    SetOrder {
        id: ObjId,
        order: XmsOrder,
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
    /// The selected objects that are not inside a group and not locked; one undo step under
    /// `label` (Delete, Cut).
    RemoveSelection {
        label: String,
    },
    /// A set of objects added under free names, moved by `offset`, and made the selection
    /// (Paste, Duplicate, a `.group` placed); one undo step under `label`.
    Paste {
        bundle: Box<crate::format::group::Bundle>,
        offset: Vec3,
        label: String,
    },
    /// The scene's content as its files hold it now (another program changed them): one undo
    /// step, the selection and the view kept.
    Reload {
        state: Box<SceneState>,
        label: String,
        /// The files now hold the scene: nothing of it is unsaved.
        clean: bool,
        /// The scene file the reading is of: it goes in only while that scene is the open one
        /// (code `another_scene` otherwise - the scene was switched while its files were read).
        of: std::path::PathBuf,
    },
    /// Attaches a shape of the scene to a spawn point (it becomes the point's own) or, with no
    /// object, detaches it (the point's shape goes back into the scene). One undo step.
    SpawnAttach {
        spawn: ObjId,
        object: Option<ObjId>,
    },
    /// Reloads each group (the Group tool's Reload References): its objects made again from the
    /// library `.group` read into `loaded`, the group where it stood. With as many objects as before,
    /// a member marked unique stays as it was and the others take their old names. The sectors are
    /// not touched. One undo step.
    ReloadGroups {
        groups: Vec<(ObjId, Box<crate::objects::Loaded>)>,
    },
    /// The Group tool's add: a library group read into `loaded`, under
    /// its name, set where `at` says (the placed object's reference is set), selected. One undo step.
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
            if let Some(SpawnEntity::Decoded(e)) = &mut s.entity {
                let _ = Arc::make_mut(e).set_position(p);
            }
        }
        _ => o.xform.position = p,
    }
}

fn set_rotation(o: &mut Object, r: Vec3) {
    // a terrain turns about the vertical alone: its mesh, its cut and the game read the heading
    let r = if matches!(o.data, ObjData::Terrain(_)) { Vec3::new(0.0, r.y, 0.0) } else { r };
    o.xform.rotation = r;
    if let ObjData::Spawn(s) = &mut o.data {
        if let Some(a) = &mut s.attached {
            Arc::make_mut(a).xform.rotation = r;
        }
        if let Some(SpawnEntity::Decoded(e)) = &mut s.entity {
            let _ = Arc::make_mut(e).set_angle(r);
        }
    }
}

fn set_scale(o: &mut Object, v: Vec3) {
    match &mut o.data {
        ObjData::Glow(g) => {
            // the component that changed is the radius
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
        // a terrain's size is its `size`
        ObjData::Terrain(_) => {}
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

    /// A locked object (or one of a locked kind) is not changed, and neither is one of the
    /// level the scene lies over.
    pub(crate) fn check_editable(&self, id: ObjId) -> Result<(), CmdError> {
        match self.get(id) {
            Some(o) if o.marks.base => Err(self.level_s_own(o)),
            Some(o) if !self.changeable(o) => Err(CmdError::new(
                "refused",
                format!("'{}' is locked: unlock it (or its kind) to change it", o.name),
            )),
            _ => Ok(()),
        }
    }

    /// Why an object of the level the scene lies over is not changed.
    fn level_s_own(&self, o: &Object) -> CmdError {
        let what = if o.class == ObjClass::SpawnPoint {
            "a scene only takes it out of the level (`removed = true`, or delete it)"
        } else {
            "a scene changes nothing of it"
        };
        CmdError::new(
            "refused",
            format!("'{}' is the level's own (the game's {}): {what}", o.name, self.meta.base.level),
        )
    }

    /// Whether `new` is an object of the level with nothing changed but whether the scene takes
    /// it out: the one change a scene makes of the level's own.
    fn only_removes(&self, new: &Object) -> bool {
        self.get(new.id).is_some_and(|old| {
            old.marks.base && old.class == ObjClass::SpawnPoint && {
                let mut same = (**old).clone();
                same.marks.removed = new.marks.removed;
                same == *new
            }
        })
    }

    fn edit_object(&mut self, id: ObjId, f: impl FnOnce(&mut Object)) -> Result<(), CmdError> {
        let mut o = (**self.get(id).ok_or_else(|| missing(id))?).clone();
        f(&mut o);
        self.replace(o);
        Ok(())
    }

    /// One transform, with a group's members carried when `members` is set.
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
                self.check_editable(m)?;
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
        // Auto Key keys a selected motion's object where it now stands
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

    /// The selection into a new group, its pivot where its first object stands.
    fn group_selection(&mut self, name: SmolStr) -> Result<ObjId, CmdError> {
        let selected = self.selected_in_order();
        // the level's own stay where the level has them: a group is the scene's, and moving or
        // deleting it would move or delete what it holds
        let level = selected.iter().filter(|id| self.get(**id).is_some_and(|o| o.marks.base)).count();
        let ids: Vec<ObjId> = selected
            .into_iter()
            .filter(|id| {
                self.get(*id).is_some_and(|o| {
                    o.flags.visible
                        && !o.marks.base
                        && o.flags.co & co::IN_GROUP == 0
                        && o.owner.is_none()
                        && o.class != ObjClass::Sector
                        && o.class != ObjClass::Group
                })
            })
            .collect();
        if ids.is_empty() {
            return Err(if level > 0 {
                CmdError::new(
                    "refused",
                    format!(
                        "the selection is the level's own (the game's {}): a group holds what the scene adds",
                        self.meta.base.level
                    ),
                )
            } else {
                CmdError::invalid("Group can't be created.")
            });
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

    /// Every selected group let go of its members, which become the selection.
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

    /// A group as read, its objects carried from where the file had it to `at`.
    pub(crate) fn insert_group_at(
        &mut self,
        loaded: crate::objects::Loaded,
        at: crate::math::Transform,
    ) -> Result<ObjId, CmdError> {
        let gid = crate::sdk::insert_loaded(self, loaded);
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
    /// object's meshes leave the sectors that hold them.
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

    /// A scene object's meshes out of every sector: a sector left without a mesh goes too.
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

    /// What a delete of the selection takes: every selected object that is not inside a group
    /// and not locked; a way of which points are selected loses those points and goes only when
    /// none are left.
    fn remove_selection(&mut self) -> usize {
        let sel = self.selected_in_order();
        let mut count = 0;
        for id in sel {
            let Some(o) = self.get(id) else { continue };
            // a spawn point of the level the scene lies over is taken out of it
            if o.marks.base && o.class == ObjClass::SpawnPoint {
                if !o.marks.removed {
                    let mut o2 = (**o).clone();
                    o2.marks.removed = true;
                    self.replace(o2);
                    count += 1;
                }
                continue;
            }
            if o.flags.co & co::IN_GROUP != 0 || !self.changeable(o) {
                continue;
            }
            if let ObjData::Way(w) = &o.data
                && w.points.iter().any(|p| p.selected)
                && w.points.iter().any(|p| !p.selected)
            {
                let mut w2 = w.clone();
                crate::objtools::way::remove_selected_points(&mut w2);
                let mut n = (**o).clone();
                n.data = ObjData::Way(w2);
                self.replace(n);
                count += 1;
                continue;
            }
            self.delete(id);
            count += 1;
        }
        count
    }

    /// A set of objects added: free names, the author's, the selection. Answers how many.
    fn paste(&mut self, bundle: crate::format::group::Bundle, offset: Vec3) -> usize {
        self.selection.clear();
        let ids = crate::format::group::insert(self, bundle, offset);
        for id in &ids {
            if let Some(o) = self.get(*id) {
                let mut o = (**o).clone();
                // what is added is the scene's own, whatever it was a copy of
                o.marks.authored = true;
                o.marks.base = false;
                o.marks.removed = false;
                let top = o.owner.is_none();
                self.replace(o);
                if top {
                    self.selection.insert(*id);
                }
            }
        }
        ids.len()
    }

    /// A spawn's entity says what its object says: its name, where it stands and how it turns.
    fn sync_entity(o: &mut Object) {
        let (name, p, r) = (o.name.clone(), o.xform.position, o.xform.rotation);
        if let ObjData::Spawn(s) = &mut o.data
            && let Some(SpawnEntity::Decoded(e)) = &mut s.entity
            && (e.name() != name || e.position() != p || e.angle() != r)
        {
            let e = Arc::make_mut(e);
            let _ = e.set_name(&name);
            let _ = e.set_position(p);
            let _ = e.set_angle(r);
        }
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
                if !self.name_free(&obj.name) {
                    return Err(CmdError::invalid(format!("an object is named '{}' already", obj.name)));
                }
                if self.tools.part(obj.class).locked {
                    return Err(CmdError::new(
                        "refused",
                        format!("the {} are locked: unlock them to add one", obj.class.label_plural()),
                    ));
                }
                self.selection.clear();
                let mut obj = *obj;
                obj.marks.authored = true;
                Self::sync_entity(&mut obj);
                let id = self.insert(obj);
                self.selection.insert(id);
                self.bump();
                cx.undoable("Add Object");
                Ok(SceneReply::Id(id))
            }
            SceneCmd::Delete(ids) => {
                // nothing is deleted when one of them is the level's own and no spawn point
                for id in &ids {
                    if let Some(o) = self.get(*id)
                        && o.marks.base
                        && o.class != ObjClass::SpawnPoint
                    {
                        return Err(self.level_s_own(o));
                    }
                }
                let mut n = 0;
                for id in ids {
                    if let Some(o) = self.get(id)
                        && o.marks.base
                    {
                        if !o.marks.removed {
                            self.edit_object(id, |o| o.marks.removed = true)?;
                            n += 1;
                        }
                        continue;
                    }
                    if self.get(id).is_some_and(|o| self.changeable(o)) {
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
                let count = self.remove_selection();
                self.bump();
                cx.undoable("Delete");
                Ok(SceneReply::Count(count))
            }
            SceneCmd::Rename { id, name } => {
                if name.trim().is_empty() {
                    return Err(CmdError::invalid("a name cannot be empty"));
                }
                if self.find_any(&name).is_some_and(|o| o.id != id) {
                    return Err(CmdError::invalid(format!("an object is named '{name}' already")));
                }
                self.check_editable(id)?;
                self.edit_object(id, |o| {
                    o.name = name;
                    Self::sync_entity(o);
                })?;
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
            SceneCmd::SelectAll { classes, select, invert } => {
                let classes: Vec<ObjClass> = classes.unwrap_or_else(|| ObjClass::ALL.to_vec());
                let mut n = 0;
                for c in classes {
                    for id in self.by_class[c.index()].clone() {
                        let Some(o) = self.get(id) else { continue };
                        if !self.pickable(o) || o.flags.co & co::IN_GROUP != 0 && !o.editable() {
                            continue;
                        }
                        let on = if invert { !self.selection.contains(&id) } else { select };
                        if on {
                            self.selection.insert(id);
                            n += 1;
                        } else {
                            self.selection.remove(&id);
                        }
                    }
                }
                Ok(SceneReply::Count(n))
            }
            SceneCmd::SetMode(mode) => {
                // drawing the level's bound by hand belongs to the zones
                if mode != SceneMode::Zones && self.tools.obj.edit_level_bound {
                    self.tools.obj.edit_level_bound = false;
                    crate::objtools::show_level_bounds(self, false);
                }
                self.tools.mode = mode;
                Ok(SceneReply::None)
            }
            SceneCmd::Arm(class) => {
                let mode = match class {
                    ObjClass::AiMap => SceneMode::AiMap,
                    ObjClass::DetailObjects => SceneMode::Details,
                    ObjClass::Wallmark => SceneMode::Wallmarks,
                    ObjClass::Sector | ObjClass::Portal => SceneMode::Zones,
                    _ => SceneMode::Place,
                };
                if class.is_object_tool() {
                    self.tools.placing = class;
                }
                self.apply(SceneCmd::SetMode(mode), cx)
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
                if n > 0 {
                    cx.content_changed();
                }
                Ok(SceneReply::Count(n))
            }
            SceneCmd::Lock { ids, locked } => {
                let mut n = 0;
                for id in ids {
                    if self.get(id).is_some_and(|o| o.flags.locked != locked) {
                        self.edit_object(id, |o| o.flags.locked = locked)?;
                        n += 1;
                    }
                }
                if n > 0 {
                    cx.content_changed();
                }
                Ok(SceneReply::Count(n))
            }
            SceneCmd::ClassView { class, visible, locked } => {
                let p = self.tools.part_mut(class);
                let mut changed = 0;
                if let Some(v) = visible
                    && p.visible != v
                {
                    p.visible = v;
                    changed += 1;
                }
                if let Some(l) = locked
                    && p.locked != l
                {
                    p.locked = l;
                    changed += 1;
                }
                if visible == Some(false) || locked == Some(true) {
                    for id in self.by_class[class.index()].clone() {
                        if visible == Some(false) {
                            self.selection.remove(&id);
                        }
                    }
                }
                if changed > 0 {
                    cx.content_changed();
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
                    if !self.only_removes(&o) {
                        self.check_editable(o.id)?;
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
                // over a level of the game the mark is implied: nothing to set
                let ids = if self.meta.base.is_set() { Vec::new() } else { self.selected_in_order() };
                for id in ids {
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
            SceneCmd::RemoveSelection { label } => {
                let n = self.remove_selection();
                self.bump();
                cx.undoable(label);
                Ok(SceneReply::Count(n))
            }
            SceneCmd::Paste { bundle, offset, label } => {
                let n = self.paste(*bundle, offset);
                self.bump();
                cx.undoable(label);
                Ok(SceneReply::Count(n))
            }
            SceneCmd::Reload { state, label, clean, of } => {
                if self.meta.file.as_deref() != Some(of.as_path()) {
                    cx.unchanged();
                    return Err(CmdError::new("another_scene", "another scene is open now"));
                }
                let view = (self.selection.clone(), self.tools.mode, self.meta.file.clone());
                let serial = self.serial + 1;
                let next = self.next_id.max(state.next_id);
                *self = *state;
                self.serial = serial;
                self.next_id = next;
                self.selection = view.0.into_iter().filter(|id| self.objects.contains_key(id)).collect();
                self.tools.mode = view.1;
                self.meta.file = view.2;
                cx.undoable(label);
                if clean {
                    cx.mark_saved();
                }
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
                if self.tools.part(ObjClass::Group).locked {
                    return Err(CmdError::new("refused", "the groups are locked: unlock them to add one"));
                }
                if self.name_used(&loaded.obj.name) {
                    return Err(CmdError::invalid(format!(
                        "an object is named '{}' already",
                        loaded.obj.name
                    )));
                }
                let mut l = *loaded;
                l.obj.marks.authored = true;
                // the first setup: nothing of a new group is unique
                for m in &mut l.members {
                    m.obj.flags.co &= !co::IN_GROUP_UNIQUE;
                    m.obj.marks.authored = true;
                }
                self.selection.clear();
                let gid = self.insert_group_at(l, at)?;
                self.selection.insert(gid);
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
                // the point's own shape goes back into the scene first
                if let Some(old) = sd.attached.take() {
                    let mut back = (*old).clone();
                    // the point's own shape is kept as it is, nameless; it is named as the
                    // Shape tool names a new one, so it can be addressed
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
                    // a shape, onto an entity that has one
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
                self.check_editable(id)?;
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

    /// Undo brings the content back; the selection of objects that still exist, the mode, the
    /// kinds' eyes and locks, the panels' switches and the file name stay.
    fn restored(&self, current: &Self) -> Self {
        let mut s = self.clone();
        s.selection = current.selection.iter().copied().filter(|id| s.objects.contains_key(id)).collect();
        s.tools.mode = current.tools.mode;
        s.tools.placing = current.tools.placing;
        for c in ObjClass::ALL {
            s.tools.part_mut(c).visible = current.tools.part(c).visible;
            s.tools.part_mut(c).locked = current.tools.part(c).locked;
        }
        s.meta.file = current.meta.file.clone();
        s.meta.imported = current.meta.imported.clone();
        s.serial = current.serial + 1;
        s.tools.obj = current.tools.obj;
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

    /// A terrain's mesh, its cut and the game read its heading and its `size`: a rotation keeps
    /// the heading alone, a scale changes nothing.
    #[test]
    fn a_terrain_turns_about_the_vertical_alone_and_does_not_scale() {
        let mut o = Object::new(ObjClass::Terrain, "Yard", ObjData::Terrain(Box::default()));
        apply_edit(&mut o, TransformEdit::Rotation(Vec3::new(0.3, 1.2, -0.4)));
        assert_eq!(o.rotation(), Vec3::new(0.0, 1.2, 0.0));
        apply_edit(&mut o, TransformEdit::Scale(Vec3::splat(3.0)));
        assert_eq!(o.scale(), Vec3::ONE);
        apply_edit(
            &mut o,
            TransformEdit::Land { position: Vec3::X, rotation: Some(Vec3::new(0.5, 0.1, 0.5)) },
        );
        assert_eq!((o.position(), o.rotation()), (Vec3::X, Vec3::new(0.0, 0.1, 0.0)));
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
        // the sector that held the object alone goes, the
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
            t.send(SceneCmd::RemoveSelection { label: "Delete".into() }),
            Ok(SceneReply::Count(1))
        ));
        let s = t.state();
        assert!(s.find_relaxed("s1").is_none(), "the sector left without a mesh goes");
        let Some(ObjData::Sector(d)) = s.get(s2).map(|o| &o.data) else { panic!("s2 stays") };
        assert_eq!(d.items, vec![(SmolStr::new("b"), SmolStr::new("m"))]);
    }

    #[test]
    fn a_light_and_a_glow_are_deleted_together_and_a_locked_object_stays() {
        let mut s = SceneState::default();
        let l = s.insert(Object::new(ObjClass::Light, "lamp", ObjData::Light(LightData::default())));
        let g = s.insert(Object::new(ObjClass::Glow, "halo", ObjData::Glow(GlowData::default())));
        let mut kept = Object::new(ObjClass::Glow, "kept", ObjData::Glow(GlowData::default()));
        kept.flags.locked = true;
        let k = s.insert(kept);
        s.selection.insert(l);
        s.selection.insert(g);
        s.selection.insert(k);
        let t = T::new(s);
        t.send(SceneCmd::RemoveSelection { label: "Delete".into() }).unwrap();
        let s = t.state();
        assert!(s.get(l).is_none() && s.get(g).is_none(), "both kinds go in one step");
        assert!(s.get(k).is_some(), "a locked object is not changed");
        // one undo step brings both back
        assert_eq!(t.rt.block_on(t.store.undo()).as_deref(), Some("Delete"));
        let s = t.state();
        assert!(s.get(l).is_some() && s.get(g).is_some());
    }

    /// What `scene.set set={"authored": true}` sends: the objects with the mark, one undo step.
    #[test]
    fn the_module_mark_set_on_objects_is_one_undo_step() {
        let mut s = SceneState::default();
        let a = s.insert(Object::new(ObjClass::Light, "a", ObjData::Light(LightData::default())));
        let b = s.insert(Object::new(ObjClass::Glow, "b", ObjData::Glow(GlowData::default())));
        let t = T::new(s);
        let snap = t.state();
        let pcx =
            crate::props::PropCx { lib: None, entity: &gz_entities::PropContext::default(), radio: None };
        let objects: Vec<Object> = [a, b]
            .iter()
            .map(|id| {
                let o = snap.get(*id).unwrap();
                let p = crate::props::Parsed::Bool(true);
                match crate::props::set_object(&snap, o, "authored", &p, &pcx).unwrap() {
                    crate::props::Effect::Object(n) => n,
                    _ => panic!(),
                }
            })
            .collect();
        t.send(SceneCmd::ReplaceObjects { objects, serial: snap.serial, label: "Set Properties".into() })
            .unwrap();
        let s = t.state();
        assert!(s.get(a).unwrap().marks.authored && s.get(b).unwrap().marks.authored);
        assert_eq!(t.rt.block_on(t.store.undo()).as_deref(), Some("Set Properties"));
        let s = t.state();
        assert!(!s.get(a).unwrap().marks.authored && !s.get(b).unwrap().marks.authored);
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
