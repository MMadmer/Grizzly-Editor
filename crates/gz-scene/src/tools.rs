//! What the scene keeps per kind of object (its eye and lock, its viewport flags, the light
//! controls, the random placement), the editor's mode, the level options and the painted
//! layers' data (the AI map, the detail objects, the wallmarks).

use std::sync::Arc;

use gz_base::{Blob, SmolStr};
use gz_fmt_level::{AiMapNode, SceneAiMap, SceneAiNode, SceneDetails};
use serde::{Deserialize, Serialize};

use crate::SceneError;
use crate::class::{CLASSES, ObjClass};

/// The compiler's build parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct BuildParams {
    pub sm_angle: f32,
    pub weld_distance: f32,
    pub lm_pixels_per_meter: f32,
    pub lm_jitter_samples: u32,
    pub lm_rms_zero: u32,
    pub lm_rms: u32,
    /// Draft 0, high 1, custom 2.
    pub quality: u16,
    pub u_reserved: u16,
    pub f_reserved: [f32; 6],
}

impl Default for BuildParams {
    /// The high quality preset.
    fn default() -> Self {
        Self {
            sm_angle: 75.0,
            weld_distance: 0.005,
            lm_pixels_per_meter: 10.0,
            lm_jitter_samples: 9,
            lm_rms_zero: 4,
            lm_rms: 4,
            quality: 1,
            u_reserved: 0,
            f_reserved: [0.0; 6],
        }
    }
}

/// The level's options.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelOptions {
    pub level_path: SmolStr,
    pub level_prefix: SmolStr,
    /// Lines the build adds to the level's `level.ltx`.
    pub bop: String,
    pub map_version: SmolStr,
    pub light_hemi_quality: u8,
    pub light_sun_quality: u8,
    pub build: BuildParams,
    /// The game types, one bit each.
    pub map_usage: u16,
    /// The size of the level's streaming cells in metres; 0: the level does not stream.
    pub cell_size: u32,
}

impl Default for LevelOptions {
    fn default() -> Self {
        Self {
            level_path: SmolStr::new_static("level"),
            level_prefix: SmolStr::new_static("level_prefix"),
            bop: String::new(),
            map_version: SmolStr::new_static("1.0"),
            light_hemi_quality: 3,
            light_sun_quality: 3,
            build: BuildParams::default(),
            map_usage: 0xffff,
            cell_size: 0,
        }
    }
}

impl LevelOptions {
    /// The quality presets: draft (0) and high (1) set the values; custom (2) keeps them.
    pub fn set_quality(&mut self, q: u16) {
        match q {
            0 => {
                self.build.quality = 0;
                self.build.lm_pixels_per_meter = 0.1;
                self.build.lm_jitter_samples = 1;
                self.light_hemi_quality = 0;
                self.light_sun_quality = 0;
            }
            1 => {
                self.build.quality = 1;
                self.build.lm_pixels_per_meter = 10.0;
                self.build.lm_jitter_samples = 9;
                self.light_hemi_quality = 3;
                self.light_sun_quality = 3;
            }
            _ => self.build.quality = 2,
        }
    }
}

/// The lights' settings: the sun's direction and the light controls.
#[derive(Clone, Debug, PartialEq)]
pub struct LightTool {
    /// Altitude, longitude (radians).
    pub sun_shadow_dir: [f32; 2],
    pub lcontrol_last_idx: u32,
    /// The light controls: name and id, in order.
    pub lcontrols: Vec<(SmolStr, u32)>,
}

impl Default for LightTool {
    fn default() -> Self {
        Self {
            sun_shadow_dir: [(-25.0f32).to_radians(), 292.0f32.to_radians()],
            lcontrol_last_idx: 3,
            lcontrols: vec![
                (SmolStr::new_static("$static"), 0),
                (SmolStr::new_static("$hemi"), 1),
                (SmolStr::new_static("$sun"), 2),
            ],
        }
    }
}

impl LightTool {
    /// Adds a control; a name already there is kept as it is.
    pub fn append(&mut self, name: &str, idx: Option<u32>) {
        let name = name.trim();
        if name.is_empty() || self.lcontrols.iter().any(|(n, _)| n == name) {
            return;
        }
        let id = idx.unwrap_or_else(|| {
            self.lcontrol_last_idx += 1;
            self.lcontrol_last_idx - 1
        });
        self.lcontrols.push((SmolStr::new(name), id));
    }

    /// The name of a control id.
    #[must_use]
    pub fn name_of(&self, id: u32) -> Option<&str> {
        self.lcontrols.iter().find(|(_, i)| *i == id).map(|(n, _)| n.as_str())
    }
}

