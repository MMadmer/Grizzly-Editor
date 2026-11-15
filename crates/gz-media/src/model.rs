//! The model asset (`*.meshasset`, `mesh = 1`) as its file says: a source or `parts`, the
//! transform applied as it is read, the surfaces laid over the source's, and a dynamic model's
//! bones, motions and user data.

use gz_luadata::{Seg, Value};

use crate::fields::{ANIMATION, BONE, MODEL, PART};
use crate::read::{self, Cx, item_seg};

/// A surface row: the source's surface by name, and what is restated of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SurfaceRow {
    pub name: String,
    pub texture: Option<String>,
    pub shader: Option<String>,
    pub compile: Option<String>,
    pub material: Option<String>,
    pub two_sided: Option<bool>,
}

/// A piece of geometry written in the asset.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    /// [`crate::fields::SHAPES`].
    pub shape: u32,
    pub size: Option<Vec<f64>>,
    pub pos: [f64; 3],
    pub rot: [f64; 3],
    pub surface: String,
    pub tile: f64,
    pub radius: f64,
    pub height: f64,
    pub sides: u32,
    pub outline: Vec<[f64; 2]>,
}

pub const BOX: u32 = 0;
pub const RAMP: u32 = 1;
pub const CYLINDER: u32 = 2;
pub const PLANE: u32 = 3;
pub const PRISM: u32 = 4;

