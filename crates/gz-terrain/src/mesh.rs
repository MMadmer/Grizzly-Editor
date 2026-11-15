//! The terrain's model: the grid as triangles with one diagonal throughout, the skirt round the
//! border, one surface per distinct look, the texture coordinates of each face, every edge soft.

use std::collections::HashMap;
use std::sync::Arc;

use glam::{Vec2, Vec3};
use gz_base::SmolStr;
use gz_fmt_object::{Face, FaceVert, Mesh, Object, SurfFaces, Surface, SurfaceFlags, VMap, VMapKind, VMapPt};

use crate::grid::Grid;
use crate::ground::GroundTri;
use crate::model::{Look, SKIRT_DROP, SKIRT_WIDTH, Terrain};

/// What the faces wear.
#[derive(Clone, Copy, Debug)]
pub enum Looks<'a> {
    /// One look, laid flat every `tile` metres.
    Table(&'a Look),
    /// The ground's: per face of the grid the ground triangle under its middle
    /// ([`crate::GroundSample::face_tris`]), the ground's triangles and their looks.
    Ground { faces: &'a [Option<u32>], tris: &'a [GroundTri], looks: &'a [Look] },
}

/// What one face wears: its look and how its texture coordinates are made.
#[derive(Clone, Copy, Debug)]
pub enum FaceLook<'a> {
    Flat(&'a Look),
    /// The ground triangle's mapping, carried to the face's corners.
    Ground(&'a Look, &'a GroundTri),
}

impl FaceLook<'_> {
    fn look(&self) -> &Look {
        match self {
            Self::Flat(l) | Self::Ground(l, _) => l,
        }
    }

    fn uv(&self, p: Vec3) -> Vec2 {
        match self {
            Self::Flat(l) => Vec2::new(p.x, p.z) / l.tile,
            Self::Ground(_, t) => {
                let (a, b, c) = (t.p[0], t.p[1], t.p[2]);
                let det = (b.x - a.x) * (c.z - a.z) - (c.x - a.x) * (b.z - a.z);
                if det.abs() < 1e-12 {
                    return t.uv[0];
                }
                let u = ((p.x - a.x) * (c.z - a.z) - (c.x - a.x) * (p.z - a.z)) / det;
                let v = ((b.x - a.x) * (p.z - a.z) - (p.x - a.x) * (b.z - a.z)) / det;
                t.uv[0] + (t.uv[1] - t.uv[0]) * u + (t.uv[2] - t.uv[0]) * v
            }
        }
    }
}

type LookKey = (String, String, String, String);

fn key(l: &Look) -> LookKey {
    (l.texture.clone(), l.shader.clone(), l.compile.clone(), l.material.clone())
}

struct Builder {
    points: Vec<Vec3>,
    faces: Vec<Face>,
    face_surface: Vec<u32>,
    refs: Vec<(u32, Vec2)>,
    ref_of: HashMap<(u32, [u32; 2]), u32>,
    surfaces: Vec<Surface>,
    surface_of: HashMap<LookKey, u32>,
}

impl Builder {
    fn surface(&mut self, l: &Look) -> u32 {
        let k = key(l);
        if let Some(s) = self.surface_of.get(&k) {
            return *s;
        }
        let n = self.surfaces.len() as u32;
        self.surfaces.push(Surface {
            name: SmolStr::new(format!("terrain_{n}")),
            texture: SmolStr::new(&l.texture),
            vmap: SmolStr::new_static("Texture"),
            shader: SmolStr::new(&l.shader),
            shader_xrlc: SmolStr::new(&l.compile),
            game_mtl: SmolStr::new(&l.material),
            flags: SurfaceFlags::empty(),
            // a position, a normal and one texture coordinate set
            fvf: 0x112,
        });
        self.surface_of.insert(k, n);
        n
    }

    fn corner(&mut self, point: u32, uv: Vec2) -> FaceVert {
        let uv = if uv.is_finite() { uv } else { Vec2::ZERO };
        let k = (point, [uv.x.to_bits(), uv.y.to_bits()]);
        let vmref = match self.ref_of.get(&k) {
            Some(r) => *r,
            None => {
                let r = self.refs.len() as u32;
                self.refs.push((point, uv));
                self.ref_of.insert(k, r);
                r
            }
        };
        FaceVert { point, vmref }
    }

    fn face(&mut self, p: [u32; 3], look: &FaceLook<'_>) {
        let s = self.surface(look.look());
        let mut f = Face::default();
        for (k, point) in p.iter().enumerate() {
            let uv = look.uv(self.points[*point as usize]);
            f.v[k] = self.corner(*point, uv);
        }
        self.faces.push(f);
        self.face_surface.push(s);
    }
}

