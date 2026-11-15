//! The first-person picture of an item: the hands and the item's HUD model as the player sees
//! them (docs/UI_DESIGN.md, section 6, "Items": In the Hands). This module draws no word of its
//! own: what the picture lacks is a [`Missing`], and the surface that shows it says so.
//!
//! Where everything goes is the game's own math, ported line by line from the game's
//! first-person code: [`measures`] reads a HUD section's offsets as the game reads them,
//! [`hud_motion`] one of its motion lines, [`motion_key`] says which line the game plays,
//! [`place`] hangs the hands on the camera and the item on the hands, [`timing`] picks the cycles
//! each model plays, [`hud_fov`] is the HUD's field of view. The camera is the game's: at the
//! origin of its own space, looking along +Z, with the HUD's projection and near plane.
//!
//! The models and every motion set are read and parsed on the pools ([`Rigs`], kept while the
//! project is open); a pose, its skinning and the frame's packet are made on the CPU pool; a
//! frame only looks at what has arrived and asks for a picture when what it shows changed.

use std::collections::BTreeMap;
use std::f32::consts::PI;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use egui::{CornerRadius, Rect, Ui};
use glam::{Mat4, Vec3};
use gz_app::WindowLink;
use gz_editor::Editor;
use gz_fmt_object::xmath::Mat4 as XMat4;
use gz_fmt_ogf::{MotionSet, Skeleton, Visual, read_omf};
use gz_render::{
    Camera, Color, DrawList, FramePacket, Instance, InstanceFlags, MaterialTable, MeshData, MeshHandle,
    Projection, Renderer, View, ViewFlags, ViewMode, build_packet,
};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::image_pass::{ImageDraw, ImageMode, paint_image};
use crate::loader::{Found, Sources};
use crate::pending::Pending;

/// The console's `hud_fov` as the game ships it.
pub const HUD_FOV_DEF: f32 = 0.375;
/// The world camera's field of view as the game ships it (the console's `fov`).
pub const WORLD_FOV: f32 = 67.5;
/// The hands of a player who wears no suit: the player HUD section the game loads by default.
pub const DEFAULT_HANDS: &str = "actor_hud_05";
/// The near plane of the game's HUD projection.
const HUD_NEAR: f32 = 0.005;
/// The HUD draws nothing farther than an arm's reach; the far plane only bounds the depth.
const HUD_FAR: f32 = 20.0;
/// The game's base screen, 1024 by 768: a screen wider than it (and a hundredth) is wide.
const UI_BASE_ASPECT: f32 = 1024.0 / 768.0;
/// The renderer's names of the two models on show: a new one replaces it.
const HANDS_MESH: &str = "gz-ui-previews\\hands";
const ITEM_MESH: &str = "gz-ui-previews\\hands_item";
/// How many models the cache keeps.
const RIGS_KEPT: usize = 8;

/// The keys of a section as the game holds them, by name.
pub type Keys = BTreeMap<String, String>;

/// What the picture shows: the item's HUD section as it is edited and the hands that hold it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudSpec {
    /// The HUD section's name.
    pub section: String,
    /// Its keys as the game holds them, the form's edits in.
    pub keys: Keys,
    /// The hands' model: the player HUD section's `visual`.
    pub hands: String,
    /// The hands' attach bones (`ancor_*` of the player HUD section, in its order).
    pub anchors: Vec<String>,
    /// More motion files of the hands model (`[da_hud_animations]` of its file's stem).
    pub hand_motions: Vec<String>,
    /// The HUD's vertical field of view, degrees ([`hud_fov`]).
    pub fov: f32,
    /// The HUD row the author is on: a key of the section.
    pub row: Option<String>,
    /// The item model's bones the game hides ([`hidden_addons`]).
    pub hidden: Vec<String>,
}

impl HudSpec {
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.keys.get(key).map(String::as_str)
    }

    /// The item's first-person model, as the game finds it: `item_visual` hangs on the hands,
    /// `visual` is a model with the hands in it (true).
    #[must_use]
    pub fn item_model(&self) -> Option<(String, bool)> {
        let clean = |v: &str| v.trim().trim_matches('"').to_owned();
        if let Some(v) = self.get("item_visual").map(clean).filter(|v| !v.is_empty()) {
            return Some((v, false));
        }
        self.get("visual").map(clean).filter(|v| !v.is_empty()).map(|v| (v, true))
    }

    /// `attach_place_idx` (0 the right hand, 1 the left).
    #[must_use]
    pub fn place(&self) -> u16 {
        self.get("attach_place_idx")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .map_or(0, |v| v.max(0.0) as u16)
    }

    /// Which of the hands' offsets the row the author is on shows: 0 at the hip, 1 aimed, 2
    /// aimed with the grenade launcher.
    #[must_use]
    pub fn aim(&self) -> usize {
        let Some(row) = self.row.as_deref() else { return 0 };
        if row.starts_with("aim_hud_offset") || (row.starts_with("zoom_") && !row.starts_with("zoom_hide")) {
            1
        } else if row.starts_with("gl_hud_offset") || row.starts_with("grenade_zoom") {
            2
        } else {
            0
        }
    }
}

/// Three numbers, as the game reads a vector; what is not there is 0.
fn vec3_of(text: Option<&str>) -> Vec3 {
    let mut v = [0.0f32; 3];
    for (slot, part) in v.iter_mut().zip(text.unwrap_or("").split(',')) {
        *slot = part.trim().parse().unwrap_or(0.0);
    }
    Vec3::from(v)
}

fn float_of(text: Option<&str>) -> Option<f32> {
    text.and_then(|t| t.trim().trim_matches('"').parse::<f32>().ok()).filter(|v| v.is_finite())
}

/// The HUD's vertical field of view in degrees, at full health and with nothing in front of the
/// camera, as the game frames its first-person view: a hundred times the HUD's share (the world's
/// field of view cancels out), the share being the console's `hud_fov` for anything but a
/// weapon, and for a weapon that share plus its `hud_fov_addition_modifier`, or its
/// `hud_fov_absolute` over the world's field of view, times its `hud_fov_factor` (`item`: a key
/// of the item's section, `hud`: of its HUD section, which wins when it has the factor).
/// `aimed`: the zoom's factor, as an aimed weapon has it.
#[must_use]
pub fn hud_fov(
    weapon: bool,
    aimed: bool,
    item: &dyn Fn(&str) -> Option<String>,
    hud: &dyn Fn(&str) -> Option<String>,
) -> f32 {
    if !weapon {
        return HUD_FOV_DEF * 100.0;
    }
    let add_mod = float_of(item("hud_fov_addition_modifier").as_deref()).unwrap_or(0.0);
    // the multipliers live in the HUD section when it has them, else in the item's
    let from = |k: &str| if hud("hud_fov_factor").is_some() { hud(k) } else { item(k) };
    let factor = float_of(from("hud_fov_factor").as_deref()).unwrap_or(1.0);
    let zoom_factor = float_of(from("hud_fov_zoom_factor").as_deref()).unwrap_or(factor);
    let absolute = float_of(from("hud_fov_absolute").as_deref()).unwrap_or(0.0);
    // the pull towards a wall in front, at its far end, is the base share itself
    let last = if absolute > 0.0 { absolute / WORLD_FOV.max(1.0) } else { (HUD_FOV_DEF + add_mod).max(0.0) };
    let k = if aimed { zoom_factor } else { factor };
    last * k * 100.0
}