/// What placing a library object picks at random.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectTool {
    pub min_scale: glam::Vec3,
    pub max_scale: glam::Vec3,
    pub min_rotation: glam::Vec3,
    pub max_rotation: glam::Vec3,
    pub objects: Vec<SmolStr>,
}

impl Default for ObjectTool {
    fn default() -> Self {
        Self {
            min_scale: glam::Vec3::ONE,
            max_scale: glam::Vec3::ONE,
            min_rotation: glam::Vec3::ZERO,
            max_rotation: glam::Vec3::ZERO,
            objects: Vec::new(),
        }
    }
}

/// Random placement is on.
pub const FL_APPEND_RANDOM: u32 = 1 << 29;
/// One random factor for the three axes.
pub const FL_APPEND_RANDOM_PROPORTIONAL: u32 = 1 << 28;
pub const FL_APPEND_RANDOM_SCALE: u32 = 1 << 30;
pub const FL_APPEND_RANDOM_ROTATION: u32 = 1 << 31;
/// Every bit of the random placement in the objects' flags.
pub const FL_APPEND_RANDOM_ALL: u32 =
    FL_APPEND_RANDOM | FL_APPEND_RANDOM_PROPORTIONAL | FL_APPEND_RANDOM_SCALE | FL_APPEND_RANDOM_ROTATION;

/// The class-specific settings of a kind.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum ToolOpts {
    #[default]
    Plain,
    Light(LightTool),
    Objects(ObjectTool),
}

/// One kind's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolPart {
    pub class: ObjClass,
    /// The kind's eye: its objects are drawn and picked.
    pub visible: bool,
    /// The kind's lock: its objects are not picked or changed.
    pub locked: bool,
    /// What the viewport draws of the kind (names, crosses, solid sectors) and, for objects,
    /// the random placement's switches.
    pub flags: u32,
    pub opts: ToolOpts,
}

impl ToolPart {
    #[must_use]
    pub fn new(class: ObjClass) -> Self {
        let opts = match class {
            ObjClass::Light => ToolOpts::Light(LightTool::default()),
            ObjClass::SceneObject => ToolOpts::Objects(ObjectTool::default()),
            _ => ToolOpts::Plain,
        };
        Self { class, visible: true, locked: false, flags: 0, opts }
    }

    /// Whether the kind's objects may be changed.
    #[must_use]
    pub fn editable(&self) -> bool {
        !self.locked
    }

    #[must_use]
    pub fn light(&self) -> Option<&LightTool> {
        match &self.opts {
            ToolOpts::Light(l) => Some(l),
            _ => None,
        }
    }

    pub fn light_mut(&mut self) -> Option<&mut LightTool> {
        match &mut self.opts {
            ToolOpts::Light(l) => Some(l),
            _ => None,
        }
    }
}

/// What the viewport's clicks do: select (and move what is selected), place what the Place
/// panel armed, paint one of the painted layers, edit the visibility cells. A mode owns the
/// clicks while it is on; it never decides what may be selected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SceneMode {
    #[default]
    Select,
    Place,
    /// Paint the AI map's nodes.
    AiMap,
    /// Paint the detail objects (foliage).
    Details,
    /// Lay wallmarks.
    Wallmarks,
    /// Draw sectors and portals by hand.
    Zones,
}

impl SceneMode {
    /// The painted layer the mode paints.
    #[must_use]
    pub fn layer(self) -> Option<ObjClass> {
        match self {
            Self::AiMap => Some(ObjClass::AiMap),
            Self::Details => Some(ObjClass::DetailObjects),
            Self::Wallmarks => Some(ObjClass::Wallmark),
            _ => None,
        }
    }

    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Select => "select",
            Self::Place => "place",
            Self::AiMap => "ai_map",
            Self::Details => "details",
            Self::Wallmarks => "wallmarks",
            Self::Zones => "zones",
        }
    }
}

/// Every kind's settings and the mode.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSettings {
    pub parts: [ToolPart; CLASSES],
    /// What the viewport's clicks do (view state, not saved).
    pub mode: SceneMode,
    /// The kind of thing a click of Place mode places (and whose settings the Place panel
    /// shows). Nothing else reads it: selection, deletion and the gizmo take any kind.
    pub placing: ObjClass,
    /// The switches of the sector, way, shape and fog volume panels (not saved).
    pub obj: crate::objtools::ObjToolState,
    /// The switches of the AI map panel and the detail slots' selection (not saved).
    pub bulk: crate::bulk::BulkToolState,
    /// The compiler's error list the viewport draws (not saved).
    pub debug: Arc<crate::debug::DebugDraw>,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            parts: ObjClass::ALL.map(ToolPart::new),
            mode: SceneMode::default(),
            placing: ObjClass::SceneObject,
            obj: crate::objtools::ObjToolState::default(),
            bulk: crate::bulk::BulkToolState::default(),
            debug: Arc::new(crate::debug::DebugDraw::default()),
        }
    }
}

