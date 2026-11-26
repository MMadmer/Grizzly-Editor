//! The keys of the scene's files: one table per kind of object, and one of the scene itself.
//!
//! Every key the reader takes, the writer writes and the Details panel shows is a [`Field`]
//! here - its key, its type and default, its label, unit and meaning - so the files, the
//! property rows and `docs/assets/SCENE.md` (generated from these tables) cannot drift apart.
//! A value equal to its default is not written.

use gz_luadata::{Table, Value};

use super::values::*;
use crate::class::ObjClass;
use crate::math::Vec3;
use crate::model::{MotionRaw, ObjData, Object, SceneState, co};
use crate::objects::{
    CF_BOX, CF_SPHERE, Fuzzy, LT_DIRECT, LT_POINT, LT_SPOT, SHAPE_COMMON, SHAPE_LEVEL_BOUND, SO_USE_SURFACE,
    SO_VISUAL_MODE, ShapeData, ShapeDef, Stash, SurfaceOverride, WayPoint, light_flags,
};

/// What a key holds, for the reference and the property rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ty {
    Text,
    /// The object's name: unique in the scene.
    Name,
    /// A name picked from a list (`object`, `texture`, `particles`, `spawn_section`, ...).
    Pick(&'static str),
    Bool,
    Number {
        min: f32,
        max: f32,
        dec: u8,
    },
    Integer {
        min: i64,
        max: i64,
    },
    /// `{ x, y, z }`.
    Vec3,
    /// `{ x, y, z }` in degrees.
    Angles,
    /// One angle in degrees.
    Angle {
        min: f32,
        max: f32,
    },
    /// `{ from, to }`.
    Range,
    /// `{ r, g, b }` from 0 to 1.
    Color,
    /// One word of a list.
    Words(&'static [(&'static str, u32)]),
    /// A table; the text says what it holds.
    Table(&'static str),
}

impl Ty {
    /// How the reference names the type.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Ty::Text | Ty::Name => "string".into(),
            Ty::Pick(list) => format!("string ({list})"),
            Ty::Bool => "boolean".into(),
            Ty::Number { .. } => "number".into(),
            Ty::Integer { .. } => "integer".into(),
            Ty::Vec3 => "{ x, y, z }".into(),
            Ty::Angles => "{ x, y, z }".into(),
            Ty::Angle { .. } => "number".into(),
            Ty::Range => "{ from, to }".into(),
            Ty::Color => "{ r, g, b }".into(),
            Ty::Words(w) => w.iter().map(|(x, _)| format!("`\"{x}\"`")).collect::<Vec<_>>().join(" "),
            Ty::Table(what) => (*what).into(),
        }
    }
}

/// One key of a kind of thing `T` (an object, the scene).
pub struct Field<T: 'static> {
    pub key: &'static str,
    /// Title Case plain English.
    pub label: &'static str,
    /// The section of the Details panel it is shown in (empty: the top).
    pub group: &'static str,
    /// `m`, `deg`, `s`, `%` or empty.
    pub unit: &'static str,
    pub ty: Ty,
    /// The default as the reference writes it.
    pub default: &'static str,
    /// What it means.
    pub help: &'static str,
    /// Whether the key belongs to this object at all.
    pub applies: fn(&T) -> bool,
    /// Its value now.
    pub get: fn(&T) -> Value,
    pub set: fn(&mut T, &Value) -> Result<(), Bad>,
}

fn always<T>(_: &T) -> bool {
    true
}

/// The fields every object has, then those of its class.
pub fn object_fields(class: ObjClass) -> impl Iterator<Item = &'static Field<Object>> {
    COMMON.iter().chain(class_fields(class).iter())
}

/// The fields of a class's own data.
#[must_use]
pub fn class_fields(class: ObjClass) -> &'static [Field<Object>] {
    match class {
        ObjClass::SceneObject => SCENE_OBJECT,
        ObjClass::Light => LIGHT,
        ObjClass::Glow => GLOW,
        ObjClass::SoundSrc => SOUND_SOURCE,
        ObjClass::SoundEnv => SOUND_ENV,
        ObjClass::Particles => PARTICLES,
        ObjClass::Shape => SHAPE,
        ObjClass::FogVolume => FOG,
        ObjClass::SpawnPoint => super::spawn::FIELDS,
        ObjClass::Way => WAY,
        ObjClass::Sector => SECTOR,
        ObjClass::Portal => PORTAL,
        ObjClass::Group => GROUP,
        ObjClass::Terrain => super::terrain::TERRAIN,
        ObjClass::DetailObjects | ObjClass::AiMap | ObjClass::Wallmark => &[],
    }
}

/// A field of an object by its key.
#[must_use]
pub fn object_field(class: ObjClass, key: &str) -> Option<&'static Field<Object>> {
    object_fields(class).find(|f| f.key == key)
}

// ---------------------------------------------------------------- the common part

/// The bits of an object's flags the keys above say for this object: the rest is `flags`.
fn named_bits(o: &Object) -> u32 {
    let mut b = co::RENDER_IF_SELECTED | co::IN_GROUP_UNIQUE | co::AUTHOR_PLACED | co::BASE_REMOVED;
    // a member's group says it is one; a group's members are read after their objects
    if o.owner.is_some() {
        b |= co::IN_GROUP;
    }
    if o.motion.is_some() {
        b |= co::MOTION | crate::objmotion::AUTO_KEY | crate::objmotion::CAMERA_VIEW;
    }
    b
}

/// Bits the SDK files carry that mean nothing any more (selected, visible, locked of old).
pub const CO_LEGACY: u32 = 0b111;

fn moves(o: &Object) -> bool {
    o.class.caps().moves && o.class != ObjClass::Way
}

fn rotates(o: &Object) -> bool {
    o.class.caps().rotates
}

fn scales(o: &Object) -> bool {
    o.class.caps().scales && o.class != ObjClass::Glow
}

fn set_flag(word: &mut u32, bit: u32, on: bool) {
    if on {
        *word |= bit;
    } else {
        *word &= !bit;
    }
}

fn motion_value(o: &Object) -> Value {
    let Some(m) = &o.motion else { return Value::Nil };
    let mut t = Table::new();
    let bytes: &[u8] = &m.motion;
    t.put("data", hex(bytes));
    if let Some(time) = m.t_current {
        t.put("time", num(time));
    }
    if o.flags.co & crate::objmotion::AUTO_KEY != 0 {
        t.put("auto_key", true);
    }
    if o.flags.co & crate::objmotion::CAMERA_VIEW != 0 {
        t.put("camera_view", true);
    }
    Value::Table(t)
}

