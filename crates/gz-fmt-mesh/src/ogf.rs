//! A compiled model of the game (`.ogf`) read into the source model.
//!
//! The game's own models are compiled: vertex buffers per texture, already in the game's space
//! and winding. Read into an [`Object`] they are placed, drawn, hit by rays and compiled into a
//! level like any library object. The model comes out in its rest pose, without a skeleton: the
//! bones, their collision shapes and the motions stay the compiled model's (the game plays them
//! from its own file), and whoever needs them reads the [`Visual`].
//!
//! - One mesh per part of the model that holds geometry, at its finest level of detail.
//! - One surface per distinct texture and shader pair, named after the texture; the engine shader
//!   is the model's own, the compiler shader and the game material are `default` (a compiled
//!   model states neither).

use gz_base::SmolStr;
use gz_fmt_object::glam::{Vec2, Vec3};
use gz_fmt_object::{Object, Surface};
use gz_fmt_ogf::Visual;

use crate::assemble::{self, Corner, Soup, Surfaces, Tri, Weights};
use crate::{Bad, MAX_ELEMENTS, MeshError, MeshSource, Result};

/// The triangles of a compiled model's parts, each with its texture and shader, at the finest
/// level of detail: `(texture, shader, corners)` with three corners a triangle.
pub fn ogf_parts(visual: &Visual) -> std::result::Result<Vec<OgfPart>, Bad> {
    let mut out = Vec::new();
    let mut total = 0usize;
    for v in visual.meshes() {
        let (Some(vb), Some(ib)) = (&v.vertices, &v.indices) else { continue };
        // a progressive model's first window is its finest level
        let range = match v.swi.as_ref().and_then(|s| s.windows.first()) {
            Some(w) => w.offset as usize..w.offset as usize + w.num_tris as usize * 3,
            None => 0..ib.len(),
        };
        let Some(tris) = ib.get(range) else {
            return Err("a level of detail of the model lies outside its indices".to_owned());
        };
        if tris.len() < 3 {
            continue;
        }
        total += tris.len();
        if total > MAX_ELEMENTS {
            return Err(format!("the model has more than {MAX_ELEMENTS} triangle corners"));
        }
        let count = vb.vertices.len();
        if let Some(bad) = tris.iter().find(|i| usize::from(**i) >= count) {
            return Err(format!("an index of the model points at vertex {bad} of {count}"));
        }
        let (texture, shader) = v.texture.clone().unwrap_or_default();
        // a level's visual may name its lightmaps after a comma: the first name is the picture
        let texture = SmolStr::new(texture.split(',').next().unwrap_or("").trim());
        out.push(OgfPart {
            texture,
            shader,
            corners: tris
                .iter()
                .map(|&i| {
                    let s = &vb.vertices[usize::from(i)];
                    (s.position, s.normal, s.uv)
                })
                .collect(),
        });
    }
    Ok(out)
}

/// One part of a compiled model: what it wears and its triangles.
#[derive(Clone, Debug, PartialEq)]
pub struct OgfPart {
    pub texture: SmolStr,
    pub shader: SmolStr,
    /// Position, normal and texture coordinate, three corners a triangle.
    pub corners: Vec<(Vec3, Vec3, Vec2)>,
}

/// The surface a part of a compiled model becomes: named after its texture, wearing it.
fn surface_of(name: &str, texture: &str, shader: &str) -> Surface {
    let mut s = assemble::surface(name, false);
    s.texture = SmolStr::new(texture);
    if !shader.trim().is_empty() {
        s.shader = SmolStr::new(shader);
    }
    s
}

/// Parts of textured triangles as the source model: one mesh a part, one surface per distinct
/// texture and shader pair (named after the texture, wearing it, the compiler shader and the
/// game material `default`). `materials` of the answer is parallel to the object's surfaces.
pub fn object_of_parts(parts: &[OgfPart], file: &str) -> Result<MeshSource> {
    let bad = |message: Bad| MeshError::Invalid { file: file.to_owned(), message };
    let mut surfaces: Surfaces<(SmolStr, SmolStr)> = Surfaces::default();
    let mut soups = Vec::with_capacity(parts.len());
    for (n, part) in parts.iter().enumerate() {
        let leaf = part.texture.rsplit(['\\', '/']).next().unwrap_or("").to_owned();
        let surface = surfaces.get((part.texture.clone(), part.shader.clone()), || (leaf, None, false));
        let tris = part
            .corners
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| Tri {
                c: [0, 1, 2].map(|k| Corner {
                    pos: c[k].0,
                    normal: Some(c[k].1),
                    uv: c[k].2,
                    weights: Weights::new(),
                }),
                surface,
                group: 0,
            })
            .collect();
        soups.push(Soup { name: format!("part_{n}"), tris });
    }
    let mut notes = Vec::new();
    let meshes = assemble::build_meshes(&soups, &[], &mut notes).map_err(bad)?;
    // the surfaces as the parts state them: the texture and the engine shader of each pair
    let mut made = surfaces.surfaces;
    let mut seen: Vec<(SmolStr, SmolStr)> = Vec::new();
    for part in parts {
        let key = (part.texture.clone(), part.shader.clone());
        if !seen.contains(&key) {
            if let Some(s) = made.get_mut(seen.len()) {
                *s = surface_of(&s.name.clone(), &part.texture, &part.shader);
            }
            seen.push(key);
        }
    }
    let object = Object { surfaces: made, meshes, ..Object::default() };
    Ok(MeshSource { object, materials: surfaces.materials, pictures: Vec::new(), notes })
}

/// A compiled model as the source model: [`MeshSource::materials`] is parallel to the object's
/// surfaces, and no picture comes with it (a surface names its texture itself).
pub fn object_of_ogf(visual: &Visual, file: &str) -> Result<MeshSource> {
    let parts = ogf_parts(visual).map_err(|message| MeshError::Invalid { file: file.to_owned(), message })?;
    let mut src = object_of_parts(&parts, file)?;
    src.object.user_data = visual.user_data.clone().unwrap_or_default();
    src.object.lod_ref = visual.lod_ref.clone().unwrap_or_default();
    Ok(src)
}

/// Reads the bytes of an `.ogf` into the source model.
pub fn read_ogf_bytes(bytes: &[u8], file: &str) -> Result<MeshSource> {
    let visual = Visual::read(bytes)
        .map_err(|e| MeshError::Invalid { file: file.to_owned(), message: e.to_string() })?;
    object_of_ogf(&visual, file)
}
