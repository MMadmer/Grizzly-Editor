//! The scene as one value: what [`crate::SceneService`]'s store holds.

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::{Blob, SmolStr};
use gz_store::imbl::{HashMap, OrdMap, OrdSet};
use serde::Serialize;

use crate::class::{CLASSES, Caps, ObjClass};
pub use crate::math::Transform;
use crate::math::{Aabb, Mat4, Vec3};
use crate::objects::{
    FogData, GlowData, GroupData, KeptObject, LightData, ParticlesData, PortalData, SceneObjectData,
    SectorData, ShapeData, SoundEnvData, SoundSrcData, SpawnData, TerrainData, WayData,
};
pub use crate::tools::{BulkData, LevelOptions, SceneMode, ToolPart, ToolSettings};

/// A placed object's identity for as long as the scene is open. Not stored in a file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ObjId(pub u32);

/// An object's flags as bits: what the keys `group_editable`, `draw_selected`, `motion`,
/// `authored` and `removed` of its file say.
pub mod co {
    pub const MOTION: u32 = 1 << 3;
    pub const RENDER_IF_SELECTED: u32 = 1 << 4;
    pub const IN_GROUP: u32 = 1 << 5;
    pub const IN_GROUP_UNIQUE: u32 = 1 << 6;
    pub const AUTHOR_PLACED: u32 = 1 << 7;
    pub const BASE_REMOVED: u32 = 1 << 8;
}

/// What an object is flagged with: its flags (the mod marks live in [`ModMarks`]), its eye and
/// its lock - all saved with the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjFlags {
    pub co: u32,
    /// Drawn and picked in the viewport (the file's `hidden` is its opposite).
    pub visible: bool,
    /// Drawn, but not picked or changed in the viewport.
    pub locked: bool,
}

impl Default for ObjFlags {
    fn default() -> Self {
        Self { co: 0, visible: true, locked: false }
    }
}

/// The module layer: authored objects ship, removed base spawns become remove ops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ModMarks {
    pub authored: bool,
    pub removed: bool,
}

/// An object's motion: the X-Ray SDK's object motion (`.anm` body), kept as read until it is
/// edited, and where its playback stands.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionRaw {
    pub motion: Blob,
    pub t_current: Option<f32>,
}

/// One placed object.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    pub id: ObjId,
    pub class: ObjClass,
    /// Unique in the scene in any case; kept in the case it was given.
    pub name: SmolStr,
    /// Position, rotation (radians) and scale; [`Object::position`] is where it stands.
    pub xform: Transform,
    pub flags: ObjFlags,
    /// The group that holds it.
    pub owner: Option<ObjId>,
    pub marks: ModMarks,
    pub data: ObjData,
    pub motion: Option<Arc<MotionRaw>>,
}

/// The class data, one variant per class.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjData {
    SceneObject(SceneObjectData),
    Light(LightData),
    Glow(GlowData),
    Shape(ShapeData),
    SoundSrc(SoundSrcData),
    SoundEnv(SoundEnvData),
    Spawn(Box<SpawnData>),
    Way(WayData),
    Sector(SectorData),
    Portal(PortalData),
    Group(GroupData),
    Particles(ParticlesData),
    FogVolume(FogData),
    /// A height grid over a rectangle (`crate::objects::terrain`).
    Terrain(Box<TerrainData>),
    /// An object of an X-Ray SDK scene of a version the importer does not read.
    Kept(KeptObject),
}

impl ObjData {
    /// The data with a group's member list emptied: what compares across two loads, whose ids
    /// differ.
    #[must_use]
    pub fn without_members(mut self) -> Self {
        if let ObjData::Group(g) = &mut self {
            g.members.clear();
        }
        self
    }
}

impl Object {
    #[must_use]
    pub fn new(class: ObjClass, name: &str, data: ObjData) -> Self {
        Self {
            id: ObjId(0),
            class,
            name: SmolStr::new(name),
            xform: Transform::default(),
            flags: ObjFlags::default(),
            owner: None,
            marks: ModMarks::default(),
            data,
            motion: None,
        }
    }

    /// The flags with the mod marks.
    #[must_use]
    pub fn co_flags(&self) -> u32 {
        let mut f = self.flags.co & !(co::AUTHOR_PLACED | co::BASE_REMOVED);
        if self.marks.authored {
            f |= co::AUTHOR_PLACED;
        }
        if self.marks.removed {
            f |= co::BASE_REMOVED;
        }
        f
    }

