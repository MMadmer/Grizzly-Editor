//! The geometry phases before the light: the pre-optimisation and the vertex isolation,
//! the T-junction correction, the adaptive hierarchical tesselation (its qslim variant is
//! commented out in X-Ray's compiler) and the tangent basis.

use glam::{Vec2, Vec3};
use gz_lc_light::math::{EPS, EPS_L, distance, distance_sqr, i_floor, lerp};
use gz_lc_light::{
    BaseColor, Basis, ColorC, Ctl, FaceId, LP_DONT_RGB, LP_DONT_SUN, LcGlobalData, LightCache, Tracer,
    VertId, is_soft_edge, light_point,
};
use rayon::prelude::*;

use crate::mender::{MVertex, mend};
use crate::{Error, Log, Result};

/// The average of two 2D vectors.
fn average(a: Vec2, b: Vec2) -> Vec2 {
    Vec2::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)
}

/// The lists without the slots a phase emptied (`std::remove(..., NULL)`).
pub fn compact_lists(g: &mut LcGlobalData) {
    let mesh = &mut g.mesh;
    let alive_v: Vec<VertId> =
        mesh.vertices().iter().copied().filter(|&v| mesh.vertex(v).is_some()).collect();
    *mesh.vertices_mut() = alive_v;
    let alive_f: Vec<FaceId> = mesh.faces().iter().copied().filter(|&f| mesh.face(f).is_some()).collect();
    *mesh.faces_mut() = alive_f;
}

const HDIM_X: i32 = 56;
const HDIM_Y: i32 = 24;
const HDIM_Z: i32 = 56;

/// `std::sort(g_vertices)`: the list in the order of the vertices' addresses.
pub fn sort_vertices_by_addr(g: &mut LcGlobalData) {
    let mut list = g.mesh.vertices().to_vec();
    list.sort_unstable_by_key(|&v| g.mesh.vertex_addr(v));
    *g.mesh.vertices_mut() = list;
}

