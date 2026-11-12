//! A bounding volume hierarchy over triangles: what the editor picks, drops and selects with.
//!
//! Not a file format. The tree is a flat array of nodes in depth-first order (a node's left
//! child follows it), built with a binned surface area heuristic on as many threads as the
//! caller's pool has; the triangles are copied next to it in leaf order. Every query takes
//! `&self`, so one tree serves any number of threads.

use std::ops::ControlFlow;

use glam::{BVec3A, Vec3, Vec3A, Vec4};
use rayon::prelude::*;
use smallvec::SmallVec;

use crate::{Aabb, CForm, LevelError, Result};

/// Triangles a leaf may hold when another split would not pay.
const LEAF_MAX: usize = 4;
const BINS: usize = 16;
/// A subtree with fewer triangles is built on the thread that reached it.
const PAR_BUILD: usize = 4096;
/// A node with more triangles bins them on several threads.
const PAR_BIN: usize = 1 << 16;
/// Past this depth the heuristic gives way to median splits, which bound the depth.
const SAH_DEPTH: u32 = 48;

/// A ray: `origin + dir * t` for `t` in `0..=t_max`. `dir` need not be of unit length; hit
/// distances are in multiples of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
    pub t_max: f32,
    /// Skip triangles that face away (clockwise seen from the origin).
    pub cull_back: bool,
}

impl Ray {
    #[must_use]
    pub fn new(origin: Vec3, dir: Vec3, t_max: f32) -> Self {
        Self { origin, dir, t_max, cull_back: false }
    }
}

/// Where a ray met a triangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// The triangle's index in the list the tree was built from.
    pub tri: u32,
    /// The distance along the ray.
    pub t: f32,
    /// Barycentric weights of the second and third vertex; the first has `1 - u - v`.
    pub u: f32,
    pub v: f32,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    min: Vec3,
    /// A leaf: its first triangle. An inner node: its right child (the left one follows it).
    a: u32,
    max: Vec3,
    /// Triangles of a leaf; 0 for an inner node.
    count: u32,
}

impl Node {
    const EMPTY: Node = Node { min: Vec3::ZERO, a: 0, max: Vec3::ZERO, count: 0 };
}

/// A triangle while the tree is built: its box and where it came from.
#[derive(Clone, Copy)]
struct Prim {
    min: Vec3A,
    max: Vec3A,
    id: u32,
}

#[derive(Clone, Copy)]
struct Bin {
    min: Vec3A,
    max: Vec3A,
    count: u32,
}

impl Bin {
    const EMPTY: Bin = Bin { min: Vec3A::INFINITY, max: Vec3A::NEG_INFINITY, count: 0 };

    #[inline]
    fn add(&mut self, p: &Prim) {
        self.min = self.min.min(p.min);
        self.max = self.max.max(p.max);
        self.count += 1;
    }

    #[inline]
    fn merge(&mut self, o: &Bin) {
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
        self.count += o.count;
    }

    fn area(&self) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        let d = self.max - self.min;
        d.x * d.y + d.y * d.z + d.z * d.x
    }
}

type Bins = [[Bin; BINS]; 3];

/// The bins of the three axes a triangle's centre falls into. `lo` and `scale` work on doubled
/// centres (`min + max`), which saves a multiplication per triangle.
#[inline]
fn bin_of(p: &Prim, lo: Vec3A, scale: Vec3A) -> [usize; 3] {
    let rel = ((p.min + p.max) - lo) * scale;
    let [x, y, z] = rel.to_array();
    // a float to usize cast saturates and sends NaN to 0: never out of range after the min
    [(x as usize).min(BINS - 1), (y as usize).min(BINS - 1), (z as usize).min(BINS - 1)]
}

fn fill_bins(prims: &[Prim], lo: Vec3A, scale: Vec3A) -> Bins {
    let mut bins = [[Bin::EMPTY; BINS]; 3];
    for p in prims {
        let at = bin_of(p, lo, scale);
        for (axis, row) in bins.iter_mut().enumerate() {
            if let Some(bin) = row.get_mut(at[axis]) {
                bin.add(p);
            }
        }
    }
    bins
}

fn merge_bins(mut a: Bins, b: Bins) -> Bins {
    for (row, other) in a.iter_mut().zip(b.iter()) {
        for (bin, o) in row.iter_mut().zip(other.iter()) {
            bin.merge(o);
        }
    }
    a
}

/// The box of the triangles and the box of their doubled centres.
fn bounds(prims: &[Prim]) -> (Bin, Vec3A, Vec3A) {
    let fold = |chunk: &[Prim]| {
        let mut all = Bin::EMPTY;
        let (mut lo, mut hi) = (Vec3A::INFINITY, Vec3A::NEG_INFINITY);
        for p in chunk {
            all.add(p);
            let c = p.min + p.max;
            lo = lo.min(c);
            hi = hi.max(c);
        }
        (all, lo, hi)
    };
    if prims.len() < PAR_BIN {
        return fold(prims);
    }
    prims.par_chunks(PAR_BIN / 4).map(fold).reduce(
        || (Bin::EMPTY, Vec3A::INFINITY, Vec3A::NEG_INFINITY),
        |mut a, b| {
            a.0.merge(&b.0);
            (a.0, a.1.min(b.1), a.2.max(b.2))
        },
    )
}

/// Moves the items the predicate accepts to the front; returns how many there are.
fn partition(items: &mut [Prim], pred: impl Fn(&Prim) -> bool) -> usize {
    let mut i = 0;
    let mut j = items.len();
    loop {
        while i < j && items.get(i).is_some_and(&pred) {
            i += 1;
        }
        while i < j && items.get(j - 1).is_some_and(|p| !pred(p)) {
            j -= 1;
        }
        if i >= j {
            return i;
        }
        items.swap(i, j - 1);
        i += 1;
        j -= 1;
    }
}

