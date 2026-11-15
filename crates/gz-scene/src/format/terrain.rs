//! The keys of a terrain (`terrain.lua`, the list `terrains`; the contract is
//! `docs/plan/LEVEL_DESIGN.md` 2.5). `pos` is the centre and `rot` turns it about the vertical
//! alone; the rest is here.

use glam::Vec2;
use gz_luadata::{Table, Value};
use gz_terrain::{Base, CELL_MAX, CELL_MIN, Edit, Look, Spec, Surface};

use super::fields::{Field, Ty};
use super::values::*;
use crate::model::{ObjData, Object};

fn spec(o: &Object) -> Option<&Spec> {
    match &o.data {
        ObjData::Terrain(d) => Some(d),
        _ => None,
    }
}

fn spec_mut(o: &mut Object) -> Option<&mut Spec> {
    match &mut o.data {
        ObjData::Terrain(d) => Some(d),
        _ => None,
    }
}

fn is_terrain(o: &Object) -> bool {
    matches!(o.data, ObjData::Terrain(_))
}

fn on_ground(o: &Object) -> bool {
    spec(o).is_some_and(|s| s.base == Base::Ground)
}

fn pair(v: Vec2) -> Value {
    list([num(v.x), num(v.y)])
}

fn as_pair(v: &Value, what: &str) -> Result<Vec2, Bad> {
    let t = v
        .as_table()
        .filter(|t| t.arr.len() == 2 && t.fields.is_empty())
        .ok_or_else(|| format!("takes {what}"))?;
    let x = as_f32(&t.arr[0]).map_err(|_| format!("takes {what}"))?;
    let z = as_f32(&t.arr[1]).map_err(|_| format!("takes {what}"))?;
    Ok(Vec2::new(x, z))
}

fn finite(v: f32, what: &str) -> Result<f32, Bad> {
    if v.is_finite() { Ok(v) } else { Err(format!("takes a finite number of {what}")) }
}

/// An edit as the file writes it: `op` first, then its keys.
fn edit_value(e: &Edit) -> Value {
    let mut t = Table::new();
    t.put("op", text(e.op()));
    match e {
        Edit::Flatten { at, radius, falloff, height } => {
            t.put("at", pair(*at));
            t.put("radius", num(*radius));
            t.put("falloff", num(*falloff));
            t.put("height", num(*height));
        }
        Edit::Rect { min, max, falloff, height } => {
            t.put("min", pair(*min));
            t.put("max", pair(*max));
            t.put("falloff", num(*falloff));
            t.put("height", num(*height));
        }
        Edit::Raise { at, radius, falloff, amount } => {
            t.put("at", pair(*at));
            t.put("radius", num(*radius));
            t.put("falloff", num(*falloff));
            t.put("amount", num(*amount));
        }
        Edit::Ramp { from, to, width, falloff, heights } => {
            t.put("from", pair(*from));
            t.put("to", pair(*to));
            t.put("width", num(*width));
            t.put("falloff", num(*falloff));
            t.put("heights", list([num(heights[0]), num(heights[1])]));
        }
        Edit::Smooth { at, radius, falloff, strength } => {
            t.put("at", pair(*at));
            t.put("radius", num(*radius));
            if *falloff != 0.0 {
                t.put("falloff", num(*falloff));
            }
            t.put("strength", num(*strength));
        }
        Edit::Noise { amount, scale, seed } => {
            t.put("amount", num(*amount));
            t.put("scale", num(*scale));
            t.put("seed", int(*seed));
        }
    }
    Value::Table(t)
}

/// The keys of each `op`, for the sentence of a key it does not take.
fn keys_of(op: &str) -> &'static [&'static str] {
    match op {
        "flatten" => &["at", "radius", "falloff", "height"],
        "rect" => &["min", "max", "falloff", "height"],
        "raise" => &["at", "radius", "falloff", "amount"],
        "ramp" => &["from", "to", "width", "falloff", "heights"],
        "smooth" => &["at", "radius", "falloff", "strength"],
        _ => &["amount", "scale", "seed"],
    }
}

