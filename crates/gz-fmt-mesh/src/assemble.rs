//! Triangles in the game's space into the model's meshes and surfaces.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use gz_base::SmolStr;
use gz_fmt_object::glam::{Vec2, Vec3};
use gz_fmt_object::{Face, FaceVert, Mesh, SurfFaces, Surface, SurfaceFlags, VMap, VMapKind, VMapPt};
use smallvec::SmallVec;

use crate::{Bad, SourceMaterial};

/// The weights of a corner: bone (an index into the bone list being built) and weight.
pub(crate) type Weights = SmallVec<[(u16, f32); 4]>;

/// A corner of a triangle, in the game's space.
#[derive(Clone, Debug, Default)]
pub(crate) struct Corner {
    pub pos: Vec3,
    pub normal: Option<Vec3>,
    pub uv: Vec2,
    pub weights: Weights,
}

/// A triangle in the game's winding.
#[derive(Clone, Debug)]
pub(crate) struct Tri {
    pub c: [Corner; 3],
    /// Index into the surfaces.
    pub surface: u32,
    /// Where corners carry no normal: the smoothing group; 0 is flat. Two triangles of one group
    /// are smooth across their shared edge.
    pub group: u32,
}

/// The triangles of one mesh.
#[derive(Clone, Debug, Default)]
pub(crate) struct Soup {
    pub name: String,
    pub tris: Vec<Tri>,
}

/// Normals closer than this (the cosine of about 1.8 degrees) are one normal: the edge is soft.
const SAME_NORMAL: f32 = 0.9995;

/// The keys of a corner that may share a vertex map reference.
type CornerKey = (u32, [u32; 2], SmallVec<[(u16, u32); 4]>);

fn bits(v: f32) -> u32 {
    // -0 and 0 are one place
    if v == 0.0 { 0 } else { v.to_bits() }
}

