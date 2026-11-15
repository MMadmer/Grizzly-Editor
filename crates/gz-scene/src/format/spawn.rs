//! Spawn points in `spawns.lua`: a game entity of a section with its named properties, or a
//! point of the editor's own (the Player Start, a respawn point, an environment modifier), with
//! a stalker's standing order and a zone's shape.
//!
//! The entity's properties are `gz-entities`' named properties ([`gz_entities::Named`]): the
//! fields that differ from a new entity of the section. They are read back into an entity once
//! the linked game's configs are known; until then the spawn keeps them as the file has them.

use std::sync::Arc;

use gz_base::{Blob, SmolStr};
use gz_entities::{Entity, Placement, Settings};
use gz_luadata::{Table, Value};
use serde_json::{Map, Number, Value as Json};

use super::fields::{Field, Ty, game_types, game_types_of, shapes_of, shapes_value};
use super::values::*;
use crate::class::ObjClass;
use crate::model::{ObjData, Object};
use crate::objects::{
    EnvMod, ORDER_HOSTILE, ORDER_RUN, PointKind, RPoint, ShapeData, SpawnData, SpawnEntity, XmsOrder,
    env_flags,
};

/// A Lua value as JSON: a whole number as an integer, a list as an array, a table as an object.
#[must_use]
pub fn to_json(v: &Value) -> Json {
    match v {
        Value::Nil => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 9.0e15 {
                if *n < 0.0 { Json::from(*n as i64) } else { Json::from(*n as u64) }
            } else {
                Number::from_f64(*n).map_or(Json::Null, Json::Number)
            }
        }
        // the spelling of a number the file has no number for (`"-0"`)
        Value::String(s) => match special(s).and_then(Number::from_f64) {
            Some(n) => Json::Number(n),
            None => Json::String(s.to_string()),
        },
        Value::Table(t) => {
            if t.fields.is_empty() && !t.arr.is_empty() {
                Json::Array(t.arr.iter().map(to_json).collect())
            } else {
                let mut m = Map::new();
                for (i, x) in t.arr.iter().enumerate() {
                    m.insert((i + 1).to_string(), to_json(x));
                }
                for (k, x) in &t.fields {
                    m.insert(k.to_string(), to_json(x));
                }
                Json::Object(m)
            }
        }
    }
}

/// JSON as a Lua value, the keys of an object in its order.
#[must_use]
pub fn from_json(v: &Json) -> Value {
    match v {
        Json::Null => Value::Nil,
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => {
            let f = n.as_f64().unwrap_or(0.0);
            // a float of the entity is written as the shortest decimal that reads back to it
            number(if n.is_f64() && f64::from(f as f32) == f { shortest(f as f32) } else { f })
        }
        Json::String(s) => Value::String(SmolStr::new(s)),
        Json::Array(a) => list(a.iter().map(from_json)),
        Json::Object(m) => {
            let mut t = Table::new();
            for (k, x) in m {
                t.put(k, from_json(x));
            }
            Value::Table(t)
        }
    }
}

/// Where the spawn's entity stands: the object's name and transform.
#[must_use]
pub fn placement(o: &Object) -> Placement<'_> {
    Placement { name: &o.name, position: o.xform.position, angle: o.xform.rotation }
}

/// What a spawn's entity is in the file: its section and either its named properties, its
/// packet or the numbered values of the X-Ray SDK text scene it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityText {
    pub section: SmolStr,
    pub props: Value,
    pub raw: Option<Vec<u8>>,
    /// An entity of an X-Ray SDK text scene that has no packet (the linked game does not know
    /// its class): the section's numbered values.
    pub values: Option<Vec<String>>,
}

impl EntityText {
    fn packet(section: SmolStr, raw: Vec<u8>) -> Self {
        Self { section, props: Value::Nil, raw: Some(raw), values: None }
    }
}

/// The numbered values of an entity read from an X-Ray SDK text scene that writes no packet.
fn sdk_values(ent: &Entity) -> Option<Vec<String>> {
    if !ent.to_spawn_packet().is_empty() {
        return None;
    }
    ent.to_scene().map(|v| v.iter().map(gz_entities::IniValue::to_ltx).collect())
}

