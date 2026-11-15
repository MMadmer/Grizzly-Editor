//! The scene's own files (`docs/assets/SCENE.md`, the contract in `docs/ASSETS.md` section 7).
//!
//! A scene is `<name>.scene` - Lua data with the header `scene = 1`: the level's options, its
//! build settings, the sun, the light controls, the camera, the kinds' eyes and locks - and the
//! folder `<name>\` beside it: one Lua data file per kind of object (`objects.lua`,
//! `lights.lua`, ... - a file only while it holds something) and the painted layers as binary
//! files ([`layers`]).
//!
//! Reading takes the texts and gives a [`SceneState`] with the documents it was read from
//! ([`SceneFiles`]); writing gives the files of a save. A save of a scene read from its files
//! applies the difference ([`gz_luadata::LuaDoc::apply_diff`]): an object nobody changed keeps
//! its bytes, the author's comments and keys the editor does not know stay where they are, and
//! a file with nothing changed is not written.

pub mod doc;
pub mod fields;
pub mod group;
pub mod layers;
pub mod spawn;
pub mod values;

use std::sync::Arc;

use gz_base::{Problem, RelPath, SmolStr};
use gz_entities::Settings;
use gz_luadata::{DiffRules, Limits, LuaDoc, Seg, Table, Value};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::class::ObjClass;
use crate::model::{ObjData, ObjId, Object, SceneState, co};
use crate::objects::{SpawnEntity, default_data};
use crate::tools::{LevelOptions, ToolOpts};
use fields::{Field, Ty};
use values::*;

/// The extension of a scene's main file.
pub const EXT: &str = "scene";
/// The header key, and the version this editor writes and the newest it reads.
pub const HEADER: &str = "scene";
pub const VERSION: u32 = 1;

/// One file of the scene's folder: its name and the lists it holds, each of one class.
#[derive(Clone, Copy, Debug)]
pub struct ClassFile {
    pub file: &'static str,
    pub lists: &'static [(&'static str, ObjClass)],
}

pub const CLASS_FILES: [ClassFile; 11] = [
    ClassFile { file: "objects.lua", lists: &[("objects", ObjClass::SceneObject)] },
    ClassFile { file: "lights.lua", lists: &[("lights", ObjClass::Light)] },
    ClassFile { file: "glows.lua", lists: &[("glows", ObjClass::Glow)] },
    ClassFile {
        file: "sounds.lua",
        lists: &[("sources", ObjClass::SoundSrc), ("environments", ObjClass::SoundEnv)],
    },
    ClassFile { file: "particles.lua", lists: &[("particles", ObjClass::Particles)] },
    ClassFile { file: "shapes.lua", lists: &[("shapes", ObjClass::Shape)] },
    ClassFile { file: "fog.lua", lists: &[("volumes", ObjClass::FogVolume)] },
    ClassFile { file: "spawns.lua", lists: &[("spawns", ObjClass::SpawnPoint)] },
    ClassFile { file: "ways.lua", lists: &[("ways", ObjClass::Way)] },
    ClassFile { file: "zones.lua", lists: &[("sectors", ObjClass::Sector), ("portals", ObjClass::Portal)] },
    ClassFile { file: "groups.lua", lists: &[("groups", ObjClass::Group)] },
];

/// The file and the list of a class.
#[must_use]
pub fn file_of(class: ObjClass) -> Option<(&'static str, &'static str)> {
    CLASS_FILES.iter().find_map(|f| f.lists.iter().find(|(_, c)| *c == class).map(|(l, _)| (f.file, *l)))
}

/// `<dir>\<stem>\`: the scene's folder.
#[must_use]
pub fn folder_of(scene: &std::path::Path) -> std::path::PathBuf {
    let stem = scene.file_stem().map(|s| s.to_os_string()).unwrap_or_default();
    scene.parent().map_or_else(|| std::path::PathBuf::from(&stem), |p| p.join(&stem))
}

/// Every file name a scene's folder may hold.
#[must_use]
pub fn folder_files() -> Vec<&'static str> {
    CLASS_FILES.iter().map(|f| f.file).chain(layers::LAYERS.iter().map(|l| l.file)).collect()
}

/// How the scene's files map onto the model when a save applies the difference: the lists are
/// matched by `name`.
#[must_use]
pub fn class_rules(cf: &ClassFile) -> DiffRules {
    let mut r = DiffRules::new();
    for (list, _) in cf.lists {
        r = r.keyed(list, "name");
    }
    r.keyed("ways.*.points", "name")
}

#[must_use]
pub fn scene_rules() -> DiffRules {
    DiffRules::new().order(
        "",
        &[
            HEADER,
            "level",
            "build",
            "sun",
            "light_controls",
            "camera",
            "classes",
            "view_flags",
            "random_placement",
            "layers",
        ],
    )
}

// ---------------------------------------------------------------- the .scene

fn opts(s: &SceneState) -> &LevelOptions {
    &s.meta.level
}

const QUALITY: &[(&str, u32)] = &[("draft", 0), ("high", 1), ("custom", 2)];

macro_rules! level {
    ($key:literal, $label:literal, $group:literal, $unit:literal, $ty:expr, $default:literal, $help:literal,
     |$l:ident| $get:expr, |$lm:ident, $v:ident| $set:expr) => {
        Field {
            key: $key,
            label: $label,
            group: $group,
            unit: $unit,
            ty: $ty,
            default: $default,
            help: $help,
            applies: |_| true,
            get: |s| {
                let $l = opts(s);
                $get
            },
            set: |s, $v| {
                let $lm = &mut s.meta.level;
                $set;
                Ok(())
            },
        }
    };
}

fn sun(s: &SceneState) -> [f32; 2] {
    s.tools
        .part(ObjClass::Light)
        .light()
        .map_or(crate::tools::LightTool::default().sun_shadow_dir, |l| l.sun_shadow_dir)
}

fn sun_mut(s: &mut SceneState) -> Option<&mut [f32; 2]> {
    s.tools.part_mut(ObjClass::Light).light_mut().map(|l| &mut l.sun_shadow_dir)
}