fn set_motion(o: &mut Object, v: &Value) -> Result<(), Bad> {
    if v.is_nil() {
        o.motion = None;
        o.flags.co &= !(co::MOTION | crate::objmotion::AUTO_KEY | crate::objmotion::CAMERA_VIEW);
        return Ok(());
    }
    let data = as_hex(v.get("data").ok_or("takes { data = \"<hex>\" }")?)?;
    let t_current = v.get("time").map(as_f32).transpose()?;
    o.motion =
        Some(std::sync::Arc::new(MotionRaw { motion: gz_base::Blob::from(data.as_slice()), t_current }));
    o.flags.co |= co::MOTION;
    let flag = |k: &str| v.get(k).map(as_bool).transpose().map(|b| b.unwrap_or(false));
    set_flag(&mut o.flags.co, crate::objmotion::AUTO_KEY, flag("auto_key")?);
    set_flag(&mut o.flags.co, crate::objmotion::CAMERA_VIEW, flag("camera_view")?);
    Ok(())
}

pub static COMMON: &[Field<Object>] = &[
    Field {
        key: "name",
        label: "Name",
        group: "",
        unit: "",
        ty: Ty::Name,
        default: "",
        help: "Unique in the scene: quests, orders, sectors and groups call the object by it.",
        applies: always,
        get: |o| text(&o.name),
        set: |o, v| {
            o.name = as_text(v)?;
            Ok(())
        },
    },
    Field {
        key: "pos",
        label: "Position",
        group: "Transform",
        unit: "m",
        ty: Ty::Vec3,
        default: "{ 0, 0, 0 }",
        help: "Where it stands, in metres.",
        applies: moves,
        get: |o| vec3(o.position()),
        set: |o, v| {
            let p = as_vec3(v)?;
            // a sound source stands where its sound plays
            if let ObjData::SoundSrc(s) = &mut o.data {
                s.position = p;
            }
            o.xform.position = p;
            Ok(())
        },
    },
    Field {
        key: "rot",
        label: "Rotation",
        group: "Transform",
        unit: "deg",
        ty: Ty::Angles,
        default: "{ 0, 0, 0 }",
        help: "Pitch, heading and bank, in degrees.",
        applies: rotates,
        get: |o| angles(o.xform.rotation),
        set: |o, v| {
            let r = as_angles(v)?;
            // a terrain lies flat: only its heading turns it
            o.xform.rotation = if o.class == ObjClass::Terrain { Vec3::new(0.0, r.y, 0.0) } else { r };
            Ok(())
        },
    },
    Field {
        key: "scale",
        label: "Scale",
        group: "Transform",
        unit: "",
        ty: Ty::Vec3,
        default: "{ 1, 1, 1 }",
        help: "A factor per axis.",
        applies: scales,
        get: |o| vec3(o.xform.scale),
        set: |o, v| {
            o.xform.scale = as_vec3(v)?;
            Ok(())
        },
    },
    Field {
        key: "authored",
        label: "Added by the Module",
        group: "Module",
        unit: "",
        ty: Ty::Bool,
        default: "false",
        help: "The module adds this object to the base level: it ships in the module's level layers.",
        applies: always,
        get: |o| Value::Bool(o.marks.authored),
        set: |o, v| {
            o.marks.authored = as_bool(v)?;
            Ok(())
        },
    },
    Field {
        key: "removed",
        label: "Removed from the Game",
        group: "Module",
        unit: "",
        ty: Ty::Bool,
        default: "false",
        help: "The module takes this spawn of the base level out of the game.",
        applies: |o| o.class == ObjClass::SpawnPoint && !o.marks.authored,
        get: |o| Value::Bool(o.marks.removed),
        set: |o, v| {
            o.marks.removed = as_bool(v)?;
            Ok(())
        },
    },
    Field {
        key: "hidden",
        label: "Hidden",
        group: "Editor",
        unit: "",
        ty: Ty::Bool,
        default: "false",
        help: "Not drawn and not picked in the viewport; the build takes it all the same.",
        applies: always,
        get: |o| Value::Bool(!o.flags.visible),
        set: |o, v| {
            o.flags.visible = !as_bool(v)?;
            Ok(())
        },
    },
    Field {
        key: "locked",
        label: "Locked",
        group: "Editor",
        unit: "",
        ty: Ty::Bool,
        default: "false",
        help: "Drawn, but not picked or changed in the viewport.",
        applies: always,
        get: |o| Value::Bool(o.flags.locked),
        set: |o, v| {
            o.flags.locked = as_bool(v)?;
            Ok(())
        },
    },
    Field {
        key: "group_editable",
        label: "Editable in Its Group",
        group: "Editor",
        unit: "",
        ty: Ty::Bool,
        default: "false",
        help: "A member of a group that is picked and changed by itself; reloading the group keeps it as it is.",
        applies: |o| o.owner.is_some() || o.flags.co & co::IN_GROUP_UNIQUE != 0,
        get: |o| Value::Bool(o.flags.co & co::IN_GROUP_UNIQUE != 0),
        set: |o, v| {
            set_flag(&mut o.flags.co, co::IN_GROUP_UNIQUE, as_bool(v)?);
            Ok(())
        },
    },
    Field {
        key: "draw_selected",
        label: "Drawn While Selected",
        group: "Editor",
        unit: "",
        ty: Ty::Bool,
        default: "false",
        help: "Drawn while it is selected even when its kind is hidden.",
        applies: |o| o.flags.co & co::RENDER_IF_SELECTED != 0,
        get: |o| Value::Bool(o.flags.co & co::RENDER_IF_SELECTED != 0),
        set: |o, v| {
            set_flag(&mut o.flags.co, co::RENDER_IF_SELECTED, as_bool(v)?);
            Ok(())
        },
    },
    Field {
        key: "motion",
        label: "Motion",
        group: "Motion",
        unit: "",
        ty: Ty::Table(
            "`{ data = \"<hex>\", time = s, auto_key = true, camera_view = true }`: the X-Ray SDK object motion (.anm body), hex",
        ),
        default: "none",
        help: "The path a dynamic object moves along, and where its playback stands.",
        applies: |o| o.motion.is_some(),
        get: motion_value,
        set: set_motion,
    },
    Field {
        key: "flags",
        label: "Other Flags",
        group: "Editor",
        unit: "",
        ty: Ty::Integer { min: 0, max: u32::MAX as i64 },
        default: "0",
        help: "Bits of the object's flags no key above names, kept as they were.",
        applies: |o| o.flags.co & !named_bits(o) & !CO_LEGACY != 0,
        get: |o| int(o.flags.co & !named_bits(o) & !CO_LEGACY),
        set: |o, v| {
            let n = as_u32(v)?;
            // the keys read before this one set their own bits
            o.flags.co = (o.flags.co & named_bits(o)) | (n & !CO_LEGACY);
            Ok(())
        },
    },
];