fn edit_of(v: &Value) -> Result<Edit, Bad> {
    let t = v.as_table().ok_or("takes a table: { op = \"flatten\", ... }")?;
    let op = t
        .get("op")
        .and_then(Value::as_str)
        .ok_or("takes an op: flatten, rect, raise, ramp, smooth or noise")?;
    let op = op.to_ascii_lowercase();
    if !Edit::OPS.contains(&op.as_str()) {
        return Err(format!("has the unknown op '{op}': flatten, rect, raise, ramp, smooth or noise"));
    }
    if let Some((k, _)) = t.fields.iter().find(|(k, _)| k != "op" && !keys_of(&op).contains(&k.as_str())) {
        return Err(format!("({op}) takes no key '{k}': it takes {}", keys_of(&op).join(", ")));
    }
    let need = |k: &str| t.get(k).ok_or_else(|| format!("({op}) takes {k}"));
    let number = |k: &str, default: Option<f32>| -> Result<f32, Bad> {
        match (t.get(k), default) {
            (Some(x), _) => as_f32(x).map_err(|e| format!("({op}) {k} {e}")),
            (None, Some(d)) => Ok(d),
            (None, None) => Err(format!("({op}) takes {k}")),
        }
    };
    let at = |k: &str| as_pair(need(k)?, "{ x, z } in metres").map_err(|e| format!("({op}) {k} {e}"));
    let e = match op.as_str() {
        "flatten" => Edit::Flatten {
            at: at("at")?,
            radius: number("radius", None)?,
            falloff: number("falloff", Some(0.0))?,
            height: number("height", None)?,
        },
        "rect" => Edit::Rect {
            min: at("min")?,
            max: at("max")?,
            falloff: number("falloff", Some(0.0))?,
            height: number("height", None)?,
        },
        "raise" => Edit::Raise {
            at: at("at")?,
            radius: number("radius", None)?,
            falloff: number("falloff", Some(0.0))?,
            amount: number("amount", None)?,
        },
        "ramp" => {
            let h = as_pair(need("heights")?, "{ from, to } in metres")
                .map_err(|e| format!("(ramp) heights {e}"))?;
            Edit::Ramp {
                from: at("from")?,
                to: at("to")?,
                width: number("width", None)?,
                falloff: number("falloff", Some(0.0))?,
                heights: [h.x, h.y],
            }
        }
        "smooth" => Edit::Smooth {
            at: at("at")?,
            radius: number("radius", None)?,
            falloff: number("falloff", Some(0.0))?,
            strength: number("strength", Some(0.5))?,
        },
        _ => Edit::Noise {
            amount: number("amount", None)?,
            scale: number("scale", Some(8.0))?,
            seed: t
                .get("seed")
                .map(as_u32)
                .transpose()
                .map_err(|e| format!("(noise) seed {e}"))?
                .unwrap_or(0),
        },
    };
    e.check()?;
    Ok(e)
}

fn edits_of(v: &Value) -> Result<Vec<Edit>, Bad> {
    let t = v.as_table().ok_or("takes a list of edits")?;
    t.arr.iter().enumerate().map(|(i, x)| edit_of(x).map_err(|e| format!("item {}: {e}", i + 1))).collect()
}

fn rows_of(v: &Value) -> Result<Vec<Vec<f32>>, Bad> {
    let t = v.as_table().ok_or("takes a list of rows of heights")?;
    t.arr
        .iter()
        .enumerate()
        .map(|(j, r)| {
            let r = r.as_table().ok_or_else(|| format!("row {} is no list of heights", j + 1))?;
            r.arr
                .iter()
                .map(|h| {
                    as_f32(h).and_then(|h| finite(h, "metres")).map_err(|e| format!("row {}: {e}", j + 1))
                })
                .collect()
        })
        .collect()
}

fn surface_value(s: &Surface) -> Value {
    match s {
        Surface::Ground => text("ground"),
        Surface::Look(l) => {
            let d = Look::default();
            let mut t = Table::new();
            t.put("texture", text(&l.texture));
            for (k, v, dv) in [
                ("shader", &l.shader, &d.shader),
                ("compile", &l.compile, &d.compile),
                ("material", &l.material, &d.material),
            ] {
                if v != dv {
                    t.put(k, text(v));
                }
            }
            if l.tile != d.tile {
                t.put("tile", num(l.tile));
            }
            Value::Table(t)
        }
    }
}

