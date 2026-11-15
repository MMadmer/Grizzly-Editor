//! Mesh primitives into triangles of the game's space; materials and their pictures.

use std::path::Path;

use gz_fmt_object::glam::{Mat3, Mat4, Vec2, Vec3};

use super::anim::Skeleton;
use super::data::{Data, data_uri, file_of};
use super::doc::Doc;
use super::{Mode, Tree};
use crate::assemble::{Corner, Soup, Surfaces, Tri, Weights};
use crate::math::mirror;
use crate::{Bad, MAX_ELEMENTS, PictureData, SourcePicture};

/// The pictures materials name, each once.
#[derive(Default)]
pub(crate) struct Pictures {
    of_image: Vec<(usize, Option<usize>)>,
    pub list: Vec<SourcePicture>,
}

impl Pictures {
    fn image(
        &mut self,
        doc: &Doc,
        data: &Data<'_>,
        dir: &Path,
        j: usize,
        notes: &mut Vec<String>,
    ) -> Option<usize> {
        if let Some((_, p)) = self.of_image.iter().find(|(i, _)| *i == j) {
            return *p;
        }
        let made = Self::make(doc, data, dir, j);
        let at = match made {
            Ok(p) => {
                self.list.push(p);
                Some(self.list.len() - 1)
            }
            Err(e) => {
                notes.push(format!("image {j} was left out: {e}"));
                None
            }
        };
        self.of_image.push((j, at));
        at
    }

    fn make(doc: &Doc, data: &Data<'_>, dir: &Path, j: usize) -> Result<SourcePicture, Bad> {
        let image = doc.images.get(j).ok_or_else(|| "the document lacks it".to_owned())?;
        let mut mime = image.mime_type.clone().unwrap_or_default();
        let mut name = image.name.clone().unwrap_or_default();
        let data = if let Some(uri) = &image.uri {
            if let Some(decoded) = data_uri(uri) {
                let (m, bytes) = decoded?;
                if mime.is_empty() {
                    mime = m;
                }
                PictureData::Embedded(bytes)
            } else {
                let path =
                    file_of(dir, uri).ok_or_else(|| format!("`{uri}` is no file beside the document"))?;
                if name.is_empty() {
                    name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                }
                if mime.is_empty() {
                    mime = crate::mime_of(&path.to_string_lossy()).to_owned();
                }
                PictureData::File(path)
            }
        } else if let Some(v) = image.buffer_view {
            PictureData::Embedded(data.view(v)?.to_vec())
        } else {
            return Err("it names neither a file nor a buffer view".to_owned());
        };
        if name.is_empty() {
            name = format!("image_{j}");
        }
        Ok(SourcePicture { name, mime, data })
    }
}

/// Where a node's vertices go.
struct Placement<'a> {
    /// Applied to a position already in the game's space.
    m: Mat4,
    normal: Mat3,
    /// Corners 1 and 2 swap.
    flip: bool,
    /// The joints of the node's skin as bones.
    joints: Option<Vec<u16>>,
    /// The bone that carries a vertex no skin weighs.
    bone: Option<u16>,
    skeleton: Option<&'a Skeleton>,
}

