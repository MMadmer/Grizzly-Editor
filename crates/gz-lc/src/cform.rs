//! The collision form: the collision faces of the level and of every MU reference welded into one soup,
//! then decimated (QSlim) along edges that keep their material, sector and smoothing, and saved as `level.cform`.
//!
//! The soup is what X-Ray's packed collector (`gz_fmt_level::CollectorPacked`) makes of the faces added
//! in their order; it is welded once all of them are in ([`weld_packed`]), on every core.

use std::time::Instant;

use glam::Vec3;
use gz_fmt_level::{Aabb, CForm, Tri};
use gz_lc_light::math::{Corners, EPS, mknormal, weld_packed};
use gz_lc_light::{
    Ctl, FaceId, LcError, LcGlobalData, QUALITY_DRAFT, ShaderXrlc, VertId, connect_by_edge_flags, shader_of,
};
use rayon::prelude::*;

use crate::geom::timed;
use crate::qslim::{EdgeQSlim, Model, Placement, Weighting};
use crate::{Log, Result};

/// Faces as the packed collector takes them, three corners each (with the vertex each stands for)
/// with the packed word and the flags.
#[derive(Default)]
struct Soup {
    points: Corners,
    words: Vec<u32>,
    flags: Vec<u32>,
}

/// The soup welded: the vertices, the triangles and every triangle's flags.
struct Welded {
    verts: Vec<Vec3>,
    tris: Vec<Tri>,
    flags: Vec<u32>,
}

impl Soup {
    fn with_capacity(faces: usize) -> Self {
        Self {
            points: Corners::with_capacity(faces * 3),
            words: Vec::with_capacity(faces),
            flags: Vec::with_capacity(faces),
        }
    }

    /// `add_face` of a face of vertices `v` (numbers of the mesh being added) at `p`.
    fn add_face(&mut self, v: [u32; 3], p: [Vec3; 3], material: u16, sector: u16, flags: u32) {
        for (v, p) in v.into_iter().zip(p) {
            self.points.push(v, p);
        }
        self.words.push(Tri::new([0; 3], material, sector).dummy);
        self.flags.push(flags);
    }

    /// What the collector over `bb` gives for these faces.
    fn weld(self, bb: &Aabb, ctl: &Ctl<'_>) -> Result<Welded> {
        let stop = || ctl.cancelled();
        let Some((verts, index)) = weld_packed(bb, &self.points.points, Some(&self.points.same), &stop)
        else {
            ctl.check()?;
            return Err(LcError::Cancelled.into());
        };
        let tris = index
            .as_chunks::<3>()
            .0
            .iter()
            .zip(&self.words)
            .map(|(v, &dummy)| Tri { verts: *v, dummy })
            .collect();
        Ok(Welded { verts, tris, flags: self.flags })
    }
}

const MAX_DECIMATE_ERROR: f32 = 0.0005;
const COMPACTNESS_RATIO: f64 = 0.001;

struct FaceProps {
    material: u16,
    sector: u16,
    flags: u32,
    norm: Vec3,
}

/// `common_edge_idx`.
fn common_edge(base: [u32; 3], e: usize, test: [u32; 3]) -> u16 {
    let (mut b0, mut b1) = (base[e], base[(e + 1) % 3]);
    if b0 > b1 {
        std::mem::swap(&mut b0, &mut b1);
    }
    for i in 0..3 {
        let (mut t0, mut t1) = (test[i], test[(i + 1) % 3]);
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        if b0 == t0 && b1 == t1 {
            return i as u16;
        }
    }
    u16::MAX
}

fn fsimilar(a: f32, b: f32, e: f32) -> bool {
    (a - b).abs() < e
}