/// Vertices within the weld distance merged through an overlapping hash
/// grid, degenerate faces dropped, every other face verified, unused vertices dropped.
pub fn pre_optimize(
    g: &mut LcGlobalData,
    scene_bb: &gz_fmt_level::Aabb,
    log: &mut Log,
    no_invalid_check: bool,
) -> Result<()> {
    let bb = scene_bb;
    let vscale = bb.max - bb.min;
    let vmin = bb.min;
    let pick = |e: f32| if e < EPS_L { e } else { EPS_L };
    let eps = Vec3::new(
        pick(vscale.x / HDIM_X as f32 / 2.0),
        pick(vscale.y / HDIM_Y as f32 / 2.0),
        pick(vscale.z / HDIM_Z as f32 / 2.0),
    );
    let scale = Vec3::new(HDIM_X as f32 / vscale.x, HDIM_Y as f32 / vscale.y, HDIM_Z as f32 / vscale.z);
    let cells = ((HDIM_X + 1) * (HDIM_Y + 1) * (HDIM_Z + 1)) as usize;
    let mut hash: Vec<Vec<VertId>> = vec![Vec::new(); cells];
    let idx = |x: u32, y: u32, z: u32| {
        ((x as usize * (HDIM_Y as usize + 1)) + y as usize) * (HDIM_Z as usize + 1) + z as usize
    };
    let weld = g.params.weld_distance;
    let vcount = g.mesh.vertices().len();
    let fcount = g.mesh.faces().len();
    let list = g.mesh.vertices().to_vec();
    for &test in &list {
        let Some(v) = g.mesh.vertex(test).map(|x| x.p) else { continue };
        let ix = i_floor((v.x - vmin.x) * scale.x) as u32;
        let iy = i_floor((v.y - vmin.y) * scale.y) as u32;
        let iz = i_floor((v.z - vmin.z) * scale.z) as u32;
        if ix > HDIM_X as u32 || iy > HDIM_Y as u32 || iz > HDIM_Z as u32 {
            return Err(Error::Invalid("A vertex lies outside the level's box (PreOptimize).".into()));
        }
        let cell = idx(ix, iy, iz);
        let base = hash.get(cell).and_then(|h| {
            h.iter().copied().find(|&b| {
                g.mesh.vertex(b).is_some_and(|x| {
                    (x.p.x - v.x).abs() < weld && (x.p.y - v.y).abs() < weld && (x.p.z - v.z).abs() < weld
                })
            })
        });
        if let Some(base) = base {
            while let Some(&f) = g.mesh.vertex(test).and_then(|x| x.adjacents.first()) {
                g.mesh.v_replace(f, test, base);
            }
            g.mesh.destroy_vertex(test, false);
            continue;
        }
        if let Some(h) = hash.get_mut(cell) {
            h.push(test);
        }
        let ex = i_floor((v.x + eps.x - vmin.x) * scale.x) as u32;
        let ey = i_floor((v.y + eps.y - vmin.y) * scale.y) as u32;
        let ez = i_floor((v.z + eps.z - vmin.z) * scale.z) as u32;
        if ex > HDIM_X as u32 || ey > HDIM_Y as u32 || ez > HDIM_Z as u32 {
            return Err(Error::Invalid("A vertex lies outside the level's box (PreOptimize).".into()));
        }
        let mut put = |x: u32, y: u32, z: u32| {
            if let Some(h) = hash.get_mut(idx(x, y, z)) {
                h.push(test);
            }
        };
        if ex != ix {
            put(ex, iy, iz);
        }
        if ey != iy {
            put(ix, ey, iz);
        }
        if ez != iz {
            put(ix, iy, ez);
        }
        if ex != ix && ey != iy {
            put(ex, ey, iz);
        }
        if ex != ix && ez != iz {
            put(ex, iy, ez);
        }
        if ey != iy && ez != iz {
            put(ix, ey, ez);
        }
        if ex != ix && ey != iy && ez != iz {
            put(ex, ey, ez);
        }
    }

    let faces = g.mesh.faces().to_vec();
    for f in faces {
        if g.mesh.face(f).is_some_and(|x| x.is_degenerated()) {
            g.mesh.destroy_face(f, false);
        } else {
            g.mesh.verify_face(f);
        }
    }
    if g.mesh.errors.invalid_faces > 0 {
        if !no_invalid_check {
            return Err(Error::InvalidFaces(g.mesh.errors.invalid_faces, g.mesh.errors.write_err()));
        }
        log.warn(format!(
            "{} invalid faces kept (-no_invalidefaces); build.err holds them.",
            g.mesh.errors.invalid_faces
        ));
    }
    let verts = g.mesh.vertices().to_vec();
    for v in verts {
        if g.mesh.vertex(v).is_some_and(|x| x.adjacents.is_empty()) {
            g.mesh.destroy_vertex(v, false);
        }
    }
    compact_lists(g);
    log.msg(format!(
        "{} vertices removed. ({} left)",
        vcount - g.mesh.vertices().len(),
        g.mesh.vertices().len()
    ));
    log.msg(format!("{} faces removed. ({} left)", fcount - g.mesh.faces().len(), g.mesh.faces().len()));
    Ok(())
}

/// The squared distance from a point to a segment.
fn sqr_distance_to_segment(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let c = p - a;
    let v = b - a;
    let d = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    let v = Vec3::new(v.x / d, v.y / d, v.z / d);
    let t = v.x * c.x + v.y * c.y + v.z * c.z;
    if t <= 0.0 {
        return distance_sqr(p, a);
    }
    if t >= d {
        return distance_sqr(p, b);
    }
    let r = Vec3::new(a.x + v.x * t, a.y + v.y * t, a.z + v.z * t);
    distance_sqr(p, r)
}

/// The junctions and the long edges are counted, nothing changes.
/// A face's T-junction candidates (edge ends, the vertex on it) and its long edges.
type JunctionsAndEdges = (Vec<(VertId, VertId, VertId)>, Vec<(VertId, VertId)>);

pub fn correct_tjunctions(g: &LcGlobalData, log: &mut Log) {
    let mesh = &g.mesh;
    let pos = |v: VertId| mesh.vertex(v).map_or(Vec3::ZERO, |x| x.p);
    // per face: (junction candidates, long edges), merged in face order as X-Ray's compiler meets them
    let per_face: Vec<JunctionsAndEdges> = mesh
        .faces()
        .par_iter()
        .map(|&f| {
            let mut js = Vec::new();
            let mut es = Vec::new();
            let Some(face) = mesh.face(f) else { return (js, es) };
            for e in 0..3 {
                let (va, vb) = face.edge_verts(e);
                for (x, y, flip) in [(va, vb, false), (vb, va, true)] {
                    let Some(vx) = mesh.vertex(x) else { continue };
                    for &f1 in &vx.adjacents {
                        let Some(face1) = mesh.face(f1) else { continue };
                        for e1 in 0..3 {
                            let (v1, v2) = face1.edge_verts(e1);
                            if distance(pos(v1), pos(v2)) >= 32.0 {
                                es.push(if v1 > v2 { (v2, v1) } else { (v1, v2) });
                            }
                            let (ea, eb) = if flip { (y, x) } else { (x, y) };
                            if v1 == x && v2 != y {
                                js.push((ea, eb, v2));
                            } else if v2 == x && v1 != y {
                                js.push((ea, eb, v1));
                            }
                        }
                    }
                }
            }
            (js, es)
        })
        .collect();
    let mut junctions: Vec<VertId> = Vec::new();
    let mut seen_t = std::collections::HashSet::new();
    let mut edges = std::collections::HashSet::new();
    for (js, es) in per_face {
        for (a, b, t) in js {
            if sqr_distance_to_segment(pos(t), pos(a), pos(b)).sqrt() < 0.002 && seen_t.insert(t) {
                junctions.push(t);
            }
        }
        for e in es {
            edges.insert(e);
        }
    }
    let line = format!("*** {} junctions and {} long edges found.", junctions.len(), edges.len());
    if junctions.is_empty() && edges.is_empty() { log.msg(line) } else { log.warn(line) }
}

