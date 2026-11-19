//! A compiled model of the game described: its wrapper asset and its row of the index
//! (`docs/GAME_LIBRARY.md`, 3.1).

use gz_fmt_ogf::{MT, Skeleton, Visual};
use gz_scene::facts::{self, Limits, ModelFacts, mm, v3};
use gz_scene::library::{LibraryObject, SurfaceUse};
use serde_json::{Map, Value, json};

/// The version of what a model's card states.
pub const VERSION: u32 = 3;

/// What the game stores a model as.
#[must_use]
pub fn form_of(kind: u8) -> &'static str {
    match kind {
        MT::NORMAL => "static",
        MT::HIERRARHY => "hierarchy",
        MT::PROGRESSIVE => "progressive",
        MT::SKELETON_ANIM => "skinned",
        MT::SKELETON_RIGID => "rigid",
        MT::SKELETON_GEOMDEF_PM | MT::SKELETON_GEOMDEF_ST => "skin_part",
        MT::LOD => "lod",
        MT::TREE_ST | MT::TREE_PM => "tree",
        _ => "other",
    }
}

fn shape_word(kind: u16) -> &'static str {
    match kind {
        1 => "box",
        2 => "sphere",
        3 => "cylinder",
        _ => "none",
    }
}

fn joint_word(kind: u32) -> &'static str {
    match kind {
        0 => "rigid",
        1 => "cloth",
        2 => "joint",
        3 => "wheel",
        5 => "slider",
        _ => "none",
    }
}

/// The bones of a model with their bodies: `(rows, the mass together, how many bodies)`.
fn bones_of(visual: &Visual) -> (Vec<Value>, f32, usize) {
    let skeleton = Skeleton::of(visual);
    let mut rows = Vec::with_capacity(visual.bones.len());
    let (mut mass, mut bodies) = (0.0f32, 0usize);
    for (i, b) in visual.bones.iter().enumerate() {
        let mut row = Map::new();
        row.insert("name".into(), json!(b.name.as_str()));
        if !b.parent.is_empty() {
            row.insert("parent".into(), json!(b.parent.as_str()));
        }
        if let Some(ik) = visual.ik.get(i) {
            let s = &ik.shape;
            let word = shape_word(s.kind);
            // bit 2 of a shape's flags: the bone takes no part in physics
            let physical = word != "none" && s.flags & 0b100 == 0;
            let to_model = |p: gz_fmt_object::glam::Vec3| match skeleton.bind.get(i) {
                Some(m) => v3(m.transform_tiny(p)),
                None => v3(p),
            };
            let shape = match word {
                "box" => {
                    // the box's own axes, in the model's space: a box that is turned in its bone
                    // is told from one that is not
                    let axes: Vec<Value> = s
                        .box_rotate
                        .iter()
                        .map(|axis| {
                            let axis = gz_fmt_object::glam::Vec3::from_array(*axis);
                            match skeleton.bind.get(i) {
                                Some(m) => v3(m.transform_dir(axis)),
                                None => v3(axis),
                            }
                        })
                        .collect();
                    json!({
                        "shape": "box",
                        "size": v3(s.box_halfsize * 2.0),
                        "center": to_model(s.box_translate),
                        "axes": axes,
                    })
                }
                "sphere" => json!({
                    "shape": "sphere",
                    "radius": mm(s.sphere_radius),
                    "center": to_model(s.sphere_center),
                }),
                "cylinder" => json!({
                    "shape": "cylinder",
                    "radius": mm(s.cylinder_radius),
                    "height": mm(s.cylinder_height),
                    "center": to_model(s.cylinder_center),
                }),
                _ => json!({ "shape": "none" }),
            };
            if let Some(m) = shape.as_object() {
                row.extend(m.clone());
            }
            if physical {
                bodies += 1;
                mass += ik.mass;
                row.insert("mass".into(), json!(mm(ik.mass)));
            } else if word != "none" {
                row.insert("physics".into(), json!(false));
            }
            if !ik.game_mtl.is_empty() {
                row.insert("material".into(), json!(ik.game_mtl.as_str()));
            }
            row.insert("joint".into(), json!(joint_word(ik.joint_type)));
        }
        rows.push(Value::Object(row));
    }
    (rows, mass, bodies)
}

/// A described model.
pub struct Described {
    /// The wrapper asset: `mesh = 1`, the source in the game, the facts.
    pub card: Value,
    /// The row of the index.
    pub row: Value,
}

/// The facts of a model the scene holds, with every surface's own words: what `asset.card`
/// answers for any model, and the middle of a game model's card.
#[must_use]
pub fn facts_json(o: &LibraryObject, uses: &[SurfaceUse], limits: &Limits) -> (ModelFacts, Value) {
    let f = facts::facts(o, uses, limits);
    let surface = |i: u32| match o.object.surfaces.get(i as usize) {
        Some(s) => json!({
            "name": s.name.as_str(),
            "texture": s.texture.as_str(),
            "shader": s.shader.as_str(),
            "compile": s.shader_xrlc.as_str(),
            "material": s.game_mtl.as_str(),
        }),
        None => json!({}),
    };
    let j = f.to_json(&surface);
    (f, j)
}

