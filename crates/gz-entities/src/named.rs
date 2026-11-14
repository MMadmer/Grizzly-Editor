//! An entity as named properties: what a scene file keeps of a spawn point.
//!
//! The names are the fields of the entity's model ([`crate::model`]): `story_id`,
//! `custom_data`, `visual`, `trader = { money = 500 }`, ... Only what differs from a new entity
//! of the same section is kept, so a spawn that was placed and never edited names nothing. Its
//! header's name, position and angle are the scene object's own and are named only where they
//! differ from it.
//!
//! [`Entity::to_named`] proves what it answers: the named properties read back
//! ([`Entity::from_named`]) must give the very spawn packet the entity writes. An entity that
//! cannot be named that way - one this crate keeps as bytes, one with bytes after the fields of
//! its class - is kept as its packet instead ([`Named::raw`]).

use glam::Vec3;
use serde_json::{Map, Value};
use smol_str::SmolStr;

use crate::class::class_of_section;
use crate::create::construct;
use crate::entity::{Entity, Header, Inner, Known, M_SPAWN_OBJECT_LOCAL, M_SPAWN_VERSION, SPAWN_VERSION};
use crate::model::Object;
use crate::packet::TextEncoding;
use crate::settings::Settings;
use crate::{EntityError, Result};

/// The key of the header's fields among the named properties.
pub const HEADER: &str = "header";

/// The scene object a spawn's entity belongs to: what its header says unless it says otherwise.
#[derive(Clone, Copy, Debug)]
pub struct Placement<'a> {
    pub name: &'a str,
    pub position: Vec3,
    /// Radians, as the header holds them.
    pub angle: Vec3,
}

/// What a scene file keeps of an entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Named {
    /// The spawn section.
    pub section: SmolStr,
    /// The properties that differ from a new entity of the section; a part the entity lacks is
    /// `false`.
    pub props: Map<String, Value>,
    /// The spawn packet, when the names cannot carry the entity; `props` is empty then.
    pub raw: Option<Vec<u8>>,
}

/// The bits every header written carries, whatever the entity says.
const ALWAYS: u16 = M_SPAWN_VERSION | M_SPAWN_OBJECT_LOCAL;

fn header_tree(h: &Header) -> Value {
    let mut m = Map::new();
    m.insert("name".into(), Value::from(h.name.as_str()));
    m.insert("position".into(), vec3(h.position));
    m.insert("angle".into(), vec3(h.angle));
    m.insert("rp".into(), Value::from(h.rp));
    m.insert("respawn_time".into(), Value::from(h.respawn_time));
    m.insert("id".into(), Value::from(h.id));
    m.insert("id_parent".into(), Value::from(h.id_parent));
    m.insert("id_phantom".into(), Value::from(h.id_phantom));
    m.insert("flags".into(), Value::from(h.flags & !ALWAYS));
    m.insert("game_type".into(), Value::from(h.game_type));
    m.insert("client_data".into(), Value::from(h.client_data.clone()));
    m.insert("spawn_id".into(), Value::from(h.spawn_id));
    m.insert(
        "text".into(),
        Value::from(match h.text {
            TextEncoding::Cp1251 => "cp1251",
            TextEncoding::Utf8 => "utf8",
        }),
    );
    Value::Object(m)
}

fn vec3(v: Vec3) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn header_from(tree: &Value, h: &mut Header) -> Result<()> {
    let bad = |key: &str| EntityError::WrongType { path: format!("{HEADER}.{key}"), expected: "a number" };
    let Value::Object(m) = tree else { return Err(bad("")) };
    let num = |key: &str| m.get(key).and_then(Value::as_u64).ok_or_else(|| bad(key));
    let v3 = |key: &str| -> Result<Vec3> {
        m.get(key).and_then(|v| serde_json::from_value(v.clone()).ok()).ok_or_else(|| bad(key))
    };
    h.name = SmolStr::new(m.get("name").and_then(Value::as_str).ok_or_else(|| bad("name"))?);
    h.position = v3("position")?;
    h.angle = v3("angle")?;
    h.rp = u8::try_from(num("rp")?).map_err(|_| bad("rp"))?;
    h.respawn_time = u16::try_from(num("respawn_time")?).map_err(|_| bad("respawn_time"))?;
    h.id = u16::try_from(num("id")?).map_err(|_| bad("id"))?;
    h.id_parent = u16::try_from(num("id_parent")?).map_err(|_| bad("id_parent"))?;
    h.id_phantom = u16::try_from(num("id_phantom")?).map_err(|_| bad("id_phantom"))?;
    h.flags = u16::try_from(num("flags")?).map_err(|_| bad("flags"))?;
    h.game_type = u16::try_from(num("game_type")?).map_err(|_| bad("game_type"))?;
    h.spawn_id = u16::try_from(num("spawn_id")?).map_err(|_| bad("spawn_id"))?;
    h.client_data = m
        .get("client_data")
        .and_then(|v| serde_json::from_value::<Vec<u8>>(v.clone()).ok())
        .ok_or_else(|| bad("client_data"))?;
    h.text = match m.get("text").and_then(Value::as_str) {
        Some("utf8") => TextEncoding::Utf8,
        Some("cp1251") => TextEncoding::Cp1251,
        _ => return Err(bad("text")),
    };
    Ok(())
}