impl Default for Part {
    fn default() -> Self {
        Self {
            shape: BOX,
            size: None,
            pos: [0.0; 3],
            rot: [0.0; 3],
            surface: "default".into(),
            tile: 1.0,
            radius: 0.5,
            height: 1.0,
            sides: 16,
            outline: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoneRow {
    pub name: String,
    /// [`crate::fields::BONE_SHAPES`].
    pub shape: Option<u32>,
    pub mass: Option<f64>,
    pub material: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimationRow {
    pub name: String,
    pub clip: Option<String>,
    pub speed: Option<f64>,
    pub looped: Option<bool>,
    /// The other flags the game reads, each as written: `no_mix`, `sync_part`, `foot_steps`,
    /// `root_mover`, `idle`, `weapon_bone`.
    pub flags: [Option<bool>; 6],
}

/// The keys of [`AnimationRow::flags`], in its order.
pub const ANIMATION_FLAGS: [&str; 6] =
    ["no_mix", "sync_part", "foot_steps", "root_mover", "idle", "weapon_bone"];

/// A model asset; `None` is a key left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelAsset {
    pub source: Option<String>,
    pub name: Option<String>,
    /// `kind = "dynamic"`.
    pub dynamic: bool,
    /// The kind was written.
    pub kind_written: bool,
    pub scale: f64,
    pub rotate: [f64; 3],
    pub surfaces: Vec<SurfaceRow>,
    pub parts: Vec<Part>,
    pub bones: Vec<BoneRow>,
    pub animations: Vec<AnimationRow>,
    pub userdata: Option<String>,
}

fn rows<T>(cx: &mut Cx, v: &Value, key: &str, mut one: impl FnMut(&mut Cx, &Value) -> T) -> Vec<T> {
    let items: Vec<Value> = read::list(cx, v, key).to_vec();
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let segs = [Seg::key(key), item_seg(item, i)];
        if !item.is_table() {
            cx.within(&segs, |cx| {
                let at = cx.path.clone();
                cx.issues.push(read::Issue::new(
                    read::BAD_VALUE,
                    gz_base::Severity::Error,
                    at,
                    format!("an item of '{key}' takes a table"),
                ));
            });
            continue;
        }
        out.push(cx.within(&segs, |cx| one(cx, item)));
    }
    out
}

fn required_name(cx: &mut Cx, v: &Value, list: &str) -> String {
    match read::text(cx, v, "name") {
        Some(n) if !n.trim().is_empty() => n,
        _ => {
            if v.get("name").is_none() {
                cx.bad("name", format!("a row of '{list}' needs its 'name'"));
            }
            String::new()
        }
    }
}

impl ModelAsset {
    #[must_use]
    pub fn from_value(v: &Value) -> (Self, Vec<read::Issue>) {
        let mut cx = Cx::default();
        let kind = read::word(&mut cx, v, MODEL, "kind");
        let mut m = ModelAsset {
            source: read::text(&mut cx, v, "source"),
            name: read::text(&mut cx, v, "name"),
            dynamic: kind == Some(1),
            kind_written: kind.is_some(),
            scale: read::number(&mut cx, v, MODEL, "scale").unwrap_or(1.0),
            rotate: read::numbers::<3>(&mut cx, v, "rotate").unwrap_or([0.0; 3]),
            userdata: read::text(&mut cx, v, "userdata"),
            ..Self::default()
        };
        m.surfaces = rows(&mut cx, v, "surfaces", |cx, s| SurfaceRow {
            name: required_name(cx, s, "surfaces"),
            texture: read::text(cx, s, "texture"),
            shader: read::text(cx, s, "shader"),
            compile: read::text(cx, s, "compile"),
            material: read::text(cx, s, "material"),
            two_sided: read::boolean(cx, s, "two_sided"),
        });
        m.parts = rows(&mut cx, v, "parts", read_part);
        m.bones = rows(&mut cx, v, "bones", |cx, b| BoneRow {
            name: required_name(cx, b, "bones"),
            shape: read::word(cx, b, BONE, "shape"),
            mass: read::number(cx, b, BONE, "mass"),
            material: read::text(cx, b, "material"),
        });
        m.animations = rows(&mut cx, v, "animations", |cx, a| AnimationRow {
            name: required_name(cx, a, "animations"),
            clip: read::text(cx, a, "clip"),
            speed: read::number(cx, a, ANIMATION, "speed"),
            looped: read::boolean(cx, a, "loop"),
            flags: ANIMATION_FLAGS.map(|key| read::boolean(cx, a, key)),
        });
        (m, cx.issues)
    }
}

fn read_part(cx: &mut Cx, v: &Value) -> Part {
    let mut p = Part::default();
    match read::word(cx, v, PART, "shape") {
        Some(s) => p.shape = s,
        None if v.get("shape").is_none() => {
            cx.bad("shape", "a part needs its 'shape': box, ramp, cylinder, plane or prism")
        }
        None => {}
    }
    if let Some(raw) = v.get("size") {
        let want = if p.shape == PLANE { 2 } else { 3 };
        let ok = raw.is_table()
            && raw.arr().len() == want
            && raw
                .arr()
                .iter()
                .all(|x| x.as_number().is_some_and(|n| n.is_finite() && n > 0.0 && n <= 10_000.0));
        if ok {
            p.size = Some(raw.arr().iter().filter_map(Value::as_number).collect());
        } else {
            cx.bad("size", format!("'size' takes {want} numbers above 0, metres"));
        }
    }
    if let Some(x) = read::numbers::<3>(cx, v, "pos") {
        p.pos = x;
    }
    if let Some(x) = read::numbers::<3>(cx, v, "rot") {
        p.rot = x;
    }
    if let Some(s) = read::text(cx, v, "surface") {
        p.surface = s;
    }
    if let Some(x) = read::number(cx, v, PART, "tile") {
        p.tile = x;
    }
    if let Some(x) = read::number(cx, v, PART, "radius") {
        p.radius = x;
    }
    if let Some(x) = read::number(cx, v, PART, "height") {
        p.height = x;
    }
    if let Some(x) = read::integer(cx, v, PART, "sides") {
        p.sides = x as u32;
    }
    if let Some(raw) = v.get("outline") {
        let pts: Option<Vec<[f64; 2]>> = raw
            .is_table()
            .then(|| {
                raw.arr()
                    .iter()
                    .map(|pt| match pt.arr() {
                        [x, z] => Some([x.as_number()?, z.as_number()?]),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
            })
            .flatten();
        match pts {
            Some(pts) if pts.len() >= 3 && pts.iter().all(|p| p[0].is_finite() && p[1].is_finite()) => {
                p.outline = pts
            }
            _ => cx.bad("outline", "'outline' takes a list of at least three { x, z } points"),
        }
    } else if p.shape == PRISM {
        cx.bad("outline", "a prism needs its 'outline': at least three { x, z } points");
    }
    p
}