/// The entity of a spawn as the file keeps it.
pub fn entity_text(
    o: &Object,
    settings: Option<&(dyn Settings + Sync)>,
) -> Result<Option<EntityText>, String> {
    let ObjData::Spawn(d) = &o.data else { return Ok(None) };
    let Some(e) = &d.entity else { return Ok(None) };
    Ok(Some(match e {
        SpawnEntity::Decoded(ent) if let Some(values) = sdk_values(ent) => EntityText {
            section: SmolStr::new(ent.section()),
            props: Value::Nil,
            raw: None,
            values: Some(values),
        },
        SpawnEntity::Decoded(ent) => match settings {
            Some(s) => {
                let named = ent.to_named(&placement(o), s);
                let props =
                    if named.props.is_empty() { Value::Nil } else { from_json(&Json::Object(named.props)) };
                match named.raw {
                    Some(raw) => EntityText::packet(named.section, raw),
                    // what the file holds has to read back to the very packet: else its bytes
                    None if reads_back(o, ent, &named.section, &props, s) => {
                        EntityText { section: named.section, props, raw: None, values: None }
                    }
                    None => EntityText::packet(named.section, ent.to_spawn_packet()),
                }
            }
            // without the configs the entity cannot be told against its section: its packet
            None => EntityText::packet(SmolStr::new(ent.section()), ent.to_spawn_packet()),
        },
        SpawnEntity::Kept { section, props, raw, .. } => EntityText {
            section: section.clone(),
            props: props.clone(),
            raw: raw.as_ref().map(|b| b.to_vec()),
            values: None,
        },
        SpawnEntity::RawPacket { section, packet } => EntityText::packet(section.clone(), packet.to_vec()),
        SpawnEntity::RawLtx { section, values } => EntityText {
            section: section.clone(),
            props: Value::Nil,
            raw: None,
            values: Some(values.clone()),
        },
    }))
}

/// Whether the named properties, written into a file and read from it again with the object's
/// transform as the file keeps it, make the entity's very packet.
fn reads_back(o: &Object, ent: &Entity, section: &str, props: &Value, settings: &dyn Settings) -> bool {
    let mut t = Table::new();
    t.put("props", props.clone());
    let text = gz_luadata::write_asset("", &t);
    let Ok((_, back)) = gz_luadata::LuaDoc::parse_value(&text, "", gz_luadata::Limits::scene()) else {
        return false;
    };
    let props = match back.get("props").map(to_json) {
        Some(Json::Object(m)) => m,
        None => Map::new(),
        Some(_) => return false,
    };
    let at = placement(o);
    Entity::from_named(section, &props, &at, settings)
        .is_ok_and(|b| b.to_spawn_packet() == ent.to_spawn_packet())
}

/// A spawn's kept entity decoded once the configs are known; one that does not read stays kept
/// and says why.
pub fn resolve(o: &mut Object, settings: &dyn Settings) {
    let ObjData::Spawn(d) = &mut o.data else { return };
    if let Some(SpawnEntity::RawLtx { values, .. }) = &d.entity {
        if let Ok(e) = Entity::from_scene(values, settings) {
            d.entity = Some(SpawnEntity::Decoded(Arc::new(e)));
        }
        return;
    }
    let Some(SpawnEntity::Kept { section, props, raw, .. }) = &d.entity else { return };
    let (section, props, raw) = (section.clone(), props.clone(), raw.clone());
    let at = Placement { name: &o.name, position: o.xform.position, angle: o.xform.rotation };
    let made = match &raw {
        Some(packet) => Entity::from_spawn_packet(packet, settings),
        None => {
            let map = match to_json(&props) {
                Json::Object(m) => m,
                Json::Null => Map::new(),
                _ => {
                    d.entity =
                        Some(SpawnEntity::Kept { section, props, raw, why: "props takes a table".into() });
                    return;
                }
            };
            Entity::from_named(&section, &map, &at, settings)
        }
    };
    d.entity = Some(match made {
        Ok(e) => SpawnEntity::Decoded(Arc::new(e)),
        Err(e) => SpawnEntity::Kept { section, props, raw, why: e.to_string() },
    });
}