/// The keys of the `.scene` (dotted: `level.path` is `path` of the table `level`).
pub static SCENE_FIELDS: &[Field<SceneState>] = &[
    level!(
        "level.path",
        "Level Folder",
        "Level",
        "",
        Ty::Text,
        "\"level\"",
        "The folder of the built level under the game's `levels`.",
        |l| text(&l.level_path),
        |l, v| l.level_path = as_text(v)?
    ),
    level!(
        "level.prefix",
        "Name Prefix",
        "Level",
        "",
        Ty::Text,
        "\"level_prefix\"",
        "What the level's own names start with.",
        |l| text(&l.level_prefix),
        |l, v| l.level_prefix = as_text(v)?
    ),
    level!(
        "level.map_version",
        "Map Version",
        "Level",
        "",
        Ty::Text,
        "\"1.0\"",
        "The version the level's `level.ltx` gives in `[map_usage]`.",
        |l| text(&l.map_version),
        |l, v| l.map_version = as_text(v)?
    ),
    level!(
        "level.custom_data",
        "Level Settings Lines",
        "Level",
        "",
        Ty::Text,
        "\"\"",
        "Lines the build adds to the level's `level.ltx` as they are.",
        |l| text(&l.bop),
        |l, v| l.bop = as_text(v)?.to_string()
    ),
    level!(
        "level.game_types",
        "Game Types",
        "Level",
        "",
        fields::GAME_TYPES_TY,
        "every one",
        "The game types the level is made for.",
        |l| fields::game_types(l.map_usage),
        |l, v| l.map_usage = fields::game_types_of(v)?
    ),
    level!(
        "build.quality",
        "Quality",
        "Build",
        "",
        Ty::Words(QUALITY),
        "\"high\"",
        "Draft and high set the values below; custom keeps them as they are set.",
        |l| word(u32::from(l.build.quality), QUALITY),
        |l, v| l.build.quality = as_word_or_number(v, QUALITY)? as u16
    ),
    level!(
        "build.sky_quality",
        "Sky Light Quality",
        "Build",
        "",
        Ty::Integer { min: 0, max: 3 },
        "3",
        "How finely the build samples the light of the sky.",
        |l| int(l.light_hemi_quality),
        |l, v| l.light_hemi_quality = as_u8(v)?
    ),
    level!(
        "build.sun_quality",
        "Sun Quality",
        "Build",
        "",
        Ty::Integer { min: 0, max: 3 },
        "3",
        "How finely the build samples the sun.",
        |l| int(l.light_sun_quality),
        |l, v| l.light_sun_quality = as_u8(v)?
    ),
    level!(
        "build.pixels_per_meter",
        "Lightmap Density",
        "Build",
        "px/m",
        Ty::Number { min: 0.0, max: 20.0, dec: 2 },
        "10",
        "Lightmap pixels per metre of surface.",
        |l| num(l.build.lm_pixels_per_meter),
        |l, v| l.build.lm_pixels_per_meter = as_f32(v)?
    ),
    level!(
        "build.jitter_samples",
        "Samples per Pixel",
        "Build",
        "",
        Ty::Words(&[("1", 1), ("4", 4), ("9", 9)]),
        "9",
        "Light samples per lightmap pixel, 1, 4 or 9: more is smoother and slower to build.",
        |l| int(l.build.lm_jitter_samples),
        |l, v| l.build.lm_jitter_samples = as_u32(v)?
    ),
    level!(
        "build.smooth_angle",
        "Smoothing Angle",
        "Build",
        "deg",
        Ty::Number { min: 0.0, max: 180.0, dec: 2 },
        "75",
        "Faces that meet at a smaller angle are shaded smooth.",
        |l| num(l.build.sm_angle),
        |l, v| l.build.sm_angle = as_f32(v)?
    ),
    level!(
        "build.weld_distance",
        "Weld Distance",
        "Build",
        "m",
        Ty::Number { min: 0.0, max: 1.0, dec: 4 },
        "0.005",
        "Vertices closer than this are merged.",
        |l| num(l.build.weld_distance),
        |l, v| l.build.weld_distance = as_f32(v)?
    ),
    level!(
        "build.collapse_error",
        "Lightmap Collapse Error",
        "Build",
        "",
        Ty::Integer { min: 0, max: 255 },
        "4",
        "How much a lightmap may differ from a flat colour to be collapsed into one.",
        |l| int(l.build.lm_rms),
        |l, v| l.build.lm_rms = as_u32(v)?
    ),
    level!(
        "build.zero_error",
        "Lightmap Zero Error",
        "Build",
        "",
        Ty::Integer { min: 0, max: 255 },
        "4",
        "How dark a lightmap may be to be dropped as black.",
        |l| int(l.build.lm_rms_zero),
        |l, v| l.build.lm_rms_zero = as_u32(v)?
    ),
    level!(
        "build.cell_size",
        "Streaming Cells",
        "Build",
        "m",
        Ty::Integer { min: 0, max: 4096 },
        "0",
        "The game loads the level's picture cell by cell around the player: the size of a cell. 0 keeps the level one piece.",
        |l| int(l.cell_size),
        |l, v| l.cell_size = as_u32(v)?
    ),
    level!(
        "build.reserved",
        "Kept Build Values",
        "Build",
        "",
        Ty::Table("a list of 6 whole numbers: the bits of the floats"),
        "none",
        "Values of the build parameters no key names, kept bit for bit as an X-Ray SDK scene had them.",
        |l| if l.build.f_reserved.iter().all(|x| x.to_bits() == 0) {
            Value::Nil
        } else {
            list(l.build.f_reserved.iter().map(|x| int(x.to_bits())))
        },
        |l, v| {
            let mut r = [0f32; 6];
            for (o, x) in r.iter_mut().zip(v.arr()) {
                *o = f32::from_bits(as_u32(x)?);
            }
            l.build.f_reserved = r;
        }
    ),
    level!(
        "build.reserved_word",
        "Kept Build Word",
        "Build",
        "",
        Ty::Integer { min: 0, max: 65535 },
        "0",
        "A word of the build parameters no key names.",
        |l| int(l.build.u_reserved),
        |l, v| l.build.u_reserved = as_u16(v)?
    ),
    Field {
        key: "sun.altitude",
        label: "Sun Altitude",
        group: "Sun",
        unit: "deg",
        ty: Ty::Angle { min: -90.0, max: 0.0 },
        default: "-25",
        help: "How high the sun stands for the build's sun shadows.",
        applies: |_| true,
        get: |s| angle(sun(s)[0]),
        set: |s, v| {
            let a = as_angle(v)?;
            if let Some(d) = sun_mut(s) {
                d[0] = a;
            }
            Ok(())
        },
    },
    Field {
        key: "sun.longitude",
        label: "Sun Longitude",
        group: "Sun",
        unit: "deg",
        ty: Ty::Angle { min: 0.0, max: 360.0 },
        default: "292",
        help: "Where the sun stands round the horizon.",
        applies: |_| true,
        get: |s| angle(sun(s)[1]),
        set: |s, v| {
            let a = as_angle(v)?;
            if let Some(d) = sun_mut(s) {
                d[1] = a;
            }
            Ok(())
        },
    },
    Field {
        key: "light_controls",
        label: "Light Controls",
        group: "Lights",
        unit: "",
        ty: Ty::Table("a list of names"),
        default: "none",
        help: "The scene's own groups of lights besides `$static`, `$hemi` and `$sun`; a light names its group in `control`.",
        applies: |_| true,
        get: |s| match s.tools.part(ObjClass::Light).light() {
            Some(l) => list(
                l.lcontrols
                    .iter()
                    .filter(|(n, _)| !matches!(n.as_str(), "$static" | "$hemi" | "$sun"))
                    .map(|(n, _)| text(n)),
            ),
            None => Value::Nil,
        },
        set: |s, v| {
            let names: Vec<SmolStr> = v.arr().iter().map(as_text).collect::<Result<_, _>>()?;
            if let Some(l) = s.tools.part_mut(ObjClass::Light).light_mut() {
                for n in names {
                    l.append(&n, None);
                }
            }
            Ok(())
        },
    },
    Field {
        key: "camera.pos",
        label: "Camera Position",
        group: "Camera",
        unit: "m",
        ty: Ty::Vec3,
        default: "{ 0, 0, 0 }",
        help: "Where the viewport's camera stood when the scene was saved.",
        applies: |_| true,
        get: |s| vec3(s.meta.camera_pos),
        set: |s, v| {
            s.meta.camera_pos = as_vec3(v)?;
            Ok(())
        },
    },
    Field {
        key: "camera.heading",
        label: "Camera Heading",
        group: "Camera",
        unit: "deg",
        ty: Ty::Angle { min: -360.0, max: 360.0 },
        default: "0",
        help: "Where the camera looked, round the vertical.",
        applies: |_| true,
        get: |s| angle(s.meta.camera_hpb.x),
        set: |s, v| {
            s.meta.camera_hpb.x = as_angle(v)?;
            Ok(())
        },
    },
    Field {
        key: "camera.pitch",
        label: "Camera Pitch",
        group: "Camera",
        unit: "deg",
        ty: Ty::Angle { min: -90.0, max: 90.0 },
        default: "0",
        help: "How far the camera looked up or down.",
        applies: |_| true,
        get: |s| angle(s.meta.camera_hpb.y),
        set: |s, v| {
            s.meta.camera_hpb.y = as_angle(v)?;
            Ok(())
        },
    },
    Field {
        key: "camera.bank",
        label: "Camera Bank",
        group: "Camera",
        unit: "deg",
        ty: Ty::Angle { min: -180.0, max: 180.0 },
        default: "0",
        help: "The camera's roll.",
        applies: |_| true,
        get: |s| angle(s.meta.camera_hpb.z),
        set: |s, v| {
            s.meta.camera_hpb.z = as_angle(v)?;
            Ok(())
        },
    },
    Field {
        key: "classes",
        label: "Kinds",
        group: "Editor",
        unit: "",
        ty: Ty::Table(
            "`{ <kind> = { hidden = true, locked = true } }` - kinds: `object`, `light`, `glow`, `sound`, `sound_env`, `particles`, `shape`, `fog`, `spawn`, `way`, `sector`, `portal`, `group`, `ai_map`, `details`, `wallmarks`",
        ),
        default: "none",
        help: "The kinds of objects that are hidden or locked in the viewport.",
        applies: |_| true,
        get: |s| {
            let mut t = Table::new();
            for c in ObjClass::ALL {
                let p = s.tools.part(c);
                if !p.visible || p.locked {
                    let mut e = Table::new();
                    if !p.visible {
                        e.put("hidden", true);
                    }
                    if p.locked {
                        e.put("locked", true);
                    }
                    t.put(c.key(), Value::Table(e));
                }
            }
            Value::Table(t)
        },
        set: |s, v| {
            for (k, e) in v.fields() {
                let Some(c) = ObjClass::by_key(k) else { continue };
                let p = s.tools.part_mut(c);
                p.visible = !e.get("hidden").map(as_bool).transpose()?.unwrap_or(false);
                p.locked = e.get("locked").map(as_bool).transpose()?.unwrap_or(false);
            }
            Ok(())
        },
    },
    Field {
        key: "view_flags",
        label: "Viewport Flags",
        group: "Editor",
        unit: "",
        ty: Ty::Table("`{ <kind> = n }`"),
        default: "none",
        help: "What the viewport draws of each kind (names, crosses, solid sectors), as the kinds' flags.",
        applies: |_| true,
        get: |s| {
            let mut t = Table::new();
            for c in ObjClass::ALL {
                let f = s.tools.part(c).flags & !random_bits(c);
                if f != 0 {
                    t.put(c.key(), int(f));
                }
            }
            Value::Table(t)
        },
        set: |s, v| {
            for (k, f) in v.fields() {
                let Some(c) = ObjClass::by_key(k) else { continue };
                let p = s.tools.part_mut(c);
                p.flags = (p.flags & random_bits(c)) | (as_u32(f)? & !random_bits(c));
            }
            Ok(())
        },
    },
    Field {
        key: "random_placement",
        label: "Random Placement",
        group: "Editor",
        unit: "",
        ty: Ty::Table(
            "`{ on = true, scale = { {min}, {max} }, rotation = { {min}, {max} }, proportional = true, objects = { ref, ... } }`",
        ),
        default: "off",
        help: "What placing a library object picks at random: the object of a list, its scale and its rotation.",
        applies: |_| true,
        get: random_value,
        set: set_random,
    },
    Field {
        key: "layers",
        label: "Painted Layers",
        group: "Editor",
        unit: "",
        ty: Ty::Table(
            "`{ ai_map = \"ai_map.bin\", details = \"details.bin\", wallmarks = \"wallmarks.bin\" }`",
        ),
        default: "none",
        help: "The painted layers' files in the scene's folder.",
        applies: |_| true,
        get: |s| {
            let mut t = Table::new();
            for l in layers::LAYERS {
                if layers::has(l, &s.bulk) {
                    t.put(l.key, l.file);
                }
            }
            Value::Table(t)
        },
        set: |_, _| Ok(()),
    },
];