/// Splits at the median of the widest axis of the centres.
fn median_split(prims: &mut [Prim], extent: Vec3A) -> usize {
    let mid = prims.len() / 2;
    let [x, y, z] = extent.to_array();
    let axis = if x >= y && x >= z {
        0
    } else if y >= z {
        1
    } else {
        2
    };
    let key = move |p: &Prim| (p.min + p.max).to_array()[axis];
    if mid > 0 && mid < prims.len() {
        prims.select_nth_unstable_by(mid, |a, b| key(a).total_cmp(&key(b)));
    }
    mid
}

/// Orders the triangles of a node for a split and says where the right half starts; `None`
/// when the node is to be a leaf. The answer is always inside `1..len`.
fn split(prims: &mut [Prim], node: &Bin, lo: Vec3A, hi: Vec3A, depth: u32) -> Option<usize> {
    let n = prims.len();
    if n < 2 {
        return None;
    }
    let extent = hi - lo;
    // a NaN extent is flat too
    let flat =
        extent.max_element().partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) || !extent.is_finite();
    if flat || depth > SAH_DEPTH {
        // nothing to tell the triangles apart by, or a tree that grows too deep
        return (n > LEAF_MAX).then(|| median_split(prims, extent)).filter(|&m| m > 0 && m < n);
    }

    let scale = Vec3A::select(extent.cmpgt(Vec3A::ZERO), Vec3A::splat(BINS as f32) / extent, Vec3A::ZERO);
    let bins = if n < PAR_BIN {
        fill_bins(prims, lo, scale)
    } else {
        prims
            .par_chunks(PAR_BIN / 4)
            .map(|chunk| fill_bins(chunk, lo, scale))
            .reduce(|| [[Bin::EMPTY; BINS]; 3], merge_bins)
    };

    // the cheapest plane between two bins over the three axes
    let mut best: Option<(f32, usize, usize)> = None;
    for (axis, row) in bins.iter().enumerate() {
        let mut right = [(0.0f32, 0u32); BINS];
        let mut acc = Bin::EMPTY;
        for (i, bin) in row.iter().enumerate().rev() {
            acc.merge(bin);
            if let Some(slot) = right.get_mut(i) {
                *slot = (acc.area(), acc.count);
            }
        }
        let mut acc = Bin::EMPTY;
        for (i, bin) in row.iter().enumerate().take(BINS - 1) {
            acc.merge(bin);
            let Some(&(right_area, right_count)) = right.get(i + 1) else { continue };
            if acc.count == 0 || right_count == 0 {
                continue;
            }
            let cost = acc.area() * acc.count as f32 + right_area * right_count as f32;
            if best.is_none_or(|(c, _, _)| cost < c) {
                best = Some((cost, axis, i + 1));
            }
        }
    }

    let Some((cost, axis, first_right)) = best else {
        return (n > LEAF_MAX).then(|| median_split(prims, extent)).filter(|&m| m > 0 && m < n);
    };
    // a small node stays a leaf when splitting costs more than testing its triangles
    let area = node.area();
    if n <= LEAF_MAX && cost + area >= area * n as f32 {
        return None;
    }
    let mid = partition(prims, |p| bin_of(p, lo, scale)[axis] < first_right);
    if mid > 0 && mid < n {
        return Some(mid);
    }
    Some(median_split(prims, extent)).filter(|&m| m > 0 && m < n)
}

/// Builds the subtree of `prims` into `nodes`, which holds `2 * len - 1` slots starting at
/// index `node_at` of the whole array: the root, the left subtree, the right subtree.
fn build_range(nodes: &mut [Node], prims: &mut [Prim], node_at: u32, prim_at: u32, depth: u32) {
    let n = prims.len();
    let Some((root, rest)) = nodes.split_first_mut() else { return };
    let (all, lo, hi) = bounds(prims);
    root.min = all.min.into();
    root.max = all.max.into();
    root.a = prim_at;
    root.count = n as u32;

    let Some(mid) = split(prims, &all, lo, hi, depth) else { return };
    let Some((left_nodes, right_nodes)) = rest.split_at_mut_checked(2 * mid - 1) else { return };
    let Some((left, right)) = prims.split_at_mut_checked(mid) else { return };
    let right_at = node_at + 2 * mid as u32;
    root.a = right_at;
    root.count = 0;
    let right_prim_at = prim_at + mid as u32;
    if n >= PAR_BUILD {
        rayon::join(
            || build_range(left_nodes, left, node_at + 1, prim_at, depth + 1),
            || build_range(right_nodes, right, right_at, right_prim_at, depth + 1),
        );
    } else {
        build_range(left_nodes, left, node_at + 1, prim_at, depth + 1);
        build_range(right_nodes, right, right_at, right_prim_at, depth + 1);
    }
}

/// Drops the unused slots: the nodes in depth-first order, child links fixed.
fn compact(sparse: &[Node], used_hint: usize) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::with_capacity(used_hint);
    if sparse.is_empty() {
        return out;
    }
    // (slot in the sparse array, the parent whose right link points at it)
    let mut stack: Vec<(u32, Option<u32>)> = vec![(0, None)];
    while let Some((at, parent)) = stack.pop() {
        let Some(node) = sparse.get(at as usize).copied() else { continue };
        let here = out.len() as u32;
        if let Some(p) = parent
            && let Some(slot) = out.get_mut(p as usize)
        {
            slot.a = here;
        }
        out.push(node);
        if node.count == 0 {
            stack.push((node.a, Some(here)));
            stack.push((at + 1, None));
        }
    }
    out
}