/// One soup per node that holds a mesh.
#[allow(clippy::too_many_arguments)]
pub(crate) fn soups(
    doc: &Doc,
    data: &Data<'_>,
    dir: &Path,
    tree: &Tree,
    mesh_nodes: &[usize],
    mode: Mode,
    skeleton: Option<&Skeleton>,
    surfaces: &mut Surfaces<Option<usize>>,
    pictures: &mut Pictures,
    notes: &mut Vec<String>,
) -> Result<Vec<Soup>, Bad> {
    let mut out = Vec::new();
    let mut total = 0usize;
    for &n in mesh_nodes {
        let node = &doc.nodes[n];
        let Some(mi) = node.mesh else { continue };
        let gmesh = &doc.meshes[mi];
        let skinned = mode == Mode::Skinned && node.skin.is_some();
        let world = if skinned { Mat4::IDENTITY } else { tree.world[n] };
        let linear = Mat3::from_mat4(world);
        let det = linear.determinant();
        if det.abs() < 1e-12 {
            notes.push(format!("node {n} is scaled to nothing; its mesh is flat"));
        }
        let joints = match (skinned, node.skin, skeleton) {
            (true, Some(s), Some(sk)) => Some(
                doc.skins[s]
                    .joints
                    .iter()
                    .map(|j| sk.node_bone.get(*j).copied().flatten().unwrap_or(sk.root))
                    .collect(),
            ),
            _ => None,
        };
        let place = Placement {
            m: world,
            normal: if det.abs() < 1e-12 { Mat3::IDENTITY } else { linear.inverse().transpose() },
            // glTF's front faces turn counter-clockwise when the node's determinant is positive;
            // mirroring into the game's space turns them once more
            flip: det > 0.0,
            joints,
            bone: skeleton.map(|sk| sk.bone_of(tree, n)),
            skeleton,
        };
        let name = node.name.clone().or_else(|| gmesh.name.clone()).unwrap_or_default();
        let mut soup = Soup { name, tris: Vec::new() };
        for (pi, prim) in gmesh.primitives.iter().enumerate() {
            let what = |s: &str| format!("mesh {mi} primitive {pi} {s}");
            let mode = prim.mode.unwrap_or(4);
            match mode {
                4..=6 => {}
                0..=3 => {
                    push_note(notes, format!("mesh {mi} has points or lines; they were left out"));
                    continue;
                }
                other => {
                    push_note(
                        notes,
                        format!("mesh {mi} has primitives of the unknown mode {other}; they were left out"),
                    );
                    continue;
                }
            }
            let Some(pa) = prim.attributes.get("POSITION") else {
                push_note(notes, format!("mesh {mi} has a primitive without positions; it was left out"));
                continue;
            };
            let (pos, _) = data.floats(*pa, &what("POSITION"), &["VEC3"])?;
            let count = pos.len() / 3;
            let same_count = |key: &str| -> Result<Option<usize>, Bad> {
                let Some(a) = prim.attributes.get(key) else { return Ok(None) };
                let c = data.count(*a);
                if c != count {
                    return Err(what(&format!("has {c} elements of {key} for its {count} positions")));
                }
                Ok(Some(*a))
            };
            let normals = match same_count("NORMAL")? {
                Some(a) => Some(data.floats(a, &what("NORMAL"), &["VEC3"])?.0),
                None => None,
            };
            let uvs = match same_count("TEXCOORD_0")? {
                Some(a) => Some(data.floats(a, &what("TEXCOORD_0"), &["VEC2"])?.0),
                None => {
                    push_note(
                        notes,
                        format!("mesh {mi} has triangles without texture coordinates; they are 0"),
                    );
                    None
                }
            };
            if prim.attributes.keys().any(|k| k.starts_with("COLOR_")) {
                push_note(notes, "vertex colours were left out".to_owned());
            }
            if !prim.targets.is_empty() {
                push_note(notes, "morph targets were left out".to_owned());
            }
            let weights = vertex_weights(data, prim, count, &place, &what, notes)?;

            let indices: Vec<u32> = match prim.indices {
                Some(a) => {
                    let (ix, _) = data.uints(a, &what("indices"), &["SCALAR"])?;
                    if let Some(bad) = ix.iter().find(|i| **i as usize >= count) {
                        return Err(what(&format!("points at vertex {bad}, but it has {count}")));
                    }
                    ix
                }
                None => (0..count as u32).collect(),
            };
            let tris = triangles(&indices, mode);
            total += tris.len();
            if total > MAX_ELEMENTS {
                return Err(format!(
                    "the file holds more than {MAX_ELEMENTS} triangles, the most this editor reads"
                ));
            }

            let surface = material_surface(doc, data, dir, prim.material, surfaces, pictures, notes);
            let corner = |i: u32| -> Corner {
                let i = i as usize;
                let p = Vec3::new(pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2]);
                let normal = normals.as_ref().map(|nv| {
                    (place.normal * mirror(Vec3::new(nv[i * 3], nv[i * 3 + 1], nv[i * 3 + 2])))
                        .normalize_or_zero()
                });
                let uv = uvs.as_ref().map_or(Vec2::ZERO, |u| Vec2::new(u[i * 2], u[i * 2 + 1]));
                Corner {
                    pos: place.m.transform_point3(mirror(p)),
                    normal: normal.filter(|n| *n != Vec3::ZERO),
                    uv,
                    weights: weights.get(i).cloned().unwrap_or_default(),
                }
            };
            for [a, b, c] in tris {
                let (b, c) = if place.flip { (c, b) } else { (b, c) };
                soup.tris.push(Tri { c: [corner(a), corner(b), corner(c)], surface, group: 0 });
            }
        }
        out.push(soup);
    }
    Ok(out)
}

