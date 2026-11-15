//! The X-Ray SDK scene's level options and kinds' settings, read into the scene's model.

use gz_base::SmolStr;
use gz_chunk::Reader;

use super::ltx::IniReader;
use crate::SceneError;
use crate::class::ObjClass;
use crate::tools::{BuildParams, FL_APPEND_RANDOM, LevelOptions, ToolOpts, ToolPart};

const LO_VERSION: u32 = 0x7801;
const LO_NAMES: u32 = 0x7802;
const LO_BOP: u32 = 0x7803;
const LO_PREFIX: u32 = 0x7804;
const LO_BP_VERSION: u32 = 0x7849;
const LO_BUILD_PARAMS: u32 = 0x7850;
const LO_LIGHT_QUALITY: u32 = 0x7851;
const LO_MAP_USAGE: u32 = 0x7852;
const LO_MAP_VER: u32 = 0x7853;
/// The build parameters' layout a binary scene holds.
const LEVELOP_BP_VERSION: u32 = 9;

// game-type bits of the old `usage_*` keys
const GT_DEATHMATCH: u16 = 1 << 1;
const GT_TEAM_DEATHMATCH: u16 = 1 << 2;
const GT_ARTEFACT_HUNT: u16 = 1 << 3;
const GT_CAPTURE: u16 = 1 << 4;
const GT_DOMINATION: u16 = 1 << 5;
const GT_TEAM_DOMINATION: u16 = 1 << 6;

fn build_bin(r: &mut Reader<'_>) -> Result<BuildParams, SceneError> {
    Ok(BuildParams {
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

fn build_ltx(r: &IniReader) -> Result<BuildParams, SceneError> {
    let s = "build_params";
    let mut f_reserved = [0.0; 6];
    for (i, f) in f_reserved.iter_mut().enumerate() {
        *f = r.r_float(s, &format!("reserved_{i}"))?;
    }
    Ok(BuildParams {
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

/// The `[level_options]` of an LTX scene; `Err` for one that is incomplete.
pub fn level_ltx(r: &IniReader) -> Result<LevelOptions, SceneError> {
    let s = "level_options";
    let bad = || SceneError::load(format!("Invalid LTX level options: {}", r.file()));
    for key in ["version", "level_path", "level_prefix", "bop", "light_hemi_quality", "light_sun_quality"] {
        if !r.line_exist(s, key) {
            return Err(bad());
        }
    }
    let version = r.r_u32(s, "version")?;
    if version < 8 {
        return Err(bad());
    }
    let build = build_ltx(r).map_err(|_| bad())?;
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
    Ok(LevelOptions {
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
        cell_size: 0,
    })
}

/// The level options' chunk of a binary scene.
pub fn level_bin(r: &Reader<'_>) -> Result<LevelOptions, SceneError> {
    let bad = || SceneError::load("Invalid binary level options");
    let mut out = LevelOptions::default();
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
        out.build = build_bin(&mut c.reader())?;
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

fn has_flags(class: ObjClass) -> bool {
    matches!(
        class,
        ObjClass::Glow
            | ObjClass::Light
            | ObjClass::Portal
            | ObjClass::Sector
            | ObjClass::SpawnPoint
            | ObjClass::SceneObject
    )
}

fn version_key(class: ObjClass) -> &'static str {
    if class == ObjClass::Glow { "glow_tool_version" } else { "version" }
}

/// The sections of an LTX part a kind's settings own (besides `main` and the objects).
#[must_use]
pub fn own_sections(t: &ToolPart) -> &'static [&'static str] {
    match t.opts {
        ToolOpts::Light(_) => &["modif", "main", "guid", "lcontrols"],
        ToolOpts::Objects(_) => &["modif", "main", "guid", "appendrandom"],
        ToolOpts::Plain => &["modif", "main", "guid"],
    }
}

/// A kind's own keys of an LTX part. Answers the `[main]` keys it read.
pub fn tool_ltx(t: &mut ToolPart, r: &IniReader) -> Result<Vec<&'static str>, SceneError> {
    let desc = t.class.desc();
    let bad = |what: &str| SceneError::load(format!("{desc}: {what} ({})", r.file()));
    let vk = version_key(t.class);
    if !r.line_exist("main", vk) {
        return Err(bad("required metadata is missing"));
    }
    if r.r_u32("main", vk)? != 0 {
        return Err(bad("Unsupported version."));
    }
    if !r.line_exist("modif", "name") || !r.line_exist("modif", "time") {
        return Err(bad("required metadata is missing"));
    }
    let mut used = vec![vk, "objects_count"];
    if has_flags(t.class) {
        if !r.line_exist("main", "flags") {
            return Err(bad("required metadata is missing"));
        }
        t.flags = r.r_u32("main", "flags")?;
        used.push("flags");
    }
    match &mut t.opts {
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
            t.flags &= !FL_APPEND_RANDOM;
        }
        ToolOpts::Plain => {}
    }
    Ok(used)
}

/// A kind's own chunks of a binary stream.
pub fn tool_stream(t: &mut ToolPart, r: &Reader<'_>) -> Result<(), SceneError> {
    let desc = t.class.desc();
    if let Some(c) = r.open_chunk(0x1001)?
        && c.reader().r_u16()? != 0
    {
        return Err(SceneError::load(format!("{desc}: Unsupported version.")));
    }
    match t.class {
        ObjClass::Glow | ObjClass::Portal | ObjClass::Sector | ObjClass::SpawnPoint => {
            if let Some(c) = r.open_chunk(0x1002)? {
                t.flags = c.reader().r_u32()?;
            }
        }
        ObjClass::Light => {
            if let Some(c) = r.open_chunk(0x1004)? {
                t.flags = c.reader().r_u32()?;
            }
            if let ToolOpts::Light(l) = &mut t.opts {
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
        }
        ObjClass::SceneObject => {
            if let Some(c) = r.open_chunk(0x1003)? {
                t.flags = c.reader().r_u32()?;
            }
            if let ToolOpts::Objects(o) = &mut t.opts
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
            t.flags &= !FL_APPEND_RANDOM;
        }
        _ => {}
    }
    Ok(())
}
