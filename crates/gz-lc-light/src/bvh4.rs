//! The ray model's tree: a four-wide bounding volume hierarchy walked one ray at a time, four boxes or
//! four triangles per step in SSE lanes (`glam`'s `Vec4`).
//!
//! It answers what the scalar tree of `gz_fmt_level` answers for a shadow ray, bit for bit:
//!
//! - **The same triangle test.** A block holds up to four triangles as their first corner and two
//!   edges, and tests them with Moeller-Trumbore in exactly the order of operations of `gz_fmt_level`'s
//!   `Probe::triangle` per lane (the edges are the same subtractions, done once at the build; the cross
//!   and dot products sum in the same order; no fused multiply-add on the editor's baseline CPU), so a
//!   triangle gives the same `(t, u, v)` here and there.
//! - **The same triangles reached.** The scalar walk reaches a triangle when it enters the box of every
//!   node above it; its box test is monotonic in the box and a node's box holds its children's, so that
//!   is when it enters the box of the triangle's leaf. The binary tree here is built by the scalar
//!   tree's own builder (the same binned surface area heuristic and splits, deterministic on any number
//!   of threads), so the leaves are the same; its nodes are collapsed four by four and a subtree of four
//!   shadow casters or fewer becomes one block. A box here is entered by the formula of `Probe::enter`,
//!   lane by lane, and every box holds the leaves below it, so every triangle the scalar walk reaches is
//!   tested; a hit in a block that is more than one leaf counts only when the triangle's own leaf box
//!   is entered too (that test is made on hits alone). So even the rare hit the scalar walk loses on a
//!   box border (a triangle met on its very corner, the box's distances off by the rounding of a near
//!   cancellation) is lost here: the answers are the same, losses included.
//! - **Only what can cast a shadow is tested.** A triangle that casts none stays in the tree's boxes
//!   (they decide the leaves) but out of its blocks; the tracer passed it through anyway.
//!
//! The face a triangle stands for and whether it is opaque lie in the block beside the corners, so a
//! walk reads no second array until it hits something. Children are visited nearest first, so an opaque
//! hit ends a walk early.

use std::ops::ControlFlow;

use glam::{BVec3A, BVec4A, Vec3, Vec3A, Vec4};
use gz_fmt_level::{Ray, RayHit};
use rayon::prelude::*;
use smallvec::SmallVec;

use crate::ray::RayFace;

/// Children of a node and triangles of a block: one SSE lane each.
const LANES: usize = 4;
/// Triangles a binary leaf may hold when another split would not pay (the scalar tree's).
const LEAF_MAX: usize = 4;
const BINS: usize = 16;
/// A subtree with fewer triangles is built on the thread that reached it.
const PAR_BUILD: usize = 4096;
/// A node with more triangles bins them on several threads.
const PAR_BIN: usize = 1 << 16;
/// Past this depth the heuristic gives way to median splits, which bound the depth.
const SAH_DEPTH: u32 = 48;
/// A direction component below this is no movement along its axis (`gz_fmt_level`'s).
const STILL: f32 = 1e-30;
/// How far a box's entry and exit distances are widened (`gz_fmt_level`'s).
const WIDEN: f32 = 4e-7;
/// The walk's stack: three slots a level are enough for the deepest tree the builder makes
/// (`SAH_DEPTH` and then halving); a deeper one would go on in a vector.
const STACK: usize = 128;

/// A child slot that holds nothing (its box is empty too).
const EMPTY: u32 = u32::MAX;
/// The bit of a child that is a block: the rest is its index.
const LEAF: u32 = 1 << 31;

/// Four children: their boxes, `b[0]` the low corners and `b[1]` the high ones by axis, lane `i` for
/// child `i`. An empty slot's box is a point at infinity, which no ray of finite length enters. Two
/// whole cache lines.
#[derive(Clone, Copy, Debug)]
#[repr(C, align(64))]
struct Node4 {
    b: [[Vec4; 3]; 2],
    child: [u32; LANES],
}