/// A mesh of the triangles: positions welded, one vertex map reference per distinct corner,
/// hard edges where the normals (or the smoothing groups) part. `wmaps` names the weight map of
/// every bone. `None` when nothing is left; the count is of the degenerate triangles left out.
pub(crate) fn build_mesh(soup: &Soup, wmaps: &[SmolStr]) -> Result<(Option<Mesh>, usize), Bad> {
    let mut points: Vec<Vec3> = Vec::new();
    let mut point_of: HashMap<[u32; 3], u32> = HashMap::new();
    let mut faces: Vec<Face> = Vec::with_capacity(soup.tris.len());
    let mut face_tri: Vec<usize> = Vec::with_capacity(soup.tris.len());
    let mut ref_of: HashMap<CornerKey, u32> = HashMap::new();
    let mut ref_corners: Vec<(u32, Vec2, Weights)> = Vec::new();
    let mut degenerate = 0;

    for (ti, tri) in soup.tris.iter().enumerate() {
        if tri.c.iter().any(|c| !c.pos.is_finite()) {
            return Err(format!("mesh `{}` has a position that is not a number", soup.name));
        }
        let keys = tri.c.each_ref().map(|c| [bits(c.pos.x), bits(c.pos.y), bits(c.pos.z)]);
        if keys[0] == keys[1] || keys[1] == keys[2] || keys[0] == keys[2] {
            degenerate += 1;
            continue;
        }
        let mut face = Face::default();
        for (k, c) in tri.c.iter().enumerate() {
            let key = keys[k];
            let point = match point_of.get(&key) {
                Some(p) => *p,
                None => {
                    let p =
                        u32::try_from(points.len()).map_err(|_| "a mesh has too many points".to_owned())?;
                    points.push(c.pos);
                    point_of.insert(key, p);
                    p
                }
            };
            let uv = if c.uv.is_finite() { c.uv } else { Vec2::ZERO };
            let wkey: SmallVec<[(u16, u32); 4]> = c.weights.iter().map(|(b, w)| (*b, bits(*w))).collect();
            let ckey = (point, [bits(uv.x), bits(uv.y)], wkey);
            let vmref = match ref_of.get(&ckey) {
                Some(r) => *r,
                None => {
                    let r = ref_corners.len() as u32;
                    ref_corners.push((point, uv, c.weights.clone()));
                    ref_of.insert(ckey, r);
                    r
                }
            };
            face.v[k] = FaceVert { point, vmref };
        }
        faces.push(face);
        face_tri.push(ti);
    }
    if faces.is_empty() {
        return Ok((None, degenerate));
    }

    // the vertex maps as they come, one entry per reference; `rebuild_vmaps` lays them out
    let mut uv = VMap {
        name: SmolStr::new_static("Texture"),
        kind: VMapKind::Uv,
        polymap: false,
        values: Vec::with_capacity(ref_corners.len() * 2),
        vindices: Vec::with_capacity(ref_corners.len()),
        pindices: Vec::new(),
    };
    let mut weight_maps: Vec<VMap> = Vec::new();
    let mut map_of_bone: HashMap<u16, usize> = HashMap::new();
    let mut vmrefs = Vec::with_capacity(ref_corners.len());
    for (i, (point, c_uv, weights)) in ref_corners.iter().enumerate() {
        uv.values.push(c_uv.x);
        uv.values.push(c_uv.y);
        uv.vindices.push(*point);
        let mut refs: SmallVec<[VMapPt; 2]> = SmallVec::new();
        refs.push(VMapPt { vmap: 0, index: i as u32 });
        for (bone, w) in weights {
            let name = wmaps
                .get(usize::from(*bone))
                .ok_or_else(|| "a weight names a bone that is not there".to_owned())?;
            let at = *map_of_bone.entry(*bone).or_insert_with(|| {
                weight_maps.push(VMap {
                    name: name.clone(),
                    kind: VMapKind::Weight,
                    polymap: false,
                    values: Vec::new(),
                    vindices: Vec::new(),
                    pindices: Vec::new(),
                });
                weight_maps.len() - 1
            });
            let m = &mut weight_maps[at];
            m.values.push(*w);
            m.vindices.push(*point);
            refs.push(VMapPt { vmap: at as u32 + 1, index: (m.values.len() - 1) as u32 });
        }
        vmrefs.push(refs);
    }
    let mut vmaps = vec![uv];
    vmaps.extend(weight_maps);

    let smooth_groups = hard_edges(soup, &faces, &face_tri);

    let mut by_surface: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (f, ti) in face_tri.iter().enumerate() {
        by_surface.entry(soup.tris[*ti].surface).or_default().push(f as u32);
    }
    let surf_faces = by_surface
        .into_iter()
        .map(|(surface, faces)| SurfFaces { surface, faces: Arc::from(faces) })
        .collect();

    let mut mesh = Mesh {
        name: SmolStr::new(&soup.name),
        points: Arc::from(points),
        faces: Arc::from(faces),
        smooth_groups: Arc::from(smooth_groups),
        vmaps,
        vmrefs,
        surf_faces,
        ..Mesh::default()
    };
    mesh.rebuild_vmaps().map_err(|e| e.to_string())?;
    mesh.recompute_bbox();
    Ok((Some(mesh), degenerate))
}

