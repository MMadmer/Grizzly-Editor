//! The lighter's own tree: a binary BVH by the binned surface area heuristic, built on the
//! caller's rayon pool, then collapsed into a 4-wide one whose nodes hold their children's boxes
//! side by side, each plane in 8 bits: the kernel is bound by the bytes it reads of the tree, and
//! a node is 64 of them.
//!
//! ```text
//! GPU node (4 x vec4<u32>)
//!   [0] the grid's corner (x, y, z as f32 bits), its steps' exponents (8 bits an axis)
//!   [1] min x, max x, min y, max y      a byte a child, in grid steps from the corner
//!   [2] min z, max z, -, -
//!   [3] the children: NONE, an inner node's index, or LEAF | (count - 1) << 28 | first triangle
//! ```
//!
//! A step is a power of two, so the kernel's `corner + q * step` is the CPU's to the bit, and a
//! child's quantized box holds its true box: the planes are rounded outwards and checked. The
//! build depends on the triangles alone, never on the number of threads.

use glam::Vec3;
use rayon::prelude::*;

/// No child.
pub(crate) const NONE: u32 = u32::MAX;
/// The slots of the kernel's stack. A walk may keep one node fewer: a push past the end lands in the last slot.
pub(crate) const STACK: u32 = 64;
/// Past this depth the heuristic gives way to median splits, which bound the depth.
const SAH_DEPTH: u32 = 32;
/// A child that is a leaf.
pub(crate) const LEAF: u32 = 1 << 31;
/// A leaf's first triangle is below this.
const LEAF_FIRST: u32 = 1 << 28;
/// A leaf the heuristic may choose holds this many triangles at most.
const MAX_LEAF: usize = 4;
/// A node with more triangles is built beside its sibling.
const PAR_BUILD: usize = 4096;
/// A node with more triangles bins them on several threads.
const PAR_BIN: usize = 1 << 16;
const BINS: usize = 32;

/// One GPU node.
pub(crate) type GpuNode = [[u32; 4]; 4];

#[derive(Clone, Copy, Debug)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}

impl Bounds {
    const EMPTY: Bounds = Bounds { min: Vec3::splat(f32::MAX), max: Vec3::splat(-f32::MAX) };

    fn grow(&mut self, b: &Bounds) {
        self.min = self.min.min(b.min);
        self.max = self.max.max(b.max);
    }

    fn grow_point(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    fn join(mut self, b: &Bounds) -> Bounds {
        self.grow(b);
        self
    }

    fn area(&self) -> f32 {
        let d = (self.max - self.min).max(Vec3::ZERO);
        d.x * d.y + d.y * d.z + d.z * d.x
    }
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Leaf { start: u32, count: u32 },
    Inner { left: u32, right: u32 },
}

#[derive(Clone, Copy, Debug)]
struct Node {
    b: Bounds,
    kind: Kind,
}

/// The built tree: the kernel's nodes and the triangles' order (`order[k]` is the input
/// triangle the kernel's triangle `k` is).
#[derive(Debug, Default)]
pub(crate) struct Tree {
    pub nodes: Vec<GpuNode>,
    pub order: Vec<u32>,
    /// The most nodes the kernel's stack holds during a walk.
    pub stack: u32,
}

struct Input<'a> {
    boxes: &'a [Bounds],
    centers: &'a [Vec3],
}

/// The tree over `tris`.
pub(crate) fn build(tris: &[[Vec3; 3]]) -> Tree {
    let boxes: Vec<Bounds> = tris
        .par_iter()
        .map(|t| Bounds { min: t[0].min(t[1]).min(t[2]), max: t[0].max(t[1]).max(t[2]) })
        .collect();
    let centers: Vec<Vec3> = boxes.par_iter().map(|b| (b.min + b.max) * 0.5).collect();
    let mut order: Vec<u32> = (0..tris.len() as u32).collect();
    let input = Input { boxes: &boxes, centers: &centers };
    if order.is_empty() {
        return Tree { nodes: vec![empty_node()], order, stack: 0 };
    }
    let (nodes, _) = build_par(&input, &mut order, 0, 0);
    let (nodes, stack) = flatten(&nodes).unwrap_or_default();
    // a tree whose leaves do not fit their words: the stack check refuses it
    let stack = if nodes.is_empty() { u32::MAX } else { stack };
    let nodes = if nodes.is_empty() { vec![empty_node()] } else { nodes };
    Tree { nodes, order, stack }
}