/// Up to four triangles: corner 0 and the edges to corners 1 and 2, by axis. An empty lane is a
/// triangle of no area at the origin, which the test always refuses. Three whole cache lines.
#[derive(Clone, Copy, Debug)]
#[repr(C, align(64))]
struct Block {
    v0: [Vec4; 3],
    e1: [Vec4; 3],
    e2: [Vec4; 3],
    /// The triangle's index in the model.
    tri: [u32; LANES],
    /// The level face it stands for (valid where `FACE` is set).
    face: [u32; LANES],
    /// Per lane: `OPAQUE << lane`, `FACE << lane`; `GATED` when the block is more than one leaf.
    flags: u32,
}

const OPAQUE: u32 = 1;
const FACE: u32 = 1 << LANES;
const GATED: u32 = 1 << (2 * LANES);

impl Block {
    const EMPTY: Block = Block {
        v0: [Vec4::ZERO; 3],
        e1: [Vec4::ZERO; 3],
        e2: [Vec4::ZERO; 3],
        tri: [u32::MAX; LANES],
        face: [0; LANES],
        flags: 0,
    };
}

/// The tree. See the module text.
#[derive(Clone, Debug)]
pub(crate) struct Bvh4 {
    nodes: Vec<Node4>,
    blocks: Vec<Block>,
    /// Per block lane (`4 * block + lane`): the box of the triangle's leaf in the binary tree, for the
    /// hits of a gated block.
    gates: Vec<[[Vec3; 2]; LANES]>,
    /// The root child (`EMPTY` for a tree of nothing).
    root: u32,
}

/// What a shadow ray's walk met.
pub(crate) enum Walk {
    /// Nothing that stops the light; the transparent hits were appended.
    Through,
    /// An opaque triangle (its index) stopped the ray.
    Stopped(u32),
}

// ------------------------------------------------------------------------------------------ the build

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

/// The bins of the three axes a triangle's doubled centre falls into.
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

/// Orders the triangles of a node for a split and says where the right half starts; `None` when the
/// node is to be a leaf. The answer is always inside `1..len`. The scalar tree's, word for word: the
/// two trees must have the same leaves.
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

/// A node of the binary tree the four-wide one is collapsed from.
#[derive(Clone, Copy, Debug)]
struct BNode {
    min: Vec3,
    /// A leaf: its first triangle. An inner node: its right child (the left one follows it).
    a: u32,
    max: Vec3,
    /// Triangles of a leaf; 0 for an inner node.
    count: u32,
}

impl BNode {
    const EMPTY: BNode = BNode { min: Vec3::ZERO, a: 0, max: Vec3::ZERO, count: 0 };

    fn area(&self) -> f32 {
        let d = self.max - self.min;
        d.x * d.y + d.y * d.z + d.z * d.x
    }
}

/// Builds the binary subtree of `prims` into `nodes` (`2 * len - 1` slots from `node_at` of the whole
/// array: the root, the left subtree, the right subtree).
/// `out.spans[i]` (beside `nodes[i]`) becomes the triangles below the node, `out.leaves[j]` (beside
/// `prims[j]`) the leaf that holds the triangle.
fn build_range(
    nodes: &mut [BNode],
    prims: &mut [Prim],
    out: Spans<'_>,
    node_at: u32,
    prim_at: u32,
    depth: u32,
) {
    let n = prims.len();
    let Some((root, rest)) = nodes.split_first_mut() else { return };
    let Some((span, spans)) = out.spans.split_first_mut() else { return };
    *span = (prim_at, prim_at + n as u32);
    let (all, lo, hi) = bounds(prims);
    root.min = all.min.into();
    root.max = all.max.into();
    root.a = prim_at;
    root.count = n as u32;
    let Some(mid) = split(prims, &all, lo, hi, depth) else {
        out.leaves.iter_mut().for_each(|l| *l = node_at);
        return;
    };
    let Some((left_nodes, right_nodes)) = rest.split_at_mut_checked(2 * mid - 1) else { return };
    let Some((left_spans, right_spans)) = spans.split_at_mut_checked(2 * mid - 1) else { return };
    let Some((left, right)) = prims.split_at_mut_checked(mid) else { return };
    let Some((left_leaves, right_leaves)) = out.leaves.split_at_mut_checked(mid) else { return };
    let right_at = node_at + 2 * mid as u32;
    root.a = right_at;
    root.count = 0;
    let right_prim_at = prim_at + mid as u32;
    let lo = Spans { spans: left_spans, leaves: left_leaves };
    let hi = Spans { spans: right_spans, leaves: right_leaves };
    if n >= PAR_BUILD {
        rayon::join(
            || build_range(left_nodes, left, lo, node_at + 1, prim_at, depth + 1),
            || build_range(right_nodes, right, hi, right_at, right_prim_at, depth + 1),
        );
    } else {
        build_range(left_nodes, left, lo, node_at + 1, prim_at, depth + 1);
        build_range(right_nodes, right, hi, right_at, right_prim_at, depth + 1);
    }
}