/// A direction component below this is no movement along its axis.
const STILL: f32 = 1e-30;
/// How far a box's entry and exit distances are widened: the rounding of the difference, the
/// reciprocal and the product that make them, a few units in the last place each.
const WIDEN: f32 = 4e-7;

/// A ray made ready for box tests.
struct Probe {
    origin: Vec3A,
    dir: Vec3A,
    inv: Vec3A,
    /// The axes the ray does not move along.
    still: BVec3A,
    cull: bool,
}

impl Probe {
    fn new(ray: &Ray) -> Self {
        let d = Vec3A::from(ray.dir);
        let still = d.abs().cmplt(Vec3A::splat(STILL));
        Self {
            origin: ray.origin.into(),
            dir: d,
            // a still axis takes no reciprocal: 0 * inf would be NaN on a box face
            inv: Vec3A::ONE / Vec3A::select(still, Vec3A::ONE, d),
            still,
            cull: ray.cull_back,
        }
    }

    /// Where the ray enters the box, when it meets it before `t_max`. Monotonic in the box: a
    /// box the ray enters is entered in every box that holds it, which is what makes
    /// [`ray_triangle`] answer as a walk does.
    #[inline]
    fn enter(&self, node: &Node, t_max: f32) -> Option<f32> {
        let (lo, hi) = (Vec3A::from(node.min), Vec3A::from(node.max));
        // along a still axis the ray is inside the slab for every distance when its origin is,
        // faces included (the C++ slab test of xrCDB_ray.cpp answers the same through its NaN
        // filter), and never otherwise
        if (self.still & (self.origin.cmplt(lo) | self.origin.cmpgt(hi))).any() {
            return None;
        }
        let t0 = (lo - self.origin) * self.inv;
        let t1 = (hi - self.origin) * self.inv;
        let near = Vec3A::select(self.still, Vec3A::NEG_INFINITY, t0.min(t1)).max_element();
        let far = Vec3A::select(self.still, Vec3A::INFINITY, t0.max(t1)).min_element();
        if far < 0.0 {
            return None;
        }
        // widened by the rounding, so that a hit on the very border of a box is never lost
        let near = near.max(0.0) * (1.0 - WIDEN);
        let far = (far * (1.0 + WIDEN)).min(t_max);
        (near <= far).then_some(near)
    }

    /// Moeller-Trumbore. `None` for a miss, a degenerate triangle or a hit behind the origin.
    #[inline]
    fn triangle(&self, tri: &[Vec3; 3]) -> Option<(f32, f32, f32)> {
        let v0 = Vec3A::from(tri[0]);
        let e1 = Vec3A::from(tri[1]) - v0;
        let e2 = Vec3A::from(tri[2]) - v0;
        let p = self.dir.cross(e2);
        let det = e1.dot(p);
        if self.cull {
            if det < 1e-12 {
                return None;
            }
        } else if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let s = self.origin - v0;
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(e1);
        let v = self.dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = e2.dot(q) * inv;
        (t >= 0.0).then_some((t, u, v))
    }
}

/// The test the tree's walks make of one triangle: `(t, u, v)` of a hit with `t` in
/// `0..=t_max`, `None` for a miss. The triangle's own box is tested first, as a walk tests the
/// boxes that hold it: a hit here is a hit every walk over a tree that holds the triangle finds,
/// so a caller that keeps a triangle of its own (a cache) only ever saves a walk, never changes
/// its answer.
#[must_use]
pub fn ray_triangle(ray: &Ray, tri: &[Vec3; 3]) -> Option<(f32, f32, f32)> {
    let probe = Probe::new(ray);
    let (a, b, c) = (Vec3A::from(tri[0]), Vec3A::from(tri[1]), Vec3A::from(tri[2]));
    let own = Node { min: a.min(b).min(c).into(), a: 0, max: a.max(b).max(c).into(), count: 1 };
    probe.enter(&own, ray.t_max)?;
    probe.triangle(tri).filter(|&(t, _, _)| t <= ray.t_max)
}

/// Whether a triangle and a box overlap: the separating axis test over the box axes, the
/// triangle's normal and the nine edge cross products.
fn tri_box(center: Vec3A, half: Vec3A, tri: &[Vec3; 3]) -> bool {
    let v0 = Vec3A::from(tri[0]) - center;
    let v1 = Vec3A::from(tri[1]) - center;
    let v2 = Vec3A::from(tri[2]) - center;
    let lo = v0.min(v1).min(v2);
    let hi = v0.max(v1).max(v2);
    if lo.cmpgt(half).any() || hi.cmplt(-half).any() {
        return false;
    }
    let separated = |axis: Vec3A| {
        let (p0, p1, p2) = (axis.dot(v0), axis.dot(v1), axis.dot(v2));
        let r = half.dot(axis.abs());
        p0.min(p1).min(p2) > r || p0.max(p1).max(p2) < -r
    };
    let edges = [v1 - v0, v2 - v1, v0 - v2];
    if separated(edges[0].cross(edges[1])) {
        return false;
    }
    for e in edges {
        if separated(Vec3A::new(0.0, -e.z, e.y))
            || separated(Vec3A::new(e.z, 0.0, -e.x))
            || separated(Vec3A::new(-e.y, e.x, 0.0))
        {
            return false;
        }
    }
    true
}

/// The squared distance from a point to a triangle.
fn point_tri_dist2(p: Vec3A, tri: &[Vec3; 3]) -> f32 {
    let (a, b, c) = (Vec3A::from(tri[0]), Vec3A::from(tri[1]), Vec3A::from(tri[2]));
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return ap.length_squared();
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return bp.length_squared();
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return (p - (a + ab * v)).length_squared();
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return cp.length_squared();
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return (p - (a + ac * w)).length_squared();
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return (p - (b + (c - b) * w)).length_squared();
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    (p - (a + ab * v + ac * w)).length_squared()
}