/// The addons' bones of a weapon's HUD model the game hides for the weapon as it is found, no
/// addon on it: a scope, a silencer or a launcher whose status (`scope_status`,
/// `silencer_status`, `grenade_launcher_status` of the item's section) is not permanent (1) is
/// hidden; the launcher's bone goes by its Call of Pripyat name or its Shadow of Chernobyl one,
/// whichever the model has (both are listed).
#[must_use]
pub fn hidden_addons(weapon: bool, item: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    if !weapon {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (key, bones) in [
        ("scope_status", &["wpn_scope"][..]),
        ("silencer_status", &["wpn_silencer"][..]),
        ("grenade_launcher_status", &["wpn_launcher", "wpn_grenade_launcher"][..]),
    ] {
        let permanent = float_of(item(key).as_deref()).is_some_and(|v| v.round() as i32 == 1);
        if !permanent {
            out.extend(bones.iter().map(|b| (*b).to_owned()));
        }
    }
    out
}

/// Whether a picture of this size is a wide screen to the game: the HUD sections' `_16x9`
/// forms are read then.
#[must_use]
pub fn is_wide(size: [u32; 2]) -> bool {
    size[0] as f32 / size[1].max(1) as f32 > UI_BASE_ASPECT + 0.01
}

/// What a HUD section says of where the hands and the item hang, and of the aiming offsets.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measures {
    /// The hands: the position and the orientation in degrees.
    pub hands: [Vec3; 2],
    /// The item on the hands' attach bone, the same way.
    pub item: [Vec3; 2],
    /// The aiming offsets: `[position | rotation][0 hip, 1 aimed, 2 aimed with the launcher]`;
    /// the rotation is in radians.
    pub offset: [[Vec3; 3]; 2],
}

/// The offsets of a HUD section as the game reads them: an item that hangs on the hands reads
/// the hands' position and orientation (their `_16x9` forms on a wide screen), its own, and the
/// aiming offsets; a model with the hands in it reads its `position` and `orientation`, and a
/// weapon's zoom keys as its aiming offsets.
#[must_use]
pub fn measures(spec: &HudSpec, monolithic: bool, wide: bool) -> Measures {
    let v = |k: &str| vec3_of(spec.get(k));
    let f = |k: &str| float_of(spec.get(k)).unwrap_or(0.0);
    let mut m = Measures::default();
    if monolithic {
        m.item = [v("position"), v("orientation")];
        for (idx, prefix) in [(1, ""), (2, "grenade_")] {
            if spec.get(&format!("{prefix}zoom_offset")).is_some() {
                m.offset[0][idx] = v(&format!("{prefix}zoom_offset"));
                m.offset[1][idx] = Vec3::new(
                    f(&format!("{prefix}zoom_rotate_x")),
                    f(&format!("{prefix}zoom_rotate_y")),
                    f(&format!("{prefix}zoom_rotate_z")),
                );
            }
        }
        return m;
    }
    let p = if wide { "_16x9" } else { "" };
    m.hands = [v(&format!("hands_position{p}")), v(&format!("hands_orientation{p}"))];
    m.item = [v("item_position"), v("item_orientation")];
    m.offset[0][1] = v(&format!("aim_hud_offset_pos{p}"));
    m.offset[1][1] = v(&format!("aim_hud_offset_rot{p}"));
    m.offset[0][2] = v(&format!("gl_hud_offset_pos{p}"));
    m.offset[1][2] = v(&format!("gl_hud_offset_rot{p}"));
    m
}

/// An offset of a HUD section as a transform: a heading, pitch and bank in degrees (the game's
/// order of the three) and a place.
#[must_use]
pub fn attach(pos: Vec3, rot_deg: Vec3) -> XMat4 {
    let r = rot_deg * (PI / 180.0);
    let mut m = XMat4::hpb(r.x, r.y, r.z);
    m.set_c(pos);
    m
}

/// A turn about X, as the game's math builds one.
fn rotate_x(a: f32) -> XMat4 {
    let (s, c) = a.sin_cos();
    XMat4([[1.0, 0.0, 0.0, 0.0], [0.0, c, s, 0.0], [0.0, -s, c, 0.0], [0.0, 0.0, 0.0, 1.0]])
}

/// A turn about Y.
fn rotate_y(a: f32) -> XMat4 {
    let (s, c) = a.sin_cos();
    XMat4([[c, 0.0, -s, 0.0], [0.0, 1.0, 0.0, 0.0], [s, 0.0, c, 0.0], [0.0, 0.0, 0.0, 1.0]])
}

/// A turn about Z.
fn rotate_z(a: f32) -> XMat4 {
    let (s, c) = a.sin_cos();
    XMat4([[c, s, 0.0, 0.0], [-s, c, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]])
}

/// What aiming turns and moves the camera's frame by once the weapon is aimed all the way: the
/// turn about X, then Y, then Z, each applied after the last, the offset as its place.
#[must_use]
pub fn aimed(pos: Vec3, rot: Vec3) -> XMat4 {
    let m = XMat4::mul_43(&rotate_y(rot.y), &rotate_x(rot.x));
    let mut m = XMat4::mul_43(&rotate_z(rot.z), &m);
    m.set_c(pos);
    m
}

/// One `anm_*` line as the game reads it: the hands' motion, the item's own (the first when it
/// names none) and the speed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudMotion {
    pub key: String,
    pub base: String,
    pub additional: String,
    pub speed: f32,
}

#[must_use]
pub fn hud_motion(key: &str, value: &str) -> Option<HudMotion> {
    let items: Vec<&str> = value.split(',').map(str::trim).collect();
    let base = items.first().copied().filter(|b| !b.is_empty())?.to_owned();
    let additional =
        items.get(1).copied().filter(|a| !a.is_empty()).map_or_else(|| base.clone(), str::to_owned);
    let speed = items.get(2).and_then(|s| s.parse::<f32>().ok()).filter(|s| s.is_finite()).unwrap_or(1.0);
    Some(HudMotion { key: key.to_owned(), base, additional, speed })
}