/// What [`build_range`] records beside the tree.
struct Spans<'a> {
    spans: &'a mut [(u32, u32)],
    leaves: &'a mut [u32],
}

/// What the collapse reads: the binary tree, the triangles in its leaf order, the model.
struct Source<'a> {
    sparse: &'a [BNode],
    /// Per binary node: the triangles below it, `order[start..end]`.
    span: &'a [(u32, u32)],
    /// Shadow casters among `order[..i]`.
    casting: &'a [u32],
    /// Per position in `order`: the binary leaf holding it.
    leaf_of: &'a [u32],
    order: &'a [u32],
    tris: &'a [[Vec3; 3]],
    faces: &'a [RayFace],
}

impl Source<'_> {
    fn casters(&self, at: u32) -> u32 {
        let (s, e) = self.span.get(at as usize).copied().unwrap_or((0, 0));
        self.casting.get(e as usize).copied().unwrap_or(0)
            - self.casting.get(s as usize).copied().unwrap_or(0)
    }
}

/// The units of the four-wide tree below binary node `at`: every subtree of four shadow casters or
/// fewer (and at least one), with its box; `kinds[unit]` is the subtree's node and whether its hits
/// need their own leaf's box (it is more than one leaf, or the root).
fn unit_list(src: &Source<'_>, at: u32, units: &mut Vec<Prim>, kinds: &mut Vec<(u32, bool)>) {
    let mut stack = vec![at];
    while let Some(at) = stack.pop() {
        let Some(&node) = src.sparse.get(at as usize) else { continue };
        let casters = src.casters(at);
        if casters == 0 {
            continue;
        }
        if casters as usize <= LANES {
            units.push(Prim { min: node.min.into(), max: node.max.into(), id: kinds.len() as u32 });
            kinds.push((at, node.count == 0 || at == 0));
            continue;
        }
        stack.push(node.a);
        stack.push(at + 1);
    }
}

/// The units' tree and what its leaves stand for.
struct Plan<'a> {
    tree: &'a [BNode],
    units: &'a [Prim],
}

/// Builds the binary tree of `units` into `nodes` (`2 * len - 1` slots from `node_at`), one unit a
/// leaf, the splits [`split_units`]'s.
fn unit_tree(nodes: &mut [BNode], units: &mut [Prim], node_at: u32, first: u32, depth: u32) {
    let n = units.len();
    let Some((root, rest)) = nodes.split_first_mut() else { return };
    let b = unit_box(units);
    root.min = b.min.into();
    root.max = b.max.into();
    root.a = first;
    root.count = n as u32;
    if n < 2 {
        return;
    }
    let mid = split_units(units, depth);
    let Some((left_nodes, right_nodes)) = rest.split_at_mut_checked(2 * mid - 1) else { return };
    let Some((left, right)) = units.split_at_mut_checked(mid) else { return };
    let right_at = node_at + 2 * mid as u32;
    root.a = right_at;
    root.count = 0;
    let right_first = first + mid as u32;
    if n >= PAR_BUILD {
        rayon::join(
            || unit_tree(left_nodes, left, node_at + 1, first, depth + 1),
            || unit_tree(right_nodes, right, right_at, right_first, depth + 1),
        );
    } else {
        unit_tree(left_nodes, left, node_at + 1, first, depth + 1);
        unit_tree(right_nodes, right, right_at, right_first, depth + 1);
    }
}

/// The box of some units.
fn unit_box(units: &[Prim]) -> Bin {
    let mut b = Bin::EMPTY;
    for u in units {
        b.add(u);
    }
    b
}