impl Terrain {
    /// The model of these heights: in the world (it is drawn at the identity), one mesh.
    #[must_use]
    pub fn mesh(&self, grid: &Grid, looks: Looks<'_>) -> Object {
        let (cols, rows) = (grid.columns, grid.rows);
        let fallback = Look::default();
        let face_look = |f: usize| -> FaceLook<'_> {
            match looks {
                Looks::Table(l) => FaceLook::Flat(l),
                Looks::Ground { faces, tris, looks } => {
                    match faces.get(f).copied().flatten().and_then(|t| tris.get(t as usize)) {
                        Some(t) => FaceLook::Ground(looks.get(t.look as usize).unwrap_or(&fallback), t),
                        None => FaceLook::Flat(&fallback),
                    }
                }
            }
        };
        let mut b = Builder {
            points: Vec::with_capacity(cols * rows + 2 * (cols + rows)),
            faces: Vec::new(),
            face_surface: Vec::new(),
            refs: Vec::new(),
            ref_of: HashMap::new(),
            surfaces: Vec::new(),
            surface_of: HashMap::new(),
        };
        for j in 0..rows {
            for i in 0..cols {
                b.points.push(self.to_world(self.point(i, j), grid.at(i, j)));
            }
        }
        let at = |i: usize, j: usize| (j * cols + i) as u32;
        // one diagonal throughout: (i, j + 1) to (i + 1, j)
        for j in 0..rows - 1 {
            for i in 0..cols - 1 {
                let f = (j * (cols - 1) + i) * 2;
                b.face([at(i, j), at(i, j + 1), at(i + 1, j)], &face_look(f));
                b.face([at(i + 1, j), at(i, j + 1), at(i + 1, j + 1)], &face_look(f + 1));
            }
        }
        // the skirt: the border once round, each point with its twin out and down
        let mut ring: Vec<(usize, usize, usize)> = Vec::new();
        for i in 0..cols - 1 {
            ring.push((i, 0, (i * 2).min((cols - 2) * 2)));
        }
        for j in 0..rows - 1 {
            ring.push((cols - 1, j, (j * (cols - 1) + cols - 2) * 2 + 1));
        }
        for i in (1..cols).rev() {
            ring.push((i, rows - 1, ((rows - 2) * (cols - 1) + i - 1) * 2 + 1));
        }
        for j in (1..rows).rev() {
            ring.push((0, j, (j - 1) * (cols - 1) * 2));
        }
        let h = self.half();
        let outer: Vec<u32> = ring
            .iter()
            .map(|(i, j, _)| {
                let p = self.point(*i, *j);
                let out = Vec2::new(
                    if p.x >= h.x {
                        1.0
                    } else if p.x <= -h.x {
                        -1.0
                    } else {
                        0.0
                    },
                    if p.y >= h.y {
                        1.0
                    } else if p.y <= -h.y {
                        -1.0
                    } else {
                        0.0
                    },
                );
                b.points.push(self.to_world(p + out * SKIRT_WIDTH, grid.at(*i, *j) - SKIRT_DROP));
                (b.points.len() - 1) as u32
            })
            .collect();
        for k in 0..ring.len() {
            let n = (k + 1) % ring.len();
            let (a, c) = (at(ring[k].0, ring[k].1), at(ring[n].0, ring[n].1));
            let (sa, sc) = (outer[k], outer[n]);
            let look = face_look(ring[k].2);
            for tri in [[a, sa, c], [c, sa, sc]] {
                // the skirt faces up and out, as the grid does
                let p = tri.map(|i| b.points[i as usize]);
                let up = (p[1] - p[0]).cross(p[2] - p[0]).y;
                b.face(if up > 0.0 { tri } else { [tri[0], tri[2], tri[1]] }, &look);
            }
        }

        let Builder { points, faces, face_surface, refs, surfaces, .. } = b;
        let mut uv = VMap {
            name: SmolStr::new_static("Texture"),
            kind: VMapKind::Uv,
            polymap: false,
            values: Vec::with_capacity(refs.len() * 2),
            vindices: Vec::with_capacity(refs.len()),
            pindices: Vec::new(),
        };
        let mut vmrefs = Vec::with_capacity(refs.len());
        for (i, (point, t)) in refs.iter().enumerate() {
            uv.values.extend([t.x, t.y]);
            uv.vindices.push(*point);
            vmrefs.push(std::iter::once(VMapPt { vmap: 0, index: i as u32 }).collect());
        }
        let mut by_surface: Vec<Vec<u32>> = vec![Vec::new(); surfaces.len()];
        for (f, s) in face_surface.iter().enumerate() {
            by_surface[*s as usize].push(f as u32);
        }
        let surf_faces = by_surface
            .into_iter()
            .enumerate()
            .map(|(s, list)| SurfFaces { surface: s as u32, faces: Arc::from(list) })
            .collect();
        let count = faces.len();
        let mut mesh = Mesh {
            name: SmolStr::new_static("terrain"),
            points: Arc::from(points),
            faces: Arc::from(faces),
            // every edge soft: the ground shades smooth
            smooth_groups: Arc::from(vec![0u32; count]),
            vmaps: vec![uv],
            vmrefs,
            surf_faces,
            ..Mesh::default()
        };
        // a mesh made here holds only valid references: the rebuild cannot fail on it
        let _ = mesh.rebuild_vmaps();
        mesh.recompute_bbox();
        Object { surfaces, meshes: vec![mesh], ..Object::default() }
    }
}