impl ToolSettings {
    /// The kind the mode's clicks work with: a painting mode's layer, the cells in Zones, else
    /// the kind Place places.
    #[must_use]
    pub fn mode_kind(&self) -> ObjClass {
        self.mode.layer().unwrap_or(match self.mode {
            SceneMode::Zones => ObjClass::Sector,
            _ => self.placing,
        })
    }

    #[must_use]
    pub fn part(&self, class: ObjClass) -> &ToolPart {
        &self.parts[class.index()]
    }

    pub fn part_mut(&mut self, class: ObjClass) -> &mut ToolPart {
        &mut self.parts[class.index()]
    }
}

/// Nodes per tile of the AI map: an edit replaces one tile.
pub const AI_TILE: usize = 4096;

/// The AI map's data: the nodes as the editor works on them, and packed as the layer holds
/// them, both in tiles of [`AI_TILE`] that an edit shares where it left them alone.
#[derive(Clone, Debug, PartialEq)]
pub struct AiMapPart {
    /// The data as read; written back while nothing changed.
    pub raw: Option<Blob>,
    /// Everything but the nodes.
    pub header: SceneAiMap,
    /// The nodes packed over `header.bbox`.
    pub tiles: Vec<Arc<[SceneAiNode]>>,
    /// The nodes: exact positions and planes, links by index.
    pub nodes: Vec<Arc<[AiMapNode]>>,
}

impl AiMapPart {
    pub fn read(bytes: &[u8]) -> Result<Self, SceneError> {
        let mut map = SceneAiMap::read(bytes).map_err(|e| SceneError::load(e.to_string()))?;
        let (decoded, _) = map.decode_nodes();
        let nodes = std::mem::replace(&mut map.nodes, Arc::from([]));
        let tiles = nodes.chunks(AI_TILE).map(Arc::from).collect();
        let exact = decoded.chunks(AI_TILE).map(Arc::from).collect();
        Ok(Self { raw: Some(Blob::from(bytes)), header: map, tiles, nodes: exact })
    }

    /// The part of a set of nodes: a tile equal to `base`'s is `base`'s, and so is its packed
    /// tile while the box and the parameters stay.
    #[must_use]
    pub fn from_nodes(mut header: SceneAiMap, nodes: &[AiMapNode], base: Option<&AiMapPart>) -> Self {
        header.nodes = Arc::from([]);
        let same_packing =
            base.is_some_and(|b| b.header.bbox == header.bbox && b.header.params == header.params);
        let pack = |chunk: &[AiMapNode]| -> Arc<[SceneAiNode]> {
            chunk.iter().map(|n| crate::bulk::aimap::encode(n, &header.bbox, &header.params)).collect()
        };
        let mut exact = Vec::with_capacity(nodes.len().div_ceil(AI_TILE));
        let mut packed = Vec::with_capacity(exact.capacity());
        for (t, chunk) in nodes.chunks(AI_TILE).enumerate() {
            let old = base.and_then(|b| {
                b.nodes.get(t).filter(|o| ***o == *chunk).map(|o| (o.clone(), b.tiles.get(t).cloned()))
            });
            match old {
                Some((e, Some(p))) if same_packing => {
                    exact.push(e);
                    packed.push(p);
                }
                Some((e, _)) => {
                    packed.push(pack(chunk));
                    exact.push(e);
                }
                None => {
                    packed.push(pack(chunk));
                    exact.push(Arc::from(chunk));
                }
            }
        }
        Self { raw: None, header, tiles: packed, nodes: exact }
    }

    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.iter().map(|t| t.len()).sum()
    }

    /// The nodes in order.
    #[must_use]
    pub fn all_nodes(&self) -> Vec<AiMapNode> {
        let mut out = Vec::with_capacity(self.node_count());
        for t in &self.nodes {
            out.extend_from_slice(t);
        }
        out
    }

    /// The layer's data: the bytes as read, or the model written again.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        if let Some(raw) = &self.raw {
            return raw.to_vec();
        }
        let mut map = self.header.clone();
        map.nodes = self.tiles.iter().flat_map(|t| t.iter().copied()).collect::<Vec<_>>().into();
        map.write()
    }
}

/// The detail objects' data.
#[derive(Clone, Debug, PartialEq)]
pub struct DetailsPart {
    pub raw: Option<Blob>,
    pub model: Arc<SceneDetails>,
}

impl DetailsPart {
    pub fn read(bytes: &[u8]) -> Result<Self, SceneError> {
        let model = SceneDetails::read(bytes).map_err(|e| SceneError::load(e.to_string()))?;
        Ok(Self { raw: Some(Blob::from(bytes)), model: Arc::new(model) })
    }

    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        self.raw.as_ref().map_or_else(|| self.model.write(), |r| r.to_vec())
    }
}