/// Orders units for a cut in two and answers where the second part starts (inside `1..len` for
/// two units or more): the cheapest plane of the surface area heuristic, the median of the widest
/// axis when there is none or the tree grows deep.
fn split_units(units: &mut [Prim], depth: u32) -> usize {
    let n = units.len();
    let (mut lo, mut hi) = (Vec3A::INFINITY, Vec3A::NEG_INFINITY);
    for u in units.iter() {
        let c = u.min + u.max;
        lo = lo.min(c);
        hi = hi.max(c);
    }
    let extent = hi - lo;
    let flat =
        extent.max_element().partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) || !extent.is_finite();
    let median = |units: &mut [Prim]| {
        let m = median_split(units, extent);
        if m > 0 && m < n { m } else { n / 2 }
    };
    if flat || depth > SAH_DEPTH || n < 2 {
        return median(units);
    }
    let scale = Vec3A::select(extent.cmpgt(Vec3A::ZERO), Vec3A::splat(BINS as f32) / extent, Vec3A::ZERO);
    let bins = fill_bins(units, lo, scale);
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
    if let Some((_, axis, first_right)) = best {
        let mid = partition(units, |p| bin_of(p, lo, scale)[axis] < first_right);
        if mid > 0 && mid < n {
            return mid;
        }
    }
    median(units)
}

/// The block of the shadow casters below binary node `at` (four at most), with their leaves' boxes.
fn make_block(src: &Source<'_>, at: u32, gated: bool) -> (Block, [[Vec3; 2]; LANES]) {
    let (s, e) = src.span.get(at as usize).copied().unwrap_or((0, 0));
    let mut b = Block::EMPTY;
    let mut v0 = [[0.0f32; LANES]; 3];
    let mut e1 = [[0.0f32; LANES]; 3];
    let mut e2 = [[0.0f32; LANES]; 3];
    let mut gates = [[Vec3::ZERO; 2]; LANES];
    let mut lane = 0;
    for pos in s..e {
        let Some(&id) = src.order.get(pos as usize) else { continue };
        let (Some(t), Some(f)) = (src.tris.get(id as usize), src.faces.get(id as usize)) else {
            continue;
        };
        if !f.cast_shadow || lane >= LANES {
            continue;
        }
        // the subtractions of `Probe::triangle`, once
        let a = Vec3A::from(t[0]);
        let d1 = Vec3A::from(t[1]) - a;
        let d2 = Vec3A::from(t[2]) - a;
        for axis in 0..3 {
            v0[axis][lane] = a[axis];
            e1[axis][lane] = d1[axis];
            e2[axis][lane] = d2[axis];
        }
        b.tri[lane] = id;
        if f.opaque {
            b.flags |= OPAQUE << lane;
        }
        if let Some(face) = f.face {
            b.face[lane] = face.0;
            b.flags |= FACE << lane;
        }
        let leaf = src.leaf_of.get(pos as usize).and_then(|&l| src.sparse.get(l as usize));
        gates[lane] = leaf.map_or([Vec3::ZERO; 2], |l| [l.min, l.max]);
        lane += 1;
    }
    if gated {
        b.flags |= GATED;
    }
    b.v0 = v0.map(Vec4::from_array);
    b.e1 = e1.map(Vec4::from_array);
    b.e2 = e2.map(Vec4::from_array);
    (b, gates)
}

/// A box no ray of finite length enters: a point at infinity.
const NOWHERE: f32 = f32::INFINITY;

