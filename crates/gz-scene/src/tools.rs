//! What the scene keeps per class tool (`ESceneToolBase` and its subclasses' settings), the
//! level options (`st_LevelOptions`) and the bulk data of the AI map, detail objects and
//! wallmarks - with their part of the files.

use std::sync::Arc;

use gz_base::{Blob, SmolStr};
use gz_chunk::{Reader, Writer};
use gz_fmt_level::{AiMapNode, SceneAiMap, SceneAiNode, SceneDetails};

use crate::SceneError;
use crate::class::{CLASSES, ObjClass};
use crate::ltx::{IniReader, IniWriter};
use crate::model::ObjId;

/// `CURRENT_LEVELOP_VERSION`.
pub const LEVELOP_VERSION: u32 = 0x0C;
/// `CURRENT_LEVELOP_BP_VERSION`.
pub const LEVELOP_BP_VERSION: u32 = 9;

/// `b_params`: the compiler's build parameters (52 bytes in the binary files).
#[derive(Clone, Debug, PartialEq)]
pub struct BuildParams {
    pub sm_angle: f32,
    pub weld_distance: f32,
    pub lm_pixels_per_meter: f32,
    pub lm_jitter_samples: u32,
    pub lm_rms_zero: u32,
    pub lm_rms: u32,
    /// `ebqDraft` 0, `ebqHigh` 1, `ebqCustom` 2.
    pub quality: u16,
    pub u_reserved: u16,
    pub f_reserved: [f32; 6],
}

impl Default for BuildParams {
    /// `Init()` then `setHighQuality()`.
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

impl BuildParams {
    fn read_bin(r: &mut Reader<'_>) -> Result<Self, SceneError> {
        Ok(Self {
            sm_angle: r.r_f32()?,
            weld_distance: r.r_f32()?,
            lm_pixels_per_meter: r.r_f32()?,
            lm_jitter_samples: r.r_u32()?,
            lm_rms_zero: r.r_u32()?,
            lm_rms: r.r_u32()?,
            quality: r.r_u16()?,
            u_reserved: r.r_u16()?,
            f_reserved: [r.r_f32()?, r.r_f32()?, r.r_f32()?, r.r_f32()?, r.r_f32()?, r.r_f32()?],
        })
    }

    fn write_bin(&self, w: &mut Writer) {
        w.w_f32(self.sm_angle);
        w.w_f32(self.weld_distance);
        w.w_f32(self.lm_pixels_per_meter);
        w.w_u32(self.lm_jitter_samples);
        w.w_u32(self.lm_rms_zero);
        w.w_u32(self.lm_rms);
        w.w_u16(self.quality);
        w.w_u16(self.u_reserved);
        for f in self.f_reserved {
            w.w_f32(f);
        }
    }

    fn write_ltx(&self, w: &mut IniWriter) {
        let s = "build_params";
        w.w_float(s, "smooth_angle", self.sm_angle);
        w.w_float(s, "weld_distance", self.weld_distance);
        w.w_float(s, "light_pixel_per_meter", self.lm_pixels_per_meter);
        w.w_u32(s, "light_jitter_samples", self.lm_jitter_samples);
        w.w_u32(s, "light_rms_zero", self.lm_rms_zero);
        w.w_u32(s, "light_rms", self.lm_rms);
        w.w_u16(s, "light_quality", self.quality);
        w.w_u16(s, "light_quality_reserved", self.u_reserved);
        for (i, f) in self.f_reserved.iter().enumerate() {
            w.w_float(s, &format!("reserved_{i}"), *f);
        }
    }