/// The line of the HUD section the picture plays: the row the author is on when it is a motion,
/// else the idle the game plays there (aimed: `anm_idle_aim`, with the launcher
/// `anm_idle_g_aim`; `anm_idle`, any idle, any motion). An item of the left hand on a wide screen
/// plays the line's `_16x9` form when the section has it.
#[must_use]
pub fn motion_key(spec: &HudSpec, wide: bool) -> Option<String> {
    let has = |k: &str| spec.keys.contains_key(k);
    let row = spec.row.as_deref().filter(|r| r.starts_with("anm_") && has(r)).map(str::to_owned);
    let idle = || {
        let aimed = match spec.aim() {
            1 => Some("anm_idle_aim"),
            2 => Some("anm_idle_g_aim"),
            _ => None,
        };
        aimed
            .filter(|k| has(k))
            .map(str::to_owned)
            .or_else(|| has("anm_idle").then(|| "anm_idle".to_owned()))
            .or_else(|| spec.keys.keys().find(|k| k.starts_with("anm_idle")).cloned())
            .or_else(|| spec.keys.keys().find(|k| k.starts_with("anm_")).cloned())
    };
    let key = row.or_else(idle)?;
    if spec.place() == 1 && wide && !key.ends_with("_16x9") {
        let w = format!("{key}_16x9");
        if has(&w) {
            return Some(w);
        }
    }
    Some(key)
}

// ---- the models ----------------------------------------------------------------------------------

/// A cycle of a rig: which motion set, which motion, at what speed and how long.
#[derive(Clone, Debug, PartialEq)]
pub struct Cycle {
    pub name: String,
    set: usize,
    motion: usize,
    /// The speed its definition plays it at.
    pub speed: f32,
    /// The motion's own length, seconds.
    pub seconds: f32,
}

/// A first-person model, read: its mesh, its skeleton and every motion set in the order the game
/// loads them (the model's motion references, else its own motions, then the hands' extra
/// sets).
pub struct Rig {
    pub name: String,
    /// Why it cannot be shown; empty when it can.
    pub error: String,
    pub mesh: Option<MeshData>,
    pub skeleton: Skeleton,
    sets: Vec<(MotionSet, Vec<Option<usize>>)>,
    pub sources: Option<Sources>,
}

impl Rig {
    fn failed(name: &str, error: &str) -> Self {
        Self {
            name: name.to_owned(),
            error: error.to_owned(),
            mesh: None,
            skeleton: Skeleton::default(),
            sets: Vec::new(),
            sources: None,
        }
    }

    #[must_use]
    pub fn shown(&self) -> bool {
        self.error.is_empty() && self.mesh.is_some()
    }

    /// How many motion sets it plays from.
    #[must_use]
    pub fn sets(&self) -> usize {
        self.sets.len()
    }

    /// A cycle by its name, as the game finds one: in the last motion set that has it.
    #[must_use]
    pub fn cycle(&self, name: &str) -> Option<Cycle> {
        self.sets.iter().enumerate().rev().find_map(|(si, (set, _))| {
            set.cycle(name).map(|(def, at)| Cycle {
                name: name.to_owned(),
                set: si,
                motion: at,
                speed: def.played_speed(),
                seconds: set.motions.get(at).map_or(0.0, gz_fmt_ogf::Motion::seconds),
            })
        })
    }

    /// Every bone in model space and the skinning matrices, playing `cycle` at `t` seconds of its
    /// own time (at rest without one). `fixed_root`: the root bone stays at the model's origin,
    /// as the game holds the root of an item that hangs on the hands. A bone of `hidden` and every
    /// bone under it takes the zeroed transform the game gives a hidden bone: what hangs on it is
    /// drawn nowhere.
    #[must_use]
    pub fn pose(
        &self,
        cycle: Option<&Cycle>,
        t: f32,
        fixed_root: bool,
        hidden: &[String],
    ) -> (Vec<XMat4>, Vec<XMat4>) {
        let s = &self.skeleton;
        let mut locals = match cycle.and_then(|c| self.sets.get(c.set).map(|(set, slots)| (c, set, slots))) {
            Some((c, set, slots)) => s.locals(set, slots, c.motion, t),
            None => s.rest.clone(),
        };
        if fixed_root && let Some(root) = s.root().and_then(|r| locals.get_mut(r)) {
            *root = XMat4::IDENTITY;
        }
        let model = s.model(&locals);
        let mut skin = s.skinning(&model);
        let gone: Vec<usize> = hidden.iter().filter_map(|b| s.bone(b)).collect();
        if !gone.is_empty() {
            for (i, m) in skin.iter_mut().enumerate() {
                let mut at = Some(i);
                let mut steps = 0;
                while let Some(b) = at {
                    if gone.contains(&b) {
                        *m = XMat4([[0.0; 4]; 4]);
                        break;
                    }
                    steps += 1;
                    at = s.parent.get(b).copied().flatten().filter(|_| steps <= s.names.len());
                }
            }
        }
        (model, skin)
    }
}

/// A model of the HUD by its name: the project's file, else the linked game's, else the
/// editor's library.
fn find_model(s: &Sources, name: &str) -> Option<Found> {
    let local = s.find_visual(name);
    if matches!(local, Some(Found::Project(_))) {
        return local;
    }
    s.find_game_visual(name).or(local)
}

/// The files of one motion reference: `<name>.omf`, or every `.omf` below a folder for a mask
/// (`dir\*.omf`, in the order of their names, as the game lists them).
fn motion_files(s: &Sources, name: &str, game_first: bool, out: &mut Vec<Found>) {
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    if name.contains('*') {
        let dir = name.rfind(['\\', '/']).map_or("", |at| &name[..at]);
        let Some(files) = &s.game else { return };
        let mut keys: Vec<String> = files
            .find(&format!("meshes/{}", gz_vfs::normalize(dir)), "omf")
            .iter()
            .map(|f| f.path.to_owned())
            .collect();
        keys.sort();
        keys.dedup();
        out.extend(keys.into_iter().map(|key| Found::Game { key, loose: None }));
        return;
    }
    if let Some(f) = s.find_motions(name, game_first) {
        out.push(f);
    }
}