/// One field of a class's data: `$get` and `$set` see the data as `$d`.
macro_rules! field {
    ($variant:ident, $key:literal, $label:literal, $group:literal, $unit:literal, $ty:expr, $default:literal,
     $help:literal, |$d:ident| $get:expr, |$dm:ident, $v:ident| $set:expr $(, applies = $applies:expr)?) => {
        Field {
            key: $key,
            label: $label,
            group: $group,
            unit: $unit,
            ty: $ty,
            default: $default,
            help: $help,
            applies: field!(@applies $variant $(, $applies)?),
            get: |o| match &o.data {
                ObjData::$variant($d) => $get,
                _ => Value::Nil,
            },
            set: |o, $v| {
                if let ObjData::$variant($dm) = &mut o.data {
                    $set
                }
                Ok(())
            },
        }
    };
    (@applies $variant:ident) => {
        |o| matches!(o.data, ObjData::$variant(_))
    };
    (@applies $variant:ident, $applies:expr) => {
        $applies
    };
}

// ---------------------------------------------------------------- scene objects

fn surfaces_value(items: &[SurfaceOverride]) -> Value {
    list(items.iter().map(|s| {
        let mut t = Table::new();
        t.put("name", text(&s.name));
        t.put("texture", text(&s.texture));
        t.put("shader", text(&s.shader));
        t.put("compile", text(&s.shader_xrlc));
        t.put("material", text(&s.game_mtl));
        if !s.vmap.is_empty() {
            t.put("vmap", text(&s.vmap));
        }
        Value::Table(t)
    }))
}

fn surfaces_of(v: &Value) -> Result<Vec<SurfaceOverride>, Bad> {
    let t = v.as_table().ok_or("takes a list of surfaces")?;
    t.arr
        .iter()
        .map(|s| {
            let get = |k: &str| s.get(k).map(as_text).transpose().map(Option::unwrap_or_default);
            Ok(SurfaceOverride {
                name: get("name")?,
                shader: get("shader")?,
                shader_xrlc: get("compile")?,
                game_mtl: get("material")?,
                texture: get("texture")?,
                vmap: get("vmap")?,
            })
        })
        .collect()
}

const SO_NAMED: u32 = SO_USE_SURFACE | SO_VISUAL_MODE;

/// A stash as its key writes it: `true` while it says nothing of itself, else the keys that are
/// `false`.
fn stash_value(s: Option<Stash>) -> Value {
    let Some(s) = s else { return Value::Nil };
    if s == Stash::default() {
        return Value::Bool(true);
    }
    let mut t = Table::new();
    if !s.random {
        t.put("random", false);
    }
    if !s.loot {
        t.put("loot", false);
    }
    Value::Table(t)
}

fn stash_of(v: &Value) -> Result<Option<Stash>, Bad> {
    const TAKES: &str = "takes true, or { random = false, loot = false }";
    match v {
        Value::Nil | Value::Bool(false) => Ok(None),
        Value::Bool(true) => Ok(Some(Stash::default())),
        Value::Table(t) if t.arr.is_empty() => {
            let mut s = Stash::default();
            for (k, x) in &t.fields {
                let on = as_bool(x).map_err(|_| format!("{TAKES}: '{k}' takes true or false"))?;
                match k.as_str() {
                    "random" => s.random = on,
                    "loot" => s.loot = on,
                    _ => return Err(format!("{TAKES}, not '{k}'")),
                }
            }
            Ok(Some(s))
        }
        _ => Err(TAKES.into()),
    }
}

static SCENE_OBJECT: &[Field<Object>] = &[
    field!(
        SceneObject,
        "ref",
        "Reference",
        "Object",
        "",
        Ty::Pick("object"),
        "",
        "The library object (`props\\box_wood_01`), or the game model it names in visual mode.",
        |d| text(&d.reference),
        |d, v| d.reference = as_text(v)?
    ),
    field!(
        SceneObject,
        "visual",
        "Game Model",
        "Object",
        "",
        Ty::Bool,
        "false",
        "The reference names a game model (`meshes\\...ogf`) instead of a library object.",
        |d| Value::Bool(d.flags & SO_VISUAL_MODE != 0),
        |d, v| set_flag(&mut d.flags, SO_VISUAL_MODE, as_bool(v)?)
    ),
    field!(
        SceneObject,
        "surfaces",
        "Surfaces",
        "Surfaces",
        "",
        Ty::Table("a list of `{ name, texture, shader, compile, material }`"),
        "none",
        "Surfaces of the reference with this object's own texture, shaders and material.",
        |d| if d.flags & SO_USE_SURFACE != 0 { surfaces_value(&d.surfaces) } else { Value::Nil },
        |d, v| {
            if v.is_nil() {
                d.surfaces.clear();
                d.flags &= !SO_USE_SURFACE;
            } else {
                d.surfaces = surfaces_of(v)?;
                d.flags |= SO_USE_SURFACE;
            }
        }
    ),
    field!(
        SceneObject,
        "object_flags",
        "Other Object Flags",
        "Object",
        "",
        Ty::Integer { min: 0, max: u32::MAX as i64 },
        "0",
        "Bits of the object's own flags no key names, kept as they were.",
        |d| int(d.flags & !SO_NAMED),
        |d, v| d.flags = (d.flags & SO_NAMED) | (as_u32(v)? & !SO_NAMED),
        applies = |o| matches!(&o.data, ObjData::SceneObject(d) if d.flags & !SO_NAMED != 0)
    ),
    field!(
        SceneObject,
        "stash",
        "Stash",
        "Stash",
        "",
        Ty::Table("`true`, or `{ random = false, loot = false }`: each key `true` when left out"),
        "none",
        "The object is a stash: the build puts an `inventory_box` over it - the model's unseen case `<ref>_case` at the same place, or the box's own case where the model has none - under the object's name, and a quest reaches it by the name (an object reference `name = \"<name>\"`). `random = false`: the game never puts a random stash's loot in it; `loot = false`: no NPC searches it or takes what is in it. A false key is written into the box's custom data as `[xms_stash]`.",
        |d| stash_value(d.stash),
        |d, v| d.stash = stash_of(v)?
    ),
];

// ---------------------------------------------------------------- lights

pub const LIGHT_KINDS: &[(&str, u32)] = &[("point", LT_POINT), ("spot", LT_SPOT), ("directional", LT_DIRECT)];
const LIGHT_NAMED: u32 = light_flags::AFFECT_STATIC
    | light_flags::AFFECT_DYNAMIC
    | light_flags::PROCEDURAL
    | light_flags::BREAKING
    | light_flags::POINT_FUZZY
    | light_flags::CAST_SHADOW;
const FUZZY_SHAPES: &[(&str, u32)] = &[("sphere", 0), ("box", 1)];