    /// Splits flags into flags and marks.
    pub fn set_co_flags(&mut self, f: u32) {
        self.flags.co = f & !(co::AUTHOR_PLACED | co::BASE_REMOVED);
        self.marks.authored = f & co::AUTHOR_PLACED != 0;
        self.marks.removed = f & co::BASE_REMOVED != 0;
    }

    /// Where it stands: a sound source and a way path keep theirs in the class data.
    #[must_use]
    pub fn position(&self) -> Vec3 {
        match &self.data {
            ObjData::SoundSrc(s) => s.position,
            ObjData::Way(w) => w.points.first().map_or(self.xform.position, |p| p.position),
            _ => self.xform.position,
        }
    }

    #[must_use]
    pub fn rotation(&self) -> Vec3 {
        self.xform.rotation
    }

    /// Its scale: a glow's is its radius.
    #[must_use]
    pub fn scale(&self) -> Vec3 {
        match &self.data {
            ObjData::Glow(g) => Vec3::splat(g.radius),
            _ => self.xform.scale,
        }
    }

    /// Its world matrix.
    #[must_use]
    pub fn world(&self) -> Mat4 {
        Transform { position: self.position(), rotation: self.rotation(), scale: self.scale() }.matrix()
    }

    /// Not in a group, or marked editable inside it.
    #[must_use]
    pub fn editable(&self) -> bool {
        let in_group = self.flags.co & co::IN_GROUP != 0;
        !in_group || self.flags.co & co::IN_GROUP_UNIQUE != 0
    }

    /// The reference the class names: library object, particle effect, section, group.
    #[must_use]
    pub fn ref_name(&self) -> Option<&str> {
        match &self.data {
            ObjData::SceneObject(s) => Some(s.reference.as_str()),
            ObjData::Particles(p) => Some(p.reference.as_str()),
            ObjData::Spawn(s) => s.section(),
            ObjData::Group(g) => Some(g.reference.as_str()),
            _ => None,
        }
        .filter(|s| !s.is_empty())
    }
}

/// What the scene file says about the level beside its objects.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneMeta {
    /// The `.scene` file, absolute; `None` before the first save.
    pub file: Option<PathBuf>,
    /// The X-Ray SDK scene (`.level`) it was converted from, while its `.scene` is not written.
    pub imported: Option<PathBuf>,
    /// The camera the file was saved with: heading-pitch-bank (radians) and position.
    pub camera_hpb: Vec3,
    pub camera_pos: Vec3,
    pub level: LevelOptions,
}

impl Default for SceneMeta {
    fn default() -> Self {
        Self {
            file: None,
            imported: None,
            camera_hpb: Vec3::ZERO,
            camera_pos: Vec3::ZERO,
            level: LevelOptions::default(),
        }
    }
}