/// The per-face edge flags: bit `e` marks the edge from corner `e` to corner `e + 1` hard.
fn hard_edges(soup: &Soup, faces: &[Face], face_tri: &[usize]) -> Vec<u32> {
    let mut flags = vec![0u32; faces.len()];
    // per edge (its two points, lower first): the faces that hold it and as which of their edges
    type EdgeUse = SmallVec<[(u32, u8); 2]>;
    let mut edges: HashMap<(u32, u32), EdgeUse> = HashMap::new();
    for (f, face) in faces.iter().enumerate() {
        for e in 0..3u8 {
            let (p, q) = (face.v[usize::from(e)].point, face.v[(usize::from(e) + 1) % 3].point);
            edges.entry(if p < q { (p, q) } else { (q, p) }).or_default().push((f as u32, e));
        }
    }
    // the normal a face carries at one of its points
    let normal_at = |f: usize, point: u32| -> Option<Vec3> {
        let k = faces[f].v.iter().position(|v| v.point == point)?;
        soup.tris[face_tri[f]].c[k].normal.map(Vec3::normalize_or_zero)
    };
    for ((p, q), list) in &edges {
        for (i, (fa, ea)) in list.iter().enumerate() {
            for (fb, eb) in &list[i + 1..] {
                let (a, b) = (*fa as usize, *fb as usize);
                let soft = match [p, q].map(|pt| (normal_at(a, *pt), normal_at(b, *pt))) {
                    [(Some(na), Some(nb)), (Some(ma), Some(mb))] => {
                        na.dot(nb) >= SAME_NORMAL && ma.dot(mb) >= SAME_NORMAL
                    }
                    _ => {
                        let (ga, gb) = (soup.tris[face_tri[a]].group, soup.tris[face_tri[b]].group);
                        ga != 0 && ga == gb
                    }
                };
                if !soft {
                    flags[a] |= 1 << ea;
                    flags[b] |= 1 << eb;
                }
            }
        }
    }
    flags
}

/// A surface valid for the rest of the editor; the texture is the asset layer's to fill.
pub(crate) fn surface(name: &str, two_sided: bool) -> Surface {
    Surface {
        name: SmolStr::new(name),
        texture: SmolStr::default(),
        vmap: SmolStr::new_static("Texture"),
        shader: SmolStr::new_static("default"),
        shader_xrlc: SmolStr::new_static("default"),
        game_mtl: SmolStr::new_static("materials\\default"),
        flags: if two_sided { SurfaceFlags::TWO_SIDED } else { SurfaceFlags::empty() },
        // a position, a normal and one texture coordinate set
        fvf: 0x112,
    }
}

/// A name none of `taken` has (without case), trimmed; `fallback` stands in for an empty one.
pub(crate) fn unique(name: &str, fallback: &str, taken: &mut Vec<String>) -> String {
    let base = name.trim_matches(|c: char| c <= ' ');
    let base = if base.is_empty() { fallback } else { base };
    let mut out = base.to_owned();
    let mut n = 2;
    while taken.iter().any(|t| t.eq_ignore_ascii_case(&out)) {
        out = format!("{base}_{n}");
        n += 1;
    }
    taken.push(out.clone());
    out
}

/// The surfaces of a source as they are met, keyed by what the format names a material by.
#[derive(Default)]
pub(crate) struct Surfaces<K> {
    keys: Vec<K>,
    pub surfaces: Vec<Surface>,
    pub materials: Vec<SourceMaterial>,
    names: Vec<String>,
}

impl<K: PartialEq> Surfaces<K> {
    /// The surface of a key; `make` gives the name, the picture and two-sidedness of a new one.
    pub fn get(&mut self, key: K, make: impl FnOnce() -> (String, Option<usize>, bool)) -> u32 {
        if let Some(i) = self.keys.iter().position(|k| *k == key) {
            return i as u32;
        }
        let (name, picture, two_sided) = make();
        let name = unique(&name, "default", &mut self.names);
        self.surfaces.push(surface(&name, two_sided));
        self.materials.push(SourceMaterial { name, picture, two_sided });
        self.keys.push(key);
        (self.keys.len() - 1) as u32
    }
}

