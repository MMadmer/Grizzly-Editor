//! The subdivisions of the level (`g_XSplit`): `xrPhase_ResolveMaterials`, `Detach`,
//! `xrPhase_UVmap` (`xrPhase_UVmap.cpp`), `xrPhase_Subdivide`, `xrPhase_MergeGeometry`.

use glam::Vec3;
use gz_fmt_level::Aabb;
use gz_lc_light::math::EPS_S;
use gz_lc_light::{BORDER, Ctl, DeflId, FaceId, LMAP_SIZE, LcGlobalData, ShaderXrlc, VertId};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::{Log, Result};

/// `c_SS_LowVertLimit`.
const SS_LOW: usize = 64;
/// `c_SS_HighVertLimit`.
const SS_HIGH: usize = 2 * 1024;
/// `c_SS_maxsize`.
const SS_MAXSIZE: f32 = 32.0;

pub type Split = Vec<FaceId>;

/// `Detach`: the faces get vertices of their own, one copy per vertex they share.
pub fn detach(g: &mut LcGlobalData, faces: &[FaceId]) {
    let mut copies: FxHashMap<VertId, VertId> = FxHashMap::default();
    for &f in faces {
        for i in 0..3 {
            let Some(v) = g.mesh.face(f).map(|x| x.v[i]) else { continue };
            let vc = match copies.get(&v) {
                Some(&c) => c,
                None => {
                    let c = g.mesh.create_copy_noadj(v);
                    copies.insert(v, c);
                    c
                }
            };
            g.mesh.prep_add(vc, f);
            g.mesh.prep_remove(v, f);
            g.mesh.raw_set_vertex(f, i, vc);
        }
    }
}

fn shader_flags(g: &LcGlobalData, f: FaceId) -> u32 {
    g.mesh.face(f).map_or(0, |x| g.shader(x.material).flags)
}

/// `CBuild::xrPhase_ResolveMaterials`: one subdivision per material in the order the faces meet
/// them, the rendered faces only, the empty ones dropped, each detached.
pub fn resolve_materials(g: &mut LcGlobalData, log: &mut Log) -> Vec<Split> {
    let mut order: Vec<u16> = Vec::new();
    let mut slot: FxHashMap<u16, usize> = FxHashMap::default();
    for &f in g.mesh.faces() {
        let Some(m) = g.mesh.face(f).map(|x| x.material) else { continue };
        slot.entry(m).or_insert_with(|| {
            order.push(m);
            order.len() - 1
        });
    }
    let mut splits: Vec<Split> = vec![Vec::new(); order.len()];
    for &f in g.mesh.faces() {
        if shader_flags(g, f) & ShaderXrlc::RENDERING == 0 {
            continue;
        }
        let Some(m) = g.mesh.face(f).map(|x| x.material) else { continue };
        if let Some(&i) = slot.get(&m)
            && let Some(s) = splits.get_mut(i)
        {
            s.push(f);
        }
    }
    splits.retain(|s| !s.is_empty());
    for s in &splits {
        detach(g, s);
    }
    log.msg(format!("{} subdivisions.", splits.len()));
    splits
}

