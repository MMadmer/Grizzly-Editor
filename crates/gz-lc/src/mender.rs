//! NVIDIA's MeshMender as the compiler calls it: the tangent and
//! the binormal of every triangle from its texture gradients, smoothed around each position over
//! the triangles that share an edge by index and whose vectors are within the
//! crease angle, a corner split where two groups meet, then orthogonalised against the normal.
//! The normals are the caller's, the texture coordinates are taken as
//! they come.
//!
//! The positions are visited in the order of `operator<(D3DXVECTOR3)`, the triangles of one position
//! in the order they name it - X-Ray's `std::map` of positions, made here by a stable sort on every
//! core; the visit itself stays in that order (a split at one position changes the edges the next
//! one sees).

use std::collections::BTreeMap;

use rayon::prelude::*;
use smallvec::SmallVec;

/// A mender vertex.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MVertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub s: f32,
    pub t: f32,
    pub tangent: [f32; 3],
    pub binormal: [f32; 3],
}

const NO_GROUP: usize = usize::MAX;

#[derive(Clone, Copy, Debug)]
struct Tri {
    indices: [usize; 3],
    tangent: [f32; 3],
    binormal: [f32; 3],
    handled: bool,
    group: usize,
}

type V = [f32; 3];

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V, b: V) -> V {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(s: f32, a: V) -> V {
    [s * a[0], s * a[1], s * a[2]]
}
fn dot(a: V, b: V) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V, b: V) -> V {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn length(a: V) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

/// Normalizes: `pOut` scaled by the inverse length of `pV`.
// X-Ray's compiler scales what `pOut` holds, not `pV`; the two differ in
// the tangent orthogonalisation, where the old tangent is scaled by the new one's length
fn normalize_into(out: V, v: V) -> V {
    let s = 1.0 / (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [out[0] * s, out[1] * s, out[2] * s]
}

fn normalized(v: V) -> V {
    normalize_into(v, v)
}

/// The order of `operator<(D3DXVECTOR3)`: x, then y, then z, exactly.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Key(V);

impl Eq for Key {}

impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl Ord for Key {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::{Equal, Greater, Less};
        let lt = |a: &V, b: &V| {
            if a[0] == b[0] { if a[1] == b[1] { a[2] < b[2] } else { a[1] < b[1] } } else { a[0] < b[0] }
        };
        if lt(&self.0, &o.0) {
            Less
        } else if lt(&o.0, &self.0) {
            Greater
        } else {
            Equal
        }
    }
}

/// The texture gradients.
fn gradients(v0: &MVertex, v1: &MVertex, v2: &MVertex) -> (V, V) {
    let p = sub(v1.pos, v0.pos);
    let q = sub(v2.pos, v0.pos);
    let s1 = v1.s - v0.s;
    let t1 = v1.t - v0.t;
    let s2 = v2.s - v0.s;
    let t2 = v2.t - v0.t;
    let det = s1 * t2 - s2 * t1;
    let tmp = if det.abs() <= 0.0001 { if det > 0.0 { 1.0 } else { -1.0 } } else { 1.0 / det };
    let tangent = [t2 * p[0] - t1 * q[0], t2 * p[1] - t1 * q[1], t2 * p[2] - t1 * q[2]];
    let binormal = [s1 * q[0] - s2 * p[0], s1 * q[1] - s2 * p[1], s1 * q[2] - s2 * p[2]];
    (mul(tmp, tangent), mul(tmp, binormal))
}

fn tri_has_edge(p0: usize, p1: usize, a: usize, b: usize, c: usize) -> bool {
    (p0 == b && p1 == a)
        || (p0 == a && p1 == b)
        || (p0 == b && p1 == c)
        || (p0 == c && p1 == b)
        || (p0 == c && p1 == a)
        || (p0 == a && p1 == c)
}

/// Whether two triangles share an edge, respecting the splits.
fn shares_edge(a: &Tri, b: &Tri) -> bool {
    let [a1, b1, c1] = a.indices;
    let [a2, b2, c2] = b.indices;
    tri_has_edge(b1, a1, a2, b2, c2) || tri_has_edge(a1, c1, a2, b2, c2) || tri_has_edge(c1, b1, a2, b2, c2)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Tangent,
    Binormal,
}

struct Mender {
    tris: Vec<Tri>,
    original: usize,
    /// What a position's visit works with, kept from one position to the next.
    groups: Vec<SmallVec<[usize; 8]>>,
    stack: Vec<usize>,
}

type Groups = Vec<SmallVec<[usize; 8]>>;

impl Mender {
    fn vector(&self, t: usize, w: Which) -> V {
        self.tris.get(t).map_or([0.0; 3], |x| if w == Which::Tangent { x.tangent } else { x.binormal })
    }

    /// Whether the tangents / binormals of two triangles can be smoothed.
    fn can_smooth(&self, t1: usize, t2: usize, w: Which, min_crease: f32) -> bool {
        let a = normalized(self.vector(t1, w));
        let b = normalized(self.vector(t2, w));
        if dot(a, b) >= min_crease {
            return true;
        }
        a == [0.0; 3] && b == [0.0; 3]
    }

    fn find_neighbors(&self, tri: usize, possible: &[usize]) -> (usize, usize) {
        let mut found = [NO_GROUP; 2];
        let mut n = 0;
        let Some(t) = self.tris.get(tri) else { return (NO_GROUP, NO_GROUP) };
        for &p in possible {
            if p != tri && self.tris.get(p).is_some_and(|o| shares_edge(t, o)) {
                if let Some(slot) = found.get_mut(n) {
                    *slot = p;
                }
                n += 1;
                if n == 2 {
                    break;
                }
            }
        }
        (found[0], found[1])
    }

    /// Builds the groups, its recursion walked with a stack in the same order.
    fn build_groups(&mut self, start: usize, possible: &[usize], groups: &mut Groups, w: Which, crease: f32) {
        let mut stack = std::mem::take(&mut self.stack);
        stack.clear();
        stack.push(start);
        while let Some(tri) = stack.pop() {
            if tri == NO_GROUP || self.tris.get(tri).is_none_or(|t| t.handled) {
                continue;
            }
            let (n1, n2) = self.find_neighbors(tri, possible);
            for n in [n1, n2] {
                if n == NO_GROUP {
                    continue;
                }
                let ng = self.tris.get(n).map_or(NO_GROUP, |t| t.group);
                if ng != NO_GROUP && self.can_smooth(tri, n, w, crease) {
                    if let Some(g) = groups.get_mut(ng) {
                        g.push(tri);
                    }
                    if let Some(t) = self.tris.get_mut(tri) {
                        t.group = ng;
                    }
                }
            }
            if self.tris.get(tri).is_some_and(|t| t.group == NO_GROUP) {
                if let Some(t) = self.tris.get_mut(tri) {
                    t.group = groups.len();
                }
                let mut g = SmallVec::new();
                g.push(tri);
                groups.push(g);
            }
            if let Some(t) = self.tris.get_mut(tri) {
                t.handled = true;
            }
            stack.push(n2);
            stack.push(n1);
        }
        self.stack = stack;
    }

    fn update_indices(&mut self, old: usize, new: usize, group: &[usize]) {
        for &t in group {
            if let Some(tri) = self.tris.get_mut(t) {
                for i in &mut tri.indices {
                    if *i == old {
                        *i = new;
                    }
                }
            }
        }
    }

    fn append_mapping(&self, old: usize, mapping: &mut Vec<u32>) {
        if old >= self.original {
            let orig = mapping.get(old).copied().unwrap_or(0);
            mapping.push(orig);
        } else {
            mapping.push(old as u32);
        }
    }

    /// Processes the tangents / binormals.
    fn process(
        &mut self,
        possible: &[usize],
        verts: &mut Vec<MVertex>,
        mapping: &mut Vec<u32>,
        at: V,
        w: Which,
        crease: f32,
    ) {
        for &p in possible {
            if let Some(t) = self.tris.get_mut(p) {
                t.handled = false;
                t.group = NO_GROUP;
            }
        }
        let mut groups = std::mem::take(&mut self.groups);
        groups.clear();
        for &p in possible {
            if self.tris.get(p).is_some_and(|t| !t.handled) {
                self.build_groups(p, possible, &mut groups, w, crease);
            }
        }
        let vectors: SmallVec<[V; 8]> = groups
            .iter()
            .map(|g| {
                let mut sum = [0.0f32; 3];
                for &t in g {
                    sum = add(sum, self.vector(t, w));
                }
                normalized(sum)
            })
            .collect();
        let mut others: SmallVec<[usize; 16]> = SmallVec::new();
        for (gi, group) in groups.iter().enumerate() {
            let mut this: SmallVec<[usize; 16]> = SmallVec::new();
            let vector = vectors.get(gi).copied().unwrap_or([0.0; 3]);
            for t in 0..group.len() {
                let tid = group[t];
                for k in 0..3 {
                    let idx = self.tris.get(tid).map_or(0, |x| x.indices[k]);
                    if verts.get(idx).is_none_or(|v| v.pos != at) {
                        continue;
                    }
                    if others.contains(&idx) {
                        let mut ov = verts.get(idx).copied().unwrap_or_default();
                        match w {
                            Which::Tangent => ov.tangent = vector,
                            Which::Binormal => ov.binormal = vector,
                        }
                        let new = verts.len();
                        verts.push(ov);
                        self.append_mapping(idx, mapping);
                        self.update_indices(idx, new, group);
                        this.push(self.tris.get(tid).map_or(new, |x| x.indices[k]));
                    } else {
                        if let Some(v) = verts.get_mut(idx) {
                            match w {
                                Which::Tangent => v.tangent = vector,
                                Which::Binormal => v.binormal = vector,
                            }
                        }
                        this.push(idx);
                    }
                }
            }
            others.extend(this);
        }
        self.groups = groups;
    }
}

/// Orthogonalizes the tangents and binormals.
fn orthogonalize(verts: &mut [MVertex]) {
    for v in verts.iter_mut() {
        let (tan, nrm, bin) = (v.tangent, v.normal, v.binormal);
        let new_t = sub(tan, mul(dot(nrm, tan), nrm));
        let new_b = sub(sub(bin, mul(dot(nrm, bin), nrm)), mul(dot(new_t, bin), new_t));
        v.tangent = normalize_into(v.tangent, new_t);
        v.binormal = normalize_into(v.binormal, new_b);
        let len_t = length(v.tangent);
        let len_b = length(v.binormal);
        if len_t <= 0.001 || len_b <= 0.001 {
            if len_t > 0.5 {
                v.binormal = cross(v.normal, v.tangent);
            } else if len_b > 0.5 {
                v.tangent = cross(v.binormal, v.normal);
            } else {
                let x = [1.0, 0.0, 0.0];
                let y = [0.0, 1.0, 0.0];
                let start = if dot(x, v.normal) < dot(y, v.normal) { x } else { y };
                v.tangent = cross(v.normal, start);
                v.binormal = cross(v.normal, v.tangent);
            }
        } else if dot(v.binormal, v.tangent) > 0.999 {
            v.binormal = cross(v.normal, v.tangent);
        }
    }
}

/// The mend: `Mend(verts, indices, mapping, 1, 0.5, 0.5, 0, DONT_CALCULATE_NORMALS,
/// RESPECT_SPLITS, DONT_FIX_CYLINDRICAL)`: answers the mapping from new to old vertices; `verts` may grow and
/// `indices` point at the new vertices.
pub fn mend(
    verts: &mut Vec<MVertex>,
    indices: &mut [u32],
    tangent_crease: f32,
    binormal_crease: f32,
) -> Vec<u32> {
    mend_until(verts, indices, tangent_crease, binormal_crease, &|| false).unwrap_or_default()
}

/// [`mend`] that gives up - `None`, the vertices half mended - once `stop` says so; asked every
/// few thousand points.
pub fn mend_until(
    verts: &mut Vec<MVertex>,
    indices: &mut [u32],
    tangent_crease: f32,
    binormal_crease: f32,
    stop: &(dyn Fn() -> bool + Sync),
) -> Option<Vec<u32>> {
    mend_with(verts, indices, tangent_crease, binormal_crease, stop, false)
}

/// [`mend_until`]; `by_map` orders the positions through X-Ray's map itself, as a position with a
/// NaN always is.
fn mend_with(
    verts: &mut Vec<MVertex>,
    indices: &mut [u32],
    tangent_crease: f32,
    binormal_crease: f32,
    stop: &(dyn Fn() -> bool + Sync),
    by_map: bool,
) -> Option<Vec<u32>> {
    let mut mapping: Vec<u32> = (0..verts.len() as u32).collect();
    let original = verts.len();
    let tris: Vec<Tri> = {
        let verts = &*verts;
        indices
            .as_chunks::<3>()
            .0
            .par_iter()
            .with_min_len(4096)
            .map(|t| {
                let idx = [t[0] as usize, t[1] as usize, t[2] as usize];
                let get = |i: usize| verts.get(i).copied().unwrap_or_default();
                let (tangent, binormal) = gradients(&get(idx[0]), &get(idx[1]), &get(idx[2]));
                Tri { indices: idx, tangent, binormal, handled: false, group: NO_GROUP }
            })
            .collect()
    };
    let mut m = Mender { tris, original, groups: Vec::new(), stack: Vec::new() };
    // every corner's position with its triangle, in the order the triangles name them
    let corners: Vec<(Key, usize)> = m
        .tris
        .iter()
        .enumerate()
        .flat_map(|(i, t)| t.indices.map(|k| (Key(verts.get(k).map_or([0.0; 3], |v| v.pos)), i)))
        .collect();
    if by_map || corners.iter().any(|(k, _)| k.0.iter().any(|c| c.is_nan())) {
        // a NaN is equal to every position in that order: only X-Ray's map itself knows where it goes
        let mut children: BTreeMap<Key, Vec<usize>> = BTreeMap::new();
        for (key, i) in corners {
            children.entry(key).or_default().push(i);
        }
        for (n, (key, possible)) in children.iter().enumerate() {
            if n.is_multiple_of(4096) && stop() {
                return None;
            }
            m.process(possible, verts, &mut mapping, key.0, Which::Tangent, tangent_crease);
            m.process(possible, verts, &mut mapping, key.0, Which::Binormal, binormal_crease);
        }
    } else {
        let mut corners = corners;
        // stable: a position's triangles keep their order
        corners.par_sort_by(|a, b| a.0.cmp(&b.0));
        let mut possible: Vec<usize> = Vec::with_capacity(16);
        let mut at = 0usize;
        let mut n = 0usize;
        while let Some(&(key, _)) = corners.get(at) {
            if n.is_multiple_of(4096) && stop() {
                return None;
            }
            possible.clear();
            while let Some(&(k, i)) = corners.get(at)
                && k == key
            {
                possible.push(i);
                at += 1;
            }
            m.process(&possible, verts, &mut mapping, key.0, Which::Tangent, tangent_crease);
            m.process(&possible, verts, &mut mapping, key.0, Which::Binormal, binormal_crease);
            n += 1;
        }
    }
    for (slot, t) in indices.as_chunks_mut::<3>().0.iter_mut().zip(&m.tris) {
        for (s, &x) in slot.iter_mut().zip(&t.indices) {
            *s = x as u32;
        }
    }
    orthogonalize(verts);
    Some(mapping)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(p: [f32; 3], s: f32, t: f32) -> MVertex {
        MVertex { pos: p, normal: [0.0, 1.0, 0.0], s, t, ..MVertex::default() }
    }

    #[test]
    fn a_flat_square_gets_one_basis() {
        let mut verts = vec![
            v([0.0, 0.0, 0.0], 0.0, 0.0),
            v([0.0, 0.0, 1.0], 0.0, 1.0),
            v([1.0, 0.0, 1.0], 1.0, 1.0),
            v([1.0, 0.0, 0.0], 1.0, 0.0),
        ];
        let mut idx = vec![0, 1, 2, 0, 2, 3];
        let map = mend(&mut verts, &mut idx, 0.5, 0.5);
        assert_eq!(verts.len(), 4);
        assert_eq!(map, vec![0, 1, 2, 3]);
        for x in &verts {
            assert!((x.tangent[0] - 1.0).abs() < 1e-5, "{:?}", x.tangent);
            assert!((x.binormal[2] - 1.0).abs() < 1e-5, "{:?}", x.binormal);
        }
    }

    /// The sorted positions visit the corners as X-Ray's map does: a grid of shared corners with
    /// mirrored and sheared texture coordinates (many splits), repeated positions, -0 beside +0.
    #[test]
    fn the_sorted_positions_mend_as_the_map_did() {
        let mut s = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let n = 30;
        let mut verts = Vec::new();
        for z in 0..=n {
            for x in 0..=n {
                let flip = if next() % 3 == 0 { -1.0 } else { 1.0 };
                let px = if x == 0 && next() % 2 == 0 { -0.0 } else { x as f32 };
                let p = [px, (next() % 4) as f32 * 0.1, z as f32];
                verts.push(v(p, flip * x as f32 * 0.3, z as f32 * 0.2 + (next() % 5) as f32 * 0.05));
                // the same position again with other coordinates: a seam
                if next() % 4 == 0 {
                    verts.push(v(p, -(x as f32), z as f32));
                }
            }
        }
        let mut idx = Vec::new();
        for _ in 0..4000 {
            let a = (next() % verts.len() as u64) as u32;
            let b = (next() % verts.len() as u64) as u32;
            let c = (next() % verts.len() as u64) as u32;
            idx.extend([a, b, c]);
        }
        let (mut v1, mut i1) = (verts.clone(), idx.clone());
        let m1 = mend_with(&mut v1, &mut i1, 0.5, 0.5, &|| false, true).unwrap();
        let (mut v2, mut i2) = (verts.clone(), idx.clone());
        let m2 = mend_with(&mut v2, &mut i2, 0.5, 0.5, &|| false, false).unwrap();
        assert!(v2.len() > verts.len() + 100, "{} split: the test proves little", v2.len() - verts.len());
        assert_eq!(m1, m2);
        assert_eq!(i1, i2);
        let bits = |v: &[MVertex]| {
            v.iter()
                .flat_map(|x| {
                    x.tangent.iter().chain(&x.binormal).chain(&x.pos).map(|c| c.to_bits()).collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(bits(&v1), bits(&v2));
    }

    #[test]
    fn a_mirrored_seam_splits_the_shared_corners() {
        // two triangles sharing an edge, the second's texture mirrored: tangents opposite
        let mut verts = vec![
            v([0.0, 0.0, 0.0], 0.0, 0.0),
            v([0.0, 0.0, 1.0], 0.0, 1.0),
            v([1.0, 0.0, 0.0], 1.0, 0.0),
            v([-1.0, 0.0, 0.0], 1.0, 0.0),
        ];
        let mut idx = vec![0, 1, 2, 0, 3, 1];
        let map = mend(&mut verts, &mut idx, 0.5, 0.5);
        assert_eq!(verts.len(), 6);
        assert_eq!(&map[4..], &[0, 1]);
    }
}