fn fuzzy_value(f: &Fuzzy) -> Value {
    let mut t = Table::new();
    t.put("shape", word(u32::from(f.shape), FUZZY_SHAPES));
    t.put("radius", num(f.sphere_radius));
    t.put("box", vec3(f.box_dim));
    t.put("points", list(f.points.iter().map(|p| vec3(*p))));
    Value::Table(t)
}

fn fuzzy_of(v: &Value) -> Result<Fuzzy, Bad> {
    let mut f = Fuzzy::default();
    if let Some(s) = v.get("shape") {
        f.shape = as_word(s, FUZZY_SHAPES)? as u8;
    }
    if let Some(r) = v.get("radius") {
        f.sphere_radius = as_f32(r)?;
    }
    if let Some(b) = v.get("box") {
        f.box_dim = as_vec3(b)?;
    }
    if let Some(p) = v.get("points") {
        f.points = p.arr().iter().map(as_vec3).collect::<Result<_, _>>()?;
    }
    Ok(f)
}

macro_rules! light_flag {
    ($key:literal, $label:literal, $bit:expr, $default:literal, $help:literal) => {
        field!(
            Light,
            $key,
            $label,
            "Usage",
            "",
            Ty::Bool,
            $default,
            $help,
            |d| Value::Bool(d.flags & $bit != 0),
            |d, v| set_flag(&mut d.flags, $bit, as_bool(v)?)
        )
    };
}

static LIGHT: &[Field<Object>] = &[
    field!(
        Light,
        "kind",
        "Type",
        "Light",
        "",
        Ty::Words(LIGHT_KINDS),
        "\"point\"",
        "How the light shines: all around, inside a cone along where it looks (`rot`), or from one direction over the whole level.",
        |d| word(d.kind, LIGHT_KINDS),
        |d, v| d.kind = as_word_or_number(v, LIGHT_KINDS)?
    ),
    field!(
        Light,
        "color",
        "Color",
        "Light",
        "",
        Ty::Color,
        "{ 1, 1, 1 }",
        "The colour of its light.",
        |d| color(d.color),
        |d, v| d.color = as_color(v)?
    ),
    field!(
        Light,
        "brightness",
        "Brightness",
        "Light",
        "",
        Ty::Number { min: -3.0, max: 3.0, dec: 2 },
        "1",
        "Multiplies the colour; below 0 the light darkens.",
        |d| num(d.brightness),
        |d, v| d.brightness = as_f32(v)?
    ),
    field!(
        Light,
        "range",
        "Range",
        "Light",
        "m",
        Ty::Number { min: 0.1, max: 1000.0, dec: 2 },
        "8",
        "How far it reaches.",
        |d| num(d.range),
        |d, v| d.range = as_f32(v)?
    ),
    field!(
        Light,
        "attenuation",
        "Attenuation",
        "Light",
        "",
        Ty::Vec3,
        "{ 1, 0, 0 }",
        "Its constant, linear and quadratic falloff over the range.",
        |d| list(d.attenuation.iter().map(|x| num(*x))),
        |d, v| {
            let a = as_vec3(v)?;
            d.attenuation = [a.x, a.y, a.z];
        }
    ),
    field!(
        Light,
        "cone",
        "Cone Angle",
        "Light",
        "deg",
        Ty::Angle { min: 0.1, max: 120.0 },
        "22.5",
        "The whole opening of a spot light: its light is full on the axis and gone at the cone's edge.",
        |d| angle(d.cone),
        |d, v| d.cone = as_angle(v)?,
        applies = |o| matches!(&o.data, ObjData::Light(d) if d.kind == LT_SPOT)
    ),
    field!(
        Light,
        "texture",
        "Spot Texture",
        "Light",
        "",
        Ty::Pick("texture"),
        "",
        "The picture a spot light throws.",
        |d| text(&d.falloff_tex),
        |d, v| d.falloff_tex = as_text(v)?,
        applies = |o| matches!(&o.data, ObjData::Light(d) if d.kind == LT_SPOT)
    ),
    field!(
        Light,
        "virtual_size",
        "Virtual Size",
        "Light",
        "m",
        Ty::Number { min: 0.0, max: 100.0, dec: 2 },
        "0",
        "The size of the light's source the shadows are soft by.",
        |d| num(d.virtual_size),
        |d, v| d.virtual_size = as_f32(v)?
    ),
    field!(
        Light,
        "control",
        "Light Control",
        "Light",
        "",
        Ty::Pick("light_control"),
        "\"$static\"",
        "The group of lights it is switched with: `$static`, `$hemi`, `$sun` or one of the scene's `light_controls`.",
        |_d| Value::Nil,
        |_d, _v| {}
    ),
    field!(
        Light,
        "animation",
        "Color Animation",
        "Light",
        "",
        Ty::Pick("lightanim"),
        "",
        "A colour animation of the game's light animations.",
        |d| text(&d.anim_ref),
        |d, v| d.anim_ref = as_text(v)?
    ),
    light_flag!(
        "lightmap",
        "Lightmap",
        light_flags::AFFECT_STATIC,
        "true",
        "The build bakes it into the lightmaps: of the level, or of the module's piece over a level of the game."
    ),
    light_flag!(
        "dynamic",
        "Dynamic",
        light_flags::AFFECT_DYNAMIC,
        "false",
        "It shines in the game and lights what moves: a point light or a spot light; a directional one does not (`S221`)."
    ),
    light_flag!(
        "animated",
        "Animated",
        light_flags::PROCEDURAL,
        "false",
        "Its colour animation plays in the game."
    ),
    light_flag!("breaking", "Breaking", light_flags::BREAKING, "false", "It flickers out now and then."),
    light_flag!("shadows", "Casts Shadows", light_flags::CAST_SHADOW, "false", "Objects cast shadows in it."),
    field!(
        Light,
        "fuzzy",
        "Fuzzy",
        "Fuzzy",
        "",
        Ty::Table(
            "`{ shape = \"sphere\" | \"box\", radius = m, box = { x, y, z }, points = { { x, y, z }, ... } }`"
        ),
        "none",
        "A point light spread over random points of a sphere or a box, for softer lightmaps.",
        |d| if d.flags & light_flags::POINT_FUZZY != 0 {
            fuzzy_value(&d.fuzzy.clone().unwrap_or_default())
        } else {
            Value::Nil
        },
        |d, v| {
            if v.is_nil() {
                d.fuzzy = None;
                d.flags &= !light_flags::POINT_FUZZY;
            } else {
                d.fuzzy = Some(fuzzy_of(v)?);
                d.flags |= light_flags::POINT_FUZZY;
            }
        }
    ),
    field!(
        Light,
        "light_flags",
        "Other Light Flags",
        "Usage",
        "",
        Ty::Integer { min: 0, max: u32::MAX as i64 },
        "0",
        "Bits of the light's flags no key names, kept as they were.",
        |d| int(d.flags & !LIGHT_NAMED),
        |d, v| d.flags = (d.flags & LIGHT_NAMED) | (as_u32(v)? & !LIGHT_NAMED),
        applies = |o| matches!(&o.data, ObjData::Light(d) if d.flags & !LIGHT_NAMED != 0)
    ),
];

