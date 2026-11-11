//! Meshes on the CPU: one vertex layout for everything the editor draws, the converters from
//! `.object` and `.ogf`, and the hand-made shapes previews use.

use std::ops::Range;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{Vec2, Vec3};
use gz_fmt_object::{Object, SurfaceFlags};
use gz_fmt_ogf::{MT, VertexFormat, Visual};
use rustc_hash::FxHashMap;

use crate::color::Color;
use crate::material::{EditorMaterial, MaterialDesc, MaterialTable, texture_key};
use crate::math::Aabb;
use crate::{RenderError, Result, invalid};

/// The vertex every mesh is drawn from (56 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// The second texture set: a lightmap, unused by the editor's materials today.
    pub uv2: [f32; 2],
    /// RGBA; white for a model whose vertex colour is baked light.
    pub color: [u8; 4],
    /// Indices into the instance's pose.
    pub bones: [u16; 4],
    /// The weight of every bone, 255 = 1. All zero: not skinned.
    pub weights: [u8; 4],
}

impl MeshVertex {
    #[must_use]
    pub fn new(position: Vec3, normal: Vec3, uv: Vec2) -> Self {
        Self {
            position: position.to_array(),
            normal: normal.to_array(),
            uv: uv.to_array(),
            uv2: [0.0; 2],
            color: [255; 4],
            bones: [0; 4],
            weights: [0; 4],
        }
    }
}

/// A run of indices drawn with one material.
#[derive(Clone, Debug, PartialEq)]
pub struct SubMesh {
    pub indices: Range<u32>,
    pub material: MaterialDesc,
}

/// A mesh: triangles in index order, clockwise front faces.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshData {
    pub vertices: Arc<[MeshVertex]>,
    pub indices: Arc<[u32]>,
    pub submeshes: Vec<SubMesh>,
    pub bounds: Aabb,
    /// Whether vertices carry bone weights.
    pub skinned: bool,
}

impl MeshData {
    /// Checks every index and every range; the bounds are worked out.
    pub fn new(vertices: Vec<MeshVertex>, indices: Vec<u32>, submeshes: Vec<SubMesh>) -> Result<Self> {
        if vertices.is_empty() || indices.is_empty() || submeshes.is_empty() {
            return Err(RenderError::EmptyMesh);
        }
        if !indices.len().is_multiple_of(3) {
            return Err(invalid(format!(
                "the mesh has {} indices, which is no whole number of triangles",
                indices.len()
            )));
        }
        let count = u32::try_from(vertices.len()).map_err(|_| invalid("the mesh has too many vertices"))?;
        if let Some(bad) = indices.iter().find(|i| **i >= count) {
            return Err(invalid(format!("an index points at vertex {bad} of {count}")));
        }
        let total = u32::try_from(indices.len()).map_err(|_| invalid("the mesh has too many indices"))?;
        for s in &submeshes {
            if s.indices.start > s.indices.end
                || s.indices.end > total
                || s.indices.start % 3 != 0
                || s.indices.end % 3 != 0
            {
                return Err(invalid(format!("a part of the mesh covers indices {:?} of {total}", s.indices)));
            }
        }
        let bounds = Aabb::from_points(vertices.iter().map(|v| Vec3::from_array(v.position)));
        let skinned = vertices.iter().any(|v| v.weights != [0; 4]);
        Ok(Self { vertices: vertices.into(), indices: indices.into(), submeshes, bounds, skinned })
    }