/// The bits of a kind's flags `random_placement` says: the scene objects' random placement.
fn random_bits(c: ObjClass) -> u32 {
    if c == ObjClass::SceneObject { crate::tools::FL_APPEND_RANDOM_ALL } else { 0 }
}

fn random_value(s: &SceneState) -> Value {
    use crate::tools::{
        FL_APPEND_RANDOM, FL_APPEND_RANDOM_PROPORTIONAL, FL_APPEND_RANDOM_ROTATION, FL_APPEND_RANDOM_SCALE,
    };
    let p = s.tools.part(ObjClass::SceneObject);
    let ToolOpts::Objects(o) = &p.opts else { return Value::Nil };
    let d = crate::tools::ObjectTool::default();
    let mut t = Table::new();
    if p.flags & FL_APPEND_RANDOM != 0 {
        t.put("on", true);
    }
    if o.min_scale != d.min_scale || o.max_scale != d.max_scale {
        t.put("scale", list([vec3(o.min_scale), vec3(o.max_scale)]));
    }
    if o.min_rotation != d.min_rotation || o.max_rotation != d.max_rotation {
        t.put("rotation", list([angles(o.min_rotation), angles(o.max_rotation)]));
    }
    for (k, bit) in [
        ("random_scale", FL_APPEND_RANDOM_SCALE),
        ("proportional", FL_APPEND_RANDOM_PROPORTIONAL),
        ("random_rotation", FL_APPEND_RANDOM_ROTATION),
    ] {
        if p.flags & bit != 0 {
            t.put(k, true);
        }
    }
    if !o.objects.is_empty() {
        t.put("objects", list(o.objects.iter().map(|n| text(n))));
    }
    Value::Table(t)
}