impl Bvh4 {
    /// The tree of a model's triangles: `faces[i]` describes `tris[i]`. Runs on the caller's pool.
    pub(crate) fn build(tris: &[[Vec3; 3]], faces: &[RayFace]) -> Self {
        let empty = || Self { nodes: Vec::new(), blocks: Vec::new(), gates: Vec::new(), root: EMPTY };
        // the scalar tree leaves out what is not finite, and no more
        let mut prims: Vec<Prim> = tris
            .par_iter()
            .with_min_len(4096)
            .enumerate()
            .filter_map(|(id, t)| {
                if !t.iter().all(|c| c.is_finite()) {
                    return None;
                }
                let (a, b, c) = (Vec3A::from(t[0]), Vec3A::from(t[1]), Vec3A::from(t[2]));
                Some(Prim { min: a.min(b).min(c), max: a.max(b).max(c), id: id as u32 })
            })
            .collect();
        if prims.is_empty() || prims.len() >= (LEAF / 2) as usize {
            return empty();
        }
        let mut sparse = vec![BNode::EMPTY; 2 * prims.len() - 1];
        let mut span = vec![(0u32, 0u32); sparse.len()];
        let mut leaf_of = vec![0u32; prims.len()];
        build_range(&mut sparse, &mut prims, Spans { spans: &mut span, leaves: &mut leaf_of }, 0, 0, 0);
        let order: Vec<u32> = prims.iter().map(|p| p.id).collect();
        drop(prims);
        let mut casting = Vec::with_capacity(order.len() + 1);
        casting.push(0u32);
        let mut acc = 0u32;
        for &id in &order {
            acc += u32::from(faces.get(id as usize).is_some_and(|f| f.cast_shadow));
            casting.push(acc);
        }
        let src = Source {
            sparse: &sparse,
            span: &span,
            casting: &casting,
            leaf_of: &leaf_of,
            order: &order,
            tris,
            faces,
        };
        let mut out = Self {
            nodes: Vec::with_capacity(order.len() / 8),
            blocks: Vec::with_capacity(order.len() / 3),
            gates: Vec::new(),
            root: EMPTY,
        };
        // the blocks to be: the subtrees of four shadow casters or fewer, as units of a tree of their own
        let mut units = Vec::new();
        let mut kinds = Vec::new();
        unit_list(&src, 0, &mut units, &mut kinds);
        if units.is_empty() {
            return out;
        }
        (out.blocks, out.gates) =
            kinds.par_iter().with_min_len(1024).map(|&(at, gated)| make_block(&src, at, gated)).unzip();
        // a binary tree over them, built on the caller's pool, then collapsed four by four
        let mut tree = vec![BNode::EMPTY; 2 * units.len() - 1];
        unit_tree(&mut tree, &mut units, 0, 0, 0);
        let plan = Plan { tree: &tree, units: &units };
        let mut order = Vec::with_capacity(units.len());
        out.root = out.collapse(&plan, 0, &mut order);
        let (blocks, gates) = (&out.blocks, &out.gates);
        let (b, g): (Vec<Block>, Vec<[[Vec3; 2]; LANES]>) = order
            .par_iter()
            .with_min_len(4096)
            .map(|&u| {
                (
                    blocks.get(u as usize).copied().unwrap_or(Block::EMPTY),
                    gates.get(u as usize).copied().unwrap_or([[Vec3::ZERO; 2]; LANES]),
                )
            })
            .unzip();
        (out.blocks, out.gates) = (b, g);
        out
    }

    /// The four-wide node of a node of the units' tree, or the block of a unit: its children are the
    /// binary children, the one with the largest surface opened while there is room.
    /// The blocks are numbered in the order the walk down the tree meets them (`order[block]` is the
    /// unit), so that a subtree's blocks lie together.
    fn collapse(&mut self, plan: &Plan<'_>, at: u32, order: &mut Vec<u32>) -> u32 {
        let Some(&node) = plan.tree.get(at as usize) else { return EMPTY };
        if node.count > 0 {
            let Some(u) = plan.units.get(node.a as usize) else { return EMPTY };
            order.push(u.id);
            return (order.len() - 1) as u32 | LEAF;
        }
        let mut kids: SmallVec<[u32; LANES]> = SmallVec::from_slice(&[at + 1, node.a]);
        while kids.len() < LANES {
            let mut open: Option<(usize, f32)> = None;
            for (i, &k) in kids.iter().enumerate() {
                if let Some(n) = plan.tree.get(k as usize)
                    && n.count == 0
                    && open.is_none_or(|(_, a)| n.area() > a)
                {
                    open = Some((i, n.area()));
                }
            }
            let Some((i, _)) = open else { break };
            let Some(&k) = kids.get(i) else { break };
            let Some(n) = plan.tree.get(k as usize) else { break };
            if let Some(slot) = kids.get_mut(i) {
                *slot = k + 1;
            }
            kids.insert(i + 1, n.a);
        }
        let id = self.nodes.len();
        self.nodes.push(Node4 { b: [[Vec4::splat(NOWHERE); 3]; 2], child: [EMPTY; LANES] });
        let mut lo = [[NOWHERE; LANES]; 3];
        let mut hi = [[NOWHERE; LANES]; 3];
        let mut child = [EMPTY; LANES];
        for (lane, &k) in kids.iter().enumerate().take(LANES) {
            let Some(n) = plan.tree.get(k as usize) else { continue };
            for axis in 0..3 {
                lo[axis][lane] = n.min[axis];
                hi[axis][lane] = n.max[axis];
            }
            child[lane] = self.collapse(plan, k, order);
        }
        if let Some(slot) = self.nodes.get_mut(id) {
            *slot = Node4 { b: [lo.map(Vec4::from_array), hi.map(Vec4::from_array)], child };
        }
        id as u32
    }