    /// What the mesh occupies on the GPU.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.vertices.len() * size_of::<MeshVertex>() + self.indices.len() * 4 * 3
    }

    /// The edges of every part, each once, as pairs of indices; `ranges[i]` is part `i`'s run.
    #[must_use]
    pub fn wire_indices(&self) -> (Vec<u32>, Vec<Range<u32>>) {
        let mut out = Vec::with_capacity(self.indices.len() * 2);
        let mut ranges = Vec::with_capacity(self.submeshes.len());
        let mut seen: rustc_hash::FxHashSet<(u32, u32)> = rustc_hash::FxHashSet::default();
        for s in &self.submeshes {
            let start = out.len() as u32;
            seen.clear();
            let tris = self.indices.get(s.indices.start as usize..s.indices.end as usize).unwrap_or(&[]);
            for t in tris.as_chunks::<3>().0 {
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    if seen.insert((a.min(b), a.max(b))) {
                        out.push(a);
                        out.push(b);
                    }
                }
            }
            ranges.push(start..out.len() as u32);
        }
        (out, ranges)
    }

    /// A library object (`.object`): one part per surface, the vertex normals from its smoothing
    /// groups, bone weights when it has a skeleton.
    pub fn from_object(object: &Object, table: &MaterialTable) -> Result<Self> {
        let skinned = object.is_skeleton();
        // per surface: its vertices' indices, in face order
        let mut parts: Vec<Vec<u32>> = vec![Vec::new(); object.surfaces.len()];
        let mut vertices: Vec<MeshVertex> = Vec::new();
        let mut weld: FxHashMap<(u32, [u32; 5]), u32> = FxHashMap::default();
        for mesh in &object.meshes {
            let normals = mesh.vertex_normals();
            let skin = if skinned { Some(mesh.skin_vertices(&object.bones, 4, &normals)?) } else { None };
            for sf in &mesh.surf_faces {
                let Some(part) = parts.get_mut(sf.surface as usize) else {
                    return Err(invalid(format!(
                        "mesh '{}' uses surface {} of {}",
                        mesh.name,
                        sf.surface,
                        object.surfaces.len()
                    )));
                };
                weld.clear();
                for &f in sf.faces.iter() {
                    let Some(face) = mesh.faces.get(f as usize) else {
                        return Err(invalid(format!(
                            "mesh '{}' lists face {f} of {}",
                            mesh.name,
                            mesh.faces.len()
                        )));
                    };
                    for (k, fv) in face.v.iter().enumerate() {
                        let corner = f as usize * 3 + k;
                        let Some(p) = mesh.points.get(fv.point as usize).copied() else {
                            return Err(invalid(format!(
                                "mesh '{}' has a face at point {} of {}",
                                mesh.name,
                                fv.point,
                                mesh.points.len()
                            )));
                        };
                        let n = normals.get(corner).copied().unwrap_or(Vec3::Y);
                        let uv = mesh.corner_uv(*fv).unwrap_or(Vec2::ZERO);
                        let id = if let Some(skin) = &skin {
                            let Some(sv) = skin.get(corner) else {
                                return Err(invalid("the skin lost a face corner"));
                            };
                            let mut v = MeshVertex::new(sv.offs, sv.norm, sv.uv);
                            let mut weights = [0u8; 4];
                            for (slot, b) in sv.bones.iter().take(4).enumerate() {
                                v.bones[slot] = b.id;
                                weights[slot] = (b.w.clamp(0.0, 1.0) * 255.0).round() as u8;
                            }
                            if weights == [0; 4] {
                                weights[0] = 255;
                            }
                            v.weights = weights;
                            push_vertex(&mut vertices, v)?
                        } else {
                            let key = (
                                fv.point,
                                [uv.x.to_bits(), uv.y.to_bits(), n.x.to_bits(), n.y.to_bits(), n.z.to_bits()],
                            );
                            match weld.get(&key) {
                                Some(id) => *id,
                                None => {
                                    let id = push_vertex(&mut vertices, MeshVertex::new(p, n, uv))?;
                                    weld.insert(key, id);
                                    id
                                }
                            }
                        };
                        part.push(id);
                    }
                }
            }
        }
        let mut indices = Vec::with_capacity(parts.iter().map(Vec::len).sum());
        let mut submeshes = Vec::new();
        for (surface, part) in object.surfaces.iter().zip(parts) {
            if part.is_empty() {
                continue;
            }
            let start = indices.len() as u32;
            indices.extend(part);
            let kind = table.classify(&surface.shader);
            let material = MaterialDesc::textured(kind, &surface.texture)
                .with_two_sided(surface.flags.contains(SurfaceFlags::TWO_SIDED));
            submeshes.push(SubMesh { indices: start..indices.len() as u32, material });
        }
        Self::new(vertices, indices, submeshes)
    }

    /// A compiled visual (`.ogf`): every child with geometry becomes a part with its texture
    /// and shader; a LOD visual with no children draws its eight billboards.
    pub fn from_visual(visual: &Visual, table: &MaterialTable) -> Result<Self> {
        let mut vertices: Vec<MeshVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut submeshes = Vec::new();
        for v in visual.meshes() {
            let (Some(vb), Some(ib)) = (&v.vertices, &v.indices) else { continue };
            let skinned = matches!(vb.format, VertexFormat::Skinned { .. });
            let range = match v.swi.as_ref().and_then(|s| s.windows.first()) {
                Some(w) => w.offset as usize..w.offset as usize + w.num_tris as usize * 3,
                None => 0..ib.len(),
            };
            let Some(tris) = ib.get(range) else {
                return Err(invalid("a slide window of the model lies outside its indices"));
            };
            if tris.is_empty() {
                continue;
            }
            let base =
                u32::try_from(vertices.len()).map_err(|_| invalid("the model has too many vertices"))?;
            for src in vb.vertices.iter() {
                let mut out = MeshVertex::new(src.position, src.normal, src.uv);
                out.uv2 = src.uv2.to_array();
                if skinned {
                    out.bones = src.bones;
                    let mut w = src.weights.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
                    if w == [0; 4] {
                        w[0] = 255;
                    }
                    out.weights = w;
                }
                vertices.push(out);
            }
            let count = vb.vertices.len();
            let start = indices.len() as u32;
            for &i in tris {
                if usize::from(i) >= count {
                    return Err(invalid(format!("an index of the model points at vertex {i} of {count}")));
                }
                indices.push(base + u32::from(i));
            }
            submeshes
                .push(SubMesh { indices: start..indices.len() as u32, material: visual_material(v, table) });
        }
        if submeshes.is_empty()
            && let Some(facets) = &visual.lod
        {
            let material = visual_material(visual, table);
            let material = MaterialDesc { kind: EditorMaterial::Tree, two_sided: true, ..material };
            for facet in facets.iter() {
                let base = vertices.len() as u32;
                let [a, b, c, _] = facet.vertices;
                let n = (b.position - a.position)
                    .cross(c.position - a.position)
                    .try_normalize()
                    .unwrap_or(Vec3::Y);
                for lv in facet.vertices {
                    vertices.push(MeshVertex::new(lv.position, n, lv.uv));
                }
                indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            submeshes.push(SubMesh { indices: 0..indices.len() as u32, material });
        }
        Self::new(vertices, indices, submeshes)
    }

    /// The unit cube, -0.5 to 0.5, a texture on every face.
    #[must_use]
    pub fn cube(material: MaterialDesc) -> Self {
        let faces = [
            (Vec3::X, Vec3::Y),
            (-Vec3::X, Vec3::Y),
            (Vec3::Y, Vec3::Z),
            (-Vec3::Y, -Vec3::Z),
            (Vec3::Z, Vec3::Y),
            (-Vec3::Z, Vec3::Y),
        ];
        let mut vertices = Vec::with_capacity(24);
        let mut indices = Vec::with_capacity(36);
        for (n, up) in faces {
            quad_into(&mut vertices, &mut indices, n * 0.5, n, up, Vec2::splat(1.0));
        }
        Self::built(vertices, indices, material)
    }

    /// A rectangle `size` wide and high in the XY plane, facing -Z (a camera that looks along +Z
    /// sees its front).
    #[must_use]
    pub fn quad(material: MaterialDesc, size: Vec2) -> Self {
        let mut vertices = Vec::with_capacity(4);
        let mut indices = Vec::with_capacity(6);
        quad_into(&mut vertices, &mut indices, Vec3::ZERO, -Vec3::Z, Vec3::Y, size);
        Self::built(vertices, indices, material)
    }

    /// A sphere of radius 1, `rings` by `segments`, seen from outside (a sky dome is seen from
    /// inside: draw it two-sided).
    #[must_use]
    pub fn sphere(material: MaterialDesc, rings: u32, segments: u32) -> Self {
        let rings = rings.clamp(2, 256);
        let segments = segments.clamp(3, 512);
        let mut vertices = Vec::with_capacity(((rings + 1) * (segments + 1)) as usize);
        for r in 0..=rings {
            let v = r as f32 / rings as f32;
            let theta = v * std::f32::consts::PI;
            for s in 0..=segments {
                let u = s as f32 / segments as f32;
                let phi = u * std::f32::consts::TAU;
                let n = Vec3::new(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
                vertices.push(MeshVertex::new(n, n, Vec2::new(u, v)));
            }
        }
        let mut indices = Vec::with_capacity((rings * segments * 6) as usize);
        let row = segments + 1;
        for r in 0..rings {
            for s in 0..segments {
                let a = r * row + s;
                let b = a + row;
                indices.extend([a, a + 1, b + 1, a, b + 1, b]);
            }
        }
        Self::built(vertices, indices, material)
    }

    /// `dxEnvironmentRender`'s half box: twelve corners from just under the horizon up, each
    /// with its cube-map direction in its normal (`hbox_verts`, `hbox_faces`).
    #[must_use]
    pub fn sky_box() -> Self {
        const VERTS: [([f32; 3], [f32; 3]); 12] = [
            ([-1.0, -1.0, -1.0], [-1.0, -1.01, -1.0]),
            ([1.0, -1.0, -1.0], [1.0, -1.01, -1.0]),
            ([-1.0, -1.0, 1.0], [-1.0, -1.01, 1.0]),
            ([1.0, -1.0, 1.0], [1.0, -1.01, 1.0]),
            ([-1.0, 2.0, -1.0], [-1.0, 1.0, -1.0]),
            ([1.0, 2.0, -1.0], [1.0, 1.0, -1.0]),
            ([-1.0, 2.0, 1.0], [-1.0, 1.0, 1.0]),
            ([1.0, 2.0, 1.0], [1.0, 1.0, 1.0]),
            ([-1.0, 0.0, -1.0], [-1.0, -1.0, -1.0]),
            ([1.0, 0.0, -1.0], [1.0, -1.0, -1.0]),
            ([1.0, 0.0, 1.0], [1.0, -1.0, 1.0]),
            ([-1.0, 0.0, 1.0], [-1.0, -1.0, 1.0]),
        ];
        const FACES: [u32; 60] = [
            0, 2, 3, 3, 1, 0, 4, 5, 7, 7, 6, 4, 0, 1, 9, 9, 8, 0, 8, 9, 5, 5, 4, 8, 1, 3, 10, 10, 9, 1, 9,
            10, 7, 7, 5, 9, 3, 2, 11, 11, 10, 3, 10, 11, 6, 6, 7, 10, 2, 0, 8, 8, 11, 2, 11, 8, 4, 4, 6, 11,
        ];
        let vertices = VERTS
            .iter()
            .map(|(p, tc)| MeshVertex::new(Vec3::from_array(*p), Vec3::from_array(*tc), Vec2::ZERO))
            .collect();
        Self::built(vertices, FACES.to_vec(), MaterialDesc::new(EditorMaterial::Sky))
    }

    fn built(vertices: Vec<MeshVertex>, indices: Vec<u32>, material: MaterialDesc) -> Self {
        let bounds = Aabb::from_points(vertices.iter().map(|v| Vec3::from_array(v.position)));
        let n = indices.len() as u32;
        Self {
            vertices: vertices.into(),
            indices: indices.into(),
            submeshes: vec![SubMesh { indices: 0..n, material }],
            bounds,
            skinned: false,
        }
    }

    /// Every vertex coloured (for hand-made helper meshes).
    #[must_use]
    pub fn with_vertex_color(self, color: Color) -> Self {
        let vertices: Vec<MeshVertex> =
            self.vertices.iter().map(|v| MeshVertex { color: color.0, ..*v }).collect();
        Self { vertices: vertices.into(), ..self }
    }
}

fn push_vertex(vertices: &mut Vec<MeshVertex>, v: MeshVertex) -> Result<u32> {
    let id = u32::try_from(vertices.len()).map_err(|_| invalid("the model has too many vertices"))?;
    vertices.push(v);
    Ok(id)
}

/// A rectangle facing `n`, its top towards `up`, wound clockwise as its front is seen.
fn quad_into(
    vertices: &mut Vec<MeshVertex>,
    indices: &mut Vec<u32>,
    center: Vec3,
    n: Vec3,
    up: Vec3,
    size: Vec2,
) {
    let v = up * (size.y * 0.5);
    let u = n.cross(up) * (size.x * 0.5);
    let base = vertices.len() as u32;
    let corners = [
        (center - u + v, Vec2::new(0.0, 0.0)),
        (center + u + v, Vec2::new(1.0, 0.0)),
        (center + u - v, Vec2::new(1.0, 1.0)),
        (center - u - v, Vec2::new(0.0, 1.0)),
    ];
    for (p, uv) in corners {
        vertices.push(MeshVertex::new(p, n, uv));
    }
    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// A visual's texture and shader as a material: `"base,detail"` lists both textures.
fn visual_material(v: &Visual, table: &MaterialTable) -> MaterialDesc {
    let (texture, shader) = v.texture.as_ref().map_or(("", ""), |(t, s)| (t.as_str(), s.as_str()));
    let kind = table.classify(shader);
    let kind =
        if v.kind() == MT::LOD && kind == EditorMaterial::Opaque { EditorMaterial::Tree } else { kind };
    let mut names = texture.split(',').map(str::trim).filter(|t| !t.is_empty());
    let mut m = MaterialDesc::textured(kind, names.next().unwrap_or(""));
    m.detail = names.next().map(texture_key);
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opaque() -> MaterialDesc {
        MaterialDesc::new(EditorMaterial::Opaque)
    }

    /// Screen-space winding as a camera on the face's normal side sees it: clockwise is front.
    fn clockwise_from_outside(m: &MeshData) -> bool {
        m.indices.as_chunks::<3>().0.iter().all(|t| {
            let p = |i: u32| Vec3::from_array(m.vertices[i as usize].position);
            let n = Vec3::from_array(m.vertices[t[0] as usize].normal);
            let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
            // left-handed: a clockwise triangle seen from +n has (b-a) x (c-a) along +n
            (b - a).cross(c - a).dot(n) > 0.0
        })
    }

    #[test]
    fn hand_made_shapes_face_out() {
        let cube = MeshData::cube(opaque());
        assert_eq!(cube.vertices.len(), 24);
        assert_eq!(cube.indices.len(), 36);
        assert_eq!(cube.bounds, Aabb::UNIT);
        assert!(clockwise_from_outside(&cube));
        let quad = MeshData::quad(opaque(), Vec2::new(2.0, 1.0));
        assert!(clockwise_from_outside(&quad));
        assert_eq!(quad.bounds.size(), Vec3::new(2.0, 1.0, 0.0));
        let (wire, ranges) = cube.wire_indices();
        // 6 faces x 5 edges (4 sides and the diagonal)
        assert_eq!(wire.len(), 60);
        assert_eq!(ranges, vec![0..60]);
        let sphere = MeshData::sphere(opaque(), 8, 12);
        assert!((sphere.bounds.size() - Vec3::splat(2.0)).abs().max_element() < 1e-5);
    }

    #[test]
    fn bad_meshes_are_refused() {
        let v = vec![MeshVertex::default(); 3];
        let part = |r: Range<u32>| vec![SubMesh { indices: r, material: opaque() }];
        assert!(MeshData::new(v.clone(), vec![0, 1, 2], part(0..3)).is_ok());
        assert!(MeshData::new(v.clone(), vec![0, 1, 3], part(0..3)).is_err());
        assert!(MeshData::new(v.clone(), vec![0, 1], part(0..2)).is_err());
        assert!(MeshData::new(v.clone(), vec![0, 1, 2], part(0..6)).is_err());
        assert_eq!(MeshData::new(Vec::new(), vec![0, 1, 2], part(0..3)), Err(RenderError::EmptyMesh));
    }
}