/// The whole tree of an entity: its parts by their names, the header under [`HEADER`].
fn tree(h: &Header, o: &Object) -> Value {
    let mut v = serde_json::to_value(o).unwrap_or_else(|_| Value::Object(Map::new()));
    if let Value::Object(m) = &mut v {
        m.insert(HEADER.into(), header_tree(h));
    }
    v
}

/// A new entity of the section as the scene object would make it: the baseline the named
/// properties are told against.
fn baseline(
    section: &str,
    at: &Placement<'_>,
    settings: &dyn Settings,
) -> Result<(&'static crate::class::ClassInfo, Header, Object)> {
    let info = class_of_section(section, settings)
        .ok_or_else(|| EntityError::UnknownClass(SmolStr::new(section)))?;
    let (mut h, o) = construct(info, section, settings)?;
    h.name = SmolStr::new(at.name);
    h.position = at.position;
    h.angle = at.angle;
    h.version = SPAWN_VERSION;
    h.script_version = h.write_script_version;
    Ok((info, h, o))
}

/// What differs from `base` in `now`: a changed value whole, a table by its keys, a part
/// `now` lacks or a value it has none of as `false`. `None` when they are equal.
#[must_use]
pub fn diff(base: &Value, now: &Value) -> Option<Value> {
    if base == now {
        return None;
    }
    match (base, now) {
        (Value::Object(b), Value::Object(n)) => {
            let mut out = Map::new();
            for (k, nv) in n {
                match b.get(k) {
                    Some(bv) => {
                        if let Some(d) = diff(bv, nv) {
                            out.insert(k.clone(), d);
                        }
                    }
                    None => {
                        out.insert(k.clone(), nv.clone());
                    }
                }
            }
            for (k, bv) in b {
                if !n.contains_key(k) && bv.is_object() {
                    out.insert(k.clone(), Value::Bool(false));
                }
            }
            (!out.is_empty()).then_some(Value::Object(out))
        }
        // a value gone (an option that is none) is `false`: a text file has no null
        (b, Value::Null) if !b.is_boolean() => Some(Value::Bool(false)),
        _ => Some(now.clone()),
    }
}

/// `props` laid over `base`: a table merged by its keys, `false` where `base` holds a table
/// removes it and where it holds another value that is no flag makes it none, anything else
/// replaces.
pub fn overlay(base: &mut Value, props: &Value) {
    let (Value::Object(b), Value::Object(p)) = (base, props) else { return };
    for (k, pv) in p {
        match (b.get_mut(k), pv) {
            (Some(bv), Value::Bool(false)) if bv.is_object() => {
                b.remove(k);
            }
            (Some(bv), Value::Bool(false)) if !bv.is_boolean() && !bv.is_null() => {
                *bv = Value::Null;
            }
            (Some(bv @ Value::Object(_)), Value::Object(_)) => overlay(bv, pv),
            (None, Value::Bool(false)) => {}
            _ => {
                b.insert(k.clone(), pv.clone());
            }
        }
    }
}

impl Entity {
    /// The named properties of this entity (see the module): proven to read back to the same
    /// spawn packet, else the packet itself.
    #[must_use]
    pub fn to_named(&self, at: &Placement<'_>, settings: &dyn Settings) -> Named {
        let section = SmolStr::new(self.section());
        let packet = self.to_spawn_packet();
        let kept = || Named { section: section.clone(), props: Map::new(), raw: Some(packet.clone()) };
        let Inner::Known(k) = &self.inner else { return kept() };
        let Ok((_, bh, bo)) = baseline(&section, at, settings) else { return kept() };
        let props = match diff(&tree(&bh, &bo), &tree(&k.header, &k.object)) {
            Some(Value::Object(m)) => m,
            _ => Map::new(),
        };
        match Entity::from_named(&section, &props, at, settings) {
            Ok(back) if back.to_spawn_packet() == packet => Named { section, props, raw: None },
            _ => kept(),
        }
    }

