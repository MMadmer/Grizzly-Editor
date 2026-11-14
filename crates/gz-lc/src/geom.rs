//! The geometry phases before the light: the pre-optimisation and the vertex isolation,
//! the T-junction correction, the adaptive hierarchical tesselation (its qslim variant is
//! commented out in X-Ray's compiler) and the tangent basis.

use std::time::Instant;

use glam::{Vec2, Vec3};
use gz_lc_light::math::{EPS, EPS_L, distance, distance_sqr, greedy_weld, i_floor, lerp};
use gz_lc_light::{
    BaseColor, Basis, ColorC, Ctl, FaceId, LP_DONT_RGB, LP_DONT_SUN, LcGlobalData, LightCache, Tracer,
    VertId, is_soft_edge, light_point,
};
use rayon::prelude::*;

use crate::mender::{MVertex, mend_until};
use crate::{Error, Log, Result};

/// How many items a sequential loop takes between two looks at the Stop.
pub(crate) const STOP_EVERY: usize = 4096;

/// A line of the phase's log with the seconds a step took since `t`, which starts the next one.
pub(crate) fn timed(log: &mut Log, what: &str, t: &mut Instant) {
    let s = t.elapsed().as_secs_f64();
    log.msg(format!("{what}: {s:.3} s"));
    *t = Instant::now();
}

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

/// The vertex list in the order of the vertices' addresses.
pub fn sort_vertices_by_addr(g: &mut LcGlobalData) {
    let mut list = g.mesh.vertices().to_vec();
    list.sort_unstable_by_key(|&v| g.mesh.vertex_addr(v));
    *g.mesh.vertices_mut() = list;
}

/// `PreOptimize`'s grid over the level's box: its corner, the cells per metre, the overlap.
fn weld_grid(bb: &gz_fmt_level::Aabb) -> (Vec3, Vec3, Vec3) {
    let vscale = bb.max - bb.min;
    let pick = |e: f32| if e < EPS_L { e } else { EPS_L };
    let eps = Vec3::new(
        pick(vscale.x / HDIM_X as f32 / 2.0),
        pick(vscale.y / HDIM_Y as f32 / 2.0),
        pick(vscale.z / HDIM_Z as f32 / 2.0),
    );
    let scale = Vec3::new(HDIM_X as f32 / vscale.x, HDIM_Y as f32 / vscale.y, HDIM_Z as f32 / vscale.z);
    (bb.min, scale, eps)
}