fn surface_of(v: &Value) -> Result<Surface, Bad> {
    if let Some(w) = v.as_str() {
        return if w.eq_ignore_ascii_case("ground") {
            Ok(Surface::Ground)
        } else {
            Err("takes \"ground\" or { texture, shader, compile, material, tile }".into())
        };
    }
    let t = v.as_table().ok_or("takes \"ground\" or { texture, shader, compile, material, tile }")?;
    let mut l = Look::default();
    for (k, x) in &t.fields {
        match k.as_str() {
            "texture" => l.texture = as_text(x)?.to_string(),
            "shader" => l.shader = as_text(x)?.to_string(),
            "compile" => l.compile = as_text(x)?.to_string(),
            "material" => l.material = as_text(x)?.to_string(),
            "tile" => l.tile = as_f32(x)?,
            other => return Err(format!("takes no key '{other}': texture, shader, compile, material, tile")),
        }
    }
    if !(l.tile.is_finite() && l.tile > 0.0) {
        return Err("takes a tile above 0 metres".into());
    }
    Ok(Surface::Look(l))
}

macro_rules! key {
    ($key:literal, $label:literal, $unit:literal, $ty:expr, $default:literal, $help:literal,
     |$s:ident| $get:expr, |$sm:ident, $v:ident| $set:expr $(, applies = $applies:expr)?) => {
        Field {
            key: $key,
            label: $label,
            group: "Terrain",
            unit: $unit,
            ty: $ty,
            default: $default,
            help: $help,
            applies: key!(@applies $($applies)?),
            get: |o| match spec(o) {
                Some($s) => $get,
                None => Value::Nil,
            },
            set: |o, $v| {
                if let Some($sm) = spec_mut(o) {
                    $set
                }
                Ok(())
            },
        }
    };
    (@applies) => {
        is_terrain
    };
    (@applies $applies:expr) => {
        $applies
    };
}