fn set_random(s: &mut SceneState, v: &Value) -> Result<(), Bad> {
    use crate::tools::{
        FL_APPEND_RANDOM, FL_APPEND_RANDOM_PROPORTIONAL, FL_APPEND_RANDOM_ROTATION, FL_APPEND_RANDOM_SCALE,
    };
    let p = s.tools.part_mut(ObjClass::SceneObject);
    let pair = |k: &str,
                f: fn(&Value) -> Result<crate::math::Vec3, Bad>|
     -> Result<Option<(crate::math::Vec3, crate::math::Vec3)>, Bad> {
        match v.get(k) {
            None => Ok(None),
            Some(x) => match x.arr() {
                [a, b] => Ok(Some((f(a)?, f(b)?))),
                _ => Err(format!("{k} takes {{ min, max }}")),
            },
        }
    };
    let scale = pair("scale", as_vec3)?;
    let rotation = pair("rotation", as_angles)?;
    let objects: Option<Vec<SmolStr>> =
        v.get("objects").map(|x| x.arr().iter().map(as_text).collect::<Result<_, _>>()).transpose()?;
    for (k, bit) in [
        ("on", FL_APPEND_RANDOM),
        ("random_scale", FL_APPEND_RANDOM_SCALE),
        ("proportional", FL_APPEND_RANDOM_PROPORTIONAL),
        ("random_rotation", FL_APPEND_RANDOM_ROTATION),
    ] {
        if v.get(k).map(as_bool).transpose()?.unwrap_or(false) {
            p.flags |= bit;
        } else {
            p.flags &= !bit;
        }
    }
    if let ToolOpts::Objects(o) = &mut p.opts {
        if let Some((a, b)) = scale {
            (o.min_scale, o.max_scale) = (a, b);
        }
        if let Some((a, b)) = rotation {
            (o.min_rotation, o.max_rotation) = (a, b);
        }
        if let Some(list) = objects {
            o.objects = list;
        }
    }
    Ok(())
}

/// The path of a dotted key.
fn segs(key: &str) -> Vec<Seg> {
    key.split('.').map(Seg::key).collect()
}

fn get_dotted<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    key.split('.').try_fold(v, |at, k| at.get(k))
}

fn put_dotted(t: &mut Table, key: &str, v: Value) {
    match key.split_once('.') {
        None => t.put(key, v),
        Some((head, rest)) => {
            if t.get(head).is_none() {
                t.put(head, Value::table());
            }
            if let Some(Value::Table(sub)) = t.get_mut(head) {
                put_dotted(sub, rest, v);
            }
        }
    }
}

/// The `.scene`'s value of a state: the header, then every key that is not its default.
#[must_use]
pub fn scene_value(s: &SceneState) -> Value {
    let base = SceneState::default();
    let mut t = Table::new();
    t.put(HEADER, int(VERSION));
    for f in SCENE_FIELDS {
        let v = (f.get)(s);
        if v.is_nil() || v.is_empty() && v.is_table() || gz_luadata::data_eq(&v, &(f.get)(&base)) {
            continue;
        }
        put_dotted(&mut t, f.key, v);
    }
    Value::Table(t)
}

// ---------------------------------------------------------------- objects

/// What encoding an object needs beside it.
pub struct Cx<'a> {
    pub state: &'a SceneState,
    pub settings: Option<&'a (dyn Settings + Sync)>,
}

/// A new object of a class: what its keys are told against.
#[must_use]
pub fn default_object(class: ObjClass) -> Object {
    Object::new(class, "", default_data(class))
}

/// An object as its file's item: `name` first, then every key that applies and is not its
/// default.
pub fn object_value(cx: &Cx<'_>, o: &Object, base: &Object) -> Result<Value, String> {
    let mut t = Table::new();
    let entity = if o.class == ObjClass::SpawnPoint { spawn::entity_text(o, cx.settings)? } else { None };
    for f in fields::object_fields(o.class) {
        let applies = (f.applies)(o);
        let special = matches!(
            (o.class, f.key),
            (ObjClass::Light, "control")
                | (ObjClass::Group, "members")
                | (ObjClass::SpawnPoint, "props" | "raw" | "values" | "section")
        );
        // a key every kind has is said only where it means something; a value of the kind's own
        // data is kept while it is not the default, even where the kind does not use it
        let common = fields::COMMON.iter().any(|c| c.key == f.key);
        if !applies && (special || common) {
            continue;
        }
        let v = match (o.class, f.key) {
            (ObjClass::Light, "control") => fields::light_control(cx.state, o),
            (ObjClass::Group, "members") => fields::group_members(cx.state, o),
            (ObjClass::SpawnPoint, "props") => entity.as_ref().map_or(Value::Nil, |e| e.props.clone()),
            (ObjClass::SpawnPoint, "raw") => {
                entity.as_ref().and_then(|e| e.raw.as_deref()).map_or(Value::Nil, values::hex)
            }
            (ObjClass::SpawnPoint, "section") => entity.as_ref().map_or(Value::Nil, |e| text(&e.section)),
            (ObjClass::SpawnPoint, "values") => entity
                .as_ref()
                .and_then(|e| e.values.as_ref())
                .map_or(Value::Nil, |v| values::list(v.iter().map(|x| text(x)))),
            _ => {
                let v = (f.get)(o);
                // a value its kind does not use is kept while it is not the default
                let default = gz_luadata::data_eq(&v, &(f.get)(base));
                if f.key != "name" && default && ((f.applies)(base) || !applies) {
                    continue;
                }
                v
            }
        };
        if v.is_nil() || (v.is_table() && v.is_empty() && !matches!(f.key, "shapes" | "surfaces")) {
            continue;
        }
        t.put(f.key, v);
    }
    Ok(Value::Table(t))
}