fn empty_node() -> GpuNode {
    [[0, 0, 0, 127 * 0x1_0101], [u32::MAX, 0, u32::MAX, 0], [u32::MAX, 0, 0, 0], [NONE; 4]]
}

/// The step of an axis's grid: the smallest power of two whose 255 steps from `lo` reach `hi`,
/// as the kernel rounds `lo + 255 * step`. Answers the biased exponent.
fn step_exponent(lo: f32, hi: f32) -> u32 {
    let ext = (hi - lo).max(0.0);
    let mut e: i32 = if ext > 0.0 { (ext / 255.0).log2().ceil() as i32 + 127 } else { 1 };
    e = e.clamp(1, 254);
    while e < 254 && lo + 255.0 * f32::from_bits((e as u32) << 23) < hi {
        e += 1;
    }
    e as u32
}

/// A plane in grid steps from `corner`, rounded outwards: `down` for a minimum.
fn quantize(v: f32, corner: f32, step: f32, down: bool) -> u32 {
    let q = ((v - corner) / step).clamp(0.0, 255.0);
    let mut q = if down { q.floor() } else { q.ceil() } as u32;
    // the division rounds too: the decoded plane is checked and moved a step out when it falls in
    if down {
        while q > 0 && corner + q as f32 * step > v {
            q -= 1;
        }
    } else {
        while q < 255 && corner + q as f32 * step < v {
            q += 1;
        }
    }
    q
}

/// A subtree in a vector of its own, its root first; child indices are the vector's.
fn build_par(input: &Input<'_>, idx: &mut [u32], offset: usize, depth: u32) -> (Vec<Node>, u32) {
    if idx.len() <= PAR_BUILD {
        let mut out = Vec::with_capacity(idx.len() / 2 + 1);
        let d = build_seq(input, &mut out, idx, offset, depth);
        return (out, d);
    }
    let (b, split) = choose(input, idx, depth);
    let Some(mid) = split else {
        let leaf = Node { b, kind: Kind::Leaf { start: offset as u32, count: idx.len() as u32 } };
        return (vec![leaf], depth);
    };
    let (l, r) = idx.split_at_mut(mid);
    let ((left, dl), (right, dr)) = rayon::join(
        || build_par(input, l, offset, depth + 1),
        || build_par(input, r, offset + mid, depth + 1),
    );
    let mut out = Vec::with_capacity(1 + left.len() + right.len());
    let (lroot, rroot) = (1u32, 1 + left.len() as u32);
    out.push(Node { b, kind: Kind::Inner { left: lroot, right: rroot } });
    for (sub, base) in [(left, lroot), (right, rroot)] {
        out.extend(sub.into_iter().map(|mut n| {
            if let Kind::Inner { left, right } = &mut n.kind {
                *left += base;
                *right += base;
            }
            n
        }));
    }
    (out, dl.max(dr))
}

/// A subtree appended to `out`; answers its depth.
fn build_seq(input: &Input<'_>, out: &mut Vec<Node>, idx: &mut [u32], offset: usize, depth: u32) -> u32 {
    let (b, split) = choose(input, idx, depth);
    let me = out.len();
    let Some(mid) = split else {
        out.push(Node { b, kind: Kind::Leaf { start: offset as u32, count: idx.len() as u32 } });
        return depth;
    };
    out.push(Node { b, kind: Kind::Inner { left: 0, right: 0 } });
    let (l, r) = idx.split_at_mut(mid);
    let left = out.len() as u32;
    let dl = build_seq(input, out, l, offset, depth + 1);
    let right = out.len() as u32;
    let dr = build_seq(input, out, r, offset + mid, depth + 1);
    if let Some(n) = out.get_mut(me) {
        n.kind = Kind::Inner { left, right };
    }
    dl.max(dr)
}

#[derive(Clone, Copy)]
struct Bin {
    b: Bounds,
    n: u32,
}

impl Bin {
    const EMPTY: Bin = Bin { b: Bounds::EMPTY, n: 0 };
}