    fn read_ltx(r: &IniReader) -> Result<Self, SceneError> {
        let s = "build_params";
        let mut f_reserved = [0.0; 6];
        for (i, f) in f_reserved.iter_mut().enumerate() {
            *f = r.r_float(s, &format!("reserved_{i}"))?;
        }
        Ok(Self {
            sm_angle: r.r_float(s, "smooth_angle")?,
            weld_distance: r.r_float(s, "weld_distance")?,
            lm_pixels_per_meter: r.r_float(s, "light_pixel_per_meter")?,
            lm_jitter_samples: r.r_u32(s, "light_jitter_samples")?,
            lm_rms_zero: r.r_u32(s, "light_rms_zero")?,
            lm_rms: r.r_u32(s, "light_rms")?,
            quality: r.r_u16(s, "light_quality")?,
            u_reserved: r.r_u16(s, "light_quality_reserved")?,
            f_reserved,
        })
    }
}

/// `st_LevelOptions`.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelOptions {
    pub level_path: SmolStr,
    pub level_prefix: SmolStr,
    pub bop: String,
    pub map_version: SmolStr,
    pub light_hemi_quality: u8,
    pub light_sun_quality: u8,
    pub build: BuildParams,
    /// `m_mapUsage`: the game types, one bit each.
    pub map_usage: u16,
}

impl Default for LevelOptions {
    /// `Reset()`.
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
        }
    }
}

const LO_VERSION: u32 = 0x7801;
const LO_NAMES: u32 = 0x7802;
const LO_BOP: u32 = 0x7803;
const LO_PREFIX: u32 = 0x7804;
const LO_BP_VERSION: u32 = 0x7849;
const LO_BUILD_PARAMS: u32 = 0x7850;
const LO_LIGHT_QUALITY: u32 = 0x7851;
const LO_MAP_USAGE: u32 = 0x7852;
const LO_MAP_VER: u32 = 0x7853;

// game-type bits of the old `usage_*` keys
const GT_DEATHMATCH: u16 = 1 << 1;
const GT_TEAM_DEATHMATCH: u16 = 1 << 2;
const GT_ARTEFACT_HUNT: u16 = 1 << 3;
const GT_CAPTURE: u16 = 1 << 4;
const GT_DOMINATION: u16 = 1 << 5;
const GT_TEAM_DOMINATION: u16 = 1 << 6;

impl LevelOptions {
    /// `SetDraftQuality` / `SetHighQuality` / `SetCustomQuality`: the Quality buttons.
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

    /// `SaveLTX`.
    pub fn write_ltx(&self, w: &mut IniWriter) {
        let s = "level_options";
        w.w_u32(s, "version", LEVELOP_VERSION);
        w.w_string(s, "level_path", &self.level_path);
        w.w_string(s, "level_prefix", &self.level_prefix);
        w.w_string(s, "bop", &format!("\"{}\"", self.bop));
        w.w_string(s, "map_version", &self.map_version);
        w.w_u32(s, "version_bp", LEVELOP_BP_VERSION);
        self.build.write_ltx(w);
        w.w_u8(s, "light_hemi_quality", self.light_hemi_quality);
        w.w_u8(s, "light_sun_quality", self.light_sun_quality);
        w.w_u16(s, "game_type", self.map_usage);
    }

    /// `ReadLTX`: `false` (here `Err`) for a section that is incomplete.
    pub fn read_ltx(r: &IniReader) -> Result<Self, SceneError> {
        let s = "level_options";
        let bad = || SceneError::load(format!("Invalid LTX level options: {}", r.file()));
        for key in ["version", "level_path", "level_prefix", "bop", "light_hemi_quality", "light_sun_quality"]
        {
            if !r.line_exist(s, key) {
                return Err(bad());
            }
        }
        let version = r.r_u32(s, "version")?;
        if version < 8 {
            return Err(bad());
        }
        let build = BuildParams::read_ltx(r).map_err(|_| bad())?;
        if (version > 0x0B && !r.line_exist(s, "map_version"))
            || (version > 0x0A && !r.line_exist(s, "game_type"))
        {
            return Err(bad());
        }
        let mut map_usage = 0xffff_u16;
        if version > 0x0A {
            map_usage = r.r_u16(s, "game_type")?;
        } else {
            let mut set = |bit: u16, key: &str| -> Result<(), SceneError> {
                if !r.line_exist(s, key) {
                    return Err(bad());
                }
                if r.r_s32(s, key)? != 0 {
                    map_usage |= bit;
                } else {
                    map_usage &= !bit;
                }
                Ok(())
            };
            set(GT_DEATHMATCH, "usage_deathmatch")?;
            set(GT_TEAM_DEATHMATCH, "usage_teamdeathmatch")?;
            set(GT_ARTEFACT_HUNT, "usage_artefacthunt")?;
            if version > 8 {
                set(GT_CAPTURE, "usage_captretheartefact")?;
                set(GT_TEAM_DOMINATION, "usage_team_domination_zone")?;
                set(GT_DOMINATION, if version == 9 { "domination_zone" } else { "usage_domination_zone" })?;
            }
        }
        Ok(Self {
            level_path: r.r_string(s, "level_path")?.into(),
            level_prefix: r.r_string(s, "level_prefix")?.into(),
            bop: r.r_string_wb(s, "bop")?,
            map_version: if version > 0x0B {
                r.r_string(s, "map_version")?.into()
            } else {
                SmolStr::new_static("1.0")
            },
            light_hemi_quality: r.r_u8(s, "light_hemi_quality")?,
            light_sun_quality: r.r_u8(s, "light_sun_quality")?,
            build,
            map_usage,
        })
    }