    /// Shadow casters in the tree.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.blocks.iter().map(|b| b.tri.iter().filter(|&&t| t != u32::MAX).count()).sum()
    }

    // ---------------------------------------------------------------------------------------- the walk

    /// Walks a shadow ray: every triangle the scalar walk reaches and hits within `0..=t_max` that is
    /// not of the probe's skipped face, until an opaque one ([`Walk::Stopped`]); the transparent ones
    /// are appended to `hits` as they are met.
    pub(crate) fn shadow(&self, p: &Probe, hits: &mut Vec<RayHit>) -> Walk {
        if self.root == EMPTY {
            return Walk::Through;
        }
        if p.any_still { self.walk::<true>(p, hits) } else { self.walk::<false>(p, hits) }
    }

    fn walk<const STILL_AXES: bool>(&self, p: &Probe, hits: &mut Vec<RayHit>) -> Walk {
        let mut stack = Stack::new();
        let mut at = self.root;
        loop {
            if at & LEAF != 0 {
                let i = (at & !LEAF) as usize;
                if let Some(b) = self.blocks.get(i)
                    && let ControlFlow::Break(tri) = self.block_hits(p, b, i, hits)
                {
                    return Walk::Stopped(tri);
                }
            } else if let Some(n) = self.nodes.get(at as usize) {
                let (near, enter) = p.boxes::<STILL_AXES>(n);
                let bits = enter.bitmask();
                if bits != 0 {
                    // the nearest child is visited next, the others wait on the stack nearest on top
                    let first = bits.trailing_zeros() as usize;
                    let rest = bits & (bits - 1);
                    if rest == 0 {
                        at = n.child[first];
                        continue;
                    }
                    let near = near.to_array();
                    if rest & (rest - 1) == 0 {
                        let second = rest.trailing_zeros() as usize;
                        let (a, b) =
                            if near[second] < near[first] { (second, first) } else { (first, second) };
                        stack.push(n.child[b]);
                        at = n.child[a];
                        continue;
                    }
                    let mut order = [(0.0f32, 0u32); LANES];
                    let mut k = 0;
                    let mut left = bits;
                    while left != 0 {
                        let lane = left.trailing_zeros() as usize;
                        left &= left - 1;
                        order[k] = (near[lane], n.child[lane]);
                        k += 1;
                    }
                    for i in 1..k {
                        let mut j = i;
                        while j > 0 && order[j - 1].0 < order[j].0 {
                            order.swap(j - 1, j);
                            j -= 1;
                        }
                    }
                    for &(_, c) in &order[..k - 1] {
                        stack.push(c);
                    }
                    at = order[k - 1].1;
                    continue;
                }
            }
            match stack.pop() {
                Some(c) => at = c,
                None => return Walk::Through,
            }
        }
    }

    /// The block's triangles against the ray, then its hits: a skipped face is passed, a gated lane
    /// whose leaf box the ray does not enter too, an opaque one breaks with its triangle, a transparent
    /// one is appended.
    #[inline(always)]
    fn block_hits(&self, p: &Probe, b: &Block, block: usize, hits: &mut Vec<RayHit>) -> ControlFlow<u32> {
        let Some((mut bits, t, u, v)) = p.block(b) else { return ControlFlow::Continue(()) };
        while bits != 0 {
            let lane = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            if let Some(skip) = p.skip
                && b.flags & (FACE << lane) != 0
                && b.face[lane] == skip
            {
                continue;
            }
            if b.flags & GATED != 0
                && self.gates.get(block).and_then(|g| g.get(lane)).is_none_or(|g| !p.one.enters(g[0], g[1]))
            {
                continue;
            }
            let tri = b.tri[lane];
            if b.flags & (OPAQUE << lane) != 0 {
                return ControlFlow::Break(tri);
            }
            hits.push(RayHit { tri, t: t[lane], u: u[lane], v: v[lane] });
        }
        ControlFlow::Continue(())
    }
}