fn bounds_of(input: &Input<'_>, idx: &[u32]) -> (Bounds, Bounds) {
    let one = |acc: (Bounds, Bounds), &i: &u32| {
        let (mut b, mut c) = acc;
        if let (Some(tb), Some(&tc)) = (input.boxes.get(i as usize), input.centers.get(i as usize)) {
            b.grow(tb);
            c.grow_point(tc);
        }
        (b, c)
    };
    let empty = (Bounds::EMPTY, Bounds::EMPTY);
    if idx.len() > PAR_BIN {
        idx.par_chunks(PAR_BIN / 4)
            .map(|c| c.iter().fold(empty, one))
            .reduce(|| empty, |a, b| (a.0.join(&b.0), a.1.join(&b.1)))
    } else {
        idx.iter().fold(empty, one)
    }
}

fn bin_of(c: f32, lo: f32, scale: f32) -> usize {
    (((c - lo) * scale) as usize).min(BINS - 1)
}

/// The node's box and, unless it is a leaf, where its triangles are cut (after partitioning
/// `idx`: the left child takes `idx[..mid]`).
fn choose(input: &Input<'_>, idx: &mut [u32], depth: u32) -> (Bounds, Option<usize>) {
    let n = idx.len();
    let (b, cb) = bounds_of(input, idx);
    if n <= 1 {
        return (b, None);
    }
    let extent = cb.max - cb.min;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    if depth >= SAH_DEPTH || extent.max_element() <= 0.0 {
        if n <= 4 {
            return (b, None);
        }
        return (b, Some(median(input, idx, axis)));
    }
    // the binned heuristic over the three axes
    let mut best: Option<(f32, usize, usize)> = None;
    for ax in 0..3 {
        let (lo, ext) = (cb.min[ax], extent[ax]);
        if ext <= 0.0 {
            continue;
        }
        let scale = BINS as f32 / ext;
        let fill = |mut bins: [Bin; BINS], &i: &u32| {
            if let (Some(tb), Some(tc)) = (input.boxes.get(i as usize), input.centers.get(i as usize))
                && let Some(bin) = bins.get_mut(bin_of(tc[ax], lo, scale))
            {
                bin.b.grow(tb);
                bin.n += 1;
            }
            bins
        };
        let merge = |mut a: [Bin; BINS], b: [Bin; BINS]| {
            for (x, y) in a.iter_mut().zip(b.iter()) {
                x.b.grow(&y.b);
                x.n += y.n;
            }
            a
        };
        let bins = if n > PAR_BIN {
            idx.par_chunks(PAR_BIN / 4)
                .map(|c| c.iter().fold([Bin::EMPTY; BINS], fill))
                .reduce(|| [Bin::EMPTY; BINS], merge)
        } else {
            idx.iter().fold([Bin::EMPTY; BINS], fill)
        };
        // the right side's areas and counts, swept from the end
        let mut right = [(0.0f32, 0u32); BINS];
        let mut acc = Bin::EMPTY;
        for k in (1..BINS).rev() {
            acc.b.grow(&bins[k].b);
            acc.n += bins[k].n;
            right[k] = (acc.b.area(), acc.n);
        }
        let mut left = Bin::EMPTY;
        for k in 0..BINS - 1 {
            left.b.grow(&bins[k].b);
            left.n += bins[k].n;
            let (ra, rn) = right[k + 1];
            if left.n == 0 || rn == 0 {
                continue;
            }
            let cost = left.b.area() * left.n as f32 + ra * rn as f32;
            if best.is_none_or(|(c, _, _)| cost < c) {
                best = Some((cost, ax, k));
            }
        }
    }
    let area = b.area().max(f32::MIN_POSITIVE);
    let Some((cost, ax, k)) = best else {
        if n <= 4 {
            return (b, None);
        }
        return (b, Some(median(input, idx, axis)));
    };
    // a split costs a box test; a leaf costs its triangles
    let split_cost = 1.0 + cost / area;
    if n <= MAX_LEAF && split_cost >= n as f32 {
        return (b, None);
    }
    let (lo, scale) = (cb.min[ax], BINS as f32 / extent[ax]);
    let goes_left = |i: u32| input.centers.get(i as usize).is_some_and(|c| bin_of(c[ax], lo, scale) <= k);
    let mid = partition(idx, goes_left);
    if mid == 0 || mid == n {
        return (b, Some(median(input, idx, axis)));
    }
    (b, Some(mid))
}