/// A problem at a key of an item of a list.
fn at_item(
    file: &RelPath,
    doc: &LuaDoc,
    list: &str,
    name: Option<&str>,
    index: usize,
    key: Option<&str>,
    p: Problem,
) -> Problem {
    let mut path = vec![Seg::key(list), name.map_or(Seg::Index(index), |n| Seg::item("name", n))];
    if let Some(k) = key {
        path.push(Seg::key(k));
    }
    let s = doc.nearest_span(&path);
    p.in_file(file.clone()).at(s.line, s.column, s.end_line, s.end_column)
}

/// An item of a list as an object, with what is wrong with it.
fn object_of(item: &Value, class: ObjClass, problems: &mut Vec<(Option<&'static str>, Problem)>) -> Object {
    let mut o = default_object(class);
    let Some(t) = item.as_table() else {
        problems.push((None, Problem::error("S011", "an item of the list is not a table")));
        return o;
    };
    for (k, v) in &t.fields {
        match fields::object_field(class, k) {
            Some(f) => {
                if let Err(why) = (f.set)(&mut o, v) {
                    problems.push((
                        Some(f.key),
                        Problem::error("S011", format!("'{k}' {why}: the default is kept")),
                    ));
                }
            }
            None => problems.push((
                None,
                Problem::note("unknown_key", format!("unknown key '{k}' of a {} (kept)", class.label())),
            )),
        }
    }
    if !t.arr.is_empty() {
        problems.push((
            None,
            Problem::warning("S011", "the item holds values without keys: they are kept, not read"),
        ));
    }
    o
}

// ---------------------------------------------------------------- reading

/// The texts of a scene: its `.scene`, its folder's Lua files and layer files by name.
#[derive(Clone, Debug, Default)]
pub struct Texts {
    pub scene: String,
    pub files: Vec<(String, String)>,
    pub layers: Vec<(String, Vec<u8>)>,
}

/// A document of the scene and the file it is.
#[derive(Clone)]
pub struct Doc {
    pub file: &'static str,
    pub doc: LuaDoc,
}

/// The documents a scene was read from (or last saved to) and the state they hold: a save
/// writes the difference between that state and the one there is then.
#[derive(Clone)]
pub struct SceneFiles {
    pub scene: LuaDoc,
    pub docs: Vec<Doc>,
    pub base: SceneState,
}

/// A scene read from its files.
pub struct Read {
    pub state: SceneState,
    pub files: SceneFiles,
    pub problems: Vec<Problem>,
}

/// Why a scene was not read at all: its `.scene` does not parse or is of a newer editor.
#[derive(Clone, Debug)]
pub struct Refused {
    pub code: &'static str,
    pub problem: Box<Problem>,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.problem.line_text())
    }
}

fn syntax(file: RelPath, e: gz_luadata::LuaDataError) -> Refused {
    Refused {
        code: "syntax",
        problem: Box::new(Problem::error("syntax", e.message).in_file(file).at(
            e.line.max(1),
            e.column.max(1),
            e.line.max(1),
            e.column.max(1) + 1,
        )),
    }
}

/// The version a `.scene` says it is written in, or why it says none.
fn header_version(v: &Value) -> Option<u32> {
    match v.get(HEADER) {
        Some(Value::Number(n)) if n.fract() == 0.0 && *n >= 1.0 => Some(*n as u32),
        _ => None,
    }
}

