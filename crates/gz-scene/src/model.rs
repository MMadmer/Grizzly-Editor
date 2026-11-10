//! The scene as one value: what [`crate::SceneService`]'s store holds.

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::{Blob, SmolStr};
use gz_store::imbl::{HashMap, OrdMap, OrdSet};
use serde::Serialize;

use crate::class::{CLASSES, ObjClass};
pub use crate::math::Transform;
use crate::math::{Aabb, Mat4, Vec3};
use crate::objects::{
    FogData, GlowData, GroupData, KeptObject, LightData, ParticlesData, PortalData, SceneObjectData, SectorData, ShapeData,
    SoundEnvData, SoundSrcData, SpawnData, WayData,
};
pub use crate::tools::{BulkData, LevelOptions, ToolPart, ToolSettings};

/// A placed object's identity for as long as the scene is open. Not stored in a file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct ObjId(pub u32);

/// `CCustomObject::m_CO_Flags` bits.
pub mod co {
    pub const MOTION: u32 = 1 << 3;
    pub const RENDER_IF_SELECTED: u32 = 1 << 4;
    pub const IN_GROUP: u32 = 1 << 5;
    pub const IN_GROUP_UNIQUE: u32 = 1 << 6;
    pub const AUTHOR_PLACED: u32 = 1 << 7;
    pub const BASE_REMOVED: u32 = 1 << 8;
}

/// What an object is flagged with: the persisted `co_flags` (the mod marks live in
/// [`ModMarks`]) and the run-time visibility (`flRT_Visible`, not saved).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjFlags {
    pub co: u32,
    pub visible: bool,
}

impl Default for ObjFlags {
    fn default() -> Self {
        Self { co: 0, visible: true }
    }
}

/// The module layer: authored objects ship, removed base spawns become remove ops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ModMarks {
    /// `flAuthorPlaced`.
    pub authored: bool,
    /// `flBaseRemoved`.
    pub removed: bool,
}

/// An object motion (`CUSTOMOBJECT_CHUNK_MOTION`), kept as read: only the binary files carry it.
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
    pub name: SmolStr,
    /// `FPosition`, `FRotation`, `FScale` as stored; [`Object::position`] is `GetPosition`.
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
    /// A body this crate does not understand (a version the C++ refuses): written back as read.
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
            name: SmolStr::new(name.to_lowercase()),
            xform: Transform::default(),
            flags: ObjFlags::default(),
            owner: None,
            marks: ModMarks::default(),
            data,
            motion: None,
        }
    }

    /// `m_CO_Flags` as the files carry it.
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

    /// Splits read `co_flags` into flags and marks.
    pub fn set_co_flags(&mut self, f: u32) {
        self.flags.co = f & !(co::AUTHOR_PLACED | co::BASE_REMOVED);
        self.marks.authored = f & co::AUTHOR_PLACED != 0;
        self.marks.removed = f & co::BASE_REMOVED != 0;
    }

    /// `GetPosition`: a sound source and a way path keep theirs in the class data.
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

    /// `GetScale`: a glow's is its radius.
    #[must_use]
    pub fn scale(&self) -> Vec3 {
        match &self.data {
            ObjData::Glow(g) => Vec3::splat(g.radius),
            _ => self.xform.scale,
        }
    }

    /// `FTransform` with `GetPosition` / `GetRotation` / `GetScale`.
    #[must_use]
    pub fn world(&self) -> Mat4 {
        Transform { position: self.position(), rotation: self.rotation(), scale: self.scale() }.matrix()
    }

    /// `Editable()`: not in a group, or marked editable inside it.
    #[must_use]
    pub fn editable(&self) -> bool {
        let in_group = self.flags.co & co::IN_GROUP != 0;
        !in_group || self.flags.co & co::IN_GROUP_UNIQUE != 0
    }

    /// The reference the class names (`RefName`): library object, particle system, section.
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
    /// The `.level` file, absolute; `None` before the first save.
    pub file: Option<PathBuf>,
    /// `m_GUID`: two `u64`s.
    pub guid: [u64; 2],
    /// `level_tag`.
    pub owner: SmolStr,
    pub create_time: u32,
    /// The camera the file was saved with: heading-pitch-bank and position.
    pub camera_hpb: Vec3,
    pub camera_pos: Vec3,
    pub level: LevelOptions,
    /// Sections of the main file this crate does not read, written back as they were.
    pub kept: Vec<(String, Vec<(String, String)>)>,
}

impl Default for SceneMeta {
    fn default() -> Self {
        Self {
            file: None,
            guid: [0, 0],
            owner: SmolStr::default(),
            create_time: 0,
            camera_hpb: Vec3::ZERO,
            camera_pos: Vec3::ZERO,
            level: LevelOptions::default(),
            kept: Vec::new(),
        }
    }
}