/// The wallmarks' data: the bytes as written, the marks in them, and the model the editor edits
/// (`None` for data this crate cannot read: kept as it is, never edited).
#[derive(Clone, Debug, PartialEq)]
pub struct WallmarkPart {
    pub raw: Blob,
    /// Wallmarks in it.
    pub count: usize,
    pub model: Option<Arc<crate::bulk::wallmark::Wallmarks>>,
}

impl WallmarkPart {
    pub fn read(bytes: &[u8]) -> Result<Self, SceneError> {
        let model = crate::bulk::wallmark::Wallmarks::read(bytes)?;
        Ok(Self { raw: Blob::from(bytes), count: model.count(), model: Some(Arc::new(model)) })
    }

    /// The part of an edited model: its data written again.
    #[must_use]
    pub fn of_model(model: crate::bulk::wallmark::Wallmarks) -> Self {
        Self { raw: Blob::from(model.write()), count: model.count(), model: Some(Arc::new(model)) }
    }

    /// What empty wallmarks write.
    #[must_use]
    pub fn empty_stream() -> Vec<u8> {
        crate::bulk::wallmark::Wallmarks::default().write()
    }
}

/// AI map, details and wallmarks.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BulkData {
    pub ai_map: Option<Arc<AiMapPart>>,
    pub details: Option<Arc<DetailsPart>>,
    pub wallmarks: Option<Arc<WallmarkPart>>,
}

impl BulkData {
    /// A layer's data; a layer without data writes its empty state.
    #[must_use]
    pub fn stream_of(&self, class: ObjClass) -> Vec<u8> {
        match class {
            ObjClass::AiMap => {
                self.ai_map.as_ref().map_or_else(|| SceneAiMap::default().write(), |a| a.write())
            }
            ObjClass::DetailObjects => {
                self.details.as_ref().map_or_else(|| SceneDetails::default().write(), |d| d.write())
            }
            ObjClass::Wallmark => {
                self.wallmarks.as_ref().map_or_else(WallmarkPart::empty_stream, |w| w.raw.to_vec())
            }
            _ => Vec::new(),
        }
    }

    /// Whether a layer holds anything to keep: nodes or a snap list; slots, objects, a base
    /// texture or a snap list; any wallmark.
    #[must_use]
    pub fn needs_save(&self, class: ObjClass) -> bool {
        match class {
            ObjClass::AiMap => {
                self.ai_map.as_ref().is_some_and(|a| a.node_count() > 0 || !a.header.snap_objects.is_empty())
            }
            ObjClass::DetailObjects => self.details.as_ref().is_some_and(|d| {
                let m = &d.model;
                !m.slots.is_empty()
                    || !m.indices.objects.is_empty()
                    || m.base_texture.as_ref().is_some_and(|t| !t.is_empty())
                    || !m.snap_objects.is_empty()
            }),
            ObjClass::Wallmark => self
                .wallmarks
                .as_ref()
                .is_some_and(|w| w.model.as_ref().map_or(w.count > 0, |m| !m.slots.is_empty())),
            _ => false,
        }
    }

    /// Takes a layer's data; data that does not decode is kept as it is.
    pub fn set_stream(&mut self, class: ObjClass, bytes: &[u8]) -> Option<String> {
        match class {
            ObjClass::AiMap => match AiMapPart::read(bytes) {
                Ok(a) => self.ai_map = Some(Arc::new(a)),
                Err(e) => return Some(format!("the AI map is kept as it was read: {e}")),
            },
            ObjClass::DetailObjects => match DetailsPart::read(bytes) {
                Ok(d) => self.details = Some(Arc::new(d)),
                Err(e) => return Some(format!("the detail objects are kept as they were read: {e}")),
            },
            ObjClass::Wallmark => match WallmarkPart::read(bytes) {
                Ok(w) => self.wallmarks = Some(Arc::new(w)),
                Err(e) => {
                    self.wallmarks =
                        Some(Arc::new(WallmarkPart { raw: Blob::from(bytes), count: 0, model: None }));
                    return Some(format!("the wallmarks are kept as they were read: {e}"));
                }
            },
            _ => {}
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_wallmarks_read_back() {
        let w = WallmarkPart::read(&WallmarkPart::empty_stream()).unwrap();
        assert_eq!(w.count, 0);
    }

    #[test]
    fn a_light_control_name_is_kept_once() {
        let mut l = LightTool::default();
        l.append("my_control", None);
        l.append("my_control", None);
        assert_eq!(l.name_of(3), Some("my_control"));
        assert_eq!(l.lcontrols.len(), 4);
    }
}
