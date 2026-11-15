//! The subdivisions of the level: resolving the materials, detaching, the UV mapping,
//! the subdividing and the geometry merging.

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

/// The faces get vertices of their own, one copy per vertex they share.
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

/// One subdivision per material in the order the faces meet
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

/// The lightmapped subdivisions cut into deflectors, each grown from
/// its largest face without a deflector over the faces that fit its normal, each a subdivision of
/// its own. The subdivisions that are not lightmapped keep their places; the deflectors' follow
/// them in the order they were made. The faces are taken by area once (the largest first, the
/// first of equal ones), not searched again per deflector.
pub fn uv_map(g: &mut LcGlobalData, splits: &mut Vec<Split>, log: &mut Log, ctl: &Ctl<'_>) -> Result<()> {
    let mut kept: Vec<Split> = Vec::with_capacity(splits.len());
    let mut made: Vec<Split> = Vec::new();
    for split in std::mem::take(splits) {
        ctl.check()?;
        let Some(&front) = split.first() else {
            kept.push(split);
            continue;
        };
        let flags = shader_flags(g, front);
        if flags & ShaderXrlc::LIGHT_VERTEX != 0
            || flags & ShaderXrlc::RENDERING == 0
            || g.has_implicit_lighting(front)
        {
            kept.push(split);
            continue;
        }
        // the faces' areas, taken once: nothing moves a corner while the subdivision is cut
        let areas: Vec<f32> = split.par_iter().map(|&f| g.mesh.calc_area(f)).collect();
        let mut by_area: Vec<usize> = (0..split.len()).filter(|&i| areas[i] > 0.0).collect();
        by_area.sort_by(|&a, &b| areas[b].total_cmp(&areas[a]).then(a.cmp(&b)));
        let mut ids: Vec<DeflId> = Vec::new();
        for i in by_area {
            let start = split[i];
            if g.mesh.face(start).is_none_or(|x| x.deflector.is_some()) {
                continue;
            }
            if ids.len().is_multiple_of(256) {
                ctl.check()?;
            }
            let id = g.deflectors.create();
            let n = g.mesh.face(start).map_or(Vec3::Y, |x| x.n);
            let sm_angle = g.params.sm_angle;
            if let Some(d) = g.deflectors.get_mut(id) {
                d.oa_set_normal(n);
                d.oa_unwarp(id, &mut g.mesh, start, sm_angle);
                d.oa_export(&mut g.mesh, &g.shaders, &g.materials, &g.params);
            }
            ids.push(id);
        }
        // every deflector's faces in the subdivision's order; the faces none took stay behind
        let slot: FxHashMap<DeflId, usize> = ids.iter().enumerate().map(|(i, &d)| (d, i)).collect();
        let mut lists: Vec<Split> = vec![Vec::new(); ids.len()];
        let mut rest = Vec::new();
        for &f in &split {
            match g.mesh.face(f).and_then(|x| x.deflector).and_then(|d| slot.get(&d)) {
                Some(&i) => lists[i].push(f),
                None => rest.push(f),
            }
        }
        for l in &lists {
            detach(g, l);
        }
        if !rest.is_empty() {
            kept.push(rest);
        }
        made.extend(lists);
    }
    kept.extend(made);
    *splits = kept;
    log.msg(format!("{} subdivisions.", splits.len()));
    Ok(())
}