impl SceneMeta {
    /// The scene's name: its file's stem, or empty.
    #[must_use]
    pub fn name(&self) -> String {
        self.file
            .as_ref()
            .and_then(|f| f.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

/// The key of the name index: the name in lower case.
fn name_key(name: &str) -> SmolStr {
    SmolStr::new(name.to_lowercase())
}

/// The open level.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneState {
    pub meta: SceneMeta,
    pub objects: OrdMap<ObjId, Arc<Object>>,
    /// Each class's objects in placement order (the ids grow with placement), group members
    /// included.
    pub by_class: [OrdSet<ObjId>; CLASSES],
    /// Every object by its name in lower case: a name is unique in the scene in any case.
    pub names: HashMap<SmolStr, ObjId>,
    pub selection: OrdSet<ObjId>,
    pub tools: ToolSettings,
    pub bulk: BulkData,
    pub next_id: u32,
    /// A scene is open (loaded or created); before that the commands answer `no scene`.
    pub valid: bool,
    /// Moves with every change of the content: a command computed from a snapshot names the
    /// serial it saw and is refused when the scene changed meanwhile.
    pub serial: u64,
}

impl Default for SceneState {
    fn default() -> Self {
        Self {
            meta: SceneMeta::default(),
            objects: OrdMap::new(),
            by_class: std::array::from_fn(|_| OrdSet::new()),
            names: HashMap::new(),
            selection: OrdSet::new(),
            tools: ToolSettings::default(),
            bulk: BulkData::default(),
            next_id: 1,
            valid: true,
            serial: 0,
        }
    }
}

/// What a renderer draws of one object.
#[derive(Clone, Debug)]
pub struct RenderItem {
    pub id: ObjId,
    pub class: ObjClass,
    pub world: Mat4,
    pub bounds: Option<Aabb>,
    pub visible: bool,
    pub selected: bool,
    pub object: Arc<Object>,
}

impl SceneState {
    #[must_use]
    pub fn get(&self, id: ObjId) -> Option<&Arc<Object>> {
        self.objects.get(&id)
    }

    /// The objects of a class in their order.
    pub fn of_class(&self, class: ObjClass) -> impl Iterator<Item = &Arc<Object>> + '_ {
        self.by_class[class.index()].iter().filter_map(|id| self.objects.get(id))
    }

    /// An object of a class by its name, any case.
    #[must_use]
    pub fn find(&self, class: ObjClass, name: &str) -> Option<&Arc<Object>> {
        self.find_any(name).filter(|o| o.class == class)
    }

    /// An object by its name, any case.
    #[must_use]
    pub fn find_any(&self, name: &str) -> Option<&Arc<Object>> {
        self.names.get(&name_key(name)).and_then(|id| self.objects.get(id))
    }

    /// [`SceneState::find_any`].
    #[must_use]
    pub fn find_relaxed(&self, name: &str) -> Option<&Arc<Object>> {
        self.find_any(name)
    }

    #[must_use]
    pub fn name_used(&self, name: &str) -> bool {
        self.find_any(name).is_some()
    }

    /// Whether a new object may take the name: no object has it in any case.
    #[must_use]
    pub fn name_free(&self, name: &str) -> bool {
        !name.is_empty() && !self.name_used(name)
    }

    /// A name no object uses, made from `pref`: `pref` itself when it is free, else
    /// `pref_01`, `pref_02` ... (a counter `pref` ends in already is counted on).
    #[must_use]
    pub fn unique_name(&self, pref: &str) -> String {
        if self.name_free(pref) {
            return pref.to_owned();
        }
        self.counted(pref)
    }

    /// `<stem>_01`, `<stem>_02` ...: the first no object uses.
    fn counted(&self, pref: &str) -> String {
        let stem = match pref.rsplit_once('_') {
            Some((s, n)) if !s.is_empty() && n.len() >= 2 && n.bytes().all(|b| b.is_ascii_digit()) => s,
            _ => pref,
        };
        let stem = if stem.is_empty() { "object" } else { stem };
        (1u32..)
            .map(|i| format!("{stem}_{i:02}"))
            .find(|n| !self.name_used(n))
            .unwrap_or_else(|| stem.to_owned())
    }

    /// The name a new object of a reference takes: the reference's leaf and a counter
    /// (`statics\shkaf\cabinet_village` gives `cabinet_village_01`), the case kept. Without a
    /// reference, `object_01`, ...
    #[must_use]
    pub fn gen_name(&self, pref: Option<&str>) -> String {
        let leaf = pref
            .map(|p| p.rsplit(['\\', '/']).next().unwrap_or(p))
            .map(|l| l.strip_prefix('$').unwrap_or(l))
            .filter(|l| !l.is_empty())
            .unwrap_or("object");
        self.counted(leaf)
    }

    #[must_use]
    pub fn is_selected(&self, id: ObjId) -> bool {
        self.selection.contains(&id)
    }

    #[must_use]
    pub fn count(&self, class: ObjClass) -> usize {
        self.by_class[class.index()].len()
    }

    /// Adds an object with a fresh id (the name is the caller's to make unique).
    pub fn insert(&mut self, mut obj: Object) -> ObjId {
        let id = ObjId(self.next_id);
        self.next_id += 1;
        obj.id = id;
        self.by_class[obj.class.index()].insert(id);
        self.names.insert(name_key(&obj.name), id);
        self.objects.insert(id, Arc::new(obj));
        id
    }

    /// Replaces an object in place, keeping the name index right.
    pub fn replace(&mut self, obj: Object) {
        let id = obj.id;
        if let Some(old) = self.objects.get(&id)
            && old.name != obj.name
        {
            let key = name_key(&old.name);
            if self.names.get(&key) == Some(&id) {
                self.names.remove(&key);
            }
            self.names.insert(name_key(&obj.name), id);
        }
        self.objects.insert(id, Arc::new(obj));
    }

    /// Removes an object and everything that points at it (selection, snap list, a group's
    /// member list).
    pub fn remove(&mut self, id: ObjId) -> Option<Arc<Object>> {
        let obj = self.objects.remove(&id)?;
        self.by_class[obj.class.index()].remove(&id);
        let key = name_key(&obj.name);
        if self.names.get(&key) == Some(&id) {
            self.names.remove(&key);
        }
        self.selection.remove(&id);
        if let Some(owner) = obj.owner
            && let Some(group) = self.objects.get(&owner).cloned()
            && let ObjData::Group(g) = &group.data
        {
            let mut g2 = (*group).clone();
            let mut gd = g.clone();
            gd.members.retain(|m| *m != id);
            g2.data = ObjData::Group(gd);
            self.objects.insert(owner, Arc::new(g2));
        }
        Some(obj)
    }

    /// The ids of the selection in class order, then placement order.
    #[must_use]
    pub fn selected_in_order(&self) -> Vec<ObjId> {
        ObjClass::ALL
            .iter()
            .flat_map(|c| self.by_class[c.index()].iter().copied())
            .filter(|id| self.selection.contains(id))
            .collect()
    }

    /// Whether the viewport may pick an object: it and its kind are shown and not locked.
    #[must_use]
    pub fn pickable(&self, o: &Object) -> bool {
        let part = self.tools.part(o.class);
        o.flags.visible && !o.flags.locked && part.visible && !part.locked
    }

    /// Whether an object is drawn: it and its kind are shown.
    #[must_use]
    pub fn shown(&self, o: &Object) -> bool {
        o.flags.visible && self.tools.part(o.class).visible
    }

    /// Whether an object may be changed: neither it nor its kind is locked.
    #[must_use]
    pub fn changeable(&self, o: &Object) -> bool {
        !o.flags.locked && !self.tools.part(o.class).locked
    }

    /// What the gizmo may do to the selection: what every selected object allows (a locked one
    /// allows nothing); nothing for an empty selection.
    #[must_use]
    pub fn selection_caps(&self) -> Caps {
        let mut caps: Option<Caps> = None;
        for id in &self.selection {
            let Some(o) = self.get(*id) else { continue };
            let c = if self.changeable(o) { o.class.caps() } else { Caps::NONE };
            caps = Some(caps.map_or(c, |a| a.and(c)));
        }
        caps.unwrap_or(Caps::NONE)
    }

    /// Everything a renderer draws, one item per object, with the world boxes the library gives.
    #[must_use]
    pub fn render_items(&self, bounds: &dyn Fn(&Object) -> Option<Aabb>) -> Vec<RenderItem> {
        self.objects
            .values()
            .map(|o| RenderItem {
                id: o.id,
                class: o.class,
                world: o.world(),
                bounds: bounds(o),
                visible: self.shown(o),
                selected: self.selection.contains(&o.id),
                object: o.clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::GlowData;

    #[test]
    fn names_are_the_leaf_of_the_reference_and_a_counter_and_keep_their_case() {
        let mut s = SceneState::default();
        assert_eq!(s.gen_name(Some("statics\\shkaf\\Cabinet_Village")), "Cabinet_Village_01");
        s.insert(Object::new(ObjClass::Glow, "Cabinet_Village_01", ObjData::Glow(GlowData::default())));
        assert_eq!(s.gen_name(Some("statics\\shkaf\\cabinet_village")), "cabinet_village_02");
        assert_eq!(s.gen_name(None), "object_01");
        assert_eq!(s.gen_name(Some("$player_start")), "player_start_01");
        // a name is unique in any case, found in any case and kept in its own
        assert!(s.find_relaxed("cabinet_village_01").is_some());
        assert!(!s.name_free("CABINET_VILLAGE_01"));
        assert_eq!(s.unique_name("Cabinet_Village_01"), "Cabinet_Village_02");
        assert_eq!(s.unique_name("Lamp"), "Lamp");
        assert_eq!(s.find_any("cabinet_village_01").unwrap().name, "Cabinet_Village_01");
    }

    #[test]
    fn hidden_and_locked_things_are_not_picked() {
        let mut s = SceneState::default();
        let a = s.insert(Object::new(ObjClass::Light, "a", ObjData::Glow(GlowData::default())));
        let mut o = (**s.get(a).unwrap()).clone();
        assert!(s.pickable(&o));
        o.flags.locked = true;
        assert!(!s.pickable(&o) && s.shown(&o));
        o.flags.locked = false;
        s.tools.part_mut(ObjClass::Light).locked = true;
        assert!(!s.pickable(&o) && !s.changeable(&o));
        s.tools.part_mut(ObjClass::Light).locked = false;
        s.tools.part_mut(ObjClass::Light).visible = false;
        assert!(!s.pickable(&o) && !s.shown(&o));
    }

    #[test]
    fn co_flags_split_and_join() {
        let mut o = Object::new(ObjClass::Glow, "g", ObjData::Glow(GlowData::default()));
        o.set_co_flags(co::AUTHOR_PLACED | co::IN_GROUP | 1 << 30);
        assert!(o.marks.authored && !o.marks.removed);
        assert_eq!(o.co_flags(), co::AUTHOR_PLACED | co::IN_GROUP | 1 << 30);
    }
}