/// Reads a rig: the file and its motion files on the IO pool, the parse on the CPU pool.
pub async fn load_rig(editor: Arc<Editor>, name: String, extra: Vec<String>) -> Rig {
    let sources = Sources::of(&editor).await;
    let exec = editor.exec.clone();
    let n = name.clone();
    let read = exec
        .io(move || {
            let found = find_model(&sources, &n)?;
            let bytes = sources.read(&found).ok().filter(|b| !b.is_empty())?;
            Some((sources, found, bytes))
        })
        .await;
    let Some((sources, found, bytes)) = read else {
        return Rig::failed(&name, "not found");
    };
    let visual = match exec.cpu(move || Visual::read(&bytes)).await {
        Ok(v) => v,
        Err(_) => return Rig::failed(&name, "not an OGF model"),
    };
    let visual = Arc::new(visual);
    let game_first = matches!(found, Found::Game { .. });
    let (v, s) = (visual.clone(), sources.clone());
    let motion_bytes: Vec<Vec<u8>> = exec
        .io(move || {
            let mut files = Vec::new();
            for r in &v.motion_refs {
                motion_files(&s, r, game_first, &mut files);
            }
            for e in &extra {
                motion_files(&s, e, game_first, &mut files);
            }
            files.iter().filter_map(|f| s.read(f).ok()).collect()
        })
        .await;
    exec.cpu(move || {
        let mesh = match MeshData::from_visual(&visual, &MaterialTable::default()) {
            Ok(m) => m,
            Err(_) => return Rig::failed(&name, "model_Create failed"),
        };
        let skeleton = Skeleton::of(&visual);
        let mut sets: Vec<MotionSet> = Vec::new();
        // the references stand for the model's own motions, as the game loads them
        if visual.motion_refs.is_empty()
            && let Some(own) = &visual.motions
        {
            sets.push(own.clone());
        }
        sets.extend(motion_bytes.iter().filter_map(|b| read_omf(b).ok()));
        let sets = sets
            .into_iter()
            .map(|set| {
                let slots = skeleton.slots_of(&set);
                (set, slots)
            })
            .collect();
        Rig { name, error: String::new(), mesh: Some(mesh), skeleton, sets, sources: Some(sources) }
    })
    .await
}

/// The rigs of the open project (`session.services`), by name and extra motions: the hands are
/// read once whatever item holds them.
#[derive(Default)]
pub struct Rigs {
    kept: Mutex<Vec<(String, Pending<Rig>)>>,
}

impl Rigs {
    #[must_use]
    pub fn of(editor: &Editor) -> Option<Arc<Self>> {
        let session = editor.project_opt()?;
        Some(session.services.get_or_init(Self::default))
    }

    /// The rig of a model, read the first time it is asked for.
    pub fn get(&self, editor: &Arc<Editor>, name: &str, extra: &[String]) -> Pending<Rig> {
        let key = format!("{}|{}", name.to_ascii_lowercase(), extra.join(","));
        let mut kept = self.kept.lock();
        if let Some(at) = kept.iter().position(|(k, _)| *k == key) {
            let hit = kept.remove(at);
            let p = hit.1.clone();
            kept.push(hit);
            return p;
        }
        let p = Pending::spawn(&editor.exec, load_rig(editor.clone(), name.to_owned(), extra.to_vec()));
        kept.push((key, p.clone()));
        if kept.len() > RIGS_KEPT {
            kept.remove(0);
        }
        p
    }

    /// Forgets every model: the next ask reads it again (Reload).
    pub fn clear(&self) {
        self.kept.lock().clear();
    }
}

// ---- the picture ---------------------------------------------------------------------------------

/// What keeps the picture from being shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Missing {
    /// The HUD section names no model.
    NoItemModel,
    /// The item's HUD model is nowhere: its name and why.
    ItemModel(String, String),
    /// No hands: the player HUD section is not there or names no model.
    NoHands,
    /// The hands' model is nowhere.
    HandsModel(String, String),
    /// The section has no motion line at all.
    NoMotion,
    /// A motion neither the hands nor the item hold: the line and the motion.
    Motion(String, String),
    /// The hands have no bone the item hangs on.
    Bone(String),
}

/// Where everything stands at one moment.
#[derive(Clone, Debug, Default)]
pub struct Placed {
    /// The line played and what it names.
    pub line: Option<HudMotion>,
    /// The hands' cycle and the item's.
    pub hands_cycle: Option<Cycle>,
    pub item_cycle: Option<Cycle>,
    /// The motion's length in real seconds (the hands', or a monolithic model's own).
    pub length: f32,
    /// The hands' transform in the camera's space; none for a model with the hands in it.
    pub hands: Option<XMat4>,
    /// The item's.
    pub item: XMat4,
    pub hands_skin: Vec<XMat4>,
    pub item_skin: Vec<XMat4>,
    pub missing: Vec<Missing>,
}

/// What plays and for how long, without a pose: cheap enough for a frame to ask.
#[derive(Clone, Debug, Default)]
pub struct Timing {
    pub line: Option<HudMotion>,
    pub hands_cycle: Option<Cycle>,
    pub item_cycle: Option<Cycle>,
    /// The motion's length in real seconds.
    pub length: f32,
    pub missing: Vec<Missing>,
}

/// The cycles the game plays for the line the picture shows: the hands the line's first motion
/// (`name`, then `name1` .. `name8`: the first the hands hold - the game picks one of them at
/// random), the item the line's second, or the name the hands play, or its `idle`; each at its
/// definition's speed times the line's. A model with the hands in it plays both on itself.
#[must_use]
pub fn timing(spec: &HudSpec, hands: Option<&Rig>, item: &Rig, wide: bool) -> Timing {
    let mut out = Timing::default();
    let Some((_, monolithic)) = spec.item_model() else {
        out.missing.push(Missing::NoItemModel);
        return out;
    };
    let line = motion_key(spec, wide).and_then(|k| spec.get(&k).and_then(|v| hud_motion(&k, v)));
    if line.is_none() {
        out.missing.push(Missing::NoMotion);
    }
    let player = if monolithic { Some(item) } else { hands };
    let hands_cycle = line.as_ref().and_then(|l| {
        let p = player?;
        (0..=8).find_map(|i| p.cycle(&if i == 0 { l.base.clone() } else { format!("{}{i}", l.base) }))
    });
    let item_cycle = line.as_ref().and_then(|l| {
        let name = if l.base != l.additional {
            l.additional.clone()
        } else {
            hands_cycle.as_ref().map_or_else(|| l.base.clone(), |c| c.name.clone())
        };
        item.cycle(&name).or_else(|| item.cycle("idle"))
    });
    if let Some(l) = &line
        && hands_cycle.is_none()
    {
        out.missing.push(Missing::Motion(l.key.clone(), l.base.clone()));
    }
    if !monolithic
        && let Some(h) = hands.filter(|h| h.shown())
        && let Some(a) = spec.anchors.get(usize::from(spec.place()))
        && h.skeleton.bone(a).is_none()
    {
        out.missing.push(Missing::Bone(a.clone()));
    }
    out.length = hands_cycle.as_ref().map_or(0.0, |c| c.seconds / out_rate(c, line.as_ref()));
    out.line = line;
    out.hands_cycle = hands_cycle;
    out.item_cycle = item_cycle;
    out
}