/// Reads a scene. `dir` is the folder's path as problems name it (`levels/escape`), `main` the
/// `.scene`'s. Objects whose kind is in a file that does not read are left out, with the
/// reader's sentence; a `.scene` that does not read refuses the whole scene.
pub fn read(
    texts: &Texts,
    main: &RelPath,
    dir: &RelPath,
    settings: Option<&(dyn Settings + Sync)>,
) -> Result<Read, Refused> {
    let (scene_doc, scene_v) =
        LuaDoc::parse_value(&texts.scene, HEADER, Limits::scene()).map_err(|e| syntax(main.clone(), e))?;
    let mut problems = Vec::new();
    match header_version(&scene_v) {
        Some(v) if v > VERSION => {
            let s = scene_doc.span(&[Seg::key(HEADER)]).unwrap_or_else(|| scene_doc.head_span());
            return Err(Refused {
                code: "newer_version",
                problem: Problem::error(
                    "newer_version",
                    format!("{main} was made by a newer Grizzly Editor (scene = {v}; this one reads up to {VERSION}): update the editor to open it"),
                )
                .in_file(main.clone())
                .at(s.line, s.column, s.end_line, s.end_column)
                .into(),
            });
        }
        Some(_) => {}
        None => {
            let s = scene_doc.head_span();
            problems.push(
                Problem::error(
                    "E001",
                    format!("the header '{HEADER} = {VERSION}' is missing (saving adds it)"),
                )
                .with_fix(format!("{HEADER} = {VERSION}"))
                .in_file(main.clone())
                .at(s.line, s.column, s.end_line, s.end_column),
            );
        }
    }
    let mut state = SceneState::default();
    for f in SCENE_FIELDS {
        if let Some(v) = get_dotted(&scene_v, f.key)
            && let Err(why) = (f.set)(&mut state, v)
        {
            let s = scene_doc.nearest_span(&segs(f.key));
            problems.push(
                Problem::error("S011", format!("'{}' {why}: the default is kept", f.key))
                    .in_file(main.clone())
                    .at(s.line, s.column, s.end_line, s.end_column),
            );
        }
    }
    for (k, _) in scene_v.fields() {
        if k != HEADER && !SCENE_FIELDS.iter().any(|f| f.key.split('.').next() == Some(k.as_str())) {
            let s = scene_doc.nearest_span(&[Seg::key(k)]);
            problems.push(
                Problem::note("unknown_key", format!("unknown key '{k}' (kept)")).in_file(main.clone()).at(
                    s.line,
                    s.column,
                    s.end_line,
                    s.end_column,
                ),
            );
        }
    }

    // every class file in parallel: its document and its objects as read
    struct Part {
        file: &'static str,
        doc: LuaDoc,
        objects: Vec<(ObjClass, Object)>,
        problems: Vec<Problem>,
        controls: Vec<(usize, SmolStr)>,
        members: Vec<(usize, Vec<SmolStr>)>,
    }
    let parts: Vec<Result<Part, Box<Problem>>> = CLASS_FILES
        .par_iter()
        .filter_map(|cf| {
            texts.files.iter().find(|(n, _)| n.eq_ignore_ascii_case(cf.file)).map(|(_, t)| (cf, t))
        })
        .map(|(cf, text)| {
            let rel = dir.join(cf.file).unwrap_or_else(|_| dir.clone());
            let (doc, v) = LuaDoc::parse_value(text, cf.file, Limits::scene())
                .map_err(|e| syntax(rel.clone(), e).problem)?;
            let mut part = Part {
                file: cf.file,
                doc,
                objects: Vec::new(),
                problems: Vec::new(),
                controls: Vec::new(),
                members: Vec::new(),
            };
            for (k, _) in v.fields() {
                if !cf.lists.iter().any(|(l, _)| l == k) {
                    let s = part.doc.nearest_span(&[Seg::key(k)]);
                    part.problems.push(
                        Problem::note("unknown_key", format!("unknown key '{k}' (kept)"))
                            .in_file(rel.clone())
                            .at(s.line, s.column, s.end_line, s.end_column),
                    );
                }
            }
            for (list, class) in cf.lists {
                let Some(items) = v.get(list) else { continue };
                for (i, item) in items.arr().iter().enumerate() {
                    let mut found = Vec::new();
                    let mut o = object_of(item, *class, &mut found);
                    let name = item.get("name").and_then(Value::as_str).map(str::to_owned);
                    if name.is_none() {
                        found.push((
                            None,
                            Problem::error(
                                "S012",
                                format!("a {} without a name: it is named for now", class.label()),
                            ),
                        ));
                    }
                    for (key, p) in found {
                        part.problems.push(at_item(&rel, &part.doc, list, name.as_deref(), i, key, p));
                    }
                    let n = part.objects.len();
                    if *class == ObjClass::Light
                        && let Some(c) = item.get("control").and_then(Value::as_str)
                    {
                        part.controls.push((n, SmolStr::new(c)));
                    }
                    if *class == ObjClass::Group {
                        let m: Vec<SmolStr> = item
                            .get("members")
                            .map(|x| x.arr().iter().filter_map(Value::as_str).map(SmolStr::new).collect())
                            .unwrap_or_default();
                        part.members.push((n, m));
                    }
                    if o.name.is_empty() {
                        o.name = SmolStr::new(class.key());
                    }
                    if let (ObjData::Spawn(_), Some(st)) = (&o.data, settings) {
                        spawn::resolve(&mut o, st);
                    }
                    part.objects.push((*class, o));
                }
            }
            Ok(part)
        })
        .collect();

    let mut docs = Vec::new();
    let mut groups: Vec<(ObjId, Vec<SmolStr>)> = Vec::new();
    for part in parts {
        let mut part = match part {
            Ok(p) => p,
            Err(p) => {
                problems.push(*p);
                continue;
            }
        };
        problems.append(&mut part.problems);
        let mut ids = Vec::with_capacity(part.objects.len());
        for (i, (_, mut o)) in part.objects.into_iter().enumerate() {
            if let Some((_, c)) = part.controls.iter().find(|(n, _)| *n == i)
                && let ObjData::Light(d) = &mut o.data
            {
                d.lcontrol = match fields::light_control_of(&state, &Value::from(c.as_str())) {
                    Ok(id) => id,
                    Err(_) => fields::light_control_added(&mut state, c),
                };
            }
            if !state.name_free(&o.name) {
                let fresh = state.unique_name(&o.name);
                problems.push(
                    Problem::warning(
                        "S013",
                        format!(
                            "the name '{}' is used twice: this {} is '{fresh}' now",
                            o.name,
                            o.class.label()
                        ),
                    )
                    .in_file(dir.join(part.file).unwrap_or_else(|_| dir.clone())),
                );
                o.name = SmolStr::new(fresh);
            }
            if let ObjData::Spawn(d) = &o.data
                && let Some(SpawnEntity::Kept { why, .. }) = &d.entity
                && !why.is_empty()
            {
                problems.push(
                    Problem::warning(
                        "S021",
                        format!("the spawn point '{}' is kept as the file has it: {why}", o.name),
                    )
                    .in_file(dir.join(part.file).unwrap_or_else(|_| dir.clone())),
                );
            }
            ids.push(state.insert(o));
        }
        for (i, m) in part.members {
            if let Some(id) = ids.get(i) {
                groups.push((*id, m));
            }
        }
        docs.push(Doc { file: part.file, doc: part.doc });
    }
    // what names other objects: groups, the snap list
    for (gid, names) in groups {
        let mut members = Vec::new();
        for n in names {
            match state.find_any(&n).map(|o| o.id) {
                Some(id) if id != gid => {
                    let mut o = (**state.get(id).unwrap_or_else(|| unreachable!())).clone();
                    o.owner = Some(gid);
                    o.flags.co |= co::IN_GROUP;
                    state.replace(o);
                    members.push(id);
                }
                _ => problems.push(Problem::warning(
                    "S020",
                    format!("a group names '{n}', which is no object of the scene"),
                )),
            }
        }
        if let Some(g) = state.get(gid) {
            let mut g = (**g).clone();
            if let ObjData::Group(gd) = &mut g.data {
                gd.members = members;
            }
            state.replace(g);
        }
    }
    // the painted layers the scene names
    if let Some(named) = scene_v.get("layers") {
        for (k, f) in named.fields() {
            let Some(layer) = layers::by_key(k) else { continue };
            let file = f.as_str().unwrap_or(layer.file);
            match texts.layers.iter().find(|(n, _)| n.eq_ignore_ascii_case(file)) {
                Some((_, bytes)) => match layers::read(layer, bytes, &mut state.bulk) {
                    Ok(None) => {}
                    Ok(Some(note)) | Err(note) => problems.push(
                        Problem::warning("S030", note)
                            .in_file(dir.join(file).unwrap_or_else(|_| dir.clone())),
                    ),
                },
                None => problems.push(
                    Problem::warning("S030", format!("the layer file {file} is not in the scene's folder"))
                        .in_file(main.clone()),
                ),
            }
        }
    }
    Ok(Read { files: SceneFiles { scene: scene_doc, docs, base: state.clone() }, state, problems })
}

// ---------------------------------------------------------------- writing

/// A file a save writes, or (`None`) removes. The path is the scene's `.scene` (`""`) or a
/// file of its folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Out {
    pub file: String,
    pub bytes: Option<Vec<u8>>,
}

/// What a save writes, and the documents after it.
pub struct Plan {
    pub files: Vec<Out>,
    pub after: SceneFiles,
}