// ---------------------------------------------------------------- glows

const GLOW_FIXED: u16 = 1 << 0;

static GLOW: &[Field<Object>] = &[
    field!(
        Glow,
        "texture",
        "Texture",
        "Glow",
        "",
        Ty::Pick("texture"),
        "",
        "The glow's picture.",
        |d| text(&d.texture),
        |d, v| d.texture = as_text(v)?
    ),
    field!(
        Glow,
        "shader",
        "Shader",
        "Glow",
        "",
        Ty::Pick("e_shader"),
        "\"effects\\\\glow\"",
        "How it is drawn: left out (or `effects\\glow`, `effects\\flare`) like a lens flare - its picture added to the frame, gone when something of the level stands between the eye and it; `effects\\sun` the same, hidden by depth as well. The game draws a glow with no other shader (`S225`).",
        |d| text(&d.shader),
        |d, v| d.shader = as_text(v)?
    ),
    field!(
        Glow,
        "radius",
        "Radius",
        "Glow",
        "m",
        Ty::Number { min: 0.01, max: 10000.0, dec: 2 },
        "0.5",
        "How big it is.",
        |d| num(d.radius),
        |d, v| d.radius = as_f32(v)?
    ),
    field!(
        Glow,
        "fixed_size",
        "Fixed Size",
        "Glow",
        "",
        Ty::Bool,
        "false",
        "The same size on the screen at any distance: `radius` is then a share of half the screen's height (0.02 is a small light), not metres.",
        |d| Value::Bool(d.flags & GLOW_FIXED != 0),
        |d, v| d.flags = if as_bool(v)? { d.flags | GLOW_FIXED } else { d.flags & !GLOW_FIXED }
    ),
    field!(
        Glow,
        "glow_flags",
        "Other Glow Flags",
        "Glow",
        "",
        Ty::Integer { min: 0, max: 65535 },
        "0",
        "Bits of the glow's flags no key names.",
        |d| int(d.flags & !GLOW_FIXED),
        |d, v| d.flags = (d.flags & GLOW_FIXED) | (as_u16(v)? & !GLOW_FIXED),
        applies = |o| matches!(&o.data, ObjData::Glow(d) if d.flags & !GLOW_FIXED != 0)
    ),
];

// ---------------------------------------------------------------- sounds

fn range(v: [f32; 2]) -> Value {
    list([num(v[0]), num(v[1])])
}

fn range_of(v: &Value) -> Result<[f32; 2], Bad> {
    let t = v.as_table().ok_or("takes { from, to }")?;
    match t.arr.as_slice() {
        [a, b] if t.fields.is_empty() => Ok([as_f32(a)?, as_f32(b)?]),
        _ => Err("takes { from, to }".into()),
    }
}

static SOUND_SOURCE: &[Field<Object>] = &[
    field!(
        SoundSrc,
        "sound",
        "Sound",
        "Sound",
        "",
        Ty::Pick("sound_source"),
        "",
        "The game's sound it plays (`ambient\\wind_1`).",
        |d| text(&d.wav),
        |d, v| d.wav = as_text(v)?
    ),
    field!(
        SoundSrc,
        "volume",
        "Volume",
        "Sound",
        "",
        Ty::Number { min: 0.0, max: 1.0, dec: 2 },
        "1",
        "How loud, from 0 to 1.",
        |d| num(d.volume),
        |d, v| d.volume = as_f32(v)?
    ),
    field!(
        SoundSrc,
        "frequency",
        "Pitch",
        "Sound",
        "",
        Ty::Number { min: 0.0, max: 2.0, dec: 2 },
        "1",
        "Played faster (above 1) or slower.",
        |d| num(d.freq),
        |d, v| d.freq = as_f32(v)?
    ),
    field!(
        SoundSrc,
        "min_distance",
        "Full Volume Within",
        "Sound",
        "m",
        Ty::Number { min: 0.1, max: 1000.0, dec: 1 },
        "1",
        "Up to this distance it is heard at its full volume.",
        |d| num(d.min_distance),
        |d, v| d.min_distance = as_f32(v)?
    ),
    field!(
        SoundSrc,
        "max_distance",
        "Heard Up To",
        "Sound",
        "m",
        Ty::Number { min: 0.1, max: 1000.0, dec: 1 },
        "300",
        "Beyond this distance it is not heard.",
        |d| num(d.max_distance),
        |d, v| d.max_distance = as_f32(v)?
    ),
    field!(
        SoundSrc,
        "ai_distance",
        "Heard by NPCs Up To",
        "Sound",
        "m",
        Ty::Number { min: 0.1, max: 1000.0, dec: 1 },
        "300",
        "How far the game's characters hear it.",
        |d| num(d.max_ai_distance),
        |d, v| d.max_ai_distance = as_f32(v)?
    ),
    field!(
        SoundSrc,
        "pause",
        "Pause",
        "Playing",
        "s",
        Ty::Range,
        "{ 0, 0 }",
        "A random pause between plays, from and to; 0 plays it looped.",
        |d| range(d.random_pause),
        |d, v| d.random_pause = range_of(v)?
    ),
    field!(
        SoundSrc,
        "active",
        "Active Hours",
        "Playing",
        "s",
        Ty::Range,
        "{ 0, 0 }",
        "The time of the game's day it plays in, from and to, in seconds after midnight; 0 to 0 is round the clock.",
        |d| range(d.active_time),
        |d, v| d.active_time = range_of(v)?
    ),
    field!(
        SoundSrc,
        "play",
        "Play Time",
        "Playing",
        "s",
        Ty::Range,
        "{ 0, 0 }",
        "How long each play lasts, from and to; 0 plays the sound once.",
        |d| range(d.play_time),
        |d, v| d.play_time = range_of(v)?
    ),
    field!(
        SoundSrc,
        "sound_flags",
        "Sound Flags",
        "Sound",
        "",
        Ty::Integer { min: 0, max: u32::MAX as i64 },
        "1",
        "The source's flags, kept as they were.",
        |d| int(d.flags),
        |d, v| d.flags = as_u32(v)?
    ),
];

// ---------------------------------------------------------------- shapes

/// The keys a shape of a file may say.
const SHAPE_KEYS: [&str; 9] = ["kind", "center", "radius", "size", "rot", "x", "y", "z", "raw"];

/// The sides of a box whose axes lie along the shape's own: what `size` says.
fn box_size(s: &ShapeDef) -> Option<Vec3> {
    let d = &s.data;
    let straight = [d[1], d[2], d[4], d[6], d[8], d[9]].iter().all(|x| *x == 0.0);
    (straight && [d[3], d[7], d[11], d[15]] == [0.0, 0.0, 0.0, 1.0] && s.pad == [0; 3])
        .then(|| Vec3::new(d[0], d[5], d[10]))
}