/// The triangles of a list, strip or fan, in the source's winding.
fn triangles(ix: &[u32], mode: u32) -> Vec<[u32; 3]> {
    match mode {
        5 => (0..ix.len().saturating_sub(2))
            .map(|i| if i % 2 == 0 { [ix[i], ix[i + 1], ix[i + 2]] } else { [ix[i], ix[i + 2], ix[i + 1]] })
            .collect(),
        6 => (0..ix.len().saturating_sub(2)).map(|i| [ix[i + 1], ix[i + 2], ix[0]]).collect(),
        _ => ix.as_chunks::<3>().0.to_vec(),
    }
}

/// The bones and weights of every vertex: the four heaviest, brought to a sum of one.
fn vertex_weights(
    data: &Data<'_>,
    prim: &super::doc::Primitive,
    count: usize,
    place: &Placement<'_>,
    what: &dyn Fn(&str) -> String,
    notes: &mut Vec<String>,
) -> Result<Vec<Weights>, Bad> {
    let Some(sk) = place.skeleton else { return Ok(Vec::new()) };
    let fallback = place.bone.unwrap_or(sk.root);
    let Some(joints) = &place.joints else {
        let one: Weights = smallvec::smallvec![(fallback, 1.0)];
        return Ok(vec![one; count]);
    };
    let mut sets = Vec::new();
    for k in 0.. {
        let (Some(ja), Some(wa)) =
            (prim.attributes.get(&format!("JOINTS_{k}")), prim.attributes.get(&format!("WEIGHTS_{k}")))
        else {
            break;
        };
        for (key, a) in [("JOINTS", ja), ("WEIGHTS", wa)] {
            if data.count(*a) != count {
                return Err(what(&format!(
                    "has {} elements of {key}_{k} for its {count} positions",
                    data.count(*a)
                )));
            }
        }
        let (j, _) = data.uints(*ja, &what(&format!("JOINTS_{k}")), &["VEC4"])?;
        let (w, _) = data.floats(*wa, &what(&format!("WEIGHTS_{k}")), &["VEC4"])?;
        sets.push((j, w));
    }
    let mut unbound = 0usize;
    let mut out = Vec::with_capacity(count);
    for v in 0..count {
        let mut list: Vec<(u16, f32)> = Vec::new();
        for (j, w) in &sets {
            for c in 0..4 {
                let (joint, weight) = (j[v * 4 + c] as usize, w[v * 4 + c]);
                if !(weight.is_finite() && weight > 0.0) {
                    continue;
                }
                let bone = *joints.get(joint).ok_or_else(|| {
                    what(&format!("weighs joint {joint}, but its skin has {}", joints.len()))
                })?;
                match list.iter_mut().find(|(b, _)| *b == bone) {
                    Some(e) => e.1 += weight,
                    None => list.push((bone, weight)),
                }
            }
        }
        list.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        list.truncate(4);
        let sum: f32 = list.iter().map(|(_, w)| w).sum();
        if list.is_empty() || sum <= 0.0 {
            unbound += 1;
            out.push(smallvec::smallvec![(fallback, 1.0)]);
            continue;
        }
        out.push(list.into_iter().map(|(b, w)| (b, w / sum)).collect());
    }
    if unbound > 0 {
        notes.push(format!("{unbound} vertices weigh no joint; they follow the bone of their node"));
    }
    Ok(out)
}

fn material_surface(
    doc: &Doc,
    data: &Data<'_>,
    dir: &Path,
    material: Option<usize>,
    surfaces: &mut Surfaces<Option<usize>>,
    pictures: &mut Pictures,
    notes: &mut Vec<String>,
) -> u32 {
    surfaces.get(material, || {
        let Some(i) = material else { return ("default".to_owned(), None, false) };
        let Some(m) = doc.materials.get(i) else {
            notes.push(format!("a primitive names material {i}, which the document lacks"));
            return (format!("material_{i}"), None, false);
        };
        let name = m.name.clone().unwrap_or_else(|| format!("material_{i}"));
        let picture =
            m.pbr_metallic_roughness.as_ref().and_then(|p| p.base_color_texture.as_ref()).and_then(|t| {
                if t.tex_coord != 0 {
                    notes.push(format!(
                        "material `{name}` takes its picture by texture coordinate set {}; set 0 is used",
                        t.tex_coord
                    ));
                }
                let Some(tex) = doc.textures.get(t.index) else {
                    notes.push(format!(
                        "material `{name}` names texture {}, which the document lacks",
                        t.index
                    ));
                    return None;
                };
                pictures.image(doc, data, dir, tex.source?, notes)
            });
        (name, picture, m.double_sided)
    })
}

fn push_note(notes: &mut Vec<String>, note: String) {
    if !notes.contains(&note) {
        notes.push(note);
    }
}