/// `CBuild::xrPhase_UVmap`: the lightmapped subdivisions cut into deflectors, each grown from
/// its largest face over the faces that fit its normal, each a subdivision of its own.
pub fn uv_map(g: &mut LcGlobalData, splits: &mut Vec<Split>, log: &mut Log, ctl: &Ctl<'_>) -> Result<()> {
    let mut sp = 0usize;
    while sp < splits.len() {
        ctl.check()?;
        let Some(&front) = splits[sp].first() else {
            sp += 1;
            continue;
        };
        let flags = shader_flags(g, front);
        if flags & ShaderXrlc::LIGHT_VERTEX != 0
            || flags & ShaderXrlc::RENDERING == 0
            || g.has_implicit_lighting(front)
        {
            sp += 1;
            continue;
        }
        // the faces' areas, taken once: nothing moves a corner while the subdivision is cut
        let mut areas: Vec<f32> = splits[sp].par_iter().map(|&f| g.mesh.calc_area(f)).collect();
        loop {
            let mut ms: Option<FaceId> = None;
            let mut ma = 0.0f32;
            for (&f, &a) in splits[sp].iter().zip(&areas) {
                if a > ma && g.mesh.face(f).is_some_and(|x| x.deflector.is_none()) {
                    ms = Some(f);
                    ma = a;
                }
            }
            let Some(start) = ms else {
                if splits[sp].is_empty() {
                    splits.remove(sp);
                    sp = sp.wrapping_sub(1);
                }
                break;
            };
            let id = g.deflectors.create();
            let n = g.mesh.face(start).map_or(Vec3::Y, |x| x.n);
            let sm_angle = g.params.sm_angle;
            if let Some(d) = g.deflectors.get_mut(id) {
                d.oa_set_normal(n);
                d.oa_unwarp(id, &mut g.mesh, start, sm_angle);
                d.oa_export(&mut g.mesh, &g.shaders, &g.materials, &g.params);
            }
            let mut affected = Vec::new();
            let mut kept_areas = Vec::with_capacity(areas.len());
            let mut k = 0usize;
            splits[sp].retain(|&f| {
                let mine = g.mesh.face(f).is_some_and(|x| x.deflector == Some(id));
                if mine {
                    affected.push(f);
                } else {
                    kept_areas.push(areas[k]);
                }
                k += 1;
                !mine
            });
            areas = kept_areas;
            detach(g, &affected);
            splits.push(affected);
        }
        sp = sp.wrapping_add(1);
    }
    log.msg(format!("{} subdivisions...", splits.len()));
    Ok(())
}

fn faces_box(g: &LcGlobalData, faces: &[FaceId]) -> Aabb {
    let mut bb = Aabb::INVALID;
    for &f in faces {
        if let Some(p) = g.mesh.positions(f) {
            for q in p {
                bb.modify(q);
            }
        }
    }
    bb
}