    /// `Save(IWriter&)`.
    pub fn write_bin(&self, w: &mut Writer) {
        w.chunk(LO_VERSION, |w| w.w_u32(LEVELOP_VERSION));
        w.chunk(LO_NAMES, |w| w.w_stringz(&self.level_path));
        w.chunk(LO_PREFIX, |w| w.w_stringz(&self.level_prefix));
        w.chunk(LO_BOP, |w| w.w_stringz(&self.bop));
        w.chunk(LO_MAP_VER, |w| {
            w.w_stringz(if self.map_version.is_empty() { "1.0" } else { &self.map_version })
        });
        w.chunk(LO_BP_VERSION, |w| w.w_u32(LEVELOP_BP_VERSION));
        w.chunk(LO_BUILD_PARAMS, |w| self.build.write_bin(w));
        w.chunk(LO_LIGHT_QUALITY, |w| {
            w.w_u8(self.light_hemi_quality);
            w.w_u8(self.light_sun_quality);
        });
        w.chunk(LO_MAP_USAGE, |w| w.w_u16(self.map_usage));
    }

    /// `Read(IReader&)`.
    pub fn read_bin(r: &Reader<'_>) -> Result<Self, SceneError> {
        let bad = || SceneError::load("Invalid binary level options");
        let mut out = Self::default();
        let v = r.find_chunk(LO_VERSION).ok_or_else(bad)?;
        if v.0.len() != 4 || u32::from_le_bytes([v.0[0], v.0[1], v.0[2], v.0[3]]) < 8 {
            return Err(bad());
        }
        let string = |id: u32, required: bool| -> Result<Option<String>, SceneError> {
            match r.open_chunk(id)? {
                None if required => Err(bad()),
                None => Ok(None),
                Some(c) => {
                    if !c.bytes().contains(&0) {
                        return Err(bad());
                    }
                    Ok(Some(c.reader().r_stringz()?))
                }
            }
        };
        if let Some(s) = string(LO_NAMES, true)? {
            out.level_path = s.into();
        }
        if let Some(s) = string(LO_PREFIX, false)? {
            out.level_prefix = s.into();
        }
        if let Some(s) = string(LO_BOP, true)? {
            out.bop = s;
        }
        if let Some(s) = string(LO_MAP_VER, false)? {
            out.map_version = s.into();
        }
        let mut bp_version = 0;
        if let Some(c) = r.open_chunk(LO_BP_VERSION)? {
            if c.bytes().len() != 4 {
                return Err(bad());
            }
            bp_version = c.reader().r_u32()?;
        }
        if bp_version == LEVELOP_BP_VERSION {
            let c = r.open_chunk(LO_BUILD_PARAMS)?.ok_or_else(bad)?;
            if c.bytes().len() != 52 {
                return Err(bad());
            }
            out.build = BuildParams::read_bin(&mut c.reader())?;
        }
        if let Some(c) = r.open_chunk(LO_LIGHT_QUALITY)? {
            if c.bytes().len() != 2 {
                return Err(bad());
            }
            let mut q = c.reader();
            out.light_hemi_quality = q.r_u8()?;
            out.light_sun_quality = q.r_u8()?;
        }
        if let Some(c) = r.open_chunk(LO_MAP_USAGE)? {
            let mut q = c.reader();
            if bp_version > 8 {
                if c.bytes().len() != 2 {
                    return Err(bad());
                }
                out.map_usage = q.r_u16()?;
            } else {
                if c.bytes().len() != 12 {
                    return Err(bad());
                }
                let mut m = 0;
                for bit in [GT_DEATHMATCH, GT_TEAM_DEATHMATCH, GT_ARTEFACT_HUNT] {
                    if q.r_i32()? != 0 {
                        m |= bit;
                    }
                }
                out.map_usage = m;
            }
        }
        Ok(out)
    }
}