/// A cycle's seconds per real second: its definition's speed times its line's.
fn out_rate(c: &Cycle, line: Option<&HudMotion>) -> f32 {
    (c.speed * line.map_or(1.0, |l| l.speed)).max(1e-4)
}

/// Everything at `t` seconds since the motion started, for a picture wide or not, as the game
/// places it: the hands at the camera's frame (moved by aiming, [`aimed`]) times the hands'
/// offset; the item at the hands' transform times its attach bone's transform in the posed hands
/// times the item's offset (a model with the hands in it: the camera's frame times its own
/// offset); each posed by the cycles of [`timing`].
#[must_use]
pub fn place(spec: &HudSpec, hands: Option<&Rig>, item: &Rig, t: f32, wide: bool) -> Placed {
    let tm = timing(spec, hands, item, wide);
    let mut out = Placed { item: XMat4::IDENTITY, ..Placed::default() };
    let Some((_, monolithic)) = spec.item_model() else {
        out.missing = tm.missing;
        return out;
    };
    let m = measures(spec, monolithic, wide);
    let idx = spec.aim();
    let trans = if idx > 0 { aimed(m.offset[0][idx], m.offset[1][idx]) } else { XMat4::IDENTITY };
    let t = if tm.length > 0.0 { t.rem_euclid(tm.length) } else { 0.0 };
    let at = |c: Option<&Cycle>| c.map_or(0.0, |c| t * out_rate(c, tm.line.as_ref()));
    let offset = attach(m.item[0], m.item[1]);
    if monolithic {
        // the model is the hands: it plays the cycle the line names on itself
        let cycle = tm.item_cycle.as_ref().or(tm.hands_cycle.as_ref());
        let (_, skin) = item.pose(cycle, at(cycle), false, &spec.hidden);
        out.item = XMat4::mul_43(&trans, &offset);
        out.item_skin = skin;
    } else {
        let hands_m = XMat4::mul_43(&trans, &attach(m.hands[0], m.hands[1]));
        let (model, skin) = hands.map_or((Vec::new(), Vec::new()), |h| {
            h.pose(tm.hands_cycle.as_ref(), at(tm.hands_cycle.as_ref()), false, &[])
        });
        let bone =
            spec.anchors.get(usize::from(spec.place())).and_then(|a| hands.and_then(|h| h.skeleton.bone(a)));
        out.item = match bone.and_then(|b| model.get(b)) {
            Some(anchor_m) => XMat4::mul_43(&XMat4::mul_43(&hands_m, anchor_m), &offset),
            None => XMat4::mul_43(&hands_m, &offset),
        };
        let (_, item_skin) =
            item.pose(tm.item_cycle.as_ref(), at(tm.item_cycle.as_ref()), true, &spec.hidden);
        out.item_skin = item_skin;
        out.hands = Some(hands_m);
        out.hands_skin = skin;
    }
    out.line = tm.line;
    out.hands_cycle = tm.hands_cycle;
    out.item_cycle = tm.item_cycle;
    out.length = tm.length;
    out.missing = tm.missing;
    out
}

fn to_glam(m: &XMat4) -> Mat4 {
    // the game's rows i, j, k, c are glam's columns
    Mat4::from_cols_array_2d(&m.0)
}

/// Where the picture is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Nothing asked for.
    Off,
    /// The models are being read.
    Loading,
    /// The window has no graphics device, or the viewport could not start: why.
    NoDevice(String),
    Missing(Missing),
    Ready,
}

/// The two models as the renderer holds them, and the rigs they are of (by address).
type Meshes = ((usize, usize), Option<MeshHandle>, Option<MeshHandle>);

/// What a frame last asked a picture for.
#[derive(Clone, Debug, PartialEq)]
struct DrawKey {
    spec: HudSpec,
    /// The moment, in tenths of milliseconds.
    t: u64,
    size: [u32; 2],
    rigs: (usize, usize),
    background: [u8; 3],
}

/// The In the Hands picture of one surface: its renderer, the rigs it shows and the clock.
pub struct HandsView {
    editor: Arc<Editor>,
    renderer: Option<Pending<Result<Arc<Renderer>, String>>>,
    spec: Option<HudSpec>,
    hands: Option<(String, Pending<Rig>)>,
    item: Option<(String, Pending<Rig>)>,
    /// The rigs whose meshes the renderer holds, by address.
    uploaded: (usize, usize),
    /// The meshes the renderer holds, once a pool thread handed them over, for the rigs they are
    /// of.
    meshes: Arc<Mutex<Option<Meshes>>>,
    playing: bool,
    /// The moment the motion was at when the clock last started or stopped, and when that was.
    held: f32,
    since: Instant,
    last: Option<DrawKey>,
    packet: Arc<Mutex<Option<Arc<FramePacket>>>>,
    target: Option<(u64, Arc<ImageDraw>)>,
    submitted: Arc<AtomicU64>,
    /// The picture's size as last drawn, pixels.
    size: [u32; 2],
    /// Why the viewport cannot start.
    device_error: Option<String>,
}

impl HandsView {
    #[must_use]
    pub fn new(editor: &Arc<Editor>) -> Self {
        Self {
            editor: editor.clone(),
            renderer: None,
            spec: None,
            hands: None,
            item: None,
            uploaded: (0, 0),
            meshes: Arc::default(),
            playing: false,
            held: 0.0,
            since: Instant::now(),
            last: None,
            packet: Arc::new(Mutex::new(None)),
            target: None,
            submitted: Arc::new(AtomicU64::new(0)),
            size: [16, 9],
            device_error: None,
        }
    }