/// What a volume makes of a node's box.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Outside,
    Crossing,
    Inside,
}

/// The tree. See the module text.
#[derive(Clone, Debug)]
pub struct Bvh {
    nodes: Vec<Node>,
    /// The triangles in leaf order.
    tris: Vec<[Vec3; 3]>,
    /// For every triangle in leaf order, its index in the caller's list.
    ids: Vec<u32>,
    /// Triangles of the caller's list that were left out: a vertex that is not finite.
    skipped: usize,
}

impl Bvh {
    /// Builds the tree of an indexed triangle list. Runs on the rayon pool it is called in.
    /// A triangle naming a vertex that does not exist is an error; a triangle with a NaN or an
    /// infinity is left out (see [`Self::skipped`]).
    pub fn build(verts: &[Vec3], tris: &[[u32; 3]]) -> Result<Self> {
        if tris.len() >= (u32::MAX / 2) as usize {
            return Err(LevelError::Invalid("too many triangles for one collision tree".into()));
        }
        let corner = |i: u32| verts.get(i as usize).copied();
        let prims: Vec<Option<Option<Prim>>> = tris
            .par_iter()
            .with_min_len(4096)
            .enumerate()
            .map(|(id, t)| {
                let (a, b, c) = (corner(t[0])?, corner(t[1])?, corner(t[2])?);
                if !(a.is_finite() && b.is_finite() && c.is_finite()) {
                    return Some(None);
                }
                let (a, b, c) = (Vec3A::from(a), Vec3A::from(b), Vec3A::from(c));
                Some(Some(Prim { min: a.min(b).min(c), max: a.max(b).max(c), id: id as u32 }))
            })
            .collect();
        if let Some(bad) = prims.iter().position(Option::is_none) {
            return Err(LevelError::Invalid(format!(
                "triangle {bad} names a vertex outside the {} that were given",
                verts.len()
            )));
        }
        let mut prims: Vec<Prim> = prims.into_iter().flatten().flatten().collect();
        let skipped = tris.len() - prims.len();
        if prims.is_empty() {
            return Ok(Self { nodes: Vec::new(), tris: Vec::new(), ids: Vec::new(), skipped });
        }

        let mut sparse = vec![Node::EMPTY; 2 * prims.len() - 1];
        build_range(&mut sparse, &mut prims, 0, 0, 0);
        let nodes = compact(&sparse, prims.len());
        drop(sparse);

        let ids: Vec<u32> = prims.iter().map(|p| p.id).collect();
        let packed: Vec<[Vec3; 3]> = ids
            .par_iter()
            .with_min_len(4096)
            .map(|&id| {
                let t = tris.get(id as usize).copied().unwrap_or_default();
                [
                    corner(t[0]).unwrap_or_default(),
                    corner(t[1]).unwrap_or_default(),
                    corner(t[2]).unwrap_or_default(),
                ]
            })
            .collect();
        Ok(Self { nodes, tris: packed, ids, skipped })
    }

    /// The tree of a collision form.
    pub fn from_cform(form: &CForm) -> Result<Self> {
        Self::build(&form.verts, &form.indices())
    }

    /// Triangles in the tree.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tris.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// Triangles of the input that are not in the tree because a vertex was not finite.
    #[must_use]
    pub fn skipped(&self) -> usize {
        self.skipped
    }

    /// The box of everything in the tree; `None` for an empty tree.
    #[must_use]
    pub fn bounds(&self) -> Option<Aabb> {
        self.nodes.first().map(|n| Aabb { min: n.min, max: n.max })
    }

    /// The nodes of the tree and its depth: figures for a profile line.
    #[must_use]
    pub fn stats(&self) -> (usize, usize) {
        let mut depth = 0usize;
        let mut stack: Vec<(u32, usize)> = if self.nodes.is_empty() { Vec::new() } else { vec![(0, 1)] };
        while let Some((at, d)) = stack.pop() {
            let Some(node) = self.nodes.get(at as usize) else { continue };
            depth = depth.max(d);
            if node.count == 0 {
                stack.push((node.a, d + 1));
                stack.push((at + 1, d + 1));
            }
        }
        (self.nodes.len(), depth)
    }

    fn leaf(&self, node: &Node) -> impl Iterator<Item = (&[Vec3; 3], u32)> {
        let start = node.a as usize;
        let end = start + node.count as usize;
        let tris = self.tris.get(start..end).unwrap_or_default();
        let ids = self.ids.get(start..end).unwrap_or_default();
        tris.iter().zip(ids.iter().copied())
    }

    /// The nearest hit of a ray.
    #[must_use]
    pub fn ray_nearest(&self, ray: &Ray) -> Option<RayHit> {
        let probe = Probe::new(ray);
        let mut t_max = ray.t_max;
        let mut best: Option<RayHit> = None;
        let mut stack: SmallVec<[(u32, f32); 64]> = SmallVec::new();
        if let Some(t) = self.nodes.first().and_then(|root| probe.enter(root, t_max)) {
            stack.push((0, t));
        }
        while let Some((at, entered)) = stack.pop() {
            // a hit found since this node was pushed may already be nearer than its box
            if entered > t_max {
                continue;
            }
            let Some(node) = self.nodes.get(at as usize) else { continue };
            if node.count > 0 {
                for (tri, id) in self.leaf(node) {
                    if let Some((t, u, v)) = probe.triangle(tri)
                        && t <= t_max
                        && best.is_none_or(|b| t < b.t)
                    {
                        t_max = t;
                        best = Some(RayHit { tri: id, t, u, v });
                    }
                }
                continue;
            }
            let (l, r) = (at + 1, node.a);
            let tl = self.nodes.get(l as usize).and_then(|n| probe.enter(n, t_max));
            let tr = self.nodes.get(r as usize).and_then(|n| probe.enter(n, t_max));
            match (tl, tr) {
                (Some(a), Some(b)) => {
                    // the nearer child on top
                    if a <= b {
                        stack.push((r, b));
                        stack.push((l, a));
                    } else {
                        stack.push((l, a));
                        stack.push((r, b));
                    }
                }
                (Some(a), None) => stack.push((l, a)),
                (None, Some(b)) => stack.push((r, b)),
                (None, None) => {}
            }
        }
        best
    }