/// `ESceneLightTool`'s settings.
#[derive(Clone, Debug, PartialEq)]
pub struct LightTool {
    /// Altitude, longitude (radians).
    pub sun_shadow_dir: [f32; 2],
    pub lcontrol_last_idx: u32,
    /// The light controls: name and id, in order.
    pub lcontrols: Vec<(SmolStr, u32)>,
}

impl Default for LightTool {
    /// `Clear()`.
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
    /// `AppendLightControl`: a name already there is kept as it is.
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

    /// The name of a control id (`GetLControlName`).
    #[must_use]
    pub fn name_of(&self, id: u32) -> Option<&str> {
        self.lcontrols.iter().find(|(_, i)| *i == id).map(|(n, _)| n.as_str())
    }
}

/// `ESceneObjectTool`'s Append Random settings.
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

/// `flAppendRandom`: cleared on every load.
pub const FL_APPEND_RANDOM: u32 = 1 << 29;
/// `flAppendRandomScaleProportional`: one random factor for the three axes.
pub const FL_APPEND_RANDOM_PROPORTIONAL: u32 = 1 << 28;
/// `flAppendRandomScale`.
pub const FL_APPEND_RANDOM_SCALE: u32 = 1 << 30;
/// `flAppendRandomRotation`.
pub const FL_APPEND_RANDOM_ROTATION: u32 = 1 << 31;

/// The class-specific settings of a tool.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum ToolOpts {
    #[default]
    Plain,
    Light(LightTool),
    Objects(ObjectTool),
}

/// One class's tool.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolPart {
    pub class: ObjClass,
    /// `flEnable`.
    pub enabled: bool,
    /// `flVisible`: the class's eye in the Tools panel and the Outliner.
    pub visible: bool,
    /// `flReadonly`.
    pub readonly: bool,
    /// `m_ModifName` / `m_ModifTime`.
    pub modif_name: SmolStr,
    pub modif_time: u32,
    /// The tool's `m_Flags` (glow, light, portal, sector, spawn, scene objects).
    pub flags: u32,
    pub opts: ToolOpts,
    /// LTX lines of the part this crate does not read: section, key, value.
    pub kept_ltx: Vec<(String, String, String)>,
    /// Chunks of the tool's stream this crate does not read.
    pub kept_chunks: Vec<(u32, Blob)>,
    /// The LTX part was cp1251 (not UTF-8): it is written back in it, as the C++ `CInifile`
    /// writes back the bytes it read.
    pub cp1251: bool,
}

impl ToolPart {
    #[must_use]
    pub fn new(class: ObjClass) -> Self {
        let opts = match class {
            ObjClass::Light => ToolOpts::Light(LightTool::default()),
            ObjClass::SceneObject => ToolOpts::Objects(ObjectTool::default()),
            _ => ToolOpts::Plain,
        };
        Self {
            class,
            enabled: true,
            visible: true,
            readonly: false,
            modif_name: SmolStr::default(),
            modif_time: 0,
            flags: 0,
            opts,
            kept_ltx: Vec::new(),
            kept_chunks: Vec::new(),
            cp1251: false,
        }
    }