/// `aht_max_edge`: `c_SS_maxsize / 2.5`.
const AHT_MAX_EDGE: f32 = 32.0 / 2.5;

fn is_ccw(a: usize, b: usize) -> bool {
    (a == 0 && b == 1) || (a == 1 && b == 2) || (a == 2 && b == 0)
}

const BACKFACE: u32 = gz_lc_light::BACKFACE_FLAG;

fn set_soft(flags: &mut u32, e: u16, soft: bool) {
    let e = if *flags & BACKFACE != 0 { (4 - e) % 3 } else { e };
    if soft {
        *flags &= !(1 << e);
    } else {
        *flags |= 1 << e;
    }
}

/// Tesselates: every edge longer than
/// [`AHT_MAX_EDGE`] split in the middle, each face sharing it replaced by two.
fn tesselate(g: &mut LcGlobalData, log: &mut Log) {
    let mut counter = 0u32;
    let mut i = 0usize;
    while i < g.mesh.faces().len() {
        let f = g.mesh.faces()[i];
        i += 1;
        let Some(face) = g.mesh.face(f) else { continue };
        if face.flags.splitted {
            if !face.flags.locked {
                g.mesh.destroy_face(f, false);
            }
            continue;
        }
        if g.mesh.calc_area(f) < EPS_L {
            continue;
        }
        // callback_edge_longest
        let mut max_err = -1.0f32;
        let mut max_id = -1i32;
        for e in 0..3 {
            let len = g.mesh.edge_len(f, e);
            if len < AHT_MAX_EDGE {
                continue;
            }
            if len > max_err {
                max_err = len;
                max_id = e as i32;
            }
        }
        if max_id < 0 {
            continue;
        }
        let Some(face) = g.mesh.face(f) else { continue };
        let (v1, v2) = face.edge_verts(max_id as usize);
        let mut adj: Vec<FaceId> = g
            .mesh
            .vertex(v1)
            .map(|x| x.adjacents.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|&a| g.mesh.face(a).is_some_and(|x| !x.flags.splitted && x.v.contains(&v2)))
            .collect();
        // X-Ray's compiler sorts them by address
        adj.sort_unstable_by_key(|&a| g.mesh.face_addr(a));
        adj.dedup();
        counter += 1;
        if counter.is_multiple_of(10000) {
            let verts = g.mesh.vertices().to_vec();
            for v in verts {
                if g.mesh.vertex(v).is_some_and(|x| x.adjacents.is_empty()) {
                    g.mesh.destroy_vertex(v, false);
                }
            }
            log.status(format!("Working: {counter} verts created, {}(now) ...", g.mesh.vertices().len()));
        }
        // tessalate_faces
        let (p1, p2) =
            (g.mesh.vertex(v1).map_or(Vec3::ZERO, |x| x.p), g.mesh.vertex(v2).map_or(Vec3::ZERO, |x| x.p));
        let nv = g.mesh.create_vertex(lerp(p1, p2, 0.5));
        for af in adj {
            let Some(a) = g.mesh.face_mut(af) else { continue };
            a.flags.splitted = true;
            let a = a.clone();
            let atc = a.tc.first().copied().unwrap_or_default();
            let (Some(id1), Some(id2)) = (a.v_index(v1), a.v_index(v2)) else { continue };
            let idb = 3 - (id1 + id2);
            let uv = average(atc.uv[id1], atc.uv[id2]);
            let mut make = |verts: [VertId; 3], uvs: [Vec2; 3], soft: [bool; 3]| {
                let nf = g.mesh.create_face(verts);
                if let Some(x) = g.mesh.face_mut(nf) {
                    x.flags.splitted = false;
                    x.flags.locked = false;
                    x.material = a.material;
                    x.material_game = a.material_game;
                    // set_backface on the default group, then the edges
                    let mut sm = gz_lc_light::NO_SM_GROUP;
                    if a.sm_group & BACKFACE != 0 {
                        sm |= BACKFACE;
                    } else {
                        sm &= !BACKFACE;
                    }
                    for (e, s) in soft.iter().enumerate() {
                        set_soft(&mut sm, e as u16, *s);
                    }
                    x.sm_group = sm;
                    x.add_channel(uvs[0], uvs[1], uvs[2]);
                    x.n = a.n;
                }
            };
            let sm = a.sm_group;
            if is_ccw(id1, id2) {
                let s12 = is_soft_edge(sm, id1 as u16);
                let s2b = is_soft_edge(sm, id2 as u16);
                let sb1 = is_soft_edge(sm, idb as u16);
                make([a.v[idb], a.v[id1], nv], [atc.uv[idb], atc.uv[id1], uv], [sb1, s12, true]);
                make([a.v[idb], nv, a.v[id2]], [atc.uv[idb], uv, atc.uv[id2]], [true, s12, s2b]);
            } else {
                let s12 = is_soft_edge(sm, id2 as u16);
                let s2b = is_soft_edge(sm, idb as u16);
                let sb1 = is_soft_edge(sm, id1 as u16);
                make([a.v[idb], nv, a.v[id1]], [atc.uv[idb], uv, atc.uv[id1]], [true, s12, sb1]);
                make([a.v[idb], a.v[id2], nv], [atc.uv[idb], atc.uv[id2], uv], [s2b, s12, true]);
            }
        }
        g.mesh.normal_from_adj(nv);
    }
    let faces = g.mesh.faces().to_vec();
    for f in faces {
        if g.mesh.face(f).is_some_and(|x| x.flags.splitted) {
            g.mesh.destroy_face(f, false);
        }
    }
    let verts = g.mesh.vertices().to_vec();
    for v in verts {
        if g.mesh.vertex(v).is_some_and(|x| x.adjacents.is_empty()) {
            g.mesh.destroy_vertex(v, false);
        }
    }
    compact_lists(g);
}