/// Every soup a mesh; unique mesh names; a note for what was left out.
pub(crate) fn build_meshes(
    soups: &[Soup],
    wmaps: &[SmolStr],
    notes: &mut Vec<String>,
) -> Result<Vec<Mesh>, Bad> {
    let mut names = Vec::new();
    let mut out = Vec::new();
    let mut degenerate = 0;
    for s in soups {
        let (mesh, d) = build_mesh(s, wmaps)?;
        degenerate += d;
        if let Some(mut m) = mesh {
            m.name = SmolStr::new(unique(&s.name, "mesh", &mut names));
            out.push(m);
        }
    }
    if degenerate > 0 {
        notes.push(format!("{degenerate} degenerate triangles (two corners at one point) were left out"));
    }
    if out.is_empty() {
        return Err("the file holds no triangles".to_owned());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corner(x: f32, y: f32, z: f32, n: Option<Vec3>) -> Corner {
        Corner { pos: Vec3::new(x, y, z), normal: n, uv: Vec2::new(x, z), weights: Weights::new() }
    }

    #[test]
    fn shared_points_are_welded_and_parted_normals_make_a_hard_edge() {
        let up = Some(Vec3::Y);
        let side = Some(Vec3::X);
        // two triangles of a floor and one up a wall over the floor's edge x = 1
        let soup = Soup {
            name: "m".into(),
            tris: vec![
                Tri {
                    c: [corner(0., 0., 0., up), corner(0., 0., 1., up), corner(1., 0., 0., up)],
                    surface: 0,
                    group: 0,
                },
                Tri {
                    c: [corner(1., 0., 0., up), corner(0., 0., 1., up), corner(1., 0., 1., up)],
                    surface: 0,
                    group: 0,
                },
                Tri {
                    c: [corner(1., 0., 0., side), corner(1., 0., 1., side), corner(1., 1., 0., side)],
                    surface: 1,
                    group: 0,
                },
                // degenerate: left out
                Tri {
                    c: [corner(5., 0., 0., up), corner(5., 0., 0., up), corner(1., 0., 0., up)],
                    surface: 0,
                    group: 0,
                },
            ],
        };
        let (mesh, degenerate) = build_mesh(&soup, &[]).unwrap();
        let mesh = mesh.unwrap();
        assert_eq!(degenerate, 1);
        assert_eq!(mesh.points.len(), 5);
        assert_eq!(mesh.faces.len(), 3);
        // the floor's shared diagonal is soft, the floor-wall edge (points 2 and 3) is hard on both sides
        assert_eq!(mesh.smooth_groups[0], 0);
        assert_eq!(mesh.smooth_groups[1], 1 << 2);
        assert_eq!(mesh.smooth_groups[2], 1 << 0);
        assert_eq!(mesh.surf_faces.len(), 2);
        assert_eq!(&mesh.surf_faces[0].faces[..], &[0, 1]);
        assert_eq!(mesh.face_uv(1).unwrap()[2], Vec2::new(1.0, 1.0));

        // without normals the smoothing groups decide
        let mut flat = soup.clone();
        for t in &mut flat.tris {
            for c in &mut t.c {
                c.normal = None;
            }
        }
        let (m, _) = build_mesh(&flat, &[]).unwrap();
        assert_eq!(m.unwrap().smooth_groups[0], 1 << 1);
        for t in &mut flat.tris {
            t.group = 3;
        }
        let (m, _) = build_mesh(&flat, &[]).unwrap();
        assert!(m.unwrap().smooth_groups.iter().all(|f| *f == 0));
    }

    #[test]
    fn names_are_unique_without_case() {
        let mut taken = Vec::new();
        assert_eq!(unique("Brick", "x", &mut taken), "Brick");
        assert_eq!(unique("brick", "x", &mut taken), "brick_2");
        assert_eq!(unique("  ", "x", &mut taken), "x");
    }

    #[test]
    fn a_position_that_is_no_number_is_an_error() {
        let soup = Soup {
            name: "m".into(),
            tris: vec![Tri {
                c: [corner(f32::NAN, 0., 0., None), corner(0., 0., 1., None), corner(1., 0., 0., None)],
                surface: 0,
                group: 0,
            }],
        };
        assert!(build_mesh(&soup, &[]).is_err());
    }
}