/// A file's value as the model read it (`old`) and as it is now (`new`), for `apply_diff`: an
/// object is told against the object of the same name the base holds; one nobody changed is its
/// name alone on both sides, so its bytes stay. `changed`: the file has a difference to write.
fn class_file_value(
    cx: &Cx<'_>,
    cf: &ClassFile,
    ids: &[Vec<ObjId>],
    base: Option<&SceneState>,
) -> Result<(Value, Value, bool), String> {
    let mut old = Table::new();
    let mut new = Table::new();
    let mut changed = false;
    let defaults: Vec<Object> = cf.lists.iter().map(|(_, c)| default_object(*c)).collect();
    for ((((list, class), mine), dflt), _) in cf.lists.iter().zip(ids).zip(&defaults).zip(0..) {
        let mut olds = Vec::new();
        let mut news: Vec<Value> = mine
            .par_iter()
            .map(|id| -> Result<(Value, Option<Value>), String> {
                let o = cx.state.get(*id).ok_or_else(|| format!("object {} is gone", id.0))?;
                match base.and_then(|b| b.find(*class, &o.name)) {
                    Some(b) if Arc::ptr_eq(b, o) && same_context(cx.state, base, o) => {
                        let mut t = Table::new();
                        t.put("name", text(&o.name));
                        let v = Value::Table(t);
                        Ok((v.clone(), Some(v)))
                    }
                    Some(b) => {
                        let bcx = Cx { state: base.unwrap_or(cx.state), settings: cx.settings };
                        Ok((object_value(cx, o, dflt)?, Some(object_value(&bcx, b, dflt)?)))
                    }
                    None => Ok((object_value(cx, o, dflt)?, None)),
                }
            })
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .map(|(n, o)| {
                if let Some(o) = o {
                    if !gz_luadata::data_eq(&n, &o) {
                        changed = true;
                    }
                    olds.push(o);
                } else {
                    changed = true;
                }
                n
            })
            .collect();
        // what the base held and the state no longer does
        if let Some(b) = base {
            for bo in b.of_class(*class) {
                if cx.state.find(*class, &bo.name).is_none() {
                    let mut t = Table::new();
                    t.put("name", text(&bo.name));
                    olds.push(Value::Table(t));
                    changed = true;
                }
            }
        }
        if !news.is_empty() {
            new.put(list, Value::Table(Table { arr: std::mem::take(&mut news), fields: Vec::new() }));
        }
        if !olds.is_empty() {
            old.put(list, Value::Table(Table { arr: olds, fields: Vec::new() }));
        }
    }
    Ok((Value::Table(old), Value::Table(new), changed))
}

/// Whether what an object's keys read from the scene beside it is as it was (a group's
/// members' names, a light's control names).
fn same_context(state: &SceneState, base: Option<&SceneState>, o: &Object) -> bool {
    let Some(b) = base else { return false };
    match &o.data {
        ObjData::Group(g) => g.members.iter().all(|m| match (state.get(*m), b.get(*m)) {
            (Some(x), Some(y)) => x.name == y.name,
            _ => false,
        }),
        ObjData::Light(_) => {
            state.tools.part(ObjClass::Light).light() == b.tools.part(ObjClass::Light).light()
        }
        _ => true,
    }
}

/// Every file a save of `state` writes: against `files` (the documents it was read from) the
/// difference, a new file written whole. A file nothing changed in is not in the plan.
pub fn write(
    state: &SceneState,
    files: Option<&SceneFiles>,
    settings: Option<&(dyn Settings + Sync)>,
) -> Result<Plan, String> {
    let cx = Cx { state, settings };
    let base = files.map(|f| &f.base);
    let mut out = Vec::new();
    // the .scene
    let new_scene = scene_value(state);
    let scene_doc = match files {
        Some(f) => {
            let old = scene_value(&f.base);
            let mut d = f.scene.clone();
            if d.apply_diff(&old, &new_scene, &scene_rules()).map_err(|e| e.message)? {
                out.push(Out { file: String::new(), bytes: Some(d.text().as_bytes().to_vec()) });
            }
            d
        }
        None => {
            let text = gz_luadata::write_asset("", new_scene.as_table().unwrap_or(&Table::new()));
            out.push(Out { file: String::new(), bytes: Some(text.clone().into_bytes()) });
            LuaDoc::parse_with(&text, HEADER, Limits::scene()).map_err(|e| e.message)?
        }
    };
    // the objects by file and list, in placement order; a group's members stay in their own
    // files, a zone's shape inside its spawn point
    let mut by: FxHashMap<ObjClass, Vec<ObjId>> = FxHashMap::default();
    for c in ObjClass::ALL {
        by.insert(c, state.by_class[c.index()].iter().copied().collect());
    }
    let mut docs = Vec::new();
    for cf in &CLASS_FILES {
        let ids: Vec<Vec<ObjId>> =
            cf.lists.iter().map(|(_, c)| by.get(c).cloned().unwrap_or_default()).collect();
        let empty = ids.iter().all(Vec::is_empty);
        let had = files.and_then(|f| f.docs.iter().find(|d| d.file == cf.file));
        if empty {
            if had.is_some() || files.is_none() {
                out.push(Out { file: cf.file.to_owned(), bytes: None });
            }
            continue;
        }
        let (old, new, changed) = class_file_value(&cx, cf, &ids, base.filter(|_| had.is_some()))?;
        match had {
            Some(d) => {
                let mut doc = d.doc.clone();
                if changed && doc.apply_diff(&old, &new, &class_rules(cf)).map_err(|e| e.message)? {
                    out.push(Out { file: cf.file.to_owned(), bytes: Some(doc.text().as_bytes().to_vec()) });
                }
                docs.push(Doc { file: cf.file, doc });
            }
            None => {
                let text = gz_luadata::write_asset("", new.as_table().unwrap_or(&Table::new()));
                let doc = LuaDoc::parse_with(&text, cf.file, Limits::scene()).map_err(|e| e.message)?;
                out.push(Out { file: cf.file.to_owned(), bytes: Some(text.into_bytes()) });
                docs.push(Doc { file: cf.file, doc });
            }
        }
    }
    // the painted layers
    for l in layers::LAYERS {
        let now = layers::has(l, &state.bulk);
        let was = base.is_some_and(|b| layers::has(l, &b.bulk));
        let same = base.is_some_and(|b| match l.class {
            ObjClass::AiMap => opt_ptr_eq(&b.bulk.ai_map, &state.bulk.ai_map),
            ObjClass::DetailObjects => opt_ptr_eq(&b.bulk.details, &state.bulk.details),
            _ => opt_ptr_eq(&b.bulk.wallmarks, &state.bulk.wallmarks),
        });
        if now && !(was && same) {
            out.push(Out { file: l.file.to_owned(), bytes: Some(layers::write(l, &state.bulk)) });
        } else if !now && (was || files.is_none()) {
            out.push(Out { file: l.file.to_owned(), bytes: None });
        }
    }
    Ok(Plan { files: out, after: SceneFiles { scene: scene_doc, docs, base: state.clone() } })
}