/// The decimation: the welded soup in, the faces left as a new soup out.
fn simplify(cl: Welded, log: &mut Log, ctl: &Ctl<'_>) -> Result<Soup> {
    let mut tm = Instant::now();
    let Welded { verts, tris, flags } = cl;
    let mut mdl = Model::with_capacity(verts.len(), tris.len());
    for v in &verts {
        mdl.add_vertex(v.to_array());
    }
    let mut props = Vec::with_capacity(tris.len());
    for (t, &fl) in tris.iter().zip(&flags) {
        mdl.add_face(t.verts);
        let g = |i: u32| verts.get(i as usize).copied().unwrap_or(Vec3::ZERO);
        props.push(FaceProps {
            material: t.material(),
            sector: t.sector(),
            flags: fl,
            norm: mknormal(g(t.verts[0]), g(t.verts[1]), g(t.verts[2])),
        });
    }
    let mut cl = Soup::default();

    let mut slim = EdgeQSlim::new(mdl);
    slim.boundary_weight = 1_000_000.0;
    slim.compactness_ratio = COMPACTNESS_RATIO;
    slim.meshing_penalty = 1_000_000.0;
    slim.placement = Placement::Endpoints;
    slim.weighting = Weighting::Uniform;
    slim.initialize();
    timed(log, "the quadrics", &mut tm);

    // the edges that must stay, found for every face at once: X-Ray's compiler marks the faces around
    // both ends of an edge (0 for N1, 1 for N0, then +1 for each time a face is in N1) and takes
    // those marked 2 - counted here without the marks; the constraints are then applied in the
    // original order
    let rm = [(0usize, 1usize), (1, 2), (2, 0)];
    let valid = slim.valid_faces;
    let model = &slim.model;
    let constraints: Vec<(u32, u32, u32)> = (0..valid)
        .into_par_iter()
        .flat_map_iter(|f| {
            let mut out = Vec::new();
            if ctl.cancelled() || !model.face_is_valid(f) {
                return out.into_iter();
            }
            let base = model.face(f);
            let Some(bp) = props.get(f as usize) else { return out.into_iter() };
            for (e, &(i, j)) in rm.iter().enumerate() {
                let n0 = model.neighbors(base[i]);
                let n1 = model.neighbors(base[j]);
                let n = if n0.len() < n1.len() { n0 } else { n1 };
                if n.is_empty() {
                    continue;
                }
                let mark =
                    |k: u32| u32::from(n0.contains(&k)) + n1.iter().filter(|&&x| x == k).count() as u32;
                let (mut pos, mut neg) = (0u32, 0u32);
                let mut need = false;
                for &k in n {
                    if f == k || !n1.contains(&k) || mark(k) != 2 {
                        continue;
                    }
                    let Some(cp) = props.get(k as usize) else { continue };
                    let ce = common_edge(base, e, model.face(k));
                    if cp.material != bp.material
                        || cp.sector != bp.sector
                        || !connect_by_edge_flags(bp.flags, cp.flags, e as u16, ce)
                    {
                        need = true;
                        break;
                    }
                    let d = bp.norm.x * cp.norm.x + bp.norm.y * cp.norm.y + bp.norm.z * cp.norm.z;
                    if fsimilar(d, -1.0, EPS) {
                        neg += 1;
                    } else if fsimilar(d, 1.0, EPS) {
                        pos += 1;
                    }
                }
                if need || (pos == 0 && neg == 1) {
                    out.push((base[i], base[j], f));
                }
            }
            out.into_iter()
        })
        .collect();
    ctl.check()?;
    for (a, b, f) in constraints {
        slim.constraint_manual(a, b, f);
    }
    timed(log, "the constraints", &mut tm);
    slim.collect_edges();
    ctl.check()?;
    let stop = || ctl.cancelled();
    timed(log, "the edges to contract", &mut tm);
    slim.decimate_until(0, MAX_DECIMATE_ERROR, &stop);
    ctl.check()?;
    timed(log, "the contractions", &mut tm);
    let mut mdl = std::mem::take(&mut slim.model);
    mdl.compact_vertices();
    for f in 0..mdl.face_count() as u32 {
        if !mdl.face_is_valid(f) {
            continue;
        }
        let fc = mdl.face(f);
        let Some(fp) = props.get(f as usize) else { continue };
        let p = fc.map(|v| Vec3::from_array(mdl.vertex(v)));
        cl.add_face(fc, p, fp.material, fp.sector, fp.flags);
    }
    timed(log, "the faces left", &mut tm);
    Ok(cl)
}