impl SceneMeta {
    /// The scene's name: its file's stem, or empty.
    #[must_use]
    pub fn name(&self) -> String {
        self.file.as_ref().and_then(|f| f.file_stem()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

/// The open level.
#[derive(Clone, Debug, PartialEq)]
pub struct SceneState {
    pub meta: SceneMeta,
    pub objects: OrdMap<ObjId, Arc<Object>>,
    /// Each class's objects in placement order (the ids grow with placement), group members
    /// included - the C++ tool lists.
    pub by_class: [OrdSet<ObjId>; CLASSES],
    pub names: HashMap<(ObjClass, SmolStr), ObjId>,
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

    /// `FindObjectByName(name, classfilter)`: exact name.
    #[must_use]
    pub fn find(&self, class: ObjClass, name: &str) -> Option<&Arc<Object>> {
        self.names.get(&(class, SmolStr::new(name))).and_then(|id| self.objects.get(id))
    }

    /// `FindObjectByName(name, nullptr)`: the first class (in id order) that has the name.
    #[must_use]
    pub fn find_any(&self, name: &str) -> Option<&Arc<Object>> {
        let key = SmolStr::new(name);
        ObjClass::ALL.iter().find_map(|c| self.names.get(&(*c, key.clone())).and_then(|id| self.objects.get(id)))
    }

    /// `FindObjectRelaxed`: names are lower-cased when set, so the lookup is too.
    #[must_use]
    pub fn find_relaxed(&self, name: &str) -> Option<&Arc<Object>> {
        self.find_any(&name.to_lowercase())
    }

    #[must_use]
    pub fn name_used(&self, name: &str) -> bool {
        self.find_any(name).is_some()
    }

    /// `GenObjectName`: `pref`, then `pref_00`, `pref_01` ... (`00`, `01` ... without one); a
    /// name no class uses.
    #[must_use]
    pub fn gen_name(&self, pref: Option<&str>) -> String {
        let pref = pref.map(str::to_lowercase);
        for i in 0u32.. {
            let name = match &pref {
                Some(p) if i == 0 => p.clone(),
                Some(p) => format!("{p}_{:02}", i - 1),
                None => format!("{i:02}"),
            };
            if !self.name_used(&name) {
                return name;
            }
        }
        String::new()
    }

    #[must_use]
    pub fn is_selected(&self, id: ObjId) -> bool {
        self.selection.contains(&id)
    }

    /// `ObjCount(cls)`.
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
        self.names.insert((obj.class, obj.name.clone()), id);
        self.objects.insert(id, Arc::new(obj));
        id
    }

    /// Replaces an object in place, keeping the name index right.
    pub fn replace(&mut self, obj: Object) {
        let id = obj.id;
        if let Some(old) = self.objects.get(&id) {
            if old.name != obj.name {
                self.names.remove(&(old.class, old.name.clone()));
                self.names.insert((obj.class, obj.name.clone()), id);
            }
        }
        self.objects.insert(id, Arc::new(obj));
    }

    /// Removes an object and everything that points at it (selection, snap list, a group's
    /// member list).
    pub fn remove(&mut self, id: ObjId) -> Option<Arc<Object>> {
        let obj = self.objects.remove(&id)?;
        self.by_class[obj.class.index()].remove(&id);
        if self.names.get(&(obj.class, obj.name.clone())) == Some(&id) {
            self.names.remove(&(obj.class, obj.name.clone()));
        }
        self.selection.remove(&id);
        self.tools.snap.retain(|s| *s != id);
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

    /// The ids of the selection in class order, then placement order: how the C++ walks it.
    #[must_use]
    pub fn selected_in_order(&self) -> Vec<ObjId> {
        ObjClass::ALL
            .iter()
            .flat_map(|c| self.by_class[c.index()].iter().copied())
            .filter(|id| self.selection.contains(id))
            .collect()
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
                visible: o.flags.visible && self.tools.part(o.class).visible,
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
    fn names_are_generated_like_the_editor() {
        let mut s = SceneState::default();
        assert_eq!(s.gen_name(Some("Box")), "box");
        s.insert(Object::new(ObjClass::Glow, "box", ObjData::Glow(GlowData::default())));
        assert_eq!(s.gen_name(Some("box")), "box_00");
        assert_eq!(s.gen_name(None), "00");
        assert!(s.find_relaxed("BOX").is_some());
    }

    #[test]
    fn co_flags_split_and_join() {
        let mut o = Object::new(ObjClass::Glow, "g", ObjData::Glow(GlowData::default()));
        o.set_co_flags(co::AUTHOR_PLACED | co::IN_GROUP | 1 << 30);
        assert!(o.marks.authored && !o.marks.removed);
        assert_eq!(o.co_flags(), co::AUTHOR_PLACED | co::IN_GROUP | 1 << 30);
    }
}