    /// `IsEditable`.
    #[must_use]
    pub fn editable(&self) -> bool {
        !self.readonly
    }

    #[must_use]
    pub fn light(&self) -> Option<&LightTool> {
        match &self.opts {
            ToolOpts::Light(l) => Some(l),
            _ => None,
        }
    }

    fn has_flags(&self) -> bool {
        matches!(
            self.class,
            ObjClass::Glow
                | ObjClass::Light
                | ObjClass::Portal
                | ObjClass::Sector
                | ObjClass::SpawnPoint
                | ObjClass::SceneObject
        )
    }

    fn version_key(&self) -> &'static str {
        if self.class == ObjClass::Glow { "glow_tool_version" } else { "version" }
    }

    /// The tool's own keys of an LTX part (`ESceneXxxTool::SaveLTX` without the objects).
    pub fn write_ltx(&self, w: &mut IniWriter) {
        w.w_string("modif", "name", &self.modif_name);
        w.w_u32("modif", "time", self.modif_time);
        if self.class == ObjClass::Group {
            w.w_u16("main", "version", 0);
        } else {
            w.w_u32("main", self.version_key(), 0);
        }
        if self.has_flags() {
            w.w_u32("main", "flags", self.flags);
        }
        match &self.opts {
            ToolOpts::Light(l) => {
                w.w_fvector2("main", "sun_shadow_dir", l.sun_shadow_dir);
                w.w_u32("main", "lcontrol_last_idx", l.lcontrol_last_idx);
                for (name, id) in &l.lcontrols {
                    w.w_u32("lcontrols", name, *id);
                }
            }
            ToolOpts::Objects(o) => {
                let s = "AppendRandom";
                w.w_fvector3(s, "AppendRandomMinScale", o.min_scale);
                w.w_fvector3(s, "AppendRandomMaxScale", o.max_scale);
                w.w_fvector3(s, "AppendRandomMinRotation", o.min_rotation);
                w.w_fvector3(s, "AppendRandomMaxRotation", o.max_rotation);
                w.w_u32(s, "AppendRandomObjects_size", u32::try_from(o.objects.len()).unwrap_or(u32::MAX));
                for (i, n) in o.objects.iter().enumerate() {
                    w.w_string(s, &format!("object_name_{i}"), n);
                }
            }
            ToolOpts::Plain => {}
        }
        for (s, k, v) in &self.kept_ltx {
            w.w_raw(s, k, v);
        }
    }

    /// The tool's own keys of an LTX part. Returns the `[main]` keys it read, so the caller
    /// can keep the rest.
    pub fn read_ltx(&mut self, r: &IniReader) -> Result<Vec<&'static str>, SceneError> {
        let desc = self.class.desc();
        let bad = |what: &str| SceneError::load(format!("{desc} tools: {what} ({})", r.file()));
        let vk = self.version_key();
        if !r.line_exist("main", vk) {
            return Err(bad("required metadata is missing"));
        }
        if r.r_u32("main", vk)? != 0 {
            return Err(bad("Unsupported version."));
        }
        // the base tool's metadata: the C++ refuses a part without it
        if !r.line_exist("modif", "name") || !r.line_exist("modif", "time") {
            return Err(bad("required metadata is missing"));
        }
        self.modif_name = r.r_string("modif", "name")?.into();
        self.modif_time = r.r_u32("modif", "time")?;
        let mut used = vec![vk, "objects_count"];
        if self.has_flags() {
            if !r.line_exist("main", "flags") {
                return Err(bad("required metadata is missing"));
            }
            self.flags = r.r_u32("main", "flags")?;
            used.push("flags");
        }
        match &mut self.opts {
            ToolOpts::Light(l) => {
                if !r.line_exist("main", "sun_shadow_dir")
                    || !r.line_exist("main", "lcontrol_last_idx")
                    || !r.section_exist("lcontrols")
                {
                    return Err(bad("required metadata is missing"));
                }
                l.sun_shadow_dir = r.r_fvector2("main", "sun_shadow_dir")?;
                l.lcontrol_last_idx = r.r_u32("main", "lcontrol_last_idx")?;
                for (k, _) in r.lines("lcontrols") {
                    let id = r.r_u32("lcontrols", &k)?;
                    l.append(&k, Some(id));
                }
                used.extend(["sun_shadow_dir", "lcontrol_last_idx"]);
            }
            ToolOpts::Objects(o) => {
                let s = "appendrandom";
                for key in [
                    "AppendRandomMinScale",
                    "AppendRandomMaxScale",
                    "AppendRandomMinRotation",
                    "AppendRandomMaxRotation",
                    "AppendRandomObjects_size",
                ] {
                    if !r.line_exist(s, key) {
                        return Err(bad("required settings are missing"));
                    }
                }
                o.min_scale = r.r_fvector3(s, "AppendRandomMinScale")?;
                o.max_scale = r.r_fvector3(s, "AppendRandomMaxScale")?;
                o.min_rotation = r.r_fvector3(s, "AppendRandomMinRotation")?;
                o.max_rotation = r.r_fvector3(s, "AppendRandomMaxRotation")?;
                let n = r.r_u32(s, "AppendRandomObjects_size")?;
                o.objects.clear();
                for i in 0..n {
                    let key = format!("object_name_{i}");
                    if !r.line_exist(s, &key) {
                        return Err(bad(&format!("required setting '{key}' is missing")));
                    }
                    o.objects.push(r.r_string(s, &key)?.into());
                }
                self.flags &= !FL_APPEND_RANDOM;
            }
            ToolOpts::Plain => {}
        }
        Ok(used)
    }

    /// The sections of an LTX part the tool itself owns (besides `main` and the objects).
    #[must_use]
    pub fn own_sections(&self) -> &'static [&'static str] {
        match self.opts {
            ToolOpts::Light(_) => &["modif", "main", "guid", "lcontrols"],
            ToolOpts::Objects(_) => &["modif", "main", "guid", "appendrandom"],
            ToolOpts::Plain => &["modif", "main", "guid"],
        }
    }

    /// The tool's chunks after the object list (`SaveStream`'s tail).
    pub fn write_stream_tail(&self, w: &mut Writer) {
        w.w_chunk(0x1001, &0u16.to_le_bytes());
        match self.class {
            ObjClass::Glow | ObjClass::Portal | ObjClass::Sector | ObjClass::SpawnPoint => {
                w.chunk(0x1002, |w| w.w_u32(self.flags));
            }
            ObjClass::Light => {
                w.chunk(0x1004, |w| w.w_u32(self.flags));
                if let ToolOpts::Light(l) = &self.opts {
                    w.chunk(0x1006, |w| {
                        w.w_u8(0);
                        w.w_f32(l.sun_shadow_dir[0]);
                        w.w_f32(l.sun_shadow_dir[1]);
                    });
                    w.chunk(0x1003, |w| w.w_u32(l.lcontrol_last_idx));
                    w.chunk(0x1002, |w| {
                        for (n, id) in &l.lcontrols {
                            w.w_stringz(n);
                            w.w_u32(*id);
                        }
                    });
                }
            }
            ObjClass::SceneObject => {
                w.chunk(0x1003, |w| w.w_u32(self.flags));
                if let ToolOpts::Objects(o) = &self.opts {
                    w.chunk(0x1002, |w| {
                        w.w_vec3(o.min_scale);
                        w.w_vec3(o.max_scale);
                        w.w_vec3(o.min_rotation);
                        w.w_vec3(o.max_rotation);
                        w.w_u32(u32::try_from(o.objects.len()).unwrap_or(u32::MAX));
                        for n in &o.objects {
                            w.w_stringz(n);
                        }
                    });
                }
            }
            _ => {}
        }
        for (id, body) in &self.kept_chunks {
            w.w_chunk(*id, body);
        }
    }

    /// The tool's chunks of a stream (`LoadStream` without the objects); unknown chunks kept.
    pub fn read_stream(&mut self, r: &Reader<'_>) -> Result<(), SceneError> {
        let desc = self.class.desc();
        if let Some(c) = r.open_chunk(0x1001)? {
            let v = c.reader().r_u16()?;
            if v != 0 {
                return Err(SceneError::load(format!("{desc} tools: Unsupported version.")));
            }
        }
        if let Some(c) = r.open_chunk(0x7777)? {
            let mut q = c.reader();
            self.modif_name = q.r_stringz()?.into();
            self.modif_time = q.r_u32()?;
        }
        let mut known: Vec<u32> = vec![0x1001, 0x7777, 2, 3];
        match self.class {
            ObjClass::Glow | ObjClass::Portal | ObjClass::Sector | ObjClass::SpawnPoint => {
                if let Some(c) = r.open_chunk(0x1002)? {
                    self.flags = c.reader().r_u32()?;
                }
                known.push(0x1002);
            }
            ObjClass::Light => {
                if let Some(c) = r.open_chunk(0x1004)? {
                    self.flags = c.reader().r_u32()?;
                }
                if let ToolOpts::Light(l) = &mut self.opts {
                    if let Some(c) = r.open_chunk(0x1006)? {
                        let mut q = c.reader();
                        q.r_u8()?;
                        l.sun_shadow_dir = [q.r_f32()?, q.r_f32()?];
                    }
                    if let Some(c) = r.open_chunk(0x1003)? {
                        l.lcontrol_last_idx = c.reader().r_u32()?;
                    }
                    if let Some(c) = r.open_chunk(0x1002)? {
                        let mut q = c.reader();
                        while !q.eof() {
                            let name = q.r_stringz()?;
                            let id = q.r_u32()?;
                            l.append(&name, Some(id));
                        }
                    }
                }
                known.extend([0x1002, 0x1003, 0x1004, 0x1006, 0x1005, 0x1007]);
            }
            ObjClass::SceneObject => {
                if let Some(c) = r.open_chunk(0x1003)? {
                    self.flags = c.reader().r_u32()?;
                }
                if let ToolOpts::Objects(o) = &mut self.opts
                    && let Some(c) = r.open_chunk(0x1002)?
                {
                    let mut q = c.reader();
                    o.min_scale = q.r_vec3()?;
                    o.max_scale = q.r_vec3()?;
                    o.min_rotation = q.r_vec3()?;
                    o.max_rotation = q.r_vec3()?;
                    let n = q.r_u32()?;
                    o.objects.clear();
                    for _ in 0..n {
                        o.objects.push(q.r_stringz()?.into());
                    }
                }
                self.flags &= !FL_APPEND_RANDOM;
                known.extend([0x1002, 0x1003]);
            }
            _ => {}
        }
        self.kept_chunks = r
            .chunks()
            .filter(|(id, _, _)| !known.contains(id))
            .map(|(id, compressed, body)| {
                (if compressed { id | gz_chunk::COMPRESSED } else { id }, Blob::from(body))
            })
            .collect();
        Ok(())
    }
}