/// Builds the collision form: answers `level.cform`.
pub fn build_cform(g: &mut LcGlobalData, log: &mut Log, ctl: &Ctl<'_>) -> Result<Vec<u8>> {
    let mut tm = Instant::now();
    // X-Ray's compiler sorts the vertex list by address; the collision vertices follow that order
    crate::geom::sort_vertices_by_addr(g);
    let mut marks: rustc_hash::FxHashSet<VertId> = rustc_hash::FxHashSet::default();
    let mut faces: Vec<FaceId> = Vec::with_capacity(g.mesh.faces().len());
    for &f in g.mesh.faces() {
        let Some(face) = g.mesh.face(f) else { continue };
        if g.shader(face.material).flags & ShaderXrlc::COLLISION != 0 {
            faces.push(f);
            for v in face.v {
                marks.insert(v);
            }
        }
    }
    // and the faces by address
    faces.sort_unstable_by_key(|&f| g.mesh.face_addr(f));
    let mut bb = Aabb::INVALID;
    for &v in g.mesh.vertices() {
        if marks.contains(&v)
            && let Some(x) = g.mesh.vertex(v)
        {
            bb.modify(x.p);
        }
    }
    log.status(format!("Items to process: {}", faces.len()));
    let mut cl = Soup::with_capacity(faces.len());
    // the multiple edges of every face, found at once and reported in face order
    let mesh = &g.mesh;
    let multi: Vec<Vec<(Vec3, Vec3)>> = faces
        .par_iter()
        .map(|&f| {
            let mut out = Vec::new();
            if ctl.cancelled() {
                return out;
            }
            let Some(face) = mesh.face(f) else { return out };
            for (a, b) in [(face.v[0], face.v[1]), (face.v[1], face.v[2]), (face.v[2], face.v[0])] {
                let n = mesh.vertex(a).map_or(0, |x| {
                    x.adjacents
                        .iter()
                        .filter(|&&t| t != f && mesh.face(t).is_some_and(|y| y.v.contains(&b)))
                        .count()
                });
                if n > 1 {
                    let pa = mesh.vertex(a).map_or(Vec3::ZERO, |x| x.p);
                    let pb = mesh.vertex(b).map_or(Vec3::ZERO, |x| x.p);
                    out.push((pa, pb));
                }
            }
            out
        })
        .collect();
    ctl.check()?;
    let mut critical = 0u32;
    for (&f, edges) in faces.iter().zip(multi) {
        for (pa, pb) in edges {
            critical += 1;
            g.mesh.errors.multiedge.push(pa);
            g.mesh.errors.multiedge.push(pb);
        }
        let Some(face) = g.mesh.face(f) else { continue };
        let Some(p) = g.mesh.positions(f) else { continue };
        let sector = g.materials.get(usize::from(face.material)).map_or(0, |m| m.sector);
        cl.add_face(face.v.map(|v| v.0), p, face.material_game, sector, face.sm_group);
    }
    if critical > 0 {
        log.warn(format!("MultipleEdges: {critical} faces"));
    }
    timed(log, "the level's collision faces", &mut tm);
    // export_cform_game: the model's vertex list sorted by address (it stays so), its collision faces
    // taken in the order of their addresses; the same for every reference of a model, found once
    let mut chosen: Vec<Option<Vec<FaceId>>> = vec![None; g.mu_models.len()];
    for r in &g.mu_refs {
        let i = r.model as usize;
        if !chosen.get(i).is_some_and(Option::is_none) {
            continue;
        }
        let Some(m) = g.mu_models.get_mut(i) else { continue };
        let mut list = m.mesh.vertices().to_vec();
        list.sort_unstable_by_key(|&v| m.mesh.vertex_addr(v));
        *m.mesh.vertices_mut() = list;
        let mut faces: Vec<FaceId> = m
            .mesh
            .faces()
            .iter()
            .copied()
            .filter(|&f| {
                m.mesh.face(f).is_some_and(|x| {
                    shader_of(x.material, &g.shaders, &g.materials).flags & ShaderXrlc::COLLISION != 0
                })
            })
            .collect();
        faces.sort_unstable_by_key(|&f| m.mesh.face_addr(f));
        if let Some(slot) = chosen.get_mut(i) {
            *slot = Some(faces);
        }
    }
    for r in &g.mu_refs {
        let i = r.model as usize;
        let (Some(m), Some(Some(faces))) = (g.mu_models.get(i), chosen.get(i)) else { continue };
        cl.points.begin();
        for &f in faces {
            let (Some(face), Some(p)) = (m.mesh.face(f), m.mesh.positions(f)) else { continue };
            let p = p.map(|q| r.xform.transform_tiny(q));
            cl.add_face(face.v.map(|v| v.0), p, face.material_game, r.sector, face.sm_group);
        }
    }
    ctl.check()?;
    timed(log, "the static models' collision faces", &mut tm);
    let mut welded = cl.weld(&bb, ctl)?;
    timed(log, "the weld", &mut tm);
    if g.params.quality != QUALITY_DRAFT {
        let soup = simplify(welded, log, ctl)?;
        tm = Instant::now();
        welded = soup.weld(&bb, ctl)?;
        timed(log, "the weld of what is left", &mut tm);
    }
    let form = CForm::new(welded.verts, welded.tris);
    // the faces no longer name a deflector (X-Ray's compiler clears what shares memory with the game
    // material id)
    let all = g.mesh.faces().to_vec();
    for f in all {
        if let Some(face) = g.mesh.face_mut(f) {
            face.deflector = None;
        }
    }
    Ok(form.write())
}