/// One shape: a sphere, or a box - by its `size` where its sides lie along the shape's axes,
/// else by the three axes of its matrix (`x`, `y`, `z`, each the direction and the length of a
/// side); `center` where it is not the shape's origin.
#[must_use]
pub fn shape_value(s: &ShapeDef) -> Value {
    let mut t = Table::new();
    let v3 = |i: usize| list([num(s.data[i]), num(s.data[i + 1]), num(s.data[i + 2])]);
    if s.kind == CF_BOX
        && let Some(size) = box_size(s)
    {
        t.put("kind", "box");
        t.put("size", list([num(size.x), num(size.y), num(size.z)]));
        if s.data[12..15].iter().any(|x| *x != 0.0) {
            t.put("center", v3(12));
        }
        return Value::Table(t);
    }
    if s.kind == CF_SPHERE {
        t.put("kind", "sphere");
        t.put("center", v3(0));
        t.put("radius", num(s.data[3]));
        if s.data[4..].iter().any(|x| *x != 0.0) || s.pad != [0; 3] {
            t.put("raw", raw_shape(s));
        }
    } else {
        t.put("kind", if s.kind == CF_BOX { Value::from("box") } else { int(s.kind) });
        t.put("x", v3(0));
        t.put("y", v3(4));
        t.put("z", v3(8));
        t.put("center", v3(12));
        if [s.data[3], s.data[7], s.data[11], s.data[15]] != [0.0, 0.0, 0.0, 1.0] || s.pad != [0; 3] {
            t.put("raw", raw_shape(s));
        }
    }
    Value::Table(t)
}

fn raw_shape(s: &ShapeDef) -> Value {
    let mut b = Vec::with_capacity(68);
    b.push(s.kind);
    b.extend_from_slice(&s.pad);
    for f in s.data {
        b.extend_from_slice(&f.to_le_bytes());
    }
    hex(&b)
}

/// A shape of a file.
pub fn shape_of(v: &Value) -> Result<ShapeDef, Bad> {
    if let Some(raw) = v.get("raw") {
        let b = as_hex(raw)?;
        if b.len() != 68 {
            return Err("raw takes the 68 bytes of a shape".into());
        }
        let mut data = [0f32; 16];
        for (i, d) in data.iter_mut().enumerate() {
            *d = f32::from_le_bytes([b[4 + i * 4], b[5 + i * 4], b[6 + i * 4], b[7 + i * 4]]);
        }
        return Ok(ShapeDef { kind: b[0], pad: [b[1], b[2], b[3]], data });
    }
    if let Some((k, _)) = v.fields().iter().find(|(k, _)| !SHAPE_KEYS.contains(&k.as_str())) {
        return Err(format!(
            "holds a shape with the key '{k}', which a shape has not: a sphere says center and radius, a box size (and rot) or x, y, z, and center"
        ));
    }
    let row = |k: &str| v.get(k).map(as_vec3).transpose().map(|x| x.unwrap_or(Vec3::ZERO));
    match v.get("kind") {
        Some(Value::String(s)) if s == "sphere" => {
            Ok(ShapeDef::sphere(row("center")?, v.get("radius").map(as_f32).transpose()?.unwrap_or(1.0)))
        }
        Some(k) => {
            let kind = if k.as_str() == Some("box") { CF_BOX } else { as_u8(k)? };
            let mut d = ShapeDef::boxed(&crate::math::Mat4::IDENTITY);
            d.kind = kind;
            // a box by its sides, turned by `rot` (degrees, as an object's) when it says one
            if let Some(size) = v.get("size") {
                if ["x", "y", "z"].iter().any(|a| v.get(a).is_some()) {
                    return Err("holds a box that says both size and its axes (x, y, z): one of them".into());
                }
                let size = as_vec3(size)?;
                let turn = match v.get("rot") {
                    Some(r) => crate::math::rotation_matrix(as_angles(r)?),
                    None => crate::math::Mat4::IDENTITY,
                };
                for (r, side) in [size.x, size.y, size.z].into_iter().enumerate() {
                    let axis = turn.row(r) * side;
                    d.data[r * 4] = axis.x;
                    d.data[r * 4 + 1] = axis.y;
                    d.data[r * 4 + 2] = axis.z;
                }
            } else if v.get("rot").is_some() {
                return Err("holds a box with rot and no size: rot turns a box said by its size".into());
            }
            for (r, key) in ["x", "y", "z", "center"].iter().enumerate() {
                if let Some(x) = v.get(key) {
                    let p = as_vec3(x)?;
                    d.data[r * 4] = p.x;
                    d.data[r * 4 + 1] = p.y;
                    d.data[r * 4 + 2] = p.z;
                }
            }
            Ok(d)
        }
        None => Err("a shape takes kind = \"sphere\" or \"box\"".into()),
    }
}

#[must_use]
pub fn shapes_value(d: &ShapeData) -> Value {
    list(d.shapes.iter().map(shape_value))
}

pub fn shapes_of(v: &Value) -> Result<Vec<ShapeDef>, Bad> {
    v.as_table().ok_or("takes a list of shapes")?.arr.iter().map(shape_of).collect()
}

const SHAPES_TY: Ty = Ty::Table(
    "a list of `{ kind = \"sphere\", center = { x, y, z }, radius = m }` and `{ kind = \"box\", size = { x, y, z }, center = { x, y, z } }` - the box's sides in metres, `rot = { x, y, z }` (deg) to turn it, or its three axes `x`, `y`, `z` instead of `size`; `center` is where it stands from the object, left out at it",
);

static SHAPE: &[Field<Object>] = &[
    field!(
        Shape,
        "shapes",
        "Shapes",
        "Shape",
        "",
        SHAPES_TY,
        "none",
        "The spheres and boxes it is made of, in its own coordinates.",
        |d| shapes_value(d),
        |d, v| d.shapes = shapes_of(v)?
    ),
    field!(
        Shape,
        "usage",
        "Usage",
        "Shape",
        "",
        Ty::Words(SHAPE_USAGES),
        "\"common\"",
        "What the shape is for: a volume of its own, the bound of the level, or the AI Map Bounds the AI map is generated inside.",
        |d| word(u32::from(d.usage), SHAPE_USAGES),
        |d, v| d.usage = as_word_or_number(v, SHAPE_USAGES)? as u8
    ),
];

const SHAPE_USAGES: &[(&str, u32)] = &[
    ("common", SHAPE_COMMON as u32),
    ("level_bound", SHAPE_LEVEL_BOUND as u32),
    ("ai_map_bounds", crate::bulk::ground::SHAPE_AI_MAP_BOUNDS as u32),
];