/// Vertices within the weld distance merged through an overlapping hash
/// grid, degenerate faces dropped, every other face verified, unused vertices dropped.
pub fn pre_optimize(
    g: &mut LcGlobalData,
    scene_bb: &gz_fmt_level::Aabb,
    log: &mut Log,
    no_invalid_check: bool,
    ctl: &Ctl<'_>,
) -> Result<()> {
    let (vmin, scale, eps) = weld_grid(scene_bb);
    let weld = g.params.weld_distance;
    let vcount = g.mesh.vertices().len();
    let fcount = g.mesh.faces().len();
    let mut tm = Instant::now();
    weld_vertices(g, weld, (vmin, scale, eps), log, ctl)?;
    timed(log, "the weld", &mut tm);

    let faces = g.mesh.faces().to_vec();
    for (n, f) in faces.into_iter().enumerate() {
        if n.is_multiple_of(STOP_EVERY) {
            ctl.check()?;
        }
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
    timed(log, "the faces checked", &mut tm);
    log.msg(format!(
        "{} vertices removed. ({} left)",
        vcount - g.mesh.vertices().len(),
        g.mesh.vertices().len()
    ));
    log.msg(format!("{} faces removed. ({} left)", fcount - g.mesh.faces().len(), g.mesh.faces().len()));
    Ok(())
}

/// `PreOptimize`'s weld: in list order, a vertex goes to the first vertex kept before it that lies
/// within the weld distance on every axis and whose cells of the grid (its own and the one `eps`
/// further on each axis) hold its own cell; it is kept when there is none, and the faces move to the
/// vertex they go to. The search runs on every core ([`greedy_weld`]), the moves in list order.
fn weld_vertices(
    g: &mut LcGlobalData,
    weld: f32,
    (vmin, scale, eps): (Vec3, Vec3, Vec3),
    log: &mut Log,
    ctl: &Ctl<'_>,
) -> Result<()> {
    let _ = &log;
    let cell = |p: Vec3| {
        [
            i_floor((p.x - vmin.x) * scale.x) as u32,
            i_floor((p.y - vmin.y) * scale.y) as u32,
            i_floor((p.z - vmin.z) * scale.z) as u32,
        ]
    };
    // the cell `eps` further, where X-Ray's compiler also puts a vertex it keeps
    let shifted = |p: Vec3| {
        [
            i_floor((p.x + eps.x - vmin.x) * scale.x) as u32,
            i_floor((p.y + eps.y - vmin.y) * scale.y) as u32,
            i_floor((p.z + eps.z - vmin.z) * scale.z) as u32,
        ]
    };
    let in_grid = |c: [u32; 3]| c[0] <= HDIM_X as u32 && c[1] <= HDIM_Y as u32 && c[2] <= HDIM_Z as u32;
    let outside = || Error::Invalid("A vertex lies outside the level's box (PreOptimize).".into());
    let list = g.mesh.vertices().to_vec();
    let alive: Vec<(VertId, Vec3)> =
        list.iter().filter_map(|&v| g.mesh.vertex(v).map(|x| (v, x.p))).collect();
    let mut seen = vec![false; list.iter().map(|v| v.0 as usize + 1).max().unwrap_or(0)];
    let listed_twice =
        alive.iter().any(|&(v, _)| seen.get_mut(v.0 as usize).is_some_and(|s| std::mem::replace(s, true)));
    if listed_twice {
        weld_in_order(g, &list, weld, (vmin, scale, eps), ctl)?;
    } else {
        // a vertex outside the box stops the phase, wherever it stands in the list
        if alive.par_iter().any(|&(_, p)| !in_grid(cell(p))) {
            return Err(outside());
        }
        // a vertex goes to the first kept one within the weld distance whose cells hold its own cell
        let points: Vec<Vec3> = alive.iter().map(|&(_, p)| p).collect();
        let to: Vec<u32> = if weld > 0.0 {
            let pred = |b: Vec3, v: Vec3| {
                if !((b.x - v.x).abs() < weld && (b.y - v.y).abs() < weld && (b.z - v.z).abs() < weld) {
                    return false;
                }
                let (c, a, e) = (cell(v), cell(b), shifted(b));
                (0..3).all(|k| c[k] == a[k] || c[k] == e[k])
            };
            let stop = || ctl.cancelled();
            let Some(to) = greedy_weld(&points, None, weld, pred, &stop) else {
                ctl.check()?;
                return Err(gz_lc_light::LcError::Cancelled.into());
            };
            to
        } else {
            (0..points.len() as u32).collect()
        };
        // a kept vertex whose further cell lies outside stops it too
        let kept_outside = to
            .par_iter()
            .enumerate()
            .any(|(i, &t)| t as usize == i && points.get(i).is_some_and(|&p| !in_grid(shifted(p))));
        if kept_outside {
            return Err(outside());
        }
        // the faces move to the vertices they are welded to in list order, as X-Ray's compiler moves them
        for (n, (&t, &(test, _))) in to.iter().zip(&alive).enumerate() {
            if n.is_multiple_of(STOP_EVERY) {
                ctl.check()?;
            }
            if t as usize == n {
                continue;
            }
            let Some(&(base, _)) = alive.get(t as usize) else { continue };
            while let Some(&f) = g.mesh.vertex(test).and_then(|x| x.adjacents.first()) {
                g.mesh.v_replace(f, test, base);
            }
            g.mesh.destroy_vertex(test, false);
        }
    }

    Ok(())
}

/// `PreOptimize`'s weld as X-Ray's compiler writes it, for a vertex list that names a vertex twice.
fn weld_in_order(
    g: &mut LcGlobalData,
    list: &[VertId],
    weld: f32,
    (vmin, scale, eps): (Vec3, Vec3, Vec3),
    ctl: &Ctl<'_>,
) -> Result<()> {
    let cells = ((HDIM_X + 1) * (HDIM_Y + 1) * (HDIM_Z + 1)) as usize;
    let mut hash: Vec<Vec<VertId>> = vec![Vec::new(); cells];
    let idx = |x: u32, y: u32, z: u32| {
        ((x as usize * (HDIM_Y as usize + 1)) + y as usize) * (HDIM_Z as usize + 1) + z as usize
    };
    for (n, &test) in list.iter().enumerate() {
        if n.is_multiple_of(STOP_EVERY) {
            ctl.check()?;
        }
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

/// The junctions and the long edges are counted, nothing changes: a junction is a vertex at the
/// end of an edge of a face around a corner of another face's edge, lying within 2 mm of that edge;
/// a long edge is an edge of 32 m or more of a face around the corner of a face. Each face is looked
/// at on its own thread and every vertex and edge counted once.
pub fn correct_tjunctions(g: &LcGlobalData, log: &mut Log, ctl: &Ctl<'_>) -> Result<()> {
    let mut tm = Instant::now();
    let mesh = &g.mesh;
    let pos = |v: VertId| mesh.vertex(v).map_or(Vec3::ZERO, |x| x.p);
    let junctions: Vec<Vec<VertId>> = mesh
        .faces()
        .par_iter()
        .map(|&f| {
            let mut js = Vec::new();
            if ctl.cancelled() {
                return js;
            }
            let Some(face) = mesh.face(f) else { return js };
            for e in 0..3 {
                let (va, vb) = face.edge_verts(e);
                for (x, y) in [(va, vb), (vb, va)] {
                    let Some(vx) = mesh.vertex(x) else { continue };
                    for &f1 in &vx.adjacents {
                        let Some(face1) = mesh.face(f1) else { continue };
                        for e1 in 0..3 {
                            let (v1, v2) = face1.edge_verts(e1);
                            let t = if v1 == x && v2 != y {
                                v2
                            } else if v2 == x && v1 != y {
                                v1
                            } else {
                                continue;
                            };
                            if sqr_distance_to_segment(pos(t), pos(va), pos(vb)).sqrt() < 0.002 {
                                js.push(t);
                            }
                        }
                    }
                }
            }
            js
        })
        .collect();
    ctl.check()?;
    let mut junctions: Vec<VertId> = junctions.into_iter().flatten().collect();
    junctions.par_sort_unstable();
    junctions.dedup();
    // the faces around the corners of the listed ones, each once
    let mut around = vec![false; mesh.faces().iter().map(|f| f.0 as usize + 1).max().unwrap_or(0)];
    for &f in mesh.faces() {
        let Some(face) = mesh.face(f) else { continue };
        for v in face.v {
            for &f1 in mesh.vertex(v).map_or(&[][..], |x| &x.adjacents) {
                if mesh.face(f1).is_some() {
                    match around.get_mut(f1.0 as usize) {
                        Some(slot) => *slot = true,
                        None => {
                            around.resize(f1.0 as usize + 1, false);
                            if let Some(slot) = around.get_mut(f1.0 as usize) {
                                *slot = true;
                            }
                        }
                    }
                }
            }
        }
    }
    let mut edges: Vec<(VertId, VertId)> = around
        .par_iter()
        .enumerate()
        .filter(|&(_, &a)| a)
        .flat_map_iter(|(i, _)| {
            let mut out = Vec::new();
            if let Some(face1) = mesh.face(FaceId(i as u32)) {
                for e1 in 0..3 {
                    let (v1, v2) = face1.edge_verts(e1);
                    if distance(pos(v1), pos(v2)) >= 32.0 {
                        out.push(if v1 > v2 { (v2, v1) } else { (v1, v2) });
                    }
                }
            }
            out.into_iter()
        })
        .collect();
    edges.par_sort_unstable();
    edges.dedup();
    timed(log, "the search", &mut tm);
    let line = format!("*** {} junctions and {} long edges found.", junctions.len(), edges.len());
    if junctions.is_empty() && edges.is_empty() {
        log.msg(line)
    } else {
        log.warn(line)
    }
    Ok(())
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
fn tesselate(g: &mut LcGlobalData, log: &mut Log, ctl: &Ctl<'_>) -> Result<()> {
    let mut counter = 0u32;
    let mut i = 0usize;
    while i < g.mesh.faces().len() {
        if i.is_multiple_of(STOP_EVERY) {
            ctl.check()?;
        }
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
            log.status(format!("{counter} vertices made, {} in the mesh", g.mesh.vertices().len()));
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
    Ok(())
}

/// Smooths the vertex colours: each vertex takes the mean of the vertices of its faces.
fn smooth_vert_colors(g: &mut LcGlobalData, count: u32, ctl: &Ctl<'_>) -> Result<()> {
    for _ in 0..count {
        let mesh = &g.mesh;
        let colors: Vec<BaseColor> = mesh
            .vertices()
            .par_iter()
            .map(|&v| {
                if ctl.cancelled() {
                    return BaseColor::default();
                }
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
        ctl.check()?;
        let verts = g.mesh.vertices().to_vec();
        for (v, c) in verts.into_iter().zip(colors) {
            if let Some(x) = g.mesh.vertex_mut(v) {
                x.c = c;
            }
        }
    }
    Ok(())
}

/// The long edges tesselated, every vertex's hemisphere taken in
/// the level's own shadow at half strength, then smoothed five times.
pub fn adaptive_ht(
    g: &mut LcGlobalData,
    scene_bb: &gz_fmt_level::Aabb,
    log: &mut Log,
    ctl: &Ctl<'_>,
) -> Result<()> {
    log.status("Tessellating the long edges".into());
    let mut tm = Instant::now();
    let faces = g.mesh.faces().to_vec();
    for f in faces {
        if let Some(x) = g.mesh.face_mut(f) {
            x.flags.splitted = false;
            x.flags.locked = true;
        }
        g.mesh.calc_normal(f);
    }
    tesselate(g, log, ctl)?;
    ctl.check()?;
    timed(log, "the long edges", &mut tm);

    log.status("Building the ray model".into());
    // X-Ray's compiler builds the model and then caches the opacity it reads while tracing
    g.light_prepare();
    g.build_rapid_with(scene_bb, ctl)?;
    timed(log, "the ray model", &mut tm);
    log.status("Lighting the vertices by the sky".into());
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
                if ctl.cancelled() {
                    return BaseColor::default();
                }
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
    timed(log, "the sky light", &mut tm);
    log.status("Smoothing the vertex light".into());
    smooth_vert_colors(g, 5, ctl)?;
    timed(log, "the smoothing", &mut tm);
    Ok(())
}

/// One vertex per corner and texture coordinate through
/// MeshMender; every face gets its corners' tangent and binormal and the coordinates back.
pub fn tangent_basis(g: &mut LcGlobalData, log: &mut Log, ctl: &Ctl<'_>) -> Result<()> {
    let mut tm = Instant::now();
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
    for (n, &f) in mesh.faces().iter().enumerate() {
        if n.is_multiple_of(STOP_EVERY) {
            ctl.check()?;
        }
        let Some(face) = mesh.face(f) else { continue };
        let tc = face.tc0();
        for (&v, &ftc) in face.v.iter().zip(tc.iter()) {
            let id = slot.get(v.0 as usize).copied().unwrap_or(u32::MAX) as usize;
            let found = remap.get(id).and_then(|m| {
                m.iter().copied().find(|&mi| {
                    verts.get(mi as usize).is_some_and(|x| fsimilar(x.s, ftc.x) && fsimilar(x.t, ftc.y))
                })
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
    timed(log, "the corners", &mut tm);
    let stop = || ctl.cancelled();
    if mend_until(&mut verts, &mut indices, 0.5, 0.5, &stop).is_none() {
        ctl.check()?;
    }
    timed(log, "the mend", &mut tm);
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
    timed(log, "the bases", &mut tm);
    Ok(())
}

/// `fsimilar(a, b)`: xrCore's default bound, `EPS`.
fn fsimilar(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}

#[cfg(test)]
mod tests {
    use gz_lc_light::{BuildParams, LightSet, Shaders};

    use super::*;

    fn rng(seed: u64) -> impl FnMut() -> u64 {
        let mut s = seed;
        move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        }
    }

    /// Points on a lattice a weld distance apart, nudged and repeated, in faces that share them.
    fn scene(seed: u64) -> (LcGlobalData, gz_fmt_level::Aabb) {
        let mut next = rng(seed);
        let params = BuildParams { weld_distance: 0.005, ..BuildParams::default() };
        let mut g =
            LcGlobalData::new(params, Shaders::new(Vec::new()), Vec::new(), Vec::new(), LightSet::default());
        // clusters a few millimetres wide, many of them on the borders of the weld grid's cells
        let (cx, cz) = (41.0 / HDIM_X as f32, 41.0 / HDIM_Z as f32);
        let centres: Vec<Vec3> = (0..400)
            .map(|i| {
                let (kx, kz) = ((next() % 48 + 1) as f32, (next() % 48 + 1) as f32);
                let on_border = i % 2 == 0;
                let off =
                    |n: &mut dyn FnMut() -> u64| if on_border { 0.0 } else { (n() % 1000) as f32 * 0.0005 };
                Vec3::new(
                    -1.0 + kx * cx + off(&mut next),
                    (next() % 10) as f32 * 0.05,
                    -1.0 + kz * cz + off(&mut next),
                )
            })
            .collect();
        let mut ids = Vec::new();
        for _ in 0..6000 {
            let c = centres[(next() % centres.len() as u64) as usize];
            let mut j = || ((next() >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.012;
            let p = c + Vec3::new(j(), j(), j());
            ids.push(g.mesh.create_vertex(p));
        }
        for _ in 0..8000 {
            let mut pick = || ids[(next() % ids.len() as u64) as usize];
            let v = [pick(), pick(), pick()];
            if v[0] != v[1] && v[1] != v[2] && v[0] != v[2] {
                g.mesh.create_face(v);
            }
        }
        let bb = gz_fmt_level::Aabb { min: Vec3::splat(-1.0), max: Vec3::new(40.0, 1.0, 40.0) };
        (g, bb)
    }

    /// Every vertex's faces and every face's vertices, by id.
    type MeshState = (Vec<Option<Vec<FaceId>>>, Vec<Option<[VertId; 3]>>);

    fn state(g: &LcGlobalData) -> MeshState {
        let verts = (0..6000).map(|i| g.mesh.vertex(VertId(i)).map(|x| x.adjacents.clone())).collect();
        let faces = (0..8000).map(|i| g.mesh.face(FaceId(i)).map(|x| x.v)).collect();
        (verts, faces)
    }

    /// The weld moves every face to the vertex the loop of X-Ray's compiler moved it to, in the same
    /// order (the adjacency lists keep it), on any number of threads.
    #[test]
    fn the_weld_is_the_loops() {
        for seed in [1, 0x2545_F491_4F6C_DD1D] {
            let (g, bb) = scene(seed);
            let grid = weld_grid(&bb);
            let weld = g.params.weld_distance;
            let mut old = g.clone();
            let list = old.mesh.vertices().to_vec();
            weld_in_order(&mut old, &list, weld, grid, &Ctl::quiet()).unwrap();
            let mut new = g.clone();
            weld_vertices(&mut new, weld, grid, &mut Log::default(), &Ctl::quiet()).unwrap();
            assert!(state(&new) == state(&old), "seed {seed}");
            let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
            let mut one = g.clone();
            pool.install(|| weld_vertices(&mut one, weld, grid, &mut Log::default(), &Ctl::quiet())).unwrap();
            assert!(state(&one) == state(&old));
            let gone = (0..6000).filter(|&i| old.mesh.vertex(VertId(i)).is_none()).count();
            assert!(gone > 1000, "seed {seed}: {gone} welded, the test proves little");
            // a vertex outside the box stops both
            let mut far = g.clone();
            if let Some(v) = far.mesh.vertex_mut(VertId(77)) {
                v.p.x = 99.0;
            }
            let mut far2 = far.clone();
            assert!(weld_in_order(&mut far, &list, weld, grid, &Ctl::quiet()).is_err());
            assert!(weld_vertices(&mut far2, weld, grid, &mut Log::default(), &Ctl::quiet()).is_err());
        }
    }

    /// `CorrectTJunctions` as it was: every candidate of every face, the sets made at the end.
    fn tjunctions_reference(g: &LcGlobalData) -> String {
        let mesh = &g.mesh;
        let pos = |v: VertId| mesh.vertex(v).map_or(Vec3::ZERO, |x| x.p);
        let mut junctions: Vec<VertId> = Vec::new();
        let mut seen_t = std::collections::HashSet::new();
        let mut edges = std::collections::HashSet::new();
        for &f in mesh.faces() {
            let Some(face) = mesh.face(f) else { continue };
            for e in 0..3 {
                let (va, vb) = face.edge_verts(e);
                for (x, y, flip) in [(va, vb, false), (vb, va, true)] {
                    let Some(vx) = mesh.vertex(x) else { continue };
                    for &f1 in &vx.adjacents {
                        let Some(face1) = mesh.face(f1) else { continue };
                        for e1 in 0..3 {
                            let (v1, v2) = face1.edge_verts(e1);
                            if distance(pos(v1), pos(v2)) >= 32.0 {
                                edges.insert(if v1 > v2 { (v2, v1) } else { (v1, v2) });
                            }
                            let (ea, eb) = if flip { (y, x) } else { (x, y) };
                            let t = if v1 == x && v2 != y {
                                v2
                            } else if v2 == x && v1 != y {
                                v1
                            } else {
                                continue;
                            };
                            if sqr_distance_to_segment(pos(t), pos(ea), pos(eb)).sqrt() < 0.002
                                && seen_t.insert(t)
                            {
                                junctions.push(t);
                            }
                        }
                    }
                }
            }
        }
        format!("*** {} junctions and {} long edges found.", junctions.len(), edges.len())
    }

    #[test]
    fn the_junctions_and_long_edges_are_counted_as_before() {
        let (mut g, _) = scene(9);
        // long faces, and vertices on their edges
        let a = g.mesh.create_vertex(Vec3::new(0.0, 0.0, 0.0));
        let b = g.mesh.create_vertex(Vec3::new(40.0, 0.0, 0.0));
        let c = g.mesh.create_vertex(Vec3::new(0.0, 0.0, 40.0));
        let m = g.mesh.create_vertex(Vec3::new(20.0, 0.0005, 0.0));
        let n = g.mesh.create_vertex(Vec3::new(0.0, 0.0, 33.0));
        g.mesh.create_face([a, b, c]);
        g.mesh.create_face([a, m, n]);
        g.mesh.create_face([m, b, n]);
        let mut log = Log::default();
        correct_tjunctions(&g, &mut log, &Ctl::quiet()).unwrap();
        let line = tjunctions_reference(&g);
        assert!(log.lines.iter().any(|l| l.ends_with(&line)), "{line} not in {:?}", log.lines);
        assert!(!line.starts_with("*** 0 junctions") && !line.ends_with(" 0 long edges found."), "{line}");
    }

    /// `trace` with the scalar tree of `gz_fmt_level`: what it was before the four-wide tree.
    #[allow(clippy::too_many_arguments)]
    fn scalar_trace(
        model: &gz_lc_light::RayModel,
        bvh: &gz_fmt_level::Bvh,
        textures: &[gz_lc_light::BuildTexture],
        scratch: &mut Vec<gz_fmt_level::RayHit>,
        cached: &mut u32,
        p: Vec3,
        d: Vec3,
        range: f32,
        skip: Option<FaceId>,
    ) -> f32 {
        use gz_lc_light::math::color_get_a;
        use gz_lc_light::{NO_TRI, Opacity, RayFace};
        use std::ops::ControlFlow;
        let ray = gz_fmt_level::Ray { origin: p, dir: d, t_max: range, cull_back: model.cull() };
        let skipped = |f: &RayFace| skip.is_some() && f.face == skip;
        if let (Some(tri), Some(face)) = (model.tri(*cached), model.face(*cached))
            && !skipped(face)
            && let Some((t, _, _)) = gz_fmt_level::ray_triangle(&ray, tri)
            && t > 0.0
            && t < range
        {
            return 0.0;
        }
        scratch.clear();
        let mut stopped = NO_TRI;
        bvh.ray_visit(&ray, |hit| {
            let Some(face) = model.face(hit.tri) else { return ControlFlow::Continue(()) };
            if skipped(face) || !face.cast_shadow {
                return ControlFlow::Continue(());
            }
            if face.opaque {
                stopped = hit.tri;
                return ControlFlow::Break(());
            }
            scratch.push(hit);
            ControlFlow::Continue(())
        });
        if stopped != NO_TRI {
            *cached = stopped;
            return 0.0;
        }
        scratch.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.tri.cmp(&b.tri)));
        let mut scale = 1.0f32;
        for hit in scratch.iter() {
            let Some(face) = model.face(hit.tri) else { continue };
            let o = textures.get(face.texture as usize).and_then(|t| {
                let surface = t.surface.as_ref()?;
                let (w, h) = (t.width, t.height);
                if w == 0 || h == 0 {
                    return None;
                }
                let b = Vec3::new(1.0 - hit.u - hit.v, hit.u, hit.v);
                let c = &face.uv;
                let uv = Vec2::new(
                    c[0].x * b.x + c[1].x * b.y + c[2].x * b.z,
                    c[0].y * b.x + c[1].y * b.y + c[2].y * b.z,
                );
                let u = (i_floor(uv.x * w as f32 + 0.5) as u32) % w;
                let v = (i_floor(uv.y * h as f32 + 0.5) as u32) % h;
                let a = color_get_a(*surface.get(v as usize * w as usize + u as usize)?) as f32 / 255.0;
                Some(match model.opacity() {
                    Opacity::Squared => 1.0 - a * a,
                    Opacity::Linear => 1.0 - a,
                })
            });
            match o {
                Some(o) => scale *= o,
                None => {
                    *cached = hit.tri;
                    return 0.0;
                }
            }
        }
        scale
    }

    /// The two tracers on a real level, taking turns in one process so that what else runs on the
    /// machine weighs on both alike: the rays `light_point` traces from the level's vertices (the
    /// hemisphere, the sun, the static lights), a cache per chunk of 256 points. Not a check: set
    /// `GZ_AB_LEVEL` (a folder with `build.prj`) and `GZ_AB_ROOT` (the data root) and run with
    /// `--ignored --nocapture` on a release build.
    #[test]
    #[ignore]
    fn the_tracers_side_by_side_on_a_level() {
        use gz_lc_light::math::{mad, normalize_safe};
        use gz_lc_light::{LT_DIRECT, LT_POINT, LT_SECONDARY, NO_TRI, RLight, TextureUse};
        let (Ok(level), Ok(root)) = (std::env::var("GZ_AB_LEVEL"), std::env::var("GZ_AB_ROOT")) else {
            return;
        };
        let prj = gz_fmt_level::BuildProject::read(
            &std::fs::read(std::path::Path::new(&level).join("build.prj")).unwrap(),
        )
        .unwrap();
        let sources = crate::Sources::of_root(std::path::Path::new(&root));
        let shaders = sources.shaders().unwrap();
        let textures = sources.textures(&prj.textures, TextureUse::Level { radiosity: false }).unwrap();
        let g =
            crate::light_scene(&prj, shaders, textures, &crate::Options::default(), &Ctl::quiet()).unwrap();
        let model = g.ray.as_ref().unwrap();
        let flat: Vec<Vec3> = model.tris().iter().flatten().copied().collect();
        let idx: Vec<[u32; 3]> = (0..model.len() as u32).map(|i| [3 * i, 3 * i + 1, 3 * i + 2]).collect();
        let scalar = gz_fmt_level::Bvh::build(&flat, &idx).unwrap();
        let verts = g.mesh.vertices();
        let stride = (verts.len() / 100_000).max(1);
        let points: Vec<(Vec3, Vec3)> = verts
            .iter()
            .step_by(stride)
            .filter_map(|&v| g.mesh.vertex(v))
            .filter(|x| x.n.length_squared() > 0.25)
            .map(|x| (x.p, x.n.normalize()))
            .collect();
        let towards = |l: &RLight, p: Vec3, n: Vec3| {
            let sq_d = gz_lc_light::math::distance_sqr(p, l.position);
            if sq_d > l.range2 {
                return None;
            }
            let ldir = normalize_safe(l.position - p);
            let d = gz_lc_light::math::dot(ldir, n);
            (d > 0.0).then_some((sq_d.sqrt(), ldir, d))
        };
        // per point, the rays in the order light_point traces them, with the light's cache slot
        let rays_of = |p: Vec3, n: Vec3, out: &mut Vec<(usize, Vec3, Vec3, f32)>| {
            out.clear();
            let pnew = mad(p, n, 0.01);
            let lights = &g.lights;
            for (i, l) in lights.rgb.iter().enumerate() {
                match l.kind {
                    LT_DIRECT => {
                        let ldir = -l.direction;
                        if gz_lc_light::math::dot(ldir, n) > 0.0 {
                            out.push((i, pnew, ldir, 1000.0));
                        }
                    }
                    LT_POINT => {
                        if let Some((r, ldir, _)) = towards(l, p, n) {
                            out.push((i, pnew, ldir, r));
                        }
                    }
                    LT_SECONDARY => {
                        if let Some((r, ldir, d)) = towards(l, p, n)
                            && d * -gz_lc_light::math::dot(ldir, l.direction) > 0.0
                        {
                            out.push((i, pnew, ldir, r));
                        }
                    }
                    _ => {}
                }
            }
            let base = lights.rgb.len();
            for (i, l) in lights.sun.iter().enumerate() {
                if l.kind == LT_DIRECT {
                    let ldir = -l.direction;
                    if gz_lc_light::math::dot(ldir, n) > 0.0 {
                        out.push((base + i, pnew, ldir, 1000.0));
                    }
                } else if let Some((r, ldir, _)) = towards(l, p, n) {
                    out.push((base + i, pnew, ldir, r));
                }
            }
            let base = base + lights.sun.len();
            for (i, l) in lights.hemi.iter().enumerate() {
                if l.kind == LT_DIRECT {
                    let ldir = -l.direction;
                    if gz_lc_light::math::dot(ldir, n) > 0.0 {
                        out.push((base + i, mad(pnew, ldir, 0.001), ldir, 1000.0));
                    }
                } else if let Some((r, ldir, _)) = towards(l, p, n) {
                    out.push((base + i, pnew, ldir, r));
                }
            }
        };
        let slots = g.lights.rgb.len() + g.lights.sun.len() + g.lights.hemi.len();
        // one chunk lit by one tracer: the seconds it took, the light it gave, the rays it traced
        let chunk_with = |chunk: &[(Vec3, Vec3)], old: bool| -> (f64, f64, usize) {
            let t = Instant::now();
            let mut hits = Vec::new();
            let mut cache = vec![NO_TRI; slots];
            let mut rays = Vec::new();
            let (mut s, mut c) = (0.0f64, 0usize);
            for &(p, n) in chunk {
                rays_of(p, n, &mut rays);
                for &(slot, from, d, r) in &rays {
                    let Some(cached) = cache.get_mut(slot) else { continue };
                    let v = if old {
                        scalar_trace(model, &scalar, &g.textures, &mut hits, cached, from, d, r, None)
                    } else {
                        model.trace(&g.textures, &mut hits, cached, from, d, r, None)
                    };
                    s += f64::from(v);
                    c += 1;
                }
            }
            (t.elapsed().as_secs_f64(), s, c)
        };
        // the two take turns chunk by chunk, so that what else the machine runs weighs on both alike
        let (old_t, new_t, rays): (f64, f64, usize) = points
            .par_chunks(256)
            .enumerate()
            .map(|(i, chunk)| {
                let (first, second) = (i % 2 == 0, i % 2 != 0);
                let a = chunk_with(chunk, first);
                let b = chunk_with(chunk, second);
                assert_eq!(a.1.to_bits(), b.1.to_bits(), "the light differs in chunk {i}");
                let (old, new) = if first { (a, b) } else { (b, a) };
                (old.0, new.0, old.2)
            })
            .reduce(|| (0.0, 0.0, 0), |x, y| (x.0 + y.0, x.1 + y.1, x.2 + y.2));
        eprintln!(
            "{} points, {} triangles: scalar {:.0} rays/s, four-wide {:.0} rays/s (a thread), {:.2} times",
            points.len(),
            model.len(),
            rays as f64 / old_t,
            rays as f64 / new_t,
            old_t / new_t
        );
    }
}