fn spawn(o: &Object) -> Option<&SpawnData> {
    match &o.data {
        ObjData::Spawn(d) => Some(d),
        _ => None,
    }
}

fn spawn_mut(o: &mut Object) -> Option<&mut SpawnData> {
    match &mut o.data {
        ObjData::Spawn(d) => Some(d),
        _ => None,
    }
}

/// The entity kept as the file has it, made on the first of its keys read.
fn kept_mut(o: &mut Object) -> Option<(&mut SmolStr, &mut Value, &mut Option<Blob>)> {
    let d = spawn_mut(o)?;
    d.kind = PointKind::Spawn;
    if !matches!(d.entity, Some(SpawnEntity::Kept { .. })) {
        let section = match &d.entity {
            Some(SpawnEntity::RawLtx { section, .. }) => section.clone(),
            _ => SmolStr::default(),
        };
        d.entity = Some(SpawnEntity::Kept { section, props: Value::Nil, raw: None, why: String::new() });
    }
    match &mut d.entity {
        Some(SpawnEntity::Kept { section, props, raw, .. }) => Some((section, props, raw)),
        _ => None,
    }
}

const POINTS: &[(&str, u32)] = &[("player_start", 3), ("respawn", 0), ("env_mod", 1)];
const ORDER_KINDS: &[(&str, u32)] = &[("guard", 1), ("patrol", 2), ("wander", 3), ("follow", 4)];
const RPOINT_KINDS: &[(&str, u32)] = &[("actor", 0), ("artefact", 1), ("item", 2)];
const ENV_USES: &[(&str, u32)] = &[
    ("view_distance", env_flags::VIEW_DIST as u32),
    ("fog_color", env_flags::FOG_COLOR as u32),
    ("fog_density", env_flags::FOG_DENSITY as u32),
    ("ambient_color", env_flags::AMBIENT_COLOR as u32),
    ("sky_color", env_flags::SKY_COLOR as u32),
    ("hemi_color", env_flags::HEMI_COLOR as u32),
];

/// `{ kind, route, look, radius, target, distance, run, hostile }`, defaults left out.
#[must_use]
pub fn order_value(o: &XmsOrder) -> Value {
    if o.is_empty() && o == &XmsOrder::default() {
        return Value::Nil;
    }
    let d = XmsOrder::default();
    let mut t = Table::new();
    t.put("kind", word(u32::from(o.kind), ORDER_KINDS).or_none(o.kind));
    for (k, v, dv) in
        [("route", &o.route, &d.route), ("look", &o.look, &d.look), ("target", &o.target, &d.target)]
    {
        if v != dv {
            t.put(k, text(v));
        }
    }
    if o.radius != d.radius {
        t.put("radius", num(o.radius));
    }
    if o.distance != d.distance {
        t.put("distance", num(o.distance));
    }
    if o.flags & ORDER_RUN != 0 {
        t.put("run", true);
    }
    if o.flags & ORDER_HOSTILE != 0 {
        t.put("hostile", true);
    }
    let rest = o.flags & !(ORDER_RUN | ORDER_HOSTILE);
    if rest != 0 {
        t.put("flags", int(rest));
    }
    Value::Table(t)
}

trait OrNone {
    fn or_none(self, kind: u8) -> Value;
}

impl OrNone for Value {
    fn or_none(self, kind: u8) -> Value {
        if kind == 0 { text("none") } else { self }
    }
}