/// The walk's stack: a fixed array, and a vector past it (never needed by a tree of this builder).
struct Stack {
    slots: [u32; STACK],
    len: usize,
    more: Vec<u32>,
}

impl Stack {
    #[inline(always)]
    fn new() -> Self {
        Self { slots: [0; STACK], len: 0, more: Vec::new() }
    }

    #[inline(always)]
    fn push(&mut self, v: u32) {
        if let Some(slot) = self.slots.get_mut(self.len) {
            *slot = v;
            self.len += 1;
        } else {
            self.more.push(v);
        }
    }

    #[inline(always)]
    fn pop(&mut self) -> Option<u32> {
        if let Some(v) = self.more.pop() {
            return Some(v);
        }
        self.len = self.len.checked_sub(1)?;
        self.slots.get(self.len).copied()
    }
}

/// The lanes of a block a ray hits (a bit each) and every lane's `t`, `u` and `v`.
type LaneHits = (u32, [f32; LANES], [f32; LANES], [f32; LANES]);

/// A ray made ready for the scalar tests: `gz_fmt_level`'s probe.
#[derive(Clone, Copy)]
pub(crate) struct Scalar {
    origin: Vec3A,
    dir: Vec3A,
    inv: Vec3A,
    still: BVec3A,
    t_max: f32,
    cull: bool,
}

/// A ray made ready for the tree's tests: its four-wide and its scalar form.
pub(crate) struct Probe {
    one: Scalar,
    o: [Vec4; 3],
    d: [Vec4; 3],
    inv: [Vec4; 3],
    /// Per axis: which corner of a box the ray enters it by (0 low, 1 high).
    side: [usize; 3],
    /// Per axis: the ray does not move along it.
    still: [BVec4A; 3],
    any_still: bool,
    t_max: Vec4,
    skip: Option<u32>,
}

impl Scalar {
    /// `Probe::new` of `gz_fmt_level`: a still axis takes no reciprocal.
    #[inline]
    pub(crate) fn new(ray: &Ray) -> Self {
        let dir = Vec3A::from(ray.dir);
        let still = dir.abs().cmplt(Vec3A::splat(STILL));
        let inv = Vec3A::ONE / Vec3A::select(still, Vec3A::ONE, dir);
        Self { origin: Vec3A::from(ray.origin), dir, inv, still, t_max: ray.t_max, cull: ray.cull_back }
    }

    /// `Probe::enter` of `gz_fmt_level` for one box, word for word.
    #[inline]
    fn enters(&self, lo: Vec3, hi: Vec3) -> bool {
        let (lo, hi) = (Vec3A::from(lo), Vec3A::from(hi));
        if (self.still & (self.origin.cmplt(lo) | self.origin.cmpgt(hi))).any() {
            return false;
        }
        let t0 = (lo - self.origin) * self.inv;
        let t1 = (hi - self.origin) * self.inv;
        let near = Vec3A::select(self.still, Vec3A::NEG_INFINITY, t0.min(t1)).max_element();
        let far = Vec3A::select(self.still, Vec3A::INFINITY, t0.max(t1)).min_element();
        if far < 0.0 {
            return false;
        }
        let near = near.max(0.0) * (1.0 - WIDEN);
        let far = (far * (1.0 + WIDEN)).min(self.t_max);
        near <= far
    }

    /// `ray_triangle` of `gz_fmt_level`, word for word: the triangle's own box, then Moeller-Trumbore;
    /// the distance of a hit within `0..=t_max`.
    #[inline]
    pub(crate) fn own(&self, tri: &[Vec3; 3]) -> Option<f32> {
        let (a, b, c) = (Vec3A::from(tri[0]), Vec3A::from(tri[1]), Vec3A::from(tri[2]));
        if !self.enters(a.min(b).min(c).into(), a.max(b).max(c).into()) {
            return None;
        }
        let e1 = b - a;
        let e2 = c - a;
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
        let s = self.origin - a;
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
        (t >= 0.0 && t <= self.t_max).then_some(t)
    }
}