pub static TERRAIN: &[Field<Object>] = &[
    key!(
        "size",
        "Size",
        "m",
        Ty::Table("`{ x, z }` in metres"),
        "{ 20, 20 }",
        "How far it reaches along its own x and z, in metres; pos is its centre.",
        |s| pair(s.size),
        |s, v| {
            let size = as_pair(v, "{ x, z } in metres")?;
            if !(size.x.is_finite() && size.y.is_finite() && size.x > 0.0 && size.y > 0.0) {
                return Err("takes two lengths above 0 metres".into());
            }
            s.size = size;
        }
    ),
    key!(
        "cell",
        "Cell",
        "m",
        Ty::Number { min: CELL_MIN, max: CELL_MAX, dec: 2 },
        "1",
        "The step of the height grid, 0.5 to 4 metres.",
        |s| num(s.cell),
        |s, v| {
            let c = as_f32(v)?;
            if !(CELL_MIN..=CELL_MAX).contains(&c) {
                return Err(format!("takes {CELL_MIN} to {CELL_MAX} metres"));
            }
            s.cell = c;
        }
    ),
    key!(
        "base",
        "Base",
        "",
        Ty::Table("`\"ground\"`, or a number: metres over the height of pos"),
        "\"ground\"",
        "Where every point starts: the level's ground under it (of the static objects that are not added by the module, the surface nearest pos under the centre, followed outward: a roof or a tree's crown over it is not the ground), or flat at a height over pos.",
        |s| match s.base {
            Base::Ground => text("ground"),
            Base::Height(h) => num(h),
        },
        |s, v| {
            s.base = match v.as_str() {
                Some(w) if w.eq_ignore_ascii_case("ground") => Base::Ground,
                Some(_) => return Err("takes \"ground\" or a number".into()),
                None => {
                    Base::Height(finite(as_f32(v).map_err(|_| "takes \"ground\" or a number")?, "metres")?)
                }
            };
        }
    ),
    key!(
        "edge",
        "Edge",
        "m",
        Ty::Number { min: 0.0, max: 1024.0, dec: 2 },
        "4",
        "On the ground: the metres along the border over which the edits blend back into the ground around, so the border meets it.",
        |s| num(s.edge),
        |s, v| {
            let e = as_f32(v)?;
            if !(0.0..=1024.0).contains(&e) {
                return Err("takes 0 to 1024 metres".into());
            }
            s.edge = e;
        },
        applies = on_ground
    ),
    key!(
        "edits",
        "Edits",
        "",
        Ty::Table(
            "a list, in order: `{ op = \"flatten\", at = { x, z }, radius, falloff, height }`, `{ op = \"rect\", min, max, falloff, height }`, `{ op = \"raise\", at, radius, falloff, amount }`, `{ op = \"ramp\", from, to, width, falloff, heights = { from, to } }`, `{ op = \"smooth\", at, radius, strength }`, `{ op = \"noise\", amount, scale, seed }`",
        ),
        "none",
        "How the heights change, one edit after the other: x and z are metres from the centre along the terrain's own axes, height is a world height, amount is relative, falloff is the width of the soft border.",
        |s| list(s.edits.iter().map(edit_value)),
        |s, v| s.edits = edits_of(v)?
    ),
    key!(
        "rows",
        "Heights",
        "",
        Ty::Table("a list of rows of heights, from -z to +z, each from -x"),
        "none",
        "Instead of the edits: every world height of the grid written out.",
        |s| if s.rows.is_empty() {
            Value::Nil
        } else {
            list(s.rows.iter().map(|r| list(r.iter().map(|h| num(*h)))))
        },
        |s, v| s.rows = rows_of(v)?
    ),
    key!(
        "surface",
        "Surface",
        "",
        Ty::Table("`\"ground\"`, or `{ texture, shader, compile, material, tile }`"),
        "\"ground\"",
        "What it looks like: the look and the texture coordinates of the ground under each face, or a surface of its own laid flat every tile metres.",
        |s| surface_value(&s.surface),
        |s, v| s.surface = surface_of(v)?
    ),
    key!(
        "cut",
        "Cuts the Ground",
        "",
        Ty::Bool,
        "false",
        "On a level of the game, the ground under it is cut out of the level: its picture, its collision and its AI map.",
        |s| Value::Bool(s.cut),
        |s, v| s.cut = as_bool(v)?
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn terrain() -> Object {
        crate::format::default_object(crate::class::ObjClass::Terrain)
    }

    fn set(o: &mut Object, key: &str, v: Value) -> Result<(), Bad> {
        let f = TERRAIN.iter().find(|f| f.key == key).expect("a key");
        (f.set)(o, &v)
    }

    #[test]
    fn every_edit_reads_back_as_it_was_written() {
        let mut o = terrain();
        let edits = vec![
            Edit::Flatten { at: Vec2::ZERO, radius: 10.0, falloff: 4.0, height: 12.5 },
            Edit::Rect { min: Vec2::new(-8.0, -6.0), max: Vec2::new(8.0, 6.0), falloff: 3.0, height: 12.5 },
            Edit::Raise { at: Vec2::new(12.0, 4.0), radius: 6.0, falloff: 6.0, amount: 2.0 },
            Edit::Ramp {
                from: Vec2::new(-10.0, 0.0),
                to: Vec2::new(10.0, 0.0),
                width: 4.0,
                falloff: 2.0,
                heights: [11.0, 13.0],
            },
            Edit::Smooth { at: Vec2::ZERO, radius: 12.0, falloff: 0.0, strength: 0.5 },
            Edit::Noise { amount: 0.3, scale: 8.0, seed: 1 },
        ];
        let v = list(edits.iter().map(edit_value));
        set(&mut o, "edits", v).expect("read");
        assert_eq!(spec(&o).map(|s| s.edits.clone()), Some(edits));
    }

    #[test]
    fn a_bad_edit_is_a_sentence() {
        let mut o = terrain();
        let mut t = Table::new();
        t.put("op", text("dig"));
        let e = set(&mut o, "edits", list([Value::Table(t)])).unwrap_err();
        assert!(e.starts_with("item 1: has the unknown op 'dig'"), "{e}");
        let mut t = Table::new();
        t.put("op", text("flatten"));
        t.put("at", list([num(0.0), num(0.0)]));
        t.put("radius", num(1.0));
        let e = set(&mut o, "edits", list([Value::Table(t)])).unwrap_err();
        assert_eq!(e, "item 1: (flatten) takes height");
        assert!(set(&mut o, "cell", num(9.0)).unwrap_err().contains("0.5 to 4"));
        assert!(set(&mut o, "base", text("sky")).is_err());
        set(&mut o, "base", num(1.5)).expect("a number");
        assert_eq!(spec(&o).map(|s| s.base), Some(Base::Height(1.5)));
    }
}