static SOUND_ENV: &[Field<Object>] = &[
    field!(
        SoundEnv,
        "inner",
        "Inside",
        "Sound Environment",
        "",
        Ty::Pick("sound_env"),
        "",
        "The sound environment inside the volume.",
        |d| text(&d.inner),
        |d, v| d.inner = as_text(v)?
    ),
    field!(
        SoundEnv,
        "outer",
        "Outside",
        "Sound Environment",
        "",
        Ty::Pick("sound_env"),
        "",
        "The sound environment outside the volume.",
        |d| text(&d.outer),
        |d, v| d.outer = as_text(v)?
    ),
    field!(
        SoundEnv,
        "shapes",
        "Shapes",
        "Sound Environment",
        "",
        SHAPES_TY,
        "one unit box",
        "The spheres and boxes the volume is made of.",
        |d| shapes_value(&d.shape),
        |d, v| d.shape.shapes = shapes_of(v)?
    ),
];

const FOG_KINDS: &[(&str, u32)] = &[("emitter", 0), ("occlusion", 1)];

static FOG: &[Field<Object>] = &[
    field!(
        FogVolume,
        "kind",
        "Type",
        "Fog",
        "",
        Ty::Words(FOG_KINDS),
        "\"emitter\"",
        "An emitter fills its box with fog; an occlusion volume keeps fog out.",
        |d| word(u32::from(d.kind), FOG_KINDS),
        |d, v| d.kind = as_word_or_number(v, FOG_KINDS)? as u8
    ),
    field!(
        FogVolume,
        "group",
        "Volume Group",
        "Fog",
        "",
        Ty::Integer { min: 0, max: u32::MAX as i64 },
        "none",
        "Volumes of one group are one fog.",
        |d| if d.group_id == u32::MAX { Value::Nil } else { int(d.group_id) },
        |d, v| d.group_id = if v.is_nil() { u32::MAX } else { as_u32(v)? }
    ),
    field!(
        FogVolume,
        "profile",
        "Profile",
        "Fog",
        "",
        Ty::Pick("fog_profile"),
        "\"environment\\\\fog\\\\default.ltx\"",
        "How the fog flows, a file of the game's configs said from there (`S220` when neither the game nor the project has it).",
        |d| text(&d.profile),
        |d, v| d.profile = as_text(v)?
    ),
];

const GAME_TYPES: &[(&str, u32)] = &[
    ("single", 1 << 0),
    ("deathmatch", 1 << 1),
    ("team_deathmatch", 1 << 2),
    ("artefact_hunt", 1 << 3),
    ("capture_the_artefact", 1 << 4),
    ("domination_zone", 1 << 5),
    ("team_domination_zone", 1 << 6),
];

/// The game types of a mask as words: `nil` for every one; bits no word names as a number.
#[must_use]
pub fn game_types(mask: u16) -> Value {
    if mask == u16::MAX {
        return Value::Nil;
    }
    let mut items: Vec<Value> =
        GAME_TYPES.iter().filter(|(_, b)| u32::from(mask) & b != 0).map(|(w, _)| text(w)).collect();
    let named: u32 = GAME_TYPES.iter().map(|(_, b)| b).sum();
    let rest = u32::from(mask) & !named;
    if rest != 0 {
        items.push(int(rest));
    }
    list(items)
}

pub fn game_types_of(v: &Value) -> Result<u16, Bad> {
    if v.is_nil() {
        return Ok(u16::MAX);
    }
    let mut mask = 0u32;
    for x in v.as_table().ok_or("takes a list of game types")?.arr.iter() {
        mask |= match x {
            Value::Number(_) => as_u32(x)?,
            _ => as_word(x, GAME_TYPES)?,
        };
    }
    u16::try_from(mask).map_err(|_| "names more game types than there are".into())
}

pub const GAME_TYPES_TY: Ty = Ty::Table(
    "a list of `\"single\"`, `\"deathmatch\"`, `\"team_deathmatch\"`, `\"artefact_hunt\"`, `\"capture_the_artefact\"`, `\"domination_zone\"`, `\"team_domination_zone\"`",
);

static PARTICLES: &[Field<Object>] = &[
    field!(
        Particles,
        "ref",
        "Effect",
        "Particles",
        "",
        Ty::Pick("particles"),
        "",
        "The particle effect of the game (`fire\\fire_01`).",
        |d| text(&d.reference),
        |d, v| d.reference = as_text(v)?
    ),
    field!(
        Particles,
        "game_types",
        "Game Types",
        "Particles",
        "",
        GAME_TYPES_TY,
        "every one",
        "The game types it plays in.",
        |d| game_types(d.game_type),
        |d, v| d.game_type = game_types_of(v)?
    ),
];

// ---------------------------------------------------------------- ways, sectors, portals, groups

const WAY_KINDS: &[(&str, u32)] = &[("patrol", 0)];

/// A way's points: `{ name, pos, flags, links }`, a link the name of the point it leads to, or
/// `{ name, probability }` when that is not 1.
#[must_use]
pub fn points_value(points: &[WayPoint]) -> Value {
    list(points.iter().map(|p| {
        let mut t = Table::new();
        t.put("name", text(&p.name));
        t.put("pos", vec3(p.position));
        if p.flags != 0 {
            t.put("flags", int(p.flags));
        }
        if !p.links.is_empty() {
            t.put(
                "links",
                list(p.links.iter().map(|(to, prob)| {
                    // a point by its name, by its number from 0 where its name is not its own
                    let name = points
                        .get(*to as usize)
                        .filter(|q| points.iter().filter(|x| x.name == q.name).count() == 1)
                        .map_or_else(|| int(*to), |q| text(&q.name));
                    if *prob == 1.0 { name } else { list([name, num(*prob)]) }
                })),
            );
        }
        Value::Table(t)
    }))
}

pub fn points_of(v: &Value) -> Result<Vec<WayPoint>, Bad> {
    let items = v.as_table().ok_or("takes a list of points")?.arr.clone();
    let mut points: Vec<WayPoint> = items
        .iter()
        .map(|p| {
            let mut wp =
                WayPoint::new(&p.get("name").map(as_text).transpose()?.unwrap_or_default(), Vec3::ZERO);
            if let Some(x) = p.get("pos") {
                wp.position = as_vec3(x)?;
            }
            if let Some(f) = p.get("flags") {
                wp.flags = as_u32(f)?;
            }
            Ok(wp)
        })
        .collect::<Result<_, Bad>>()?;
    let index = |target: &Value| -> Result<u32, Bad> {
        match target {
            Value::String(s) => points
                .iter()
                .position(|q| q.name == *s)
                .map(|i| i as u32)
                .ok_or_else(|| format!("links to '{s}', which is no point of the way")),
            _ => as_u32(target),
        }
    };
    let mut links = Vec::with_capacity(items.len());
    for p in &items {
        let mut mine = Vec::new();
        if let Some(l) = p.get("links") {
            for x in l.arr() {
                match x {
                    Value::Table(t) => {
                        let to = t.arr.first().ok_or("a link takes { point, probability }")?;
                        let prob = t.arr.get(1).map(as_f32).transpose()?.unwrap_or(1.0);
                        mine.push((index(to)?, prob));
                    }
                    other => mine.push((index(other)?, 1.0)),
                }
            }
        }
        links.push(mine);
    }
    for (p, l) in points.iter_mut().zip(links) {
        p.links = l;
    }
    Ok(points)
}