/// Puts the triangles that go left first; answers how many they are.
fn partition(idx: &mut [u32], goes_left: impl Fn(u32) -> bool) -> usize {
    let mut i = 0;
    let mut j = idx.len();
    while i < j {
        if idx.get(i).copied().is_some_and(&goes_left) {
            i += 1;
        } else {
            j -= 1;
            idx.swap(i, j);
        }
    }
    i
}

/// An object median along `axis`; ties by the triangle's index, so the cut is the same on every
/// run.
fn median(input: &Input<'_>, idx: &mut [u32], axis: usize) -> usize {
    let mid = idx.len() / 2;
    let key = |i: &u32| input.centers.get(*i as usize).map_or(0.0, |c| c[axis]);
    idx.select_nth_unstable_by(mid, |a, b| key(a).total_cmp(&key(b)).then(a.cmp(b)));
    mid
}

/// The binary nodes a 4-wide node takes as its children: the node's two, then the inner child
/// of the largest box opened, until there are four or only leaves.
fn gather(nodes: &[Node], at: u32) -> Vec<u32> {
    let mut kids = match nodes.get(at as usize).map(|n| n.kind) {
        Some(Kind::Inner { left, right }) => vec![left, right],
        _ => vec![at],
    };
    while kids.len() < 4 {
        let widest = kids
            .iter()
            .enumerate()
            .filter_map(|(k, &c)| match nodes.get(c as usize) {
                Some(Node { b, kind: Kind::Inner { left, right } }) => Some((k, b.area(), *left, *right)),
                _ => None,
            })
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
        let Some((k, _, left, right)) = widest else { break };
        kids[k] = left;
        kids.insert(k + 1, right);
    }
    kids
}