    /// What the picture shows now; a row the author moved to starts its motion again, playing
    /// when the row is a motion line.
    pub fn set_spec(&mut self, spec: Option<HudSpec>) {
        let row_moved = self.spec.as_ref().map(|s| &s.row) != spec.as_ref().map(|s| &s.row)
            || self.spec.as_ref().map(|s| &s.section) != spec.as_ref().map(|s| &s.section);
        if row_moved {
            let motion_row = spec.as_ref().and_then(|s| s.row.as_deref()).is_some_and(|r| {
                r.starts_with("anm_") && spec.as_ref().is_some_and(|s| s.keys.contains_key(r))
            });
            self.held = 0.0;
            self.since = Instant::now();
            self.playing = motion_row;
        }
        if let Some(s) = &spec {
            let rigs = Rigs::of(&self.editor);
            let want_hands =
                s.item_model().filter(|(_, mono)| !mono).map(|_| s.hands.clone()).filter(|h| !h.is_empty());
            let key_h = want_hands.as_ref().map(|h| format!("{h}|{}", s.hand_motions.join(",")));
            if self.hands.as_ref().map(|(k, _)| k) != key_h.as_ref() {
                self.hands = match (want_hands, &rigs) {
                    (Some(h), Some(r)) => {
                        Some((key_h.unwrap_or_default(), r.get(&self.editor, &h, &s.hand_motions)))
                    }
                    _ => None,
                };
            }
            let want_item = s.item_model().map(|(v, _)| v);
            if self.item.as_ref().map(|(k, _)| k) != want_item.as_ref() {
                self.item = match (want_item, &rigs) {
                    (Some(v), Some(r)) => Some((v.clone(), r.get(&self.editor, &v, &[]))),
                    _ => None,
                };
            }
        } else {
            self.hands = None;
            self.item = None;
        }
        self.spec = spec;
    }

    /// Lets go of the models: the next [`HandsView::set_spec`] reads them again (Reload, after
    /// [`Rigs::clear`]).
    pub fn forget(&mut self) {
        self.hands = None;
        self.item = None;
        self.uploaded = (0, 0);
        *self.meshes.lock() = None;
        self.last = None;
    }

    /// The seconds since the motion started, wrapped at its length when it has one.
    #[must_use]
    pub fn time(&self) -> f32 {
        let raw = if self.playing { self.held + self.since.elapsed().as_secs_f32() } else { self.held };
        let len = self.timing().map_or(0.0, |p| p.length);
        if len > 0.0 { raw.rem_euclid(len) } else { raw.max(0.0) }
    }

    #[must_use]
    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Plays or holds the motion where it is.
    pub fn set_playing(&mut self, on: bool) {
        if on == self.playing {
            return;
        }
        self.held = self.time();
        self.since = Instant::now();
        self.playing = on;
    }

    /// Moves the motion to `t` seconds (wrapped at its length); a playing motion goes on from there.
    pub fn set_time(&mut self, t: f32) {
        self.held = t.max(0.0);
        self.since = Instant::now();
    }

    fn rigs(&self) -> (Option<Arc<Rig>>, Option<Arc<Rig>>) {
        (self.hands.as_ref().and_then(|(_, p)| p.get()), self.item.as_ref().and_then(|(_, p)| p.get()))
    }

    /// Where the picture is.
    #[must_use]
    pub fn status(&self) -> Status {
        let Some(spec) = &self.spec else { return Status::Off };
        let Some((name, monolithic)) = spec.item_model() else {
            return Status::Missing(Missing::NoItemModel);
        };
        if !monolithic && spec.hands.is_empty() {
            return Status::Missing(Missing::NoHands);
        }
        let (hands, item) = self.rigs();
        if item.is_none() || (!monolithic && hands.is_none()) {
            return Status::Loading;
        }
        if let Some(i) = item.as_ref().filter(|i| !i.shown()) {
            return Status::Missing(Missing::ItemModel(name, i.error.clone()));
        }
        if let Some(h) = hands.as_ref().filter(|h| !h.shown()) {
            return Status::Missing(Missing::HandsModel(spec.hands.clone(), h.error.clone()));
        }
        if let Some(why) = &self.device_error {
            return Status::NoDevice(why.clone());
        }
        match self.timing().and_then(|p| p.missing.first().cloned()) {
            Some(m) => Status::Missing(m),
            None => Status::Ready,
        }
    }

    /// What plays and for how long, for the picture as it was last drawn.
    #[must_use]
    pub fn timing(&self) -> Option<Timing> {
        let spec = self.spec.as_ref()?;
        let (hands, item) = self.rigs();
        let item = item?;
        Some(timing(spec, hands.as_deref(), &item, is_wide(self.size)))
    }

    /// Everything at the moment the clock is at, for the picture as it was last drawn.
    #[must_use]
    pub fn placed(&self) -> Option<Placed> {
        let spec = self.spec.as_ref()?;
        let (hands, item) = self.rigs();
        let item = item?;
        let raw = if self.playing { self.held + self.since.elapsed().as_secs_f32() } else { self.held };
        Some(place(spec, hands.as_deref(), &item, raw, is_wide(self.size)))
    }

    /// The picture's state for a command: the models, the line played, its length, the time, the
    /// two transforms and what is missing.
    #[must_use]
    pub fn state_json(&self) -> Value {
        let Some(spec) = &self.spec else { return Value::Null };
        let status = self.status();
        let placed = self.placed();
        let mat = |m: &XMat4| -> Value {
            json!({ "position": m.c().to_array(), "right": m.row(0).to_array(), "up": m.row(1).to_array(), "forward": m.row(2).to_array() })
        };
        let missing = match &status {
            Status::Missing(m) => missing_json(m),
            Status::NoDevice(why) => json!({ "what": "device", "why": why }),
            _ => Value::Null,
        };
        let (hands, item) = self.rigs();
        json!({
            "status": match status {
                Status::Off => "off",
                Status::Loading => "loading",
                Status::NoDevice(_) | Status::Missing(_) => "missing",
                Status::Ready => "ready",
            },
            "section": spec.section,
            "hands": spec.item_model().is_some_and(|(_, mono)| !mono).then(|| spec.hands.clone()),
            "hands_found": hands.as_ref().map(|h| h.shown()),
            "hands_sets": hands.as_ref().map(|h| h.sets()),
            "item": spec.item_model().map(|(v, _)| v),
            "item_found": item.as_ref().map(|i| i.shown()),
            "monolithic": spec.item_model().is_some_and(|(_, mono)| mono),
            "attach_place": spec.place(),
            "anchor": spec.anchors.get(usize::from(spec.place())),
            "fov": spec.fov,
            "row": spec.row,
            "line": placed.as_ref().and_then(|p| p.line.as_ref().map(|l| l.key.clone())),
            "motion": placed.as_ref().and_then(|p| p.hands_cycle.as_ref().map(|c| c.name.clone())),
            "item_motion": placed.as_ref().and_then(|p| p.item_cycle.as_ref().map(|c| c.name.clone())),
            "length": placed.as_ref().map(|p| p.length),
            "time": self.time(),
            "playing": self.playing,
            "wide": is_wide(self.size),
            "hands_transform": placed.as_ref().and_then(|p| p.hands.as_ref().map(mat)),
            "item_transform": placed.as_ref().map(|p| mat(&p.item)),
            "missing": missing,
        })
    }