pub fn order_of(v: &Value) -> Result<XmsOrder, Bad> {
    let mut o = XmsOrder::default();
    if v.is_nil() {
        return Ok(o);
    }
    if let Some(k) = v.get("kind") {
        o.kind = match k.as_str() {
            Some(w) if w.eq_ignore_ascii_case("none") => 0,
            _ => as_word_or_number(k, ORDER_KINDS)? as u8,
        };
    }
    for (k, slot) in [("route", &mut o.route), ("look", &mut o.look), ("target", &mut o.target)] {
        if let Some(x) = v.get(k) {
            *slot = as_text(x)?;
        }
    }
    if let Some(x) = v.get("radius") {
        o.radius = as_f32(x)?;
    }
    if let Some(x) = v.get("distance") {
        o.distance = as_f32(x)?;
    }
    let mut flags = v.get("flags").map(as_u8).transpose()?.unwrap_or(0);
    if v.get("run").map(as_bool).transpose()?.unwrap_or(false) {
        flags |= ORDER_RUN;
    }
    if v.get("hostile").map(as_bool).transpose()?.unwrap_or(false) {
        flags |= ORDER_HOSTILE;
    }
    o.flags = flags;
    Ok(o)
}

fn rpoint_value(r: &RPoint) -> Value {
    let d = RPoint::default();
    let mut t = Table::new();
    if r.team != d.team {
        t.put("team", int(r.team));
    }
    if r.kind != d.kind {
        t.put("kind", word(u32::from(r.kind), RPOINT_KINDS));
    }
    if r.game_type != d.game_type {
        t.put("game_types", game_types(r.game_type));
    }
    if r.profile != d.profile {
        t.put("profile", text(&r.profile));
    }
    Value::Table(t)
}

fn rpoint_of(v: &Value) -> Result<RPoint, Bad> {
    let mut r = RPoint::default();
    if let Some(x) = v.get("team") {
        r.team = as_u8(x)?;
    }
    if let Some(x) = v.get("kind") {
        r.kind = as_word_or_number(x, RPOINT_KINDS)? as u8;
    }
    if let Some(x) = v.get("game_types") {
        r.game_type = game_types_of(x)?;
    }
    if let Some(x) = v.get("profile") {
        r.profile = as_text(x)?;
    }
    Ok(r)
}

fn env_value(e: &EnvMod) -> Value {
    let d = EnvMod::default();
    let mut t = Table::new();
    let f = |t: &mut Table, k: &str, a: f32, b: f32| {
        if a != b {
            t.put(k, num(a));
        }
    };
    f(&mut t, "radius", e.radius, d.radius);
    f(&mut t, "power", e.power, d.power);
    f(&mut t, "view_distance", e.view_dist, d.view_dist);
    f(&mut t, "fog_density", e.fog_density, d.fog_density);
    for (k, a, b) in [
        ("fog_color", e.fog_color, d.fog_color),
        ("ambient_color", e.ambient_color, d.ambient_color),
        ("sky_color", e.sky_color, d.sky_color),
        ("hemi_color", e.hemi_color, d.hemi_color),
    ] {
        if a != b {
            t.put(k, argb(a));
        }
    }
    if e.flags != d.flags {
        let mut uses: Vec<Value> =
            ENV_USES.iter().filter(|(_, b)| u32::from(e.flags) & b != 0).map(|(w, _)| text(w)).collect();
        let named: u32 = ENV_USES.iter().map(|(_, b)| b).sum();
        let rest = u32::from(e.flags) & !named;
        if rest != 0 {
            uses.push(int(rest));
        }
        t.put("uses", list(uses));
    }
    Value::Table(t)
}

fn env_of(v: &Value) -> Result<EnvMod, Bad> {
    let mut e = EnvMod::default();
    for (k, slot) in [
        ("radius", &mut e.radius),
        ("power", &mut e.power),
        ("view_distance", &mut e.view_dist),
        ("fog_density", &mut e.fog_density),
    ] {
        if let Some(x) = v.get(k) {
            *slot = as_f32(x)?;
        }
    }
    for (k, slot) in [
        ("fog_color", &mut e.fog_color),
        ("ambient_color", &mut e.ambient_color),
        ("sky_color", &mut e.sky_color),
        ("hemi_color", &mut e.hemi_color),
    ] {
        if let Some(x) = v.get(k) {
            *slot = as_argb(x)?;
        }
    }
    if let Some(u) = v.get("uses") {
        let mut mask = 0u32;
        for x in u.arr() {
            mask |= match x {
                Value::Number(_) => as_u32(x)?,
                _ => as_word(x, ENV_USES)?,
            };
        }
        e.flags = u16::try_from(mask).map_err(|_| "uses names a flag that is not there".to_owned())?;
    }
    Ok(e)
}