/// The kernel's 4-wide nodes from the binary tree, and the most entries the kernel's stack holds:
/// at a node it keeps every inner child it enters but the nearest. `None` when a leaf does not fit
/// its word (more than 8 triangles, or a first triangle past 2^28).
fn flatten(nodes: &[Node]) -> Option<(Vec<GpuNode>, u32)> {
    let mut out = vec![empty_node()];
    // (binary node, its 4-wide node, the stack's entries on the way to it)
    let mut todo = vec![(0u32, 0usize, 0u32)];
    let mut most = 0;
    while let Some((at, slot, held)) = todo.pop() {
        let kids: Vec<&Node> = gather(nodes, at).iter().filter_map(|&c| nodes.get(c as usize)).collect();
        let kid_ids = gather(nodes, at);
        let inner = kids.iter().filter(|n| matches!(n.kind, Kind::Inner { .. })).count() as u32;
        let held = held + inner.saturating_sub(1);
        most = most.max(held);
        // the grid over the children's boxes
        let mut b = Bounds::EMPTY;
        kids.iter().for_each(|n| b.grow(&n.b));
        let e = [0, 1, 2].map(|a| step_exponent(b.min[a], b.max[a]));
        let step = e.map(|e| f32::from_bits(e << 23));
        let mut g = empty_node();
        g[0] = [b.min.x.to_bits(), b.min.y.to_bits(), b.min.z.to_bits(), e[0] | e[1] << 8 | e[2] << 16];
        let mut planes = [[0u32; 4]; 6];
        for (lane, (n, &c)) in kids.iter().zip(&kid_ids).enumerate() {
            for a in 0..3 {
                planes[2 * a][lane] = quantize(n.b.min[a], b.min[a], step[a], true);
                planes[2 * a + 1][lane] = quantize(n.b.max[a], b.min[a], step[a], false);
            }
            g[3][lane] = match n.kind {
                Kind::Leaf { start, count } => {
                    if !(1..=8).contains(&count) || start >= LEAF_FIRST {
                        return None;
                    }
                    LEAF | (count - 1) << 28 | start
                }
                Kind::Inner { .. } => {
                    let id = out.len();
                    out.push(empty_node());
                    todo.push((c, id, held));
                    id as u32
                }
            };
        }
        for (r, row) in planes.iter_mut().enumerate() {
            // the lanes without a child keep an inverted box: a minimum's row is even
            for q in row.iter_mut().skip(kids.len()) {
                *q = if r % 2 == 0 { 255 } else { 0 };
            }
        }
        let bytes = |p: &[u32; 4]| p[0] | p[1] << 8 | p[2] << 16 | p[3] << 24;
        g[1] = [bytes(&planes[0]), bytes(&planes[1]), bytes(&planes[2]), bytes(&planes[3])];
        g[2] = [bytes(&planes[4]), bytes(&planes[5]), 0, 0];
        if let Some(o) = out.get_mut(slot) {
            *o = g;
        }
    }
    Some((out, most))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every triangle the tree reaches, each checked inside the quantized boxes on its way, as
    /// the kernel decodes them.
    fn walk(tree: &Tree, tris: &[[Vec3; 3]], node: u32, depth: u32, seen: &mut Vec<u32>, deepest: &mut u32) {
        let n = tree.nodes[node as usize];
        *deepest = (*deepest).max(depth);
        let corner = Vec3::new(f32::from_bits(n[0][0]), f32::from_bits(n[0][1]), f32::from_bits(n[0][2]));
        let step = [0, 8, 16].map(|sh| f32::from_bits(((n[0][3] >> sh) & 0xff) << 23));
        let byte = |w: u32, lane: usize| ((w >> (8 * lane)) & 0xff) as f32;
        for lane in 0..4 {
            let kid = n[3][lane];
            if kid == NONE {
                continue;
            }
            let lo = Vec3::new(
                corner.x + byte(n[1][0], lane) * step[0],
                corner.y + byte(n[1][2], lane) * step[1],
                corner.z + byte(n[2][0], lane) * step[2],
            );
            let hi = Vec3::new(
                corner.x + byte(n[1][1], lane) * step[0],
                corner.y + byte(n[1][3], lane) * step[1],
                corner.z + byte(n[2][1], lane) * step[2],
            );
            let inside = |k: u32| {
                let t = tris[tree.order[k as usize] as usize];
                t.iter().all(|c| c.cmpge(lo).all() && c.cmple(hi).all())
            };
            if kid & LEAF != 0 {
                let (first, count) = (kid & (LEAF_FIRST - 1), ((kid >> 28) & 7) + 1);
                for k in first..first + count {
                    assert!(inside(k), "a triangle outside its quantized box {lo} {hi}");
                    seen.push(tree.order[k as usize]);
                }
            } else {
                let before = seen.len();
                walk(tree, tris, kid, depth + 1, seen, deepest);
                for &t in &seen[before..] {
                    assert!(tris[t as usize].iter().all(|c| c.cmpge(lo).all() && c.cmple(hi).all()));
                }
            }
        }
    }

    #[test]
    fn every_triangle_is_in_one_leaf_and_inside_its_boxes() {
        let mut s = 0x1234_5678u32;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s % 10_000) as f32 * 0.01
        };
        // small and large triangles near the origin, and tiny ones far from it
        for (n, far, size) in [
            (0usize, 0.0f32, 1.0f32),
            (1, 0.0, 1.0),
            (3, 0.0, 1.0),
            (9, 0.0, 1.0),
            (1000, 0.0, 1.0),
            (20_000, 0.0, 1.0),
            (5000, 3071.3, 0.001),
            (5000, -700.0, 40.0),
        ] {
            let tris: Vec<[Vec3; 3]> = (0..n)
                .map(|_| {
                    let p = Vec3::new(next(), next() * 0.1, next()) * f32::max(size, 0.01) + Vec3::splat(far);
                    [p, p + Vec3::new(0.3, 0.0, 0.0) * size, p + Vec3::new(0.0, 0.2, 0.3) * size]
                })
                .collect();
            let tree = build(&tris);
            let (mut seen, mut deepest) = (Vec::new(), 0);
            walk(&tree, &tris, 0, 0, &mut seen, &mut deepest);
            seen.sort_unstable();
            assert_eq!(seen, (0..n as u32).collect::<Vec<_>>(), "{n} triangles");
            assert!(deepest < 40 && tree.stack < STACK, "{deepest}, {}", tree.stack);
            // the same tree on one thread
            let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
            let one = pool.install(|| build(&tris));
            assert_eq!(one.order, tree.order);
            assert_eq!(one.nodes, tree.nodes);
        }
    }

    #[test]
    fn many_equal_triangles_stay_shallow() {
        let t = [Vec3::ZERO, Vec3::X, Vec3::Y];
        let tris = vec![t; 50_000];
        let tree = build(&tris);
        let (mut seen, mut deepest) = (Vec::new(), 0);
        walk(&tree, &tris, 0, 0, &mut seen, &mut deepest);
        assert_eq!(seen.len(), 50_000);
        assert!(tree.stack < STACK, "{deepest}, {}", tree.stack);
    }
}