    /// The renderer, once it exists; made the first time a picture is wanted.
    fn renderer(&mut self) -> Result<Option<Arc<Renderer>>, String> {
        if self.renderer.is_none() {
            let Some(gpu) = self.editor.services.get::<WindowLink>().and_then(|l| l.gpu()) else {
                let why = "the editor runs without a graphics device".to_owned();
                self.device_error = Some(why.clone());
                return Err(why);
            };
            let (device, queue) = (gpu.device.clone(), gpu.queue.clone());
            let gate = gz_app::gpu_gate(&self.editor);
            let exec = self.editor.exec.clone();
            let pools = exec.clone();
            self.renderer = Some(Pending::spawn(&exec, async move {
                let inner = pools.clone();
                pools
                    .cpu(move || {
                        Renderer::new(device, queue, gate, inner).map(Arc::new).map_err(|e| e.to_string())
                    })
                    .await
            }));
        }
        match self.renderer.as_ref().and_then(Pending::get) {
            Some(r) => {
                let r = r.as_ref().clone();
                if let Err(e) = &r {
                    self.device_error = Some(e.clone());
                }
                r.map(Some)
            }
            None => Ok(None),
        }
    }

    /// Paints the picture into `rect` when it is ready: the caller says what stands there
    /// otherwise ([`HandsView::status`]). Asks for the next frame while the motion plays.
    pub fn draw(&mut self, ui: &Ui, gpu: &egui_wgpu::RenderState, rect: Rect, background: egui::Color32) {
        let ppp = ui.ctx().pixels_per_point();
        let size =
            [((rect.width() * ppp).round() as u32).max(1), ((rect.height() * ppp).round() as u32).max(1)];
        self.size = size;
        ui.painter().rect_filled(rect, CornerRadius::same(4), background);
        let renderer = match self.renderer() {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(_) => return,
        };
        let Some(spec) = self.spec.clone() else { return };
        let (hands, item) = self.rigs();
        let Some(item) = item.filter(|i| i.shown()) else { return };
        let hands = hands.filter(|h| h.shown());
        // the meshes go to the renderer once per rig
        let key_h = hands.as_ref().map_or(0, |h| Arc::as_ptr(h) as usize);
        let key_i = Arc::as_ptr(&item) as usize;
        if self.uploaded != (key_h, key_i) {
            self.uploaded = (key_h, key_i);
            if let Some(sources) = &item.sources {
                renderer.assets().set_loader(Some(Arc::new(sources.clone())));
            }
            let (slot, r, want) = (self.meshes.clone(), renderer.clone(), self.uploaded);
            let (h, i, wake) = (hands.clone(), item.clone(), self.editor.exec.wake().clone());
            self.editor.exec.spawn_cpu(move || {
                // an upload takes the graphics queue's lock: a pool thread does it, ahead of the
                // other pools, never the frame
                let _drawing = gz_win::drawing_priority();
                let hm = h.and_then(|h| h.mesh.clone()).map(|m| r.assets().add_mesh(HANDS_MESH, m));
                let im = i.mesh.clone().map(|m| r.assets().add_mesh(ITEM_MESH, m));
                *slot.lock() = Some((want, hm, im));
                wake.ping();
            });
            self.last = None;
        }
        let held = (*self.meshes.lock()).filter(|(k, _, _)| *k == self.uploaded);
        let Some((_, hands_mesh, item_mesh)) = held else { return };
        let raw = if self.playing { self.held + self.since.elapsed().as_secs_f32() } else { self.held };
        let key = DrawKey {
            spec: spec.clone(),
            t: (raw * 10_000.0) as u64,
            size,
            rigs: self.uploaded,
            background: [background.r(), background.g(), background.b()],
        };
        let stale = self.packet.lock().as_ref().is_some_and(|pk| pk.is_stale(renderer.assets()));
        if self.last.as_ref() != Some(&key) || stale {
            self.last = Some(key);
            let (slot, r, submitted) = (self.packet.clone(), renderer.clone(), self.submitted.clone());
            let bg = Color::rgb(background.r(), background.g(), background.b());
            self.editor.exec.spawn_cpu(move || {
                let placed = place(&spec, hands.as_deref(), &item, raw, is_wide(size));
                let mut list = DrawList::new();
                let flags = InstanceFlags::NO_ID | InstanceFlags::NO_CULL;
                if let (Some(mesh), Some(m)) = (hands_mesh, placed.hands.as_ref()) {
                    let skin: Arc<[Mat4]> = placed.hands_skin.iter().map(to_glam).collect();
                    list.push(Instance::new(mesh, to_glam(m)).with_flags(flags).with_pose(skin));
                }
                if let Some(mesh) = item_mesh {
                    let mut inst = Instance::new(mesh, to_glam(&placed.item)).with_flags(flags);
                    if !placed.item_skin.is_empty() {
                        inst = inst.with_pose(placed.item_skin.iter().map(to_glam).collect());
                    }
                    list.push(inst);
                }
                let camera = Camera::looking(Vec3::ZERO, Vec3::Z).with_projection(Projection::Perspective {
                    fov_y: spec.fov.clamp(1.0, 170.0).to_radians(),
                    near: HUD_NEAR,
                    far: HUD_FAR,
                });
                let mut view = View::new(camera, size);
                view.mode = ViewMode::Lit;
                view.flags = ViewFlags::empty();
                view.background = bg;
                let packet = Arc::new(build_packet(&list, &view, r.assets()));
                let serial = r.submit(packet.clone());
                *slot.lock() = Some(packet);
                submitted.fetch_max(serial, Ordering::AcqRel);
            });
        }
        if let Some(handle) = renderer.target() {
            if self.target.as_ref().is_none_or(|(serial, _)| *serial != handle.serial)
                && let Some(draw) = ImageDraw::new(gpu, &handle.view, false, &self.editor.exec)
            {
                self.target = Some((handle.serial, draw));
            }
            if let Some((_, draw)) = &self.target {
                let [u, v] = handle.uv();
                paint_image(ui.painter(), rect, rect, [0.0, 0.0, u, v], ImageMode::Opaque, draw);
            }
        }
        if self.playing {
            ui.ctx().request_repaint_after(gz_ui::state::SPINNER_FRAME);
        }
    }

    /// Whether a picture of the renderer stands in the rect yet.
    #[must_use]
    pub fn has_picture(&self) -> bool {
        self.target.is_some()
    }

    /// The serial of the last packet handed to the renderer.
    #[must_use]
    pub fn submitted(&self) -> u64 {
        self.submitted.load(Ordering::Acquire)
    }
}