/// Every class's tool, the current target class and the snap list.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSettings {
    pub parts: [ToolPart; CLASSES],
    /// `LTools->CurrentClassID()`: the class the Details panel and the snap list are about.
    pub target: ObjClass,
    /// `m_ESO_SnapObjects`: scene objects other objects snap to. Saved with the scene.
    pub snap: Vec<ObjId>,
    /// The Tools panel's Snap Objects switches: use the list, pick into it.
    pub snap_use: bool,
    pub snap_pick: bool,
    /// The switches of the sector, way, shape and fog volume tools' panels (not saved).
    pub obj: crate::objtools::ObjToolState,
    /// The switches of the AI map panel and the detail slots' selection (not saved).
    pub bulk: crate::bulk::BulkToolState,
    /// The compiler's error list the viewport draws (not saved; Build > Import Error List).
    pub debug: Arc<crate::debug::DebugDraw>,
}

impl Default for ToolSettings {
    fn default() -> Self {
        Self {
            parts: ObjClass::ALL.map(ToolPart::new),
            target: ObjClass::SceneObject,
            snap: Vec::new(),
            snap_use: false,
            snap_pick: false,
            obj: crate::objtools::ObjToolState::default(),
            bulk: crate::bulk::BulkToolState::default(),
            debug: Arc::new(crate::debug::DebugDraw::default()),
        }
    }
}