/// Smooths the vertex colours: each vertex takes the mean of the vertices of its faces.
fn smooth_vert_colors(g: &mut LcGlobalData, count: u32) {
    for _ in 0..count {
        let mesh = &g.mesh;
        let colors: Vec<BaseColor> = mesh
            .vertices()
            .par_iter()
            .map(|&v| {
                let mut circle: Vec<VertId> = Vec::new();
                if let Some(vx) = mesh.vertex(v) {
                    for &f in &vx.adjacents {
                        if let Some(face) = mesh.face(f) {
                            circle.extend_from_slice(&face.v);
                        }
                    }
                }
                // X-Ray's compiler sorts the circle by address
                circle.sort_unstable_by_key(|&c| mesh.vertex_addr(c));
                circle.dedup();
                let mut avg = ColorC::default();
                for c in &circle {
                    if let Some(x) = mesh.vertex(*c) {
                        avg.add(&x.c.get());
                    }
                }
                avg.scale(circle.len() as u32);
                BaseColor::from_c(&avg)
            })
            .collect();
        let verts = g.mesh.vertices().to_vec();
        for (v, c) in verts.into_iter().zip(colors) {
            if let Some(x) = g.mesh.vertex_mut(v) {
                x.c = c;
            }
        }
    }
}

/// The long edges tesselated, every vertex's hemisphere taken in
/// the level's own shadow at half strength, then smoothed five times.
pub fn adaptive_ht(
    g: &mut LcGlobalData,
    scene_bb: &gz_fmt_level::Aabb,
    log: &mut Log,
    ctl: &Ctl<'_>,
) -> Result<()> {
    log.status("Tesselating...".into());
    let faces = g.mesh.faces().to_vec();
    for f in faces {
        if let Some(x) = g.mesh.face_mut(f) {
            x.flags.splitted = false;
            x.flags.locked = true;
        }
        g.mesh.calc_normal(f);
    }
    tesselate(g, log);
    ctl.check()?;

    log.status("Precalculating...".into());
    // X-Ray's compiler builds the model and then caches the opacity it reads while tracing
    g.light_prepare();
    g.build_rapid(scene_bb)?;
    log.status("Precalculating : base hemisphere ...".into());
    let verts = g.mesh.vertices().to_vec();
    for &v in &verts {
        g.mesh.normal_from_adj(v);
    }
    let lit: Vec<BaseColor> = {
        let model = g.ray.as_ref().ok_or_else(|| Error::Invalid("The ray model is missing.".into()))?;
        let tracer = Tracer {
            model,
            textures: &g.textures,
            falloff: gz_lc_light::Falloff::Level { linear: g.linear },
        };
        let mesh = &g.mesh;
        let lights = &g.lights;
        verts
            .par_iter()
            .map_init(LightCache::default, |cache, &v| {
                let mut c = ColorC::default();
                if let Some(x) = mesh.vertex(v) {
                    light_point(&tracer, cache, &mut c, x.p, x.n, lights, LP_DONT_RGB + LP_DONT_SUN, None);
                }
                c.mul(0.5);
                BaseColor::from_c(&c)
            })
            .collect()
    };
    for (v, c) in verts.into_iter().zip(lit) {
        if let Some(x) = g.mesh.vertex_mut(v) {
            x.c = c;
        }
    }
    ctl.check()?;
    log.status("Gathering lighting information...".into());
    smooth_vert_colors(g, 5);
    Ok(())
}