/// What a command answers of a missing part.
#[must_use]
pub fn missing_json(m: &Missing) -> Value {
    match m {
        Missing::NoItemModel => json!({ "what": "item_model" }),
        Missing::ItemModel(name, why) => json!({ "what": "item_model", "name": name, "why": why }),
        Missing::NoHands => json!({ "what": "hands" }),
        Missing::HandsModel(name, why) => json!({ "what": "hands", "name": name, "why": why }),
        Missing::NoMotion => json!({ "what": "motion" }),
        Missing::Motion(line, motion) => json!({ "what": "motion", "line": line, "name": motion }),
        Missing::Bone(bone) => json!({ "what": "bone", "name": bone }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(keys: &[(&str, &str)]) -> HudSpec {
        HudSpec {
            section: "wpn_x_hud".into(),
            keys: keys.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
            hands: "hands".into(),
            anchors: vec!["lead_gun".into(), "l_hand".into()],
            fov: 37.5,
            ..HudSpec::default()
        }
    }

    #[test]
    fn a_motion_line_is_read_as_the_game_reads_it() {
        let m = hud_motion("anm_reload", "ak_reload, ak_reload_w, 1.25").unwrap();
        assert_eq!((m.base.as_str(), m.additional.as_str(), m.speed), ("ak_reload", "ak_reload_w", 1.25));
        let one = hud_motion("anm_idle", "ak_idle").unwrap();
        assert_eq!((one.additional.as_str(), one.speed), ("ak_idle", 1.0));
        let empty_second = hud_motion("anm_show", "d1_draw, , 2").unwrap();
        assert_eq!((empty_second.additional.as_str(), empty_second.speed), ("d1_draw", 2.0));
        assert!(hud_motion("anm_x", " ").is_none());
    }

    #[test]
    fn the_row_picks_the_line_and_the_left_hand_its_wide_form() {
        let mut s =
            spec(&[("anm_idle", "a"), ("anm_idle_aim", "b"), ("anm_show", "c"), ("anm_show_16x9", "d")]);
        assert_eq!(motion_key(&s, true).as_deref(), Some("anm_idle"));
        s.row = Some("anm_show".into());
        assert_eq!(motion_key(&s, true).as_deref(), Some("anm_show"));
        s.row = Some("aim_hud_offset_pos".into());
        assert_eq!((s.aim(), motion_key(&s, true).as_deref()), (1, Some("anm_idle_aim")));
        // a detector: the left hand plays the _16x9 line on a wide screen
        s.keys.insert("attach_place_idx".into(), "1".into());
        s.row = Some("anm_show".into());
        assert_eq!(motion_key(&s, true).as_deref(), Some("anm_show_16x9"));
        assert_eq!(motion_key(&s, false).as_deref(), Some("anm_show"));
        // a row that is no motion: the idle
        s.row = Some("hands_position".into());
        assert_eq!(motion_key(&s, false).as_deref(), Some("anm_idle"));
    }

    #[test]
    fn the_measures_read_the_wide_forms_on_a_wide_screen() {
        let s = spec(&[
            ("hands_position", "0.1, 0.2, 0.3"),
            ("hands_position_16x9", "1, 2, 3"),
            ("hands_orientation", "4, 5, 6"),
            ("item_position", "-0.003,0.011,0.008"),
            ("aim_hud_offset_pos_16x9", "7, 8, 9"),
        ]);
        let narrow = measures(&s, false, false);
        let wide = measures(&s, false, true);
        assert_eq!(narrow.hands[0], Vec3::new(0.1, 0.2, 0.3));
        assert_eq!(wide.hands[0], Vec3::new(1.0, 2.0, 3.0));
        // a key the section lacks reads as zero
        assert_eq!(wide.hands[1], Vec3::ZERO);
        assert_eq!(narrow.item[0], Vec3::new(-0.003, 0.011, 0.008));
        assert_eq!(wide.offset[0][1], Vec3::new(7.0, 8.0, 9.0));
        assert!(is_wide([1920, 1080]) && !is_wide([1024, 768]) && !is_wide([1280, 1024]));
    }

    #[test]
    fn the_attach_matrix_is_set_hpb_in_degrees_and_aiming_turns_x_then_y_then_z() {
        // 90 degrees of heading turns +Z (forward) to -X, as the game's heading does
        let m = attach(Vec3::new(0.1, 0.2, 0.3), Vec3::new(90.0, 0.0, 0.0));
        let fwd = m.transform_dir(Vec3::Z);
        assert!((fwd - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-5, "{fwd:?}");
        assert_eq!(m.c(), Vec3::new(0.1, 0.2, 0.3));
        let a = aimed(Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.3, 0.2, 0.1));
        let by_hand = XMat4::mul_43(&rotate_z(0.1), &XMat4::mul_43(&rotate_y(0.2), &rotate_x(0.3)));
        let p = Vec3::new(0.2, -0.4, 1.0);
        assert!((a.transform_dir(p) - by_hand.transform_dir(p)).length() < 1e-6);
        assert_eq!(a.c(), Vec3::X);
        // rotateX turns up (+Y) towards +Z, as the engine's does
        assert!((rotate_x(PI / 2.0).transform_dir(Vec3::Y) - Vec3::Z).length() < 1e-6);
    }

    #[test]
    fn the_hud_field_of_view_is_the_games() {
        let none = |_: &str| None;
        assert!((hud_fov(false, false, &none, &none) - 37.5).abs() < 1e-4);
        assert!((hud_fov(true, false, &none, &none) - 37.5).abs() < 1e-4);
        let item = |k: &str| (k == "hud_fov_addition_modifier").then(|| "0.05".to_owned());
        let hud = |k: &str| match k {
            "hud_fov_factor" => Some("0.9".to_owned()),
            "hud_fov_zoom_factor" => Some("0.5".to_owned()),
            _ => None,
        };
        assert!((hud_fov(true, false, &item, &hud) - 0.425 * 0.9 * 100.0).abs() < 1e-3);
        assert!((hud_fov(true, true, &item, &hud) - 0.425 * 0.5 * 100.0).abs() < 1e-3);
    }

    #[test]
    fn an_item_without_a_model_or_a_motion_says_what_it_lacks() {
        let s = spec(&[("anm_idle", "a")]);
        let item = Rig::failed("x", "not found");
        assert_eq!(place(&s, None, &item, 0.0, true).missing, vec![Missing::NoItemModel]);
        let s = spec(&[("item_visual", "dynamics\\x_hud"), ("anm_idle", "ak_idle")]);
        let placed = place(&s, None, &item, 0.0, true);
        assert!(placed.missing.contains(&Missing::Motion("anm_idle".into(), "ak_idle".into())));
        assert_eq!(s.item_model(), Some(("dynamics\\x_hud".to_owned(), false)));
        assert_eq!(spec(&[("visual", "\"m\"")]).item_model(), Some(("m".to_owned(), true)));
    }
}