/// The zone's own shape: its spheres and boxes, and its transform where it is not the point's.
fn shape_value(o: &Object, a: &Object) -> Value {
    let ObjData::Shape(s) = &a.data else { return Value::Nil };
    let mut t = Table::new();
    if !a.name.is_empty() {
        t.put("name", text(&a.name));
    }
    t.put("shapes", shapes_value(s));
    if a.xform.position != o.xform.position {
        t.put("pos", vec3(a.xform.position));
    }
    if a.xform.rotation != o.xform.rotation {
        t.put("rot", angles(a.xform.rotation));
    }
    if a.xform.scale != o.xform.scale {
        t.put("scale", vec3(a.xform.scale));
    }
    if s.usage != 0 {
        t.put("usage", int(s.usage));
    }
    if a.flags.co != 0 {
        t.put("flags", int(a.flags.co));
    }
    Value::Table(t)
}

/// The shape of a zone as read: the point's transform unless it names its own. The point's
/// keys are read first (`pos`, `rot`, `scale` stand before `shape`).
fn shape_of(o: &Object, v: &Value) -> Result<Object, Bad> {
    let mut data = ShapeData { shapes: shapes_of(v.get("shapes").unwrap_or(&Value::Nil))?, usage: 0 };
    if let Some(u) = v.get("usage") {
        data.usage = as_u8(u)?;
    }
    let mut a = Object::new(ObjClass::Shape, "", ObjData::Shape(data));
    if let Some(n) = v.get("name") {
        a.name = as_text(n)?;
    }
    a.xform = o.xform;
    if let Some(p) = v.get("pos") {
        a.xform.position = as_vec3(p)?;
    }
    if let Some(r) = v.get("rot") {
        a.xform.rotation = as_angles(r)?;
    }
    if let Some(s) = v.get("scale") {
        a.xform.scale = as_vec3(s)?;
    }
    if let Some(f) = v.get("flags") {
        a.flags.co = as_u32(f)?;
    }
    Ok(a)
}

fn is_entity(o: &Object) -> bool {
    spawn(o).is_some_and(|d| d.entity.is_some())
}

fn is_point(o: &Object) -> bool {
    spawn(o).is_some_and(|d| d.entity.is_none())
}