    /// Some hit of a ray, the first one found: for "is anything in the way".
    #[must_use]
    pub fn ray_any(&self, ray: &Ray) -> Option<RayHit> {
        let probe = Probe::new(ray);
        let mut stack: SmallVec<[u32; 64]> = SmallVec::new();
        if !self.nodes.is_empty() {
            stack.push(0);
        }
        while let Some(at) = stack.pop() {
            let Some(node) = self.nodes.get(at as usize) else { continue };
            if probe.enter(node, ray.t_max).is_none() {
                continue;
            }
            if node.count > 0 {
                for (tri, id) in self.leaf(node) {
                    if let Some((t, u, v)) = probe.triangle(tri)
                        && t <= ray.t_max
                    {
                        return Some(RayHit { tri: id, t, u, v });
                    }
                }
            } else {
                stack.push(node.a);
                stack.push(at + 1);
            }
        }
        None
    }

    /// Every hit of a ray handed to `visit` as the walk finds it, in no particular order, until
    /// `visit` breaks. Answers whether it broke.
    pub fn ray_visit(&self, ray: &Ray, mut visit: impl FnMut(RayHit) -> ControlFlow<()>) -> bool {
        let probe = Probe::new(ray);
        let mut stack: SmallVec<[u32; 64]> = SmallVec::new();
        if !self.nodes.is_empty() {
            stack.push(0);
        }
        while let Some(at) = stack.pop() {
            let Some(node) = self.nodes.get(at as usize) else { continue };
            if probe.enter(node, ray.t_max).is_none() {
                continue;
            }
            if node.count > 0 {
                for (tri, id) in self.leaf(node) {
                    if let Some((t, u, v)) = probe.triangle(tri)
                        && t <= ray.t_max
                        && visit(RayHit { tri: id, t, u, v }).is_break()
                    {
                        return true;
                    }
                }
            } else {
                stack.push(node.a);
                stack.push(at + 1);
            }
        }
        false
    }

    /// Every hit of a ray, in no particular order; `out` is appended to.
    pub fn ray_all(&self, ray: &Ray, out: &mut Vec<RayHit>) {
        let probe = Probe::new(ray);
        let mut stack: SmallVec<[u32; 64]> = SmallVec::new();
        if !self.nodes.is_empty() {
            stack.push(0);
        }
        while let Some(at) = stack.pop() {
            let Some(node) = self.nodes.get(at as usize) else { continue };
            if probe.enter(node, ray.t_max).is_none() {
                continue;
            }
            if node.count > 0 {
                for (tri, id) in self.leaf(node) {
                    if let Some((t, u, v)) = probe.triangle(tri)
                        && t <= ray.t_max
                    {
                        out.push(RayHit { tri: id, t, u, v });
                    }
                }
            } else {
                stack.push(node.a);
                stack.push(at + 1);
            }
        }
    }

    /// The nearest hit of every ray, on the rayon pool.
    #[must_use]
    pub fn rays_nearest(&self, rays: &[Ray]) -> Vec<Option<RayHit>> {
        rays.par_iter().with_min_len(64).map(|r| self.ray_nearest(r)).collect()
    }

    /// Whether each ray hits anything, on the rayon pool.
    #[must_use]
    pub fn rays_any(&self, rays: &[Ray]) -> Vec<bool> {
        rays.par_iter().with_min_len(64).map(|r| self.ray_any(r).is_some()).collect()
    }

    /// Walks the tree with a volume: `classify` judges a node's box, `test` a triangle of a box
    /// that crosses the volume's border. Triangles of a box that is inside are taken untested.
    fn query(&self, out: &mut Vec<u32>, classify: impl Fn(&Node) -> Side, test: impl Fn(&[Vec3; 3]) -> bool) {
        // (node, the volume holds its whole box)
        let mut stack: SmallVec<[(u32, bool); 64]> = SmallVec::new();
        if !self.nodes.is_empty() {
            stack.push((0, false));
        }
        while let Some((at, inside)) = stack.pop() {
            let Some(node) = self.nodes.get(at as usize) else { continue };
            let inside = if inside {
                true
            } else {
                match classify(node) {
                    Side::Outside => continue,
                    Side::Inside => true,
                    Side::Crossing => false,
                }
            };
            if node.count > 0 {
                for (tri, id) in self.leaf(node) {
                    if inside || test(tri) {
                        out.push(id);
                    }
                }
            } else {
                stack.push((node.a, inside));
                stack.push((at + 1, inside));
            }
        }
    }

    /// The triangles that overlap a box, in no particular order; `out` is appended to.
    pub fn query_box(&self, bounds: &Aabb, out: &mut Vec<u32>) {
        let (lo, hi) = (Vec3A::from(bounds.min), Vec3A::from(bounds.max));
        let center = (lo + hi) * 0.5;
        let half = (hi - lo) * 0.5;
        self.query(
            out,
            |n| {
                let (nlo, nhi) = (Vec3A::from(n.min), Vec3A::from(n.max));
                if nlo.cmpgt(hi).any() || nhi.cmplt(lo).any() {
                    Side::Outside
                } else if nlo.cmpge(lo).all() && nhi.cmple(hi).all() {
                    Side::Inside
                } else {
                    Side::Crossing
                }
            },
            |tri| tri_box(center, half, tri),
        );
    }