impl Probe {
    /// `skip`: the level face whose triangles the ray passes.
    #[inline]
    pub(crate) fn new(one: Scalar, skip: Option<u32>) -> Self {
        let (o, d, inv, still) = (one.origin, one.dir, one.inv, one.still);
        let s = [still.test(0), still.test(1), still.test(2)];
        let side = |x: f32| usize::from(x < 0.0);
        Self {
            one,
            o: [Vec4::splat(o.x), Vec4::splat(o.y), Vec4::splat(o.z)],
            d: [Vec4::splat(d.x), Vec4::splat(d.y), Vec4::splat(d.z)],
            inv: [Vec4::splat(inv.x), Vec4::splat(inv.y), Vec4::splat(inv.z)],
            side: [side(inv.x), side(inv.y), side(inv.z)],
            still: s.map(BVec4A::splat),
            any_still: s.iter().any(|&x| x),
            t_max: Vec4::splat(one.t_max),
            skip,
        }
    }

    /// `Probe::enter` of the four children: where the ray enters each box and which boxes it enters.
    /// A ray enters a slab by the corner its direction faces: the nearer of the two distances
    /// `Probe::enter` takes the smaller of (the reciprocal's sign orders them).
    #[inline(always)]
    fn boxes<const STILL_AXES: bool>(&self, n: &Node4) -> (Vec4, BVec4A) {
        let mut near = Vec4::NEG_INFINITY;
        let mut far = Vec4::INFINITY;
        let mut outside = BVec4A::splat(false);
        for axis in 0..3 {
            let s = self.side[axis];
            let t0 = (n.b[s][axis] - self.o[axis]) * self.inv[axis];
            let t1 = (n.b[1 - s][axis] - self.o[axis]) * self.inv[axis];
            if STILL_AXES {
                let still = self.still[axis];
                // along a still axis the ray is inside the slab for every distance when its origin
                // is, faces included, and never otherwise
                let o = self.o[axis];
                outside |= still & (o.cmplt(n.b[0][axis]) | o.cmpgt(n.b[1][axis]));
                near = near.max(Vec4::select(still, Vec4::NEG_INFINITY, t0));
                far = far.min(Vec4::select(still, Vec4::INFINITY, t1));
            } else {
                near = near.max(t0);
                far = far.min(t1);
            }
        }
        // widened by the rounding, so that a hit on the very border of a box is never lost; a box
        // behind the origin (`far < 0`, refused first by `Probe::enter`) stays refused: its widened far
        // is still below 0, and the near is not
        let near = near.max(Vec4::ZERO) * (1.0 - WIDEN);
        let far = (far * (1.0 + WIDEN)).min(self.t_max);
        let enter = near.cmple(far) & !outside;
        (near, enter)
    }

    /// `Probe::triangle` of the four triangles of a block: the lanes hit within `0..=t_max` and their
    /// `(t, u, v)`.
    #[inline(always)]
    fn block(&self, b: &Block) -> Option<LaneHits> {
        let [dx, dy, dz] = self.d;
        let [e1x, e1y, e1z] = b.e1;
        let [e2x, e2y, e2z] = b.e2;
        // p = d x e2
        let px = dy * e2z - dz * e2y;
        let py = dz * e2x - dx * e2z;
        let pz = dx * e2y - dy * e2x;
        let det = (e1x * px + e1y * py) + e1z * pz;
        let det_ok =
            if self.one.cull { !det.cmplt(Vec4::splat(1e-12)) } else { !det.abs().cmplt(Vec4::splat(1e-12)) };
        if det_ok.bitmask() == 0 {
            return None;
        }
        let inv = Vec4::ONE / det;
        let sx = self.o[0] - b.v0[0];
        let sy = self.o[1] - b.v0[1];
        let sz = self.o[2] - b.v0[2];
        let u = ((sx * px + sy * py) + sz * pz) * inv;
        let u_ok = u.cmpge(Vec4::ZERO) & u.cmple(Vec4::ONE);
        // q = s x e1
        let qx = sy * e1z - sz * e1y;
        let qy = sz * e1x - sx * e1z;
        let qz = sx * e1y - sy * e1x;
        let v = ((dx * qx + dy * qy) + dz * qz) * inv;
        let v_ok = !(v.cmplt(Vec4::ZERO) | (u + v).cmpgt(Vec4::ONE));
        let t = ((e2x * qx + e2y * qy) + e2z * qz) * inv;
        let t_ok = t.cmpge(Vec4::ZERO) & t.cmple(self.t_max);
        let bits = (det_ok & u_ok & v_ok & t_ok).bitmask();
        (bits != 0).then(|| (bits, t.to_array(), u.to_array(), v.to_array()))
    }
}