pub static FIELDS: &[Field<Object>] = &[
    Field {
        key: "section",
        label: "Section",
        group: "Spawn",
        unit: "",
        ty: Ty::Pick("spawn_section"),
        default: "",
        help: "The game's section the spawned thing is made from (`game.lookup type=spawn_section` lists them).",
        applies: is_entity,
        get: |o| match spawn(o).and_then(|d| d.entity.as_ref()) {
            Some(e) => text(e.section()),
            None => Value::Nil,
        },
        set: |o, v| {
            let s = as_text(v)?;
            if let Some(SpawnEntity::RawLtx { section, .. }) = spawn_mut(o).and_then(|d| d.entity.as_mut()) {
                *section = s;
            } else if let Some((section, _, _)) = kept_mut(o) {
                *section = s;
            }
            Ok(())
        },
    },
    Field {
        key: "point",
        label: "Point",
        group: "Spawn",
        unit: "",
        ty: Ty::Words(POINTS),
        default: "",
        help: "A point of the editor's own instead of a section: the module's Player Start, a respawn point of the multiplayer games, an environment modifier.",
        applies: is_point,
        get: |o| match spawn(o) {
            Some(d) if d.entity.is_none() => word(d.kind as u32, POINTS),
            _ => Value::Nil,
        },
        set: |o, v| {
            let k = as_word(v, POINTS)?;
            if let Some(d) = spawn_mut(o) {
                d.entity = None;
                d.kind = PointKind::from_u32(k).unwrap_or(PointKind::PlayerStart);
            }
            Ok(())
        },
    },
    Field {
        key: "props",
        label: "Properties",
        group: "Spawn",
        unit: "",
        ty: Ty::Table(
            "the entity's named properties that differ from a new entity of the section (`story_id`, `custom_data`, `visual`, `trader = { money = 500 }`, ...); `asset.reference kind=scene` lists them",
        ),
        default: "none",
        help: "What this spawn says beyond its section's defaults.",
        applies: is_entity,
        // the scene's to give: see `entity_text`
        get: |_| Value::Nil,
        set: |o, v| {
            let v = v.clone();
            if let Some((_, props, _)) = kept_mut(o) {
                *props = v;
            }
            Ok(())
        },
    },
    Field {
        key: "raw",
        label: "Kept Packet",
        group: "Spawn",
        unit: "",
        ty: Ty::Text,
        default: "none",
        help: "The entity's spawn packet as hex, for an entity the named properties cannot carry (its class is not known, or it holds more than its class writes): written back as it was.",
        applies: is_entity,
        get: |_| Value::Nil,
        set: |o, v| {
            let b = as_hex(v)?;
            if let Some((_, _, raw)) = kept_mut(o) {
                *raw = Some(Blob::from(b.as_slice()));
            }
            Ok(())
        },
    },
    Field {
        key: "values",
        label: "Kept Values",
        group: "Spawn",
        unit: "",
        ty: Ty::Table("a list of texts"),
        default: "none",
        help: "The numbered values an X-Ray SDK text scene gave an entity whose class the linked game does not know: written back as they were.",
        applies: is_entity,
        // the scene's to give: see `entity_text`
        get: |_| Value::Nil,
        set: |o, v| {
            let values =
                v.arr().iter().map(|x| as_text(x).map(|t| t.to_string())).collect::<Result<Vec<_>, _>>()?;
            if v.as_table().is_none_or(|t| !t.fields.is_empty()) {
                return Err("takes a list of texts".into());
            }
            if let Some(d) = spawn_mut(o) {
                let section = d.entity.as_ref().map(|e| SmolStr::new(e.section())).unwrap_or_default();
                d.kind = PointKind::Spawn;
                d.entity = Some(SpawnEntity::RawLtx { section, values });
            }
            Ok(())
        },
    },
    Field {
        key: "respawn",
        label: "Respawns in Multiplayer",
        group: "Spawn",
        unit: "",
        ty: Ty::Bool,
        default: "false",
        help: "The multiplayer games spawn it again after it is gone.",
        applies: is_entity,
        get: |o| Value::Bool(spawn(o).is_some_and(|d| d.sd_flags & 1 != 0)),
        set: |o, v| {
            let on = as_bool(v)?;
            if let Some(d) = spawn_mut(o) {
                d.sd_flags = if on { d.sd_flags | 1 } else { d.sd_flags & !1 };
            }
            Ok(())
        },
    },
    Field {
        key: "spawn_flags",
        label: "Other Spawn Flags",
        group: "Spawn",
        unit: "",
        ty: Ty::Integer { min: 0, max: 255 },
        default: "0",
        help: "Bits of the spawn's flags no key names.",
        applies: |o| spawn(o).is_some_and(|d| d.sd_flags & !1 != 0),
        get: |o| int(spawn(o).map_or(0, |d| d.sd_flags & !1)),
        set: |o, v| {
            let n = as_u8(v)?;
            if let Some(d) = spawn_mut(o) {
                d.sd_flags = (d.sd_flags & 1) | (n & !1);
            }
            Ok(())
        },
    },
    Field {
        key: "order",
        label: "Order",
        group: "Order",
        unit: "",
        ty: Ty::Table(
            "`{ kind = \"guard\" | \"patrol\" | \"wander\" | \"follow\", route = way, look = way, radius = m, target = name, distance = m, run = true, hostile = true }`",
        ),
        default: "none",
        help: "A placed stalker's standing order; the build writes it into the entity's custom data as `[xms_order]`.",
        applies: |o| spawn(o).is_some_and(|d| d.can_take_order() || d.order != XmsOrder::default()),
        get: |o| spawn(o).map_or(Value::Nil, |d| order_value(&d.order)),
        set: |o, v| {
            let order = order_of(v)?;
            if let Some(d) = spawn_mut(o) {
                d.order = order;
            }
            Ok(())
        },
    },
    Field {
        key: "radio",
        label: "Radio",
        group: "Radio",
        unit: "",
        ty: Ty::Table(
            "`{ plays = \"<playlist id>\" | \"<module>.<playlist>\" | \"game\" | \"off\" }`: a playlist of the module by its id (`mod.ltx [playlist.<id>]`), another module's, the game's music alone, silence",
        ),
        default: "none: the player's station",
        help: "What a radio of the world plays whatever station the player chose; the build writes it into the entity's custom data as `[xms_radio]`.",
        applies: |o| spawn(o).is_some_and(|d| !d.plays.is_empty()),
        get: |o| match spawn(o) {
            Some(d) if !d.plays.is_empty() => {
                let mut t = Table::new();
                t.put("plays", text(&d.plays));
                Value::Table(t)
            }
            _ => Value::Nil,
        },
        set: |o, v| {
            let plays = match v {
                Value::Nil => SmolStr::default(),
                _ => match v.get("plays") {
                    Some(p) => as_text(p)?,
                    None if v.as_table().is_some() => SmolStr::default(),
                    None => return Err("takes `{ plays = \"<playlist>\" }`".into()),
                },
            };
            if let Some(d) = spawn_mut(o) {
                d.plays = plays;
            }
            Ok(())
        },
    },
    Field {
        key: "respawn_point",
        label: "Respawn Point",
        group: "Respawn Point",
        unit: "",
        ty: Ty::Table(
            "`{ team = 1, kind = \"actor\" | \"artefact\" | \"item\", game_types = {...}, profile = section }`",
        ),
        default: "`{ team = 1, kind = \"actor\" }`",
        help: "Who or what respawns there in the multiplayer games.",
        applies: |o| spawn(o).is_some_and(|d| d.entity.is_none() && d.kind == PointKind::RPoint),
        get: |o| spawn(o).map_or(Value::Nil, |d| rpoint_value(&d.rpoint)),
        set: |o, v| {
            let r = rpoint_of(v)?;
            if let Some(d) = spawn_mut(o) {
                d.rpoint = r;
            }
            Ok(())
        },
    },
    Field {
        key: "environment",
        label: "Environment",
        group: "Environment Modifier",
        unit: "",
        ty: Ty::Table(
            "`{ radius = m, power, view_distance = m, fog_color = { r, g, b }, fog_density, ambient_color, sky_color, hemi_color, uses = { \"view_distance\", \"fog_color\", ... } }`",
        ),
        default: "radius 10, power 1, view distance 300, every value used",
        help: "How the weather looks within the modifier's radius.",
        applies: |o| spawn(o).is_some_and(|d| d.entity.is_none() && d.kind == PointKind::EnvMod),
        get: |o| spawn(o).map_or(Value::Nil, |d| env_value(&d.env)),
        set: |o, v| {
            let e = env_of(v)?;
            if let Some(d) = spawn_mut(o) {
                d.env = e;
            }
            Ok(())
        },
    },
    Field {
        key: "shape",
        label: "Shape",
        group: "Shape",
        unit: "",
        ty: Ty::Table(
            "`{ shapes = { ... } }` as in `shapes.lua`, `pos`, `rot`, `scale` where they are not the point's",
        ),
        default: "none",
        help: "The volume of a zone, a restrictor or a level changer.",
        applies: |o| spawn(o).is_some_and(|d| d.attached.is_some()),
        get: |o| match spawn(o).and_then(|d| d.attached.as_ref()) {
            Some(a) => shape_value(o, a),
            None => Value::Nil,
        },
        set: |o, v| {
            let a = if v.is_nil() { None } else { Some(Arc::new(shape_of(o, v)?)) };
            if let Some(d) = spawn_mut(o) {
                d.attached = a;
            }
            Ok(())
        },
    },
];