impl ToolSettings {
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

/// The AI Map tool's data: the nodes as the tool works on them, and packed as the files hold
/// them, both in tiles of [`AI_TILE`] that an edit shares where it left them alone.
#[derive(Clone, Debug, PartialEq)]
pub struct AiMapPart {
    /// The chunk as read; written back while nothing changed.
    pub raw: Option<Blob>,
    /// Everything but the nodes.
    pub header: SceneAiMap,
    /// The nodes packed over `header.bbox` (`SaveStream`'s arithmetic).
    pub tiles: Vec<Arc<[SceneAiNode]>>,
    /// The tool's nodes: exact positions and planes, links by index.
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

    /// The tool's nodes in order.
    #[must_use]
    pub fn all_nodes(&self) -> Vec<AiMapNode> {
        let mut out = Vec::with_capacity(self.node_count());
        for t in &self.nodes {
            out.extend_from_slice(t);
        }
        out
    }

    /// The tool's chunk: the bytes as read, or the model written again.
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

/// The Detail Objects tool's data.
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

/// The Wallmark tool's data: the chunk as written, the marks in it, and the model the tool edits
/// (`None` for a chunk this crate cannot read: kept as it is, never edited).
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

    /// The part of an edited model: its chunk written again.
    #[must_use]
    pub fn of_model(model: crate::bulk::wallmark::Wallmarks) -> Self {
        Self { raw: Blob::from(model.write()), count: model.count(), model: Some(Arc::new(model)) }
    }