fn opt_ptr_eq<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => Arc::ptr_eq(x, y),
        (None, None) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Vec3;
    use crate::objects::{LightData, SceneObjectData};

    /// The files on disk after a save of `plan` over `disk`.
    fn saved(mut disk: Texts, plan: &Plan) -> Texts {
        for o in &plan.files {
            let text = o.bytes.as_ref().map(|b| String::from_utf8(b.clone()).unwrap());
            if o.file.is_empty() {
                disk.scene = text.unwrap_or_default();
            } else if o.file.ends_with(".lua") {
                disk.files.retain(|(f, _)| *f != o.file);
                if let Some(t) = text {
                    disk.files.push((o.file.clone(), t));
                }
            } else {
                disk.layers.retain(|(f, _)| *f != o.file);
                if let Some(b) = &o.bytes {
                    disk.layers.push((o.file.clone(), b.clone()));
                }
            }
        }
        disk
    }

    fn file<'a>(t: &'a Texts, name: &str) -> &'a str {
        t.files.iter().find(|(f, _)| f == name).map_or("", |(_, s)| s.as_str())
    }

    fn scene() -> SceneState {
        let mut s = SceneState { valid: true, ..SceneState::default() };
        for (name, x) in [("Box_01", 1.0), ("Box_02", 2.0), ("Box_03", 3.0)] {
            let mut o = Object::new(
                ObjClass::SceneObject,
                name,
                ObjData::SceneObject(SceneObjectData::library(r"props\box")),
            );
            o.xform.position = Vec3::new(x, 0.5, -x);
            o.xform.rotation = Vec3::new(0.0, 0.25, 0.0);
            s.insert(o);
        }
        s.insert(Object::new(ObjClass::Light, "Lamp_01", ObjData::Light(LightData::default())));
        s
    }

    fn read_back(disk: &Texts) -> Read {
        let main = RelPath::new("levels/test.scene").unwrap();
        let dir = RelPath::new("levels/test").unwrap();
        read(disk, &main, &dir, None).unwrap()
    }

    #[test]
    fn a_scene_reads_back_as_it_was_written() {
        let s = scene();
        let disk = saved(Texts::default(), &write(&s, None, None).unwrap());
        assert!(!file(&disk, "objects.lua").is_empty() && !file(&disk, "lights.lua").is_empty());
        assert!(file(&disk, "glows.lua").is_empty(), "a kind without objects has no file");
        let r = read_back(&disk);
        assert_eq!(r.state.objects.len(), s.objects.len());
        for o in s.objects.values() {
            let b = r.state.find(o.class, &o.name).unwrap();
            assert_eq!(b.name, o.name, "the case is kept");
            assert_eq!(b.xform, o.xform);
            assert_eq!(b.data, o.data);
        }
        // and written again from what it read, nothing changes
        assert!(write(&r.state, Some(&r.files), None).unwrap().files.is_empty());
    }

    #[test]
    fn moving_one_object_changes_its_line_and_nothing_else() {
        let disk = saved(Texts::default(), &write(&scene(), None, None).unwrap());
        let r = read_back(&disk);
        let mut s = r.state.clone();
        let mut o = (**s.find(ObjClass::SceneObject, "Box_02").unwrap()).clone();
        o.xform.position.x = 7.5;
        s.replace(o);
        let plan = write(&s, Some(&r.files), None).unwrap();
        let files: Vec<&str> = plan.files.iter().map(|f| f.file.as_str()).collect();
        assert_eq!(files, ["objects.lua"]);
        let after = saved(disk.clone(), &plan);
        let (a, b) = (file(&disk, "objects.lua"), file(&after, "objects.lua"));
        let changed: Vec<(&str, &str)> = a.lines().zip(b.lines()).filter(|(x, y)| x != y).collect();
        assert_eq!(a.lines().count(), b.lines().count());
        assert_eq!(changed.len(), 1, "{changed:?}");
        assert!(changed[0].1.contains("7.5") && changed[0].1.contains("Box_02"));
    }

    #[test]
    fn comments_and_unknown_keys_survive_a_save() {
        let mut disk = saved(Texts::default(), &write(&scene(), None, None).unwrap());
        for (f, t) in &mut disk.files {
            if f == "objects.lua" {
                *t = t.replacen("name = \"Box_01\"", "name = \"Box_01\", note = \"keep me\"", 1);
                let at = t.find("{ name = \"Box_03\"").or_else(|| t.find("name = \"Box_03\"")).unwrap();
                let line = t[..at].rfind('\n').map_or(0, |i| i + 1);
                t.insert_str(line, "  -- the author's own note\n");
            }
        }
        disk.scene =
            disk.scene.replacen("scene = 1", "scene = 1,\n  -- why this level\n  author = \"me\"", 1);
        let r = read_back(&disk);
        assert!(
            r.problems.iter().any(|p| p.message.contains("note")),
            "an unknown key is a note: {:?}",
            r.problems
        );
        let mut s = r.state.clone();
        let mut o = (**s.find(ObjClass::SceneObject, "Box_01").unwrap()).clone();
        o.xform.position.y = 9.0;
        s.replace(o);
        let mut l = (**s.find(ObjClass::Light, "Lamp_01").unwrap()).clone();
        l.xform.position.y = 3.0;
        s.replace(l);
        s.meta.level.level_path = "night".into();
        let after = saved(disk.clone(), &write(&s, Some(&r.files), None).unwrap());
        let objects = file(&after, "objects.lua");
        assert!(objects.contains("note = \"keep me\""), "{objects}");
        assert!(objects.contains("-- the author's own note"), "{objects}");
        assert!(
            after.scene.contains("-- why this level") && after.scene.contains("author = \"me\""),
            "{}",
            after.scene
        );
        assert!(after.scene.contains("night"));
        // what was saved reads back with the edits
        let again = read_back(&after);
        assert_eq!(again.state.find(ObjClass::SceneObject, "Box_01").unwrap().xform.position.y, 9.0);
        assert_eq!(again.state.find(ObjClass::Light, "Lamp_01").unwrap().xform.position.y, 3.0);
    }

    #[test]
    fn a_removed_kind_takes_its_file_away() {
        let disk = saved(Texts::default(), &write(&scene(), None, None).unwrap());
        let r = read_back(&disk);
        let mut s = r.state.clone();
        let id = s.find(ObjClass::Light, "Lamp_01").unwrap().id;
        s.remove(id);
        let plan = write(&s, Some(&r.files), None).unwrap();
        assert!(plan.files.iter().any(|f| f.file == "lights.lua" && f.bytes.is_none()));
    }

    #[test]
    fn a_newer_scene_is_refused_with_its_reason() {
        let disk = Texts { scene: "return { scene = 99 }\n".into(), ..Texts::default() };
        let main = RelPath::new("a.scene").unwrap();
        let dir = RelPath::new("a").unwrap();
        let Err(r) = read(&disk, &main, &dir, None) else { panic!("read a newer scene") };
        assert_eq!(r.code, "newer_version");
    }
}