    /// The entity named properties describe, over a new entity of `section`.
    pub fn from_named(
        section: &str,
        props: &Map<String, Value>,
        at: &Placement<'_>,
        settings: &dyn Settings,
    ) -> Result<Entity> {
        let (info, mut header, object) = baseline(section, at, settings)?;
        let mut t = tree(&header, &object);
        overlay(&mut t, &Value::Object(props.clone()));
        let Value::Object(mut m) = t else { return Err(EntityError::NoProperty(String::new())) };
        let head = m.remove(HEADER).unwrap_or(Value::Null);
        header_from(&head, &mut header)?;
        let object: Object = serde_json::from_value(Value::Object(m))
            .map_err(|e| EntityError::WrongType { path: e.to_string(), expected: "what its field holds" })?;
        Ok(Entity {
            inner: Inner::Known(Box::new(Known {
                header,
                info,
                object,
                raw_spawn: None,
                raw_update: None,
                tail: std::sync::Arc::from([]),
                ini_tail: std::sync::Arc::from([]),
                raw_ini: None,
                made: true,
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create::fixtures::settings;
    use crate::props::PropContext;

    fn at() -> Placement<'static> {
        Placement { name: "box_01", position: Vec3::new(1.0, 2.0, 3.0), angle: Vec3::new(0.0, 0.5, 0.0) }
    }

    #[test]
    fn a_new_entity_names_nothing_and_an_edit_names_itself() {
        let s = settings();
        let mut custom = 0;
        for section in [
            "script_object",
            "space_restrictor",
            "smart_terrain",
            "level_changer",
            "anomaly",
            "lamp",
            "squad",
        ] {
            let mut e = Entity::create(section, &s).unwrap();
            let p = at();
            e.set_name(p.name).unwrap();
            e.set_position(p.position).unwrap();
            e.set_angle(p.angle).unwrap();
            let named = e.to_named(&p, &s);
            assert!(named.raw.is_none(), "{section}");
            assert!(named.props.is_empty(), "{section}: {:?}", named.props);
            let props = e.properties(&PropContext::default());
            if let Some(cd) = props.iter().find(|x| x.path.ends_with("Custom data")) {
                e.set(
                    &cd.path,
                    &crate::PropValue::Text("[logic]\nactive = x".into()),
                    &PropContext::default(),
                )
                .unwrap();
                let named = e.to_named(&p, &s);
                assert!(named.raw.is_none());
                assert_eq!(
                    named.props.get("custom_data").and_then(Value::as_str),
                    Some("[logic]\nactive = x")
                );
                let back = Entity::from_named(section, &named.props, &p, &s).unwrap();
                assert_eq!(back.to_spawn_packet(), e.to_spawn_packet());
                custom += 1;
            }
        }
        assert!(custom >= 4, "{custom}");
    }

    #[test]
    fn the_header_names_only_what_the_object_does_not_say() {
        let s = settings();
        let mut e = Entity::create("script_object", &s).unwrap();
        e.set_name("other").unwrap();
        let named = e.to_named(&at(), &s);
        assert!(named.raw.is_none());
        let h = named.props.get(HEADER).and_then(Value::as_object).unwrap();
        assert_eq!(h.get("name").and_then(Value::as_str), Some("other"));
        assert!(h.get("rp").is_none());
    }

    #[test]
    fn a_kept_entity_is_its_packet() {
        let s = settings();
        let e = Entity::create("script_object", &s).unwrap();
        let mut packet = e.to_spawn_packet();
        // a section the configs do not know
        let at = packet.iter().position(|b| *b == b's').unwrap();
        packet[at] = b'q';
        let opaque = Entity::from_spawn_packet(&packet, &s).unwrap();
        assert!(opaque.is_opaque());
        let named = opaque.to_named(&self::at(), &s);
        assert_eq!(named.raw.as_deref(), Some(&packet[..]));
        assert!(named.props.is_empty());
    }

    #[test]
    fn diff_and_overlay_are_inverse() {
        let base = serde_json::json!({ "a": 1, "t": { "x": 1, "y": 2 }, "part": { "z": 0 } });
        let now = serde_json::json!({ "a": 2, "t": { "x": 1, "y": 3 } });
        let d = diff(&base, &now).unwrap();
        assert_eq!(d, serde_json::json!({ "a": 2, "t": { "y": 3 }, "part": false }));
        let mut b = base.clone();
        overlay(&mut b, &d);
        assert_eq!(b, now);
    }
}