pub(crate) fn faces_box(g: &LcGlobalData, faces: &[FaceId]) -> Aabb {
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

/// A subdivision larger than 32 m on an axis, of more than 2048
/// faces or with a deflector too large for a page is cut in two along its longest axis, its
/// deflector re-made for each half. The halves join the end of the queue and are judged in
/// their turn; a subdivision that stays is the next of the result (X-Ray's order, without its
/// list's erasures).
pub fn subdivide(g: &mut LcGlobalData, splits: &mut Vec<Split>, log: &mut Log, ctl: &Ctl<'_>) -> Result<()> {
    let mut queue: std::collections::VecDeque<Split> = std::mem::take(splits).into();
    let mut out: Vec<Split> = Vec::new();
    while let Some(cur) = queue.pop_front() {
        ctl.check()?;
        if cur.is_empty() {
            continue;
        }
        if cur.len() < SS_LOW * 2 {
            out.push(cur);
            continue;
        }
        // the index the subdivision has in X-Ray's list: the ones kept before it
        let x = out.len();
        let bb = faces_box(g, &cur);
        let mut size = bb.max - bb.min;
        let mut split =
            size.x > SS_MAXSIZE || size.y > SS_MAXSIZE || size.z > SS_MAXSIZE || cur.len() > SS_HIGH;
        let base: Option<DeflId> = cur.first().and_then(|&f| g.mesh.face(f)).and_then(|f| f.deflector);
        if !split
            && let Some(d) = base.and_then(|d| g.deflectors.get(d))
            && (d.layer.width >= LMAP_SIZE - 2 * BORDER || d.layer.height >= LMAP_SIZE - 2 * BORDER)
        {
            split = true;
        }
        if !split {
            out.push(cur);
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
        let centers: Vec<Vec3> = cur.iter().map(|&f| g.mesh.calc_center(f)).collect();
        let (s1, s2) = loop {
            let mut s1 = Vec::new();
            let mut s2 = Vec::new();
            per_edge += 1;
            for (&f, &c) in cur.iter().zip(&centers) {
                if contains(&b1, c) {
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
                cur.len(),
                s1.len(),
                s2.len()
            ));
            if per_edge < 10 {
                if cur.len() > SS_LOW * 4 {
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
            out.push(cur);
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
        detach(g, &s1);
        detach(g, &s2);
        queue.push_back(s1);
        queue.push_back(s2);
    }
    *splits = out;
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

/// Few faces, and smaller than 32 m on some axis; answers the grown box.
fn need_merge(g: &LcGlobalData, s: &[FaceId]) -> (bool, Aabb) {
    let mut bb = faces_box(g, s);
    grow(&mut bb, EPS_S);
    (need_of(s.len(), &bb), bb)
}

/// Whether a subdivision of `faces` faces in the grown box `bb` wants a merge.
fn need_of(faces: usize, bb: &Aabb) -> bool {
    if faces >= 3 * SS_HIGH / 4 {
        return false;
    }
    let sz = bb.max - bb.min;
    sz.x < SS_MAXSIZE || sz.y < SS_MAXSIZE || sz.z < SS_MAXSIZE
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

/// Whether a merge is valid by linear size; its `4*c_SS_maxsize/3` is integer arithmetic: 42 m.
fn linear_ok(merged: Vec3, o1: Vec3, o2: Vec3, i: usize) -> bool {
    let m = axis(merged, i);
    let limit = (4 * SS_MAXSIZE as u32 / 3) as f32;
    !(m > limit && m > axis(o1, i) + 1.0 && m > axis(o2, i) + 1.0)
}

/// The texels the charts of one visual may take with their borders: a quarter of a page, so a
/// visual's charts always fit one page.
pub const VISUAL_TEXELS: u64 = (LMAP_SIZE as u64 * LMAP_SIZE as u64) / 4;

/// The volume of the merged box, if the merge is allowed: few enough faces, small enough on
/// every axis and in volume, and charts that fit a quarter of a page together.
fn validate_merge(
    (f1, a1): (usize, u64),
    base: &Aabb,
    base_orig: &Aabb,
    (f2, a2): (usize, u64),
    bb: &Aabb,
) -> Option<f32> {
    if f1 + f2 > 4 * SS_HIGH / 3 || a1 + a2 > VISUAL_TEXELS {
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

/// The texels the charts of a subdivision's faces take with their borders, each chart once, at
/// the size its geometry gives it (before any light).
fn chart_texels(g: &LcGlobalData, s: &[FaceId]) -> u64 {
    let mut ids: Vec<DeflId> = s.iter().filter_map(|&f| g.mesh.face(f).and_then(|x| x.deflector)).collect();
    ids.sort_unstable_by_key(|d| d.0);
    ids.dedup();
    ids.iter().filter_map(|&d| g.deflectors.get(d)).map(|d| u64::from(d.layer.area())).sum()
}

/// Small subdivisions take the candidate of the same material whose merge gives the smallest
/// box, until none is small any more; the smallest volume wins, the first of equal ones as in
/// X-Ray's compiler. It runs before the light, on geometry alone: no lightmap page decides a
/// merge, the charts' room does (a quarter of a page at most, [`VISUAL_TEXELS`]). The first faces'
/// material is a key that merging never changes (the merged front is the taken one's, of the same
/// key), so each subdivision looks only at the later ones of its key; a taken one is marked gone,
/// which keeps the others' order.
///
/// A level that streams by cells of `cells` metres (0: it does not) merges within a cell only: the
/// cell the subdivision's centre lies in is part of the key, and a merge that would make a
/// visual larger than a cell on x or z out of two that are not is refused.
pub fn merge_geometry(
    g: &LcGlobalData,
    splits: &mut Vec<Split>,
    cells: f32,
    log: &mut Log,
    ctl: &Ctl<'_>,
) -> Result<()> {
    // the material, the number of texture channels (one for every face before the light), the cell
    let cell = |s: &Split| {
        if cells > 0.0 { crate::tree::cell_of(faces_box(g, s).center(), cells) } else { [0, 0] }
    };
    let key = |s: &Split| s.first().and_then(|&f| g.mesh.face(f)).map(|x| (x.material, x.tc.len(), cell(s)));
    let keys: Vec<_> = splits.par_iter().map(key).collect();
    let mut texels: Vec<u64> = splits.par_iter().map(|s| chart_texels(g, s)).collect();
    let mut needs: Vec<(bool, Aabb)> = splits.par_iter().map(|s| need_merge(g, s)).collect();
    // per key, the candidates by the cells (x, z) their boxes touch. A candidate's box does not
    // change while it is one: only the subdivision whose turn it is grows, and it is never a
    // candidate again.
    let mut grids: FxHashMap<(u16, usize, [i32; 2]), Grid> = FxHashMap::default();
    for (i, k) in keys.iter().enumerate() {
        let (Some(k), (true, bb)) = (k, needs[i]) else { continue };
        let grid = grids.entry(*k).or_default();
        for c in Grid::range(&bb, 0.0).cells() {
            grid.cells.entry(c).or_default().push(i);
        }
    }
    let mut alive = vec![true; splits.len()];
    let mut order: Vec<(f32, [i32; 2])> = Vec::new();
    for split in 0..splits.len() {
        if split % 256 == 0 {
            ctl.check()?;
        }
        if !alive[split] {
            continue;
        }
        let Some(k) = keys[split] else { continue };
        let Some(grid) = grids.get(&k) else { continue };
        let mut orig: Option<Aabb> = None;
        loop {
            let (need, base) = needs[split];
            if !need {
                break;
            }
            let base_orig = *orig.get_or_insert(base);
            let n1 = splits[split].len();
            let judge = |i: usize| {
                let (need, bb) = needs[i];
                if i <= split || !alive[i] || !need {
                    return None;
                }
                if cells > 0.0 && !within_cell(&base, &base_orig, &bb, cells) {
                    return None;
                }
                // X-Ray's compiler starts from flt_max and takes only a smaller volume
                validate_merge((n1, texels[split]), &base, &base_orig, (splits[i].len(), texels[i]), &bb)
                    .filter(|&v| v < f32::MAX)
                    .map(|v| (v, i))
            };
            let pick =
                |a: (f32, usize), b: (f32, usize)| if b.0 < a.0 || (b.0 == a.0 && b.1 < a.1) { b } else { a };
            // the cells near the base, nearest first by the least volume a merge with anything
            // in them can have; past the best volume found, no cell can give a smaller one
            grid.near(&base, &mut order);
            let mut best: Option<(f32, usize)> = None;
            for &(bound, c) in &order {
                if best.is_some_and(|b| bound > b.0) {
                    break;
                }
                let Some(list) = grid.cells.get(&c) else { continue };
                let found = if list.len() >= 1024 {
                    list.par_iter().filter_map(|&i| judge(i)).reduce_with(pick)
                } else {
                    list.iter().filter_map(|&i| judge(i)).reduce(pick)
                };
                best = match (best, found) {
                    (Some(a), Some(b)) => Some(pick(a, b)),
                    (a, b) => a.or(b),
                };
            }
            let Some((_, sel)) = best else { break };
            alive[sel] = false;
            texels[split] += texels[sel];
            let mut merged = std::mem::take(&mut splits[sel]);
            merged.extend_from_slice(&splits[split]);
            splits[split] = merged;
            // the box of the faces is the two boxes' (each grown by the same epsilon)
            let mut bb = base;
            bb.merge(&needs[sel].1);
            needs[split] = (need_of(splits[split].len(), &bb), bb);
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

/// Whether a merge keeps a visual a cell's: no larger than a cell on x and z, unless one of the two
/// already is (then it is the level's visual anyway).
fn within_cell(base: &Aabb, base_orig: &Aabb, bb: &Aabb, cells: f32) -> bool {
    let big = |b: &Aabb| b.max.x - b.min.x > cells || b.max.z - b.min.z > cells;
    if big(base_orig) || big(bb) {
        return true;
    }
    let mut m = *base;
    m.merge(bb);
    !big(&m)
}

/// Where every subdivision's visual goes: the level when it does not stream by cells (`cells` 0),
/// else by its box ([`crate::tree::place_of`]).
pub fn places(g: &LcGlobalData, splits: &[Split], cells: f32) -> Vec<crate::tree::Place> {
    if cells <= 0.0 {
        return vec![crate::tree::Place::Base; splits.len()];
    }
    splits.par_iter().map(|s| crate::tree::place_of(&faces_box(g, s), cells)).collect()
}

/// How far from its box a subdivision looks for a candidate: past `4 * c_SS_maxsize / 3` (42 m)
/// and one metre more apart on an axis, `linear_ok` refuses the merge.
const REACH: f32 = 44.0;
/// The side of a cell of the merge's grid.
const CELL: f32 = 8.0;

/// The candidates of one key by the cells of the ground plane (x, z) their boxes touch.
#[derive(Default)]
struct Grid {
    cells: FxHashMap<[i32; 2], Vec<usize>>,
}

/// A range of cells.
struct Cells {
    x: (i32, i32),
    z: (i32, i32),
}

impl Cells {
    fn cells(&self) -> impl Iterator<Item = [i32; 2]> + '_ {
        (self.x.0..=self.x.1).flat_map(move |x| (self.z.0..=self.z.1).map(move |z| [x, z]))
    }

    fn holds(&self, c: [i32; 2]) -> bool {
        (self.x.0..=self.x.1).contains(&c[0]) && (self.z.0..=self.z.1).contains(&c[1])
    }

    fn len(&self) -> i64 {
        (i64::from(self.x.1) - i64::from(self.x.0) + 1) * (i64::from(self.z.1) - i64::from(self.z.0) + 1)
    }
}

impl Grid {
    /// The cells a box grown by `by` touches.
    fn range(b: &Aabb, by: f32) -> Cells {
        let lo = |v: f32| ((v - by) / CELL).floor() as i32;
        let hi = |v: f32| ((v + by) / CELL).floor() as i32;
        Cells { x: (lo(b.min.x), hi(b.max.x)), z: (lo(b.min.z), hi(b.max.z)) }
    }

    /// The cells within `REACH` of the base that hold candidates, each with a bound no merge with
    /// a candidate that is in it, and in no nearer cell, can go under: the base's box stretched
    /// over the gap to the cell on x and z. A candidate lies in every cell its box touches, so it
    /// is met in the nearest of them first. The bound is taken a little low: it only ever orders
    /// and cuts, the volumes themselves decide.
    fn near(&self, base: &Aabb, out: &mut Vec<(f32, [i32; 2])>) {
        out.clear();
        let range = Grid::range(base, REACH);
        let size = base.max - base.min;
        let bound = |c: [i32; 2]| {
            let gap = |lo: f32, hi: f32, cell: i32| {
                let (a, b) = (cell as f32 * CELL, (cell + 1) as f32 * CELL);
                (a - hi).max(lo - b).max(0.0)
            };
            let gx = gap(base.min.x, base.max.x, c[0]);
            let gz = gap(base.min.z, base.max.z, c[1]);
            (size.x + gx) * size.y * (size.z + gz) * 0.999
        };
        if range.len() > self.cells.len() as i64 {
            out.extend(self.cells.keys().copied().filter(|&c| range.holds(c)).map(|c| (bound(c), c)));
        } else {
            out.extend(range.cells().filter(|c| self.cells.contains_key(c)).map(|c| (bound(c), c)));
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    }
}

/// The charts of every subdivision - the deflectors of its faces, each once, in the order its
/// faces meet them -: what the packer keeps on one page.
pub fn visual_charts(g: &LcGlobalData, splits: &[Split]) -> Vec<Vec<DeflId>> {
    splits
        .par_iter()
        .map(|s| {
            let mut seen = rustc_hash::FxHashSet::default();
            s.iter()
                .filter_map(|&f| g.mesh.face(f).and_then(|x| x.deflector))
                .filter(|d| seen.insert(*d))
                .collect()
        })
        .collect()
}

/// A subdivision whose faces the packer put on several pages (a visual whose charts did not fit
/// one page: never, by the merge's limit) becomes one subdivision per page, in place, the pages
/// in the order its faces meet them, each where the visual was to go (`places`, cut alike). Answers
/// how many were cut.
pub fn split_by_page(
    g: &LcGlobalData,
    splits: &mut Vec<Split>,
    places: &mut Vec<crate::tree::Place>,
    log: &mut Log,
) -> usize {
    let mut cut = 0;
    let mut out = Vec::with_capacity(splits.len());
    let mut out_places = Vec::with_capacity(splits.len());
    for (i, s) in std::mem::take(splits).into_iter().enumerate() {
        let place = places.get(i).copied().unwrap_or_default();
        let mut pages: Vec<(Option<u32>, Split)> = Vec::new();
        for &f in &s {
            let p = g.mesh.face(f).and_then(|x| x.lmap);
            match pages.iter_mut().find(|(q, _)| *q == p) {
                Some((_, l)) => l.push(f),
                None => pages.push((p, vec![f])),
            }
        }
        if pages.len() > 1 {
            cut += 1;
            log.warn(format!(
                "The lightmaps of the visual {i} ({} faces) did not fit one page: it is made of {} visuals, one a page.",
                s.len(),
                pages.len()
            ));
            out_places.extend(std::iter::repeat_n(place, pages.len()));
            out.extend(pages.into_iter().map(|(_, l)| l));
        } else {
            out.push(s);
            out_places.push(place);
        }
    }
    *splits = out;
    *places = out_places;
    cut
}