/// Describes a model of the game. `name` is its engine name (`dynamics\box\box_wood_01`),
/// `path` its path in the game's files, `bytes` the file.
pub fn describe(name: &str, path: &str, bytes: &[u8]) -> Result<Described, String> {
    let visual = Visual::read(bytes).map_err(|e| e.to_string())?;
    let form = form_of(visual.kind());
    let (bones, mass, bodies) = bones_of(&visual);
    let physics = match bodies {
        0 => "none",
        1 => "rigid",
        _ => "jointed",
    };
    let mut facts = Map::new();
    facts.insert("ref".into(), json!(format!("{name}.ogf")));
    facts.insert("visual".into(), json!(name));
    facts.insert("form".into(), json!(form));
    facts.insert("bytes".into(), json!(bytes.len()));
    let mut textures: Vec<String> = Vec::new();
    let (mut triangles, mut collides) = (0u32, false);
    let size = match gz_fmt_mesh::object_of_ogf(&visual, path) {
        Ok(src) => {
            for s in &src.object.surfaces {
                if !s.texture.is_empty() && !textures.iter().any(|t| t.eq_ignore_ascii_case(&s.texture)) {
                    textures.push(s.texture.to_string());
                }
            }
            let lo = LibraryObject::provided(name, src.object);
            let uses = vec![SurfaceUse::BOTH; lo.object.surfaces.len()];
            let (f, j) = facts_json(&lo, &uses, &Limits::default());
            triangles = f.triangles;
            collides = f.collision.is_some();
            if let Some(m) = j.as_object() {
                facts.extend(m.clone());
            }
            v3(f.bounds.max - f.bounds.min)
        }
        Err(e) => {
            // a model without triangles of its own (a distant look, a set of motions): the box
            // the game stores is what is known
            let h = &visual.header;
            let size = v3(h.bb_max - h.bb_min);
            facts.insert("box".into(), json!({ "min": v3(h.bb_min), "max": v3(h.bb_max) }));
            facts.insert("size".into(), size.clone());
            facts.insert("triangles".into(), json!(0));
            facts.insert("note".into(), json!(e.to_string()));
            size
        }
    };
    if !bones.is_empty() {
        facts.insert("bones".into(), json!(bones));
        facts.insert("mass".into(), json!(mm(mass)));
    }
    facts.insert("physics".into(), json!(physics));
    let own = visual.motions.as_ref().map_or(0, |m| m.motions.len());
    if own > 0 || !visual.motion_refs.is_empty() {
        facts.insert(
            "motions".into(),
            json!({
                "refs": visual.motion_refs.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
                "own": own,
            }),
        );
    }
    if let Some(lod) = visual.lod_ref.as_ref().filter(|l| !l.is_empty()) {
        facts.insert("lod".into(), json!(lod.as_str()));
    }
    if let Some(text) = visual.user_data.as_ref().filter(|t| !t.trim().is_empty()) {
        facts.insert("user_data".into(), json!(text));
    }
    // every surface in the texture the game draws nothing for: there, and nobody sees it
    let invisible =
        !textures.is_empty() && textures.iter().all(|t| t.eq_ignore_ascii_case(gz_fmt_ogf::UNDRAWN_TEXTURE));
    if invisible {
        facts.insert("invisible".into(), json!(true));
    }
    facts.insert("textures".into(), json!(textures));

    let kind = if bodies > 0 { "dynamic" } else { "static" };
    let card = json!({
        "mesh": 1,
        "source": format!("game:{}", path.replace('/', "\\")),
        "kind": kind,
        "facts": Value::Object(facts),
    });
    let mut row = Map::new();
    row.insert("ref".into(), json!(format!("{name}.ogf")));
    row.insert("size".into(), size);
    row.insert("form".into(), json!(form));
    row.insert("triangles".into(), json!(triangles));
    if !bones.is_empty() {
        row.insert("bones".into(), json!(bones.len()));
    }
    row.insert("physics".into(), json!(physics));
    if bodies > 0 {
        row.insert("mass".into(), json!(mm(mass)));
    }
    row.insert("collides".into(), json!(collides || bodies > 0));
    if let Some(t) = textures.first() {
        row.insert("texture".into(), json!(t));
    }
    if invisible {
        row.insert("invisible".into(), json!(true));
    }
    Ok(Described { card, row: Value::Object(row) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_words_of_the_forms() {
        assert_eq!(form_of(MT::SKELETON_ANIM), "skinned");
        assert_eq!(form_of(MT::NORMAL), "static");
        assert_eq!(shape_word(1), "box");
        assert_eq!(joint_word(2), "joint");
        assert!(describe("a", "meshes/a.ogf", b"no model").is_err());
    }
}