fn axis(v: Vec3, i: usize) -> f32 {
    match i {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

fn set_axis(v: &mut Vec3, i: usize, x: f32) {
    match i {
        0 => v.x = x,
        1 => v.y = x,
        _ => v.z = x,
    }
}

fn setup_bbs(bb: &Aabb, edge: usize) -> (Aabb, Aabb) {
    let (mut b1, mut b2) = (*bb, *bb);
    let size = bb.max - bb.min;
    let half = axis(size, edge) / 2.0;
    let (m1, m2) = (axis(b1.max, edge) - half, axis(b2.min, edge) + half);
    set_axis(&mut b1.max, edge, m1);
    set_axis(&mut b2.min, edge, m2);
    (b1, b2)
}

fn contains(b: &Aabb, p: Vec3) -> bool {
    p.x >= b.min.x && p.x <= b.max.x && p.y >= b.min.y && p.y <= b.max.y && p.z >= b.min.z && p.z <= b.max.z
}

/// `CBuild::xrPhase_Subdivide`: a subdivision larger than 32 m on an axis, of more than 2048
/// faces or with a deflector too large for a page is cut in two along its longest axis, its
/// deflector re-made for each half.
pub fn subdivide(g: &mut LcGlobalData, splits: &mut Vec<Split>, log: &mut Log, ctl: &Ctl<'_>) -> Result<()> {
    let mut x = 0usize;
    while x < splits.len() {
        ctl.check()?;
        if splits[x].is_empty() {
            splits.remove(x);
            continue;
        }
        if splits[x].len() < SS_LOW * 2 {
            x += 1;
            continue;
        }
        let bb = faces_box(g, &splits[x]);
        let mut size = bb.max - bb.min;
        let mut split =
            size.x > SS_MAXSIZE || size.y > SS_MAXSIZE || size.z > SS_MAXSIZE || splits[x].len() > SS_HIGH;
        let base: Option<DeflId> = splits[x].first().and_then(|&f| g.mesh.face(f)).and_then(|f| f.deflector);
        if !split
            && let Some(d) = base.and_then(|d| g.deflectors.get(d))
            && (d.layer.width >= LMAP_SIZE - 2 * BORDER || d.layer.height >= LMAP_SIZE - 2 * BORDER)
        {
            split = true;
        }
        if !split {
            x += 1;
            continue;
        }
        let mut edge = if size.x >= size.y && size.x >= size.z {
            0
        } else if size.y >= size.x && size.y >= size.z {
            1
        } else {
            2
        };
        let (mut b1, mut b2) = setup_bbs(&bb, edge);
        let mut on_edge = 0u32;
        let mut per_edge = 0u32;
        let (s1, s2) = loop {
            let mut s1 = Vec::new();
            let mut s2 = Vec::new();
            per_edge += 1;
            for &f in &splits[x] {
                if contains(&b1, g.mesh.calc_center(f)) {
                    s1.push(f);
                } else {
                    s2.push(f);
                }
            }
            if s1.len() >= SS_LOW && s2.len() >= SS_LOW {
                break Some((s1, s2));
            }
            log.msg(format!(
                "! ERROR: model #{x} - split fail, faces: {}, s1/s2:{}/{}",
                splits[x].len(),
                s1.len(),
                s2.len()
            ));
            if per_edge < 10 {
                if splits[x].len() > SS_LOW * 4 {
                    if s2.len() > s1.len() {
                        size = b2.max - b2.min;
                        let v = axis(b1.max, edge) + axis(size, edge) / 2.0;
                        set_axis(&mut b1.max, edge, v);
                        set_axis(&mut b2.min, edge, v);
                    } else {
                        size = b1.max - b1.min;
                        let v = axis(b2.min, edge) - axis(size, edge) / 2.0;
                        set_axis(&mut b2.min, edge, v);
                        set_axis(&mut b1.max, edge, v);
                    }
                    continue;
                }
                break None;
            }
            per_edge = 0;
            on_edge += 1;
            if on_edge < 3 {
                edge = (edge + 1) % 3;
                (b1, b2) = setup_bbs(&bb, edge);
                continue;
            }
            break None;
        }
        .unzip_pair();
        let (Some(s1), Some(s2)) = (s1, s2) else {
            x += 1;
            continue;
        };
        if let Some(d) = base {
            g.deflectors.destroy(d);
            for half in [&s1, &s2] {
                let id = g.deflectors.create();
                if let Some(nd) = g.deflectors.get_mut(id) {
                    nd.oa_place_all(id, &mut g.mesh, half);
                    nd.oa_export(&mut g.mesh, &g.shaders, &g.materials, &g.params);
                }
            }
        }
        splits.remove(x);
        splits.push(s1.clone());
        detach(g, &s1);
        splits.push(s2.clone());
        detach(g, &s2);
    }
    log.msg(format!("{} subdivisions.", splits.len()));
    Ok(())
}

trait UnzipPair<A, B> {
    fn unzip_pair(self) -> (Option<A>, Option<B>);
}

impl<A, B> UnzipPair<A, B> for Option<(A, B)> {
    fn unzip_pair(self) -> (Option<A>, Option<B>) {
        match self {
            Some((a, b)) => (Some(a), Some(b)),
            None => (None, None),
        }
    }
}

fn grow(b: &mut Aabb, s: f32) {
    b.min -= Vec3::splat(s);
    b.max += Vec3::splat(s);
}

/// `NeedMerge`: few faces, and smaller than 32 m on some axis; answers the grown box.
fn need_merge(g: &LcGlobalData, s: &[FaceId]) -> (bool, Aabb) {
    let mut bb = faces_box(g, s);
    grow(&mut bb, EPS_S);
    if s.len() >= 3 * SS_HIGH / 4 {
        return (false, bb);
    }
    let sz = bb.max - bb.min;
    (sz.x < SS_MAXSIZE || sz.y < SS_MAXSIZE || sz.z < SS_MAXSIZE, bb)
}

fn make_cube(b: &Aabb) -> Aabb {
    let d = (b.max - b.min) * 0.5;
    let c = b.min + d;
    let m = if d.y > d.x { d.y } else { d.x };
    let m = if d.z > m { d.z } else { m };
    let mut out = Aabb::new(c, c);
    grow(&mut out, m);
    out
}

fn volume(b: &Aabb) -> f32 {
    let s = b.max - b.min;
    s.x * s.y * s.z
}

/// `ValidateMergeLinearSize`; its `4*c_SS_maxsize/3` is integer arithmetic: 42 m.
fn linear_ok(merged: Vec3, o1: Vec3, o2: Vec3, i: usize) -> bool {
    let m = axis(merged, i);
    let limit = (4 * SS_MAXSIZE as u32 / 3) as f32;
    !(m > limit && m > axis(o1, i) + 1.0 && m > axis(o2, i) + 1.0)
}

/// `ValidateMerge`: the volume of the merged box, if the merge is allowed.
fn validate_merge(f1: usize, base: &Aabb, base_orig: &Aabb, f2: usize, bb: &Aabb) -> Option<f32> {
    if f1 + f2 > 4 * SS_HIGH / 3 {
        return None;
    }
    let mut merge = *base;
    merge.merge(bb);
    let sz = merge.max - merge.min;
    let o1 = base_orig.max - base_orig.min;
    let o2 = bb.max - bb.min;
    if !(0..3).all(|i| linear_ok(sz, o1, o2, i)) {
        return None;
    }
    let v1 = volume(&make_cube(base));
    let v2 = volume(&make_cube(bb));
    let v = volume(&merge);
    if v > 2.0 * 2.0 * 2.0 * (v1 + v2) {
        return None;
    }
    Some(v)
}

/// `CBuild::xrPhase_MergeGeometry`: small subdivisions take the candidate of the same material
/// and page whose merge gives the smallest box, until none is small any more; the smallest
/// volume wins, the first of equal ones as in the C++. `FaceEqual` of the first faces is a key
/// that merging never changes (the merged front is the taken one's, of the same key), so each
/// subdivision looks only at the later ones of its key; a taken one is marked gone, which keeps
/// the others' order.
pub fn merge_geometry(g: &LcGlobalData, splits: &mut Vec<Split>, log: &mut Log, ctl: &Ctl<'_>) -> Result<()> {
    // `FaceEqual`: the material, the number of texture channels, the lightmap page
    let key = |s: &Split| s.first().and_then(|&f| g.mesh.face(f)).map(|x| (x.material, x.tc.len(), x.lmap));
    let keys: Vec<_> = splits.iter().map(key).collect();
    let mut groups: FxHashMap<(u16, usize, Option<u32>), Vec<usize>> = FxHashMap::default();
    for (i, k) in keys.iter().enumerate() {
        if let Some(k) = k {
            groups.entry(*k).or_default().push(i);
        }
    }
    let mut needs: Vec<(bool, Aabb)> = splits.par_iter().map(|s| need_merge(g, s)).collect();
    let mut alive = vec![true; splits.len()];
    for split in 0..splits.len() {
        ctl.check()?;
        if !alive[split] {
            continue;
        }
        let Some(k) = keys[split] else { continue };
        let mut orig: Option<Aabb> = None;
        loop {
            let (need, base) = needs[split];
            if !need {
                break;
            }
            let base_orig = *orig.get_or_insert(base);
            let n1 = splits[split].len();
            let Some(group) = groups.get_mut(&k) else { break };
            group.retain(|&i| alive[i]);
            let from = group.partition_point(|&i| i <= split);
            let judge = |&i: &usize| {
                let (need, bb) = needs[i];
                if !need {
                    return None;
                }
                // the C++ starts from flt_max and takes only a smaller volume
                validate_merge(n1, &base, &base_orig, splits[i].len(), &bb)
                    .filter(|&v| v < f32::MAX)
                    .map(|v| (v, i))
            };
            let pick =
                |a: (f32, usize), b: (f32, usize)| if b.0 < a.0 || (b.0 == a.0 && b.1 < a.1) { b } else { a };
            let rest = &group[from..];
            let best = if rest.len() >= 256 {
                rest.par_iter().filter_map(judge).reduce_with(pick)
            } else {
                rest.iter().filter_map(judge).reduce(pick)
            };
            let Some((_, sel)) = best else { break };
            alive[sel] = false;
            let mut merged = std::mem::take(&mut splits[sel]);
            merged.extend_from_slice(&splits[split]);
            splits[split] = merged;
            needs[split] = need_merge(g, &splits[split]);
        }
    }
    let mut i = 0;
    splits.retain(|_| {
        i += 1;
        alive[i - 1]
    });
    log.msg(format!("{} subdivisions.", splits.len()));
    Ok(())
}