static WAY: &[Field<Object>] = &[
    field!(
        Way,
        "kind",
        "Type",
        "Way",
        "",
        Ty::Words(WAY_KINDS),
        "\"patrol\"",
        "What the game takes the way for.",
        |d| word(d.kind, WAY_KINDS),
        |d, v| d.kind = as_word_or_number(v, WAY_KINDS)?
    ),
    field!(
        Way,
        "points",
        "Points",
        "Points",
        "",
        Ty::Table(
            "a list of `{ name, pos = { x, y, z }, flags = n, links = { \"point\", { \"point\", probability }, ... } }` (a point whose name another point shares is linked by its number from 0)"
        ),
        "one point",
        "The points of the way and where each leads; orders and quests walk them by name.",
        |d| points_value(&d.points),
        |d, v| d.points = points_of(v)?
    ),
];

static SECTOR: &[Field<Object>] = &[
    field!(
        Sector,
        "color",
        "Color",
        "Sector",
        "",
        Ty::Color,
        "{ 1, 1, 1 }",
        "How the viewport shows it.",
        |d| color(d.color),
        |d, v| d.color = as_color(v)?
    ),
    field!(
        Sector,
        "default",
        "Default Sector",
        "Sector",
        "",
        Ty::Bool,
        "false",
        "The sector of everything no other sector holds.",
        |d| Value::Bool(d.default),
        |d, v| d.default = as_bool(v)?
    ),
    field!(
        Sector,
        "items",
        "Meshes",
        "Sector",
        "",
        Ty::Table("a list of `{ \"object\", \"mesh\" }`"),
        "none",
        "The meshes of scene objects the sector is made of, by object and mesh name.",
        |d| list(d.items.iter().map(|(o, m)| list([text(o), text(m)]))),
        |d, v| {
            d.items = v
                .arr()
                .iter()
                .map(|x| match x.arr() {
                    [o, m] => Ok((as_text(o)?, as_text(m)?)),
                    _ => Err("a mesh takes { \"object\", \"mesh\" }".to_owned()),
                })
                .collect::<Result<_, _>>()?;
        }
    ),
    field!(
        Sector,
        "map",
        "Light Map",
        "Sector",
        "",
        Ty::Integer { min: 0, max: 3 },
        "the default",
        "The light map its faces go to.",
        |d| if d.map_idx == u8::MAX { Value::Nil } else { int(d.map_idx) },
        |d, v| d.map_idx = if v.is_nil() { u8::MAX } else { as_u8(v)? }
    ),
];

static PORTAL: &[Field<Object>] = &[
    field!(
        Portal,
        "front",
        "Front Sector",
        "Portal",
        "",
        Ty::Text,
        "",
        "The sector in front of it.",
        |d| text(&d.front),
        |d, v| d.front = as_text(v)?
    ),
    field!(
        Portal,
        "back",
        "Back Sector",
        "Portal",
        "",
        Ty::Text,
        "",
        "The sector behind it.",
        |d| text(&d.back),
        |d, v| d.back = as_text(v)?
    ),
    field!(
        Portal,
        "vertices",
        "Vertices",
        "Portal",
        "",
        Ty::Table("a list of `{ x, y, z }`"),
        "none",
        "Its polygon, in the level's coordinates.",
        |d| list(d.vertices.iter().map(|p| vec3(*p))),
        |d, v| d.vertices = v.arr().iter().map(as_vec3).collect::<Result<_, _>>()?
    ),
];

static GROUP: &[Field<Object>] = &[
    field!(
        Group,
        "ref",
        "Reference",
        "Group",
        "",
        Ty::Pick("group"),
        "",
        "The group asset or library group it was placed from.",
        |d| text(&d.reference),
        |d, v| d.reference = as_text(v)?
    ),
    Field {
        key: "members",
        label: "Members",
        group: "Group",
        unit: "",
        ty: Ty::Table("a list of object names"),
        default: "none",
        help: "The objects the group holds, by name; each stays in its own file.",
        applies: |o| matches!(o.data, ObjData::Group(_)),
        // the names are the scene's to give (see `group_members`)
        get: |_| Value::Nil,
        set: |_, _| Ok(()),
    },
];

/// The names of a group's members, for its `members` key.
#[must_use]
pub fn group_members(state: &SceneState, o: &Object) -> Value {
    match &o.data {
        ObjData::Group(g) => list(g.members.iter().filter_map(|m| state.get(*m)).map(|m| text(&m.name))),
        _ => Value::Nil,
    }
}

/// The name of a light control id, for a light's `control` key.
#[must_use]
pub fn light_control(state: &SceneState, o: &Object) -> Value {
    let ObjData::Light(d) = &o.data else { return Value::Nil };
    let lt = state.tools.part(ObjClass::Light).light();
    match lt.and_then(|l| l.name_of(d.lcontrol)) {
        Some("$static") | None if d.lcontrol == 0 => Value::Nil,
        Some(n) => text(n),
        None => int(d.lcontrol),
    }
}

/// A light's `control` key as its id: a name of the scene's controls, or an id.
pub fn light_control_of(state: &SceneState, v: &Value) -> Result<u32, Bad> {
    let lt = state.tools.part(ObjClass::Light).light();
    match v {
        Value::Number(_) => as_u32(v),
        Value::String(s) => lt
            .and_then(|l| l.lcontrols.iter().find(|(n, _)| n.eq_ignore_ascii_case(s)))
            .map(|(_, id)| *id)
            .ok_or_else(|| format!("'{s}' is no light control of the scene (light_controls in the .scene)")),
        _ => Err("takes the name of a light control".into()),
    }
}

/// A name the scene's light controls do not hold yet, as a fresh id (a light naming a control
/// the `.scene` does not list keeps it).
pub fn light_control_added(state: &mut SceneState, name: &str) -> u32 {
    if let Some(l) = state.tools.part_mut(ObjClass::Light).light_mut() {
        if let Some((_, id)) = l.lcontrols.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            return *id;
        }
        l.append(name, None);
        return l.lcontrols.last().map_or(0, |(_, id)| *id);
    }
    0
}