    /// The triangles that touch a sphere, in no particular order; `out` is appended to.
    pub fn query_sphere(&self, center: Vec3, radius: f32, out: &mut Vec<u32>) {
        let c = Vec3A::from(center);
        let r2 = radius * radius;
        self.query(
            out,
            |n| {
                let (nlo, nhi) = (Vec3A::from(n.min), Vec3A::from(n.max));
                let nearest = c.clamp(nlo, nhi);
                if (nearest - c).length_squared() > r2 {
                    return Side::Outside;
                }
                let farthest = (nlo - c).abs().max((nhi - c).abs());
                if farthest.length_squared() <= r2 { Side::Inside } else { Side::Crossing }
            },
            |tri| point_tri_dist2(c, tri) <= r2,
        );
    }

    /// The triangles inside or crossing a convex volume given by planes `(normal, d)`, in no
    /// particular order; `out` is appended to. A point is outside a plane when
    /// `normal . p + d > 0` - the engine's `CFrustum` convention, normals pointing out. A
    /// triangle is taken unless all of its vertices are outside one plane, which keeps a few
    /// near the corners of the volume, as the engine's frustum query does.
    pub fn query_frustum(&self, planes: &[Vec4], out: &mut Vec<u32>) {
        let planes: SmallVec<[(Vec3A, f32); 8]> =
            planes.iter().map(|p| (Vec3A::from(p.truncate()), p.w)).collect();
        let planes = &planes;
        self.query(
            out,
            |n| {
                let (nlo, nhi) = (Vec3A::from(n.min), Vec3A::from(n.max));
                let mut inside = true;
                for &(normal, d) in planes {
                    let positive = normal.cmpgt(Vec3A::ZERO);
                    // the corner farthest against the normal, and the one farthest along it
                    let low = Vec3A::select(positive, nlo, nhi);
                    let high = Vec3A::select(positive, nhi, nlo);
                    if normal.dot(low) + d > 0.0 {
                        return Side::Outside;
                    }
                    if normal.dot(high) + d > 0.0 {
                        inside = false;
                    }
                }
                if inside { Side::Inside } else { Side::Crossing }
            },
            |tri| {
                planes.iter().all(|&(normal, d)| tri.iter().any(|v| normal.dot(Vec3A::from(*v)) + d <= 0.0))
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift: a fixed sequence, no dependency.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        /// In `-1..1`.
        fn signed(&mut self) -> f32 {
            (self.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
        }
        fn point(&mut self, scale: f32) -> Vec3 {
            Vec3::new(self.signed(), self.signed(), self.signed()) * scale
        }
    }

    fn soup(rng: &mut Rng, count: usize) -> (Vec<Vec3>, Vec<[u32; 3]>) {
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        for _ in 0..count {
            let at = rng.point(50.0);
            let base = verts.len() as u32;
            verts.extend([at, at + rng.point(3.0), at + rng.point(3.0)]);
            tris.push([base, base + 1, base + 2]);
        }
        (verts, tris)
    }

    fn corners(verts: &[Vec3], t: &[u32; 3]) -> [Vec3; 3] {
        [verts[t[0] as usize], verts[t[1] as usize], verts[t[2] as usize]]
    }

    fn brute_nearest(verts: &[Vec3], tris: &[[u32; 3]], ray: &Ray) -> Option<RayHit> {
        let probe = Probe::new(ray);
        let mut best: Option<RayHit> = None;
        for (id, t) in tris.iter().enumerate() {
            if let Some((d, u, v)) = probe.triangle(&corners(verts, t))
                && d <= ray.t_max
                && best.is_none_or(|b| d < b.t)
            {
                best = Some(RayHit { tri: id as u32, t: d, u, v });
            }
        }
        best
    }

    #[test]
    fn rays_agree_with_brute_force_on_a_random_soup() {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        let (verts, tris) = soup(&mut rng, 3000);
        let bvh = Bvh::build(&verts, &tris).unwrap();
        assert_eq!(bvh.len(), 3000);
        assert_eq!(bvh.skipped(), 0);
        let (nodes, depth) = bvh.stats();
        assert!(nodes < 2 * 3000 && depth < 64, "{nodes} nodes, depth {depth}");

        let rays: Vec<Ray> = (0..600)
            .map(|i| {
                let mut ray = Ray::new(rng.point(60.0), rng.point(1.0), if i % 3 == 0 { 40.0 } else { 1e9 });
                ray.cull_back = i % 5 == 0;
                ray
            })
            .collect();
        let batch = bvh.rays_nearest(&rays);
        let any = bvh.rays_any(&rays);
        let mut hits = 0;
        for (i, ray) in rays.iter().enumerate() {
            let expect = brute_nearest(&verts, &tris, ray);
            assert_eq!(bvh.ray_nearest(ray), expect, "ray {i}");
            assert_eq!(batch[i], expect, "ray {i} in the batch");
            assert_eq!(any[i], expect.is_some(), "ray {i}, any");
            if let Some(hit) = expect {
                hits += 1;
                // the weights rebuild the point the distance gives
                let [a, b, c] = corners(&verts, &tris[hit.tri as usize]);
                let on_tri = a * (1.0 - hit.u - hit.v) + b * hit.u + c * hit.v;
                let on_ray = ray.origin + ray.dir * hit.t;
                assert!((on_tri - on_ray).length() < 1e-2, "ray {i}: {on_tri:?} against {on_ray:?}");

                let mut all = Vec::new();
                bvh.ray_all(ray, &mut all);
                assert!(all.contains(&hit));
                assert!(all.iter().all(|h| h.t >= hit.t));
            }
        }
        assert!(hits > 50, "only {hits} rays hit anything: the test proves little");
    }

    #[test]
    fn volumes_agree_with_brute_force_on_a_random_soup() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (verts, tris) = soup(&mut rng, 2000);
        let bvh = Bvh::build(&verts, &tris).unwrap();
        let brute = |test: &dyn Fn(&[Vec3; 3]) -> bool| -> Vec<u32> {
            (0..tris.len() as u32).filter(|&i| test(&corners(&verts, &tris[i as usize]))).collect()
        };
        // A triangle within a hair of a volume's border may fall either way with the rounding
        // of the box tests, so the answer is held between the volume shrunk and grown by `SLACK`.
        const SLACK: f32 = 1e-3;
        let between = |got: &[u32], least: &[u32], most: &[u32], what: &str| {
            assert!(least.iter().all(|i| got.contains(i)), "{what}: a triangle inside was missed");
            assert!(got.iter().all(|i| most.contains(i)), "{what}: a triangle outside was taken");
            let mut seen = got.to_vec();
            seen.sort_unstable();
            seen.dedup();
            assert_eq!(seen.len(), got.len(), "{what}: a triangle was reported twice");
        };
        let box_test = |b: &Aabb, t: &[Vec3; 3]| {
            let (lo, hi) = (Vec3A::from(b.min), Vec3A::from(b.max));
            tri_box((lo + hi) * 0.5, (hi - lo) * 0.5, t)
        };
        let planes_of = |b: &Aabb| {
            [
                Vec4::new(1.0, 0.0, 0.0, -b.max.x),
                Vec4::new(-1.0, 0.0, 0.0, b.min.x),
                Vec4::new(0.0, 1.0, 0.0, -b.max.y),
                Vec4::new(0.0, -1.0, 0.0, b.min.y),
                Vec4::new(0.0, 0.0, 1.0, -b.max.z),
                Vec4::new(0.0, 0.0, -1.0, b.min.z),
            ]
        };
        let planes_test = |planes: &[Vec4; 6], t: &[Vec3; 3]| {
            planes.iter().all(|p| t.iter().any(|v| p.truncate().dot(*v) + p.w <= 0.0))
        };

        let mut found = 0;
        for _ in 0..40 {
            let at = rng.point(40.0);
            let size = rng.point(12.0).abs() + Vec3::splat(0.5);
            let b = Aabb::new(at - size, at + size);
            let mut small = b;
            small.grow(-SLACK);
            let mut large = b;
            large.grow(SLACK);

            let mut got = Vec::new();
            bvh.query_box(&b, &mut got);
            found += got.len();
            between(&got, &brute(&|t| box_test(&small, t)), &brute(&|t| box_test(&large, t)), "box");

            let radius = size.x + 1.0;
            let within = |r: f32| brute(&|t| point_tri_dist2(at.into(), t) <= r * r);
            let mut got = Vec::new();
            bvh.query_sphere(at, radius, &mut got);
            found += got.len();
            between(&got, &within(radius - SLACK), &within(radius + SLACK), "sphere");

            // the same box as six planes, normals out
            let mut got = Vec::new();
            bvh.query_frustum(&planes_of(&b), &mut got);
            let (least, most) = (planes_of(&small), planes_of(&large));
            between(&got, &brute(&|t| planes_test(&least, t)), &brute(&|t| planes_test(&most, t)), "planes");
        }
        assert!(found > 200, "the volumes held only {found} triangles: the test proves little");
    }

    #[test]
    fn a_whole_tree_inside_a_volume_is_taken_untested() {
        let mut rng = Rng(7);
        let (verts, tris) = soup(&mut rng, 500);
        let bvh = Bvh::build(&verts, &tris).unwrap();
        let everything = Aabb::new(Vec3::splat(-1000.0), Vec3::splat(1000.0));
        let mut got = Vec::new();
        bvh.query_box(&everything, &mut got);
        assert_eq!(got.len(), 500);
        let mut got = Vec::new();
        bvh.query_sphere(Vec3::ZERO, 5000.0, &mut got);
        assert_eq!(got.len(), 500);
        let b = bvh.bounds().unwrap();
        assert!(b.min.cmpge(Vec3::splat(-60.0)).all() && b.max.cmple(Vec3::splat(60.0)).all());
    }

    #[test]
    fn simple_cases_and_edges() {
        let verts = [Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, -1.0), Vec3::new(0.0, 0.0, 1.0)];
        let bvh = Bvh::build(&verts, &[[0, 1, 2]]).unwrap();
        // counter-clockwise seen from below: a ray from above meets the back face
        let down = Ray::new(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y, 100.0);
        let hit = bvh.ray_nearest(&down).unwrap();
        assert_eq!(hit.tri, 0);
        assert!((hit.t - 5.0).abs() < 1e-5);
        assert!((hit.u - 0.25).abs() < 1e-5 && (hit.v - 0.5).abs() < 1e-5);
        assert!(bvh.ray_nearest(&Ray { cull_back: true, ..down }).is_none());
        let up = Ray { cull_back: true, ..Ray::new(Vec3::new(0.0, -5.0, 0.0), Vec3::Y, 100.0) };
        assert!(bvh.ray_nearest(&up).is_some());
        // too short, pointing away, beside
        assert!(bvh.ray_nearest(&Ray::new(down.origin, Vec3::NEG_Y, 4.0)).is_none());
        assert!(bvh.ray_any(&Ray::new(down.origin, Vec3::Y, 100.0)).is_none());
        assert!(bvh.ray_any(&Ray::new(Vec3::new(3.0, 5.0, 0.0), Vec3::NEG_Y, 100.0)).is_none());
        // a direction that is not of unit length: the distance is in its multiples
        let long = bvh.ray_nearest(&Ray::new(down.origin, Vec3::new(0.0, -10.0, 0.0), 1.0)).unwrap();
        assert!((long.t - 0.5).abs() < 1e-6);

        let empty = Bvh::build(&verts, &[]).unwrap();
        assert!(empty.is_empty() && empty.bounds().is_none());
        assert!(empty.ray_nearest(&down).is_none());
        let mut out = Vec::new();
        empty.query_box(&Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0)), &mut out);
        assert!(out.is_empty());
    }

    /// The final review's minimal case (`bvhedge`): a vertical ray through a triangle's corner,
    /// its origin on the box's far face along a still axis. The triangle test hit it at 5 and
    /// the walk lost the box (`0 * 1e30 = 0` made the far distance 0).
    #[test]
    fn a_still_axis_ray_on_a_box_face_is_inside_the_slab() {
        let verts = [Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 0.0, -1.0), Vec3::new(0.0, 0.0, 1.0)];
        let bvh = Bvh::build(&verts, &[[0, 1, 2]]).unwrap();
        for (origin, dir) in [
            (Vec3::new(1.0, 5.0, -1.0), Vec3::NEG_Y),
            (Vec3::new(-1.0, 5.0, -1.0), Vec3::NEG_Y),
            (Vec3::new(1.0, 5.0, -1.0), Vec3::new(-0.0, -1.0, -0.0)),
            (Vec3::new(0.0, -5.0, 1.0), Vec3::Y),
            (Vec3::new(1.0, -5.0, -1.0), Vec3::new(0.0, 1.0, -0.0)),
        ] {
            let ray = Ray::new(origin, dir, 100.0);
            let tri = [verts[0], verts[1], verts[2]];
            let own = ray_triangle(&ray, &tri).map(|(t, _, _)| t);
            assert_eq!(own, Some(5.0), "{origin:?} {dir:?}");
            assert_eq!(bvh.ray_nearest(&ray).map(|h| h.t), Some(5.0), "{origin:?} {dir:?}");
            assert!(bvh.ray_any(&ray).is_some());
            let mut all = Vec::new();
            bvh.ray_all(&ray, &mut all);
            assert_eq!(all.len(), 1);
        }
        // outside the slab of a still axis by the least step: no box, no hit
        let beside = Ray::new(Vec3::new(1.0f32.next_up(), 5.0, -1.0), Vec3::NEG_Y, 100.0);
        assert!(bvh.ray_nearest(&beside).is_none());
    }

    /// Rays along the axes through every corner and every edge midpoint of a soup on a grid (the
    /// rays of a vertical probe, an AI map node's height and a hemisphere sample straight up):
    /// the walks answer as the triangles alone do, and a triangle `ray_triangle` hits is one
    /// the walk reports.
    #[test]
    fn axis_rays_through_corners_and_edges_agree_with_brute_force() {
        let mut rng = Rng(0x51_7CC1_B727_220A);
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        let grid = |r: &mut Rng| {
            Vec3::new((r.next() % 21) as f32, (r.next() % 9) as f32, (r.next() % 21) as f32) * 0.5
        };
        for _ in 0..400 {
            let base = verts.len() as u32;
            verts.extend([grid(&mut rng), grid(&mut rng), grid(&mut rng)]);
            tris.push([base, base + 1, base + 2]);
        }
        let bvh = Bvh::build(&verts, &tris).unwrap();
        let mut points: Vec<Vec3> = verts.clone();
        for t in &tris {
            let [a, b, c] = corners(&verts, t);
            points.extend([(a + b) * 0.5, (b + c) * 0.5, (c + a) * 0.5]);
        }
        let dirs = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
        let (mut hits, mut cases) = (0, 0);
        for p in &points {
            for d in dirs {
                let ray = Ray::new(*p - d * 20.0, d, 100.0);
                let expect = brute_nearest(&verts, &tris, &ray);
                assert_eq!(bvh.ray_nearest(&ray).map(|h| h.t), expect.map(|h| h.t), "{p:?} along {d:?}");
                assert_eq!(bvh.ray_any(&ray).is_some(), expect.is_some(), "{p:?} along {d:?}, any");
                let mut all = Vec::new();
                bvh.ray_all(&ray, &mut all);
                for (id, t) in tris.iter().enumerate() {
                    if ray_triangle(&ray, &corners(&verts, t)).is_some() {
                        assert!(all.iter().any(|h| h.tri == id as u32), "{p:?} along {d:?}: triangle {id}");
                    }
                }
                cases += 1;
                hits += usize::from(expect.is_some());
            }
        }
        assert!(hits > cases / 4, "only {hits} of {cases} rays hit anything: the test proves little");
    }

    #[test]
    fn bad_input_is_an_error_or_left_out() {
        let verts = [Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(f32::NAN, 0.0, 0.0)];
        assert!(Bvh::build(&verts, &[[0, 1, 4]]).is_err());
        let bvh = Bvh::build(&verts, &[[0, 1, 3], [0, 1, 2]]).unwrap();
        assert_eq!((bvh.len(), bvh.skipped()), (1, 1));
        let hit = bvh.ray_nearest(&Ray::new(Vec3::new(0.2, 1.0, 0.2), Vec3::NEG_Y, 10.0)).unwrap();
        assert_eq!(hit.tri, 1);
    }

    #[test]
    fn many_identical_triangles_do_not_break_the_build() {
        let verts = [Vec3::ZERO, Vec3::X, Vec3::Z];
        let tris = vec![[0u32, 1, 2]; 1000];
        let bvh = Bvh::build(&verts, &tris).unwrap();
        let (_, depth) = bvh.stats();
        assert!(depth <= 12, "depth {depth}");
        let mut all = Vec::new();
        bvh.ray_all(&Ray::new(Vec3::new(0.2, 1.0, 0.2), Vec3::NEG_Y, 10.0), &mut all);
        assert_eq!(all.len(), 1000);
    }
}