/// One vertex per corner and texture coordinate through
/// MeshMender; every face gets its corners' tangent and binormal and the coordinates back.
pub fn tangent_basis(g: &mut LcGlobalData, log: &mut Log) {
    // X-Ray's compiler sorts the vertex list by address here and keeps that order
    sort_vertices_by_addr(g);
    let mesh = &g.mesh;
    let mut slot: Vec<u32> = vec![u32::MAX; mesh.vertices().last().map_or(0, |v| v.0 as usize + 1)];
    for (i, v) in mesh.vertices().iter().enumerate() {
        if let Some(s) = slot.get_mut(v.0 as usize) {
            *s = i as u32;
        }
    }
    let mut remap: Vec<Vec<u32>> = vec![Vec::new(); mesh.vertices().len()];
    let mut verts: Vec<MVertex> = Vec::with_capacity(mesh.vertices().len() * 4 / 3);
    let mut indices: Vec<u32> = Vec::with_capacity(mesh.faces().len() * 3);
    for &f in mesh.faces() {
        let Some(face) = mesh.face(f) else { continue };
        let tc = face.tc0();
        for (&v, &ftc) in face.v.iter().zip(tc.iter()) {
            let id = slot.get(v.0 as usize).copied().unwrap_or(u32::MAX) as usize;
            let m = remap.get(id).cloned().unwrap_or_default();
            let found = m.iter().copied().find(|&mi| {
                verts.get(mi as usize).is_some_and(|x| fsimilar(x.s, ftc.x) && fsimilar(x.t, ftc.y))
            });
            let index = match found {
                Some(i) => i,
                None => {
                    let vx = mesh.vertex(v);
                    verts.push(MVertex {
                        pos: vx.map_or([0.0; 3], |x| x.p.to_array()),
                        normal: vx.map_or([0.0; 3], |x| x.n.to_array()),
                        s: ftc.x,
                        t: ftc.y,
                        ..MVertex::default()
                    });
                    let i = (verts.len() - 1) as u32;
                    if let Some(r) = remap.get_mut(id) {
                        r.push(i);
                    }
                    i
                }
            };
            indices.push(index);
        }
    }
    let was = mesh.vertices().len();
    let become_ = verts.len();
    log.msg(format!(
        "duplication: was[{was}] / become[{become_}] - {:2.1}%",
        100.0 * (become_ as f32 - was as f32) / was as f32
    ));
    mend(&mut verts, &mut indices, 0.5, 0.5);
    let faces = g.mesh.faces().to_vec();
    for (f, t) in faces.into_iter().zip(indices.as_chunks::<3>().0) {
        let Some(face) = g.mesh.face_mut(f) else { continue };
        for (k, &corner) in t.iter().enumerate() {
            let Some(mv) = verts.get(corner as usize) else { continue };
            if let Some(tc) = face.tc.first_mut() {
                tc.uv[k] = Vec2::new(mv.s, mv.t);
            }
            face.basis_tangent[k] = Basis::of(Vec3::from_array(mv.tangent));
            face.basis_binormal[k] = Basis::of(Vec3::from_array(mv.binormal));
        }
    }
}

/// `fsimilar(a, b)`: xrCore's default bound, `EPS`.
fn fsimilar(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}