    /// What an empty tool writes (`SaveStream` with no marks and the defaults of `Clear`).
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
    /// The chunk a bulk tool writes; a tool without data writes its empty state.
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

    /// `IsNeedSave` of a bulk tool.
    #[must_use]
    pub fn needs_save(&self, class: ObjClass) -> bool {
        match class {
            // `IsNeedSave`: nodes or a snap list; slots, objects, a base texture or a snap list;
            // any slot of marks
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

    /// Takes a bulk tool's chunk; a chunk that does not decode is kept as it is.
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
    fn level_options_round_trip_both_ways() {
        let lo =
            LevelOptions { level_path: "l01_escape".into(), bop: "a b".into(), ..LevelOptions::default() };
        let mut w = IniWriter::new();
        lo.write_ltx(&mut w);
        let r = IniReader::parse(&w.save(), "t").unwrap();
        assert_eq!(LevelOptions::read_ltx(&r).unwrap(), lo);

        let mut b = Writer::new();
        lo.write_bin(&mut b);
        let bytes = b.into_bytes();
        assert_eq!(LevelOptions::read_bin(&Reader::new(&bytes)).unwrap(), lo);
    }

    #[test]
    fn tool_settings_round_trip() {
        let mut t = ToolPart::new(ObjClass::Light);
        t.flags = 1 << 31;
        if let ToolOpts::Light(l) = &mut t.opts {
            l.append("my_control", None);
        }
        let mut w = IniWriter::new();
        t.write_ltx(&mut w);
        let r = IniReader::parse(&w.save(), "t").unwrap();
        let mut back = ToolPart::new(ObjClass::Light);
        back.read_ltx(&r).unwrap();
        assert_eq!(back.flags, t.flags);
        assert_eq!(back.light().unwrap().name_of(3), Some("my_control"));

        let mut s = Writer::new();
        t.write_stream_tail(&mut s);
        let bytes = s.into_bytes();
        let mut back = ToolPart::new(ObjClass::Light);
        back.read_stream(&Reader::new(&bytes)).unwrap();
        assert_eq!(back.light(), t.light());
        assert!(back.kept_chunks.is_empty());
    }

    #[test]
    fn empty_wallmarks_read_back() {
        let w = WallmarkPart::read(&WallmarkPart::empty_stream()).unwrap();
        assert_eq!(w.count, 0);
    }
}
