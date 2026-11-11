//! `CDB::Collector` and `CDB::CollectorPacked`: triangle soups with welded vertices. The order
//! of the vertices and triangles they give is the order the C++ gives.

use std::collections::BTreeSet;

use glam::Vec3;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;

use crate::cform::Tri;
use crate::math::{EPS, EPS_L, i_floor, similar};
use crate::{Aabb, CForm};

/// `CDB::Collector`: collects faces, optionally welding vertices closer than an epsilon.
#[derive(Clone, Debug, Default)]
pub struct Collector {
    verts: Vec<Vec3>,
    faces: Vec<Tri>,
    grid: WeldGrid,
}

/// What makes `VPack` linear: vertices bucketed by cells twice the epsilon wide, so that a vertex
/// within the epsilon of a point always lies in one of the 27 cells around it.
#[derive(Clone, Debug, Default)]
struct WeldGrid {
    eps: f32,
    /// How many vertices of the collector the cells hold.
    indexed: usize,
    cells: FxHashMap<[i64; 3], SmallVec<[u32; 4]>>,
}

impl WeldGrid {
    fn cell_of(&self, v: Vec3) -> [i64; 3] {
        let size = f64::from(self.eps) * 2.0;
        [
            (f64::from(v.x) / size).floor() as i64,
            (f64::from(v.y) / size).floor() as i64,
            (f64::from(v.z) / size).floor() as i64,
        ]
    }

    fn insert(&mut self, v: Vec3, index: u32) {
        // a vertex with a NaN or an infinity is similar to nothing: it needs no cell
        if v.is_finite() {
            let cell = self.cell_of(v);
            self.cells.entry(cell).or_default().push(index);
        }
    }

    /// Brings the cells up to date with the vertex list, for this epsilon.
    fn sync(&mut self, verts: &[Vec3], eps: f32) {
        if self.eps != eps {
            self.eps = eps;
            self.indexed = 0;
            self.cells.clear();
        }
        for (i, v) in verts.iter().enumerate().skip(self.indexed) {
            self.insert(*v, i as u32);
        }
        self.indexed = verts.len();
    }

    /// The first vertex in list order that is similar to `v`.
    fn find(&self, verts: &[Vec3], v: Vec3) -> Option<u32> {
        let [cx, cy, cz] = self.cell_of(v);
        let mut best: Option<u32> = None;
        for dx in -1i64..=1 {
            for dy in -1i64..=1 {
                for dz in -1i64..=1 {
                    let cell = [cx.saturating_add(dx), cy.saturating_add(dy), cz.saturating_add(dz)];
                    let Some(list) = self.cells.get(&cell) else { continue };
                    for &i in list {
                        if best.is_some_and(|b| b <= i) {
                            continue;
                        }
                        if verts.get(i as usize).is_some_and(|p| similar(*p, v, self.eps)) {
                            best = Some(i);
                        }
                    }
                }
            }
        }
        best
    }
}

impl Collector {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `VPack`: the index of the first vertex similar to `v`, or of `v` appended.
    fn v_pack(&mut self, v: Vec3, eps: f32) -> u32 {
        let found = if eps > 0.0 && eps.is_finite() && v.is_finite() {
            self.grid.sync(&self.verts, eps);
            self.grid.find(&self.verts, v)
        } else {
            // an odd epsilon or point: the plain scan of the C++
            self.verts.iter().position(|p| similar(*p, v, eps)).map(|i| i as u32)
        };
        if let Some(i) = found {
            return i;
        }
        self.verts.push(v);
        (self.verts.len() - 1) as u32
    }

    fn push_plain(&mut self, v0: Vec3, v1: Vec3, v2: Vec3, dummy: u32) {
        let base = self.verts.len() as u32;
        self.verts.extend_from_slice(&[v0, v1, v2]);
        self.faces.push(Tri { verts: [base, base + 1, base + 2], dummy });
    }

    /// `add_face`: three new vertices, no welding.
    pub fn add_face(&mut self, v0: Vec3, v1: Vec3, v2: Vec3, material: u16, sector: u16) {
        self.push_plain(v0, v1, v2, Tri::new([0; 3], material, sector).dummy);
    }

    /// `add_face_D`: three new vertices and the packed word as given.
    pub fn add_face_d(&mut self, v0: Vec3, v1: Vec3, v2: Vec3, dummy: u32) {
        self.push_plain(v0, v1, v2, dummy);
    }

    /// `add_face_packed`: vertices welded within `eps` (the C++ default is [`EPS`]).
    pub fn add_face_packed(&mut self, v0: Vec3, v1: Vec3, v2: Vec3, material: u16, sector: u16, eps: f32) {
        self.add_face_packed_d(v0, v1, v2, Tri::new([0; 3], material, sector).dummy, eps);
    }

    /// `add_face_packed_D`.
    pub fn add_face_packed_d(&mut self, v0: Vec3, v1: Vec3, v2: Vec3, dummy: u32, eps: f32) {
        let a = self.v_pack(v0, eps);
        let b = self.v_pack(v1, eps);
        let c = self.v_pack(v2, eps);
        self.faces.push(Tri { verts: [a, b, c], dummy });
    }

    /// The epsilon `add_face_packed` uses when the C++ caller passes none.
    pub const DEFAULT_EPS: f32 = EPS;

    /// `remove_duplicate_T`: drops triangles that repeat an earlier one (the same three vertices
    /// in any order and the same word). The survivors end up where the C++ leaves them - it fills
    /// a hole with the last triangle - without its quadratic scan.
    pub fn remove_duplicate_tris(&mut self) {
        type Key = ([u32; 3], u32);
        fn key(t: &Tri) -> Key {
            let mut v = t.verts;
            v.sort_unstable();
            (v, t.dummy)
        }
        let mut classes: FxHashMap<Key, BTreeSet<u32>> = FxHashMap::default();
        for (i, t) in self.faces.iter().enumerate() {
            classes.entry(key(t)).or_default().insert(i as u32);
        }

        let mut f = 0usize;
        while let Some(face) = self.faces.get(f).copied() {
            let k = key(&face);
            loop {
                let next = classes.get(&k).and_then(|set| set.range(f as u32 + 1..).next().copied());
                let Some(t) = next else { break };
                let Some(last_face) = self.faces.pop() else { break };
                let last = self.faces.len() as u32;
                let last_key = key(&last_face);
                if last_key == k {
                    // the last triangle is a duplicate too: it lands in the hole and is met again
                    if let Some(set) = classes.get_mut(&k) {
                        set.remove(&last);
                    }
                } else {
                    if let Some(set) = classes.get_mut(&k) {
                        set.remove(&t);
                    }
                    if let Some(set) = classes.get_mut(&last_key) {
                        set.remove(&last);
                        set.insert(t);
                    }
                }
                if t != last
                    && let Some(slot) = self.faces.get_mut(t as usize)
                {
                    *slot = last_face;
                }
            }
            f += 1;
        }
    }

    /// `calc_adjacency`: for every triangle edge (`face * 3 + edge`) the triangle on the other
    /// side, or `u32::MAX`.
    // oracle: vertex ids are compared as u16, so a collector of more than 65536 vertices pairs
    // edges that only share the low 16 bits; the C++ asserts fewer than 65536 faces instead.
    #[must_use]
    pub fn calc_adjacency(&self) -> Vec<u32> {
        struct Edge {
            face: u32,
            edge: u32,
            v0: u16,
            v1: u16,
        }
        let mut edges = Vec::with_capacity(self.faces.len() * 3);
        for (face, t) in self.faces.iter().enumerate() {
            for (edge, (a, b)) in [(0usize, 1usize), (1, 2), (2, 0)].into_iter().enumerate() {
                let (a, b) = (t.verts[a] as u16, t.verts[b] as u16);
                edges.push(Edge { face: face as u32, edge: edge as u32, v0: a.min(b), v1: a.max(b) });
            }
        }
        edges.sort_unstable_by_key(|e| (e.v0, e.v1, e.face, e.edge));

        let mut dest = vec![u32::MAX; edges.len()];
        for pair in edges.windows(2) {
            let [i, j] = pair else { continue };
            if i.v0 != j.v0 || i.v1 != j.v1 {
                continue;
            }
            if let Some(d) = dest.get_mut((i.face * 3 + i.edge) as usize) {
                *d = j.face;
            }
            if let Some(d) = dest.get_mut((j.face * 3 + j.edge) as usize) {
                *d = i.face;
            }
        }
        dest
    }

    #[must_use]
    pub fn verts(&self) -> &[Vec3] {
        &self.verts
    }

    #[must_use]
    pub fn tris(&self) -> &[Tri] {
        &self.faces
    }

    pub fn clear(&mut self) {
        self.verts.clear();
        self.faces.clear();
        self.grid = WeldGrid::default();
    }

    /// The collected soup as a collision form.
    #[must_use]
    pub fn into_cform(self) -> CForm {
        CForm::new(self.verts, self.faces)
    }
}

const CLP_MX: u32 = 24;
const CLP_MY: u32 = 16;
const CLP_MZ: u32 = 24;

/// `CDB::CollectorPacked`: welds within [`EPS_L`] through a fixed 24 x 16 x 24 grid over a box
/// given up front; every face also carries a caller's flags word.
#[derive(Clone, Debug)]
pub struct CollectorPacked {
    verts: Vec<Vec3>,
    faces: Vec<Tri>,
    flags: Vec<u32>,
    vm_min: Vec3,
    vm_scale: Vec3,
    vm_eps: Vec3,
    cells: Vec<Vec<u32>>,
}

impl CollectorPacked {
    /// A collector over the box the geometry lies in; the two counts only size the buffers.
    #[must_use]
    pub fn new(bb: &Aabb, approx_verts: usize, approx_faces: usize) -> Self {
        let scale = Vec3::new(bb.max.x - bb.min.x, bb.max.y - bb.min.y, bb.max.z - bb.min.z);
        let half_cell = |s: f32, n: u32| {
            let e = s / n as f32 / 2.0;
            if e < EPS_L { e } else { EPS_L }
        };
        // a hint from the caller, not a promise: never reserve past a sane size for it
        let cap = |n: usize| n.min(1 << 22);
        Self {
            verts: Vec::with_capacity(cap(approx_verts)),
            faces: Vec::with_capacity(cap(approx_faces)),
            flags: Vec::with_capacity(cap(approx_faces)),
            vm_min: bb.min,
            vm_scale: scale,
            vm_eps: Vec3::new(
                half_cell(scale.x, CLP_MX),
                half_cell(scale.y, CLP_MY),
                half_cell(scale.z, CLP_MZ),
            ),
            cells: vec![Vec::new(); ((CLP_MX + 1) * (CLP_MY + 1) * (CLP_MZ + 1)) as usize],
        }
    }

    fn cell_index(ix: u32, iy: u32, iz: u32) -> usize {
        ((ix * (CLP_MY + 1) + iy) * (CLP_MZ + 1) + iz) as usize
    }

    /// The grid cell of a point: the C++ takes the floor into a `u32` and clamps from above, so
    /// a point below the box lands in the last cell.
    fn cell(&self, v: Vec3) -> (u32, u32, u32) {
        let one =
            |val: f32, min: f32, scale: f32, n: u32| (i_floor((val - min) / scale * n as f32) as u32).min(n);
        (
            one(v.x, self.vm_min.x, self.vm_scale.x, CLP_MX),
            one(v.y, self.vm_min.y, self.vm_scale.y, CLP_MY),
            one(v.z, self.vm_min.z, self.vm_scale.z, CLP_MZ),
        )
    }

    fn put(&mut self, ix: u32, iy: u32, iz: u32, index: u32) {
        if let Some(list) = self.cells.get_mut(Self::cell_index(ix, iy, iz)) {
            list.push(index);
        }
    }

    fn v_pack(&mut self, v: Vec3) -> u32 {
        let (ix, iy, iz) = self.cell(v);
        if let Some(list) = self.cells.get(Self::cell_index(ix, iy, iz)) {
            for &i in list {
                if self.verts.get(i as usize).is_some_and(|p| similar(*p, v, EPS_L)) {
                    return i;
                }
            }
        }
        let p = self.verts.len() as u32;
        self.verts.push(v);
        self.put(ix, iy, iz, p);

        let (ex, ey, ez) =
            self.cell(Vec3::new(v.x + self.vm_eps.x, v.y + self.vm_eps.y, v.z + self.vm_eps.z));
        if ex != ix {
            self.put(ex, iy, iz, p);
        }
        if ey != iy {
            self.put(ix, ey, iz, p);
        }
        if ez != iz {
            self.put(ix, iy, ez, p);
        }
        if ex != ix && ey != iy {
            self.put(ex, ey, iz, p);
        }
        if ex != ix && ez != iz {
            self.put(ex, iy, ez, p);
        }
        if ey != iy && ez != iz {
            self.put(ix, ey, ez, p);
        }
        if ex != ix && ey != iy && ez != iz {
            self.put(ex, ey, ez, p);
        }
        p
    }

    /// `add_face`.
    pub fn add_face(&mut self, v0: Vec3, v1: Vec3, v2: Vec3, material: u16, sector: u16, flags: u32) {
        self.add_face_d(v0, v1, v2, Tri::new([0; 3], material, sector).dummy, flags);
    }

    /// `add_face_D`.
    pub fn add_face_d(&mut self, v0: Vec3, v1: Vec3, v2: Vec3, dummy: u32, flags: u32) {
        let a = self.v_pack(v0);
        let b = self.v_pack(v1);
        let c = self.v_pack(v2);
        self.faces.push(Tri { verts: [a, b, c], dummy });
        self.flags.push(flags);
    }

    #[must_use]
    pub fn verts(&self) -> &[Vec3] {
        &self.verts
    }

    #[must_use]
    pub fn tris(&self) -> &[Tri] {
        &self.faces
    }

    /// The flags word of every face, in face order.
    #[must_use]
    pub fn flags(&self) -> &[u32] {
        &self.flags
    }

    pub fn clear(&mut self) {
        self.verts.clear();
        self.faces.clear();
        self.flags.clear();
        for cell in &mut self.cells {
            cell.clear();
        }
    }

    /// The collected soup as a collision form.
    #[must_use]
    pub fn into_cform(self) -> CForm {
        CForm::new(self.verts, self.faces)
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
        fn unit(&mut self) -> f32 {
            (self.next() >> 40) as f32 / (1u64 << 24) as f32
        }
    }

    /// `Collector::VPack` as the C++ writes it.
    fn naive_pack(verts: &mut Vec<Vec3>, v: Vec3, eps: f32) -> u32 {
        if let Some(i) = verts.iter().position(|p| similar(*p, v, eps)) {
            return i as u32;
        }
        verts.push(v);
        (verts.len() - 1) as u32
    }

    #[test]
    fn welding_matches_the_linear_scan() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let eps = 0.01;
        let mut c = Collector::new();
        let mut verts = Vec::new();
        let mut faces = Vec::new();
        for _ in 0..400 {
            // points on a coarse lattice with jitter around the epsilon: plenty of near misses
            let mut p = || {
                let cell =
                    Vec3::new((rng.next() % 6) as f32, (rng.next() % 6) as f32, (rng.next() % 6) as f32);
                cell * 0.02 + Vec3::new(rng.unit(), rng.unit(), rng.unit()) * 0.012 - Vec3::splat(0.05)
            };
            let (a, b, d) = (p(), p(), p());
            c.add_face_packed(a, b, d, 1, 2, eps);
            faces.push([
                naive_pack(&mut verts, a, eps),
                naive_pack(&mut verts, b, eps),
                naive_pack(&mut verts, d, eps),
            ]);
        }
        assert_eq!(c.verts(), verts.as_slice());
        let got: Vec<[u32; 3]> = c.tris().iter().map(|t| t.verts).collect();
        assert_eq!(got, faces);
        assert!(c.verts().len() < 1200, "nothing was welded");
    }

    #[test]
    fn unwelded_faces_are_seen_by_later_welded_ones() {
        let mut c = Collector::new();
        c.add_face(Vec3::ZERO, Vec3::X, Vec3::Y, 3, 4);
        c.add_face_packed(Vec3::X, Vec3::Y, Vec3::Z, 3, 4, Collector::DEFAULT_EPS);
        assert_eq!(c.verts().len(), 4);
        assert_eq!(c.tris()[1].verts, [1, 2, 3]);
        assert_eq!(c.tris()[0].material(), 3);
        assert_eq!(c.tris()[0].sector(), 4);
    }

    /// `remove_duplicate_T` as the C++ writes it.
    fn naive_dedup(faces: &mut Vec<Tri>) {
        fn same(a: &Tri, b: &Tri) -> bool {
            let (mut x, mut y) = (a.verts, b.verts);
            x.sort_unstable();
            y.sort_unstable();
            x == y && a.dummy == b.dummy
        }
        let mut f = 0;
        while f < faces.len() {
            let mut t = f + 1;
            while t < faces.len() {
                if same(&faces[f], &faces[t]) {
                    let last = faces[faces.len() - 1];
                    faces[t] = last;
                    faces.pop();
                } else {
                    t += 1;
                }
            }
            f += 1;
        }
    }

    #[test]
    fn duplicates_go_the_way_the_quadratic_loop_removes_them() {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        for round in 0..20 {
            let mut c = Collector::new();
            for _ in 0..(40 + round * 7) {
                let v = [(rng.next() % 4) as u32, (rng.next() % 4) as u32, (rng.next() % 4) as u32];
                c.faces.push(Tri { verts: v, dummy: (rng.next() % 2) as u32 });
            }
            let mut expect = c.faces.clone();
            naive_dedup(&mut expect);
            c.remove_duplicate_tris();
            assert_eq!(c.faces, expect, "round {round}");
        }
    }

    #[test]
    fn adjacency_pairs_shared_edges() {
        let mut c = Collector::new();
        let (a, b, d, e) = (Vec3::ZERO, Vec3::X, Vec3::Z, Vec3::new(1.0, 0.0, 1.0));
        c.add_face_packed(a, b, d, 0, 0, 0.001);
        c.add_face_packed(b, e, d, 0, 0, 0.001);
        let adj = c.calc_adjacency();
        // face 0: edges (a,b), (b,d), (d,a); face 1: (b,e), (e,d), (d,b)
        assert_eq!(adj, vec![u32::MAX, 1, u32::MAX, u32::MAX, u32::MAX, 0]);
    }

    #[test]
    fn the_packed_collector_welds_and_keeps_flags() {
        let bb = Aabb::new(Vec3::ZERO, Vec3::splat(10.0));
        let mut c = CollectorPacked::new(&bb, 16, 16);
        c.add_face(Vec3::ZERO, Vec3::X, Vec3::Z, 5, 1, 0xaa);
        c.add_face(Vec3::X, Vec3::new(1.0, 0.0, 1.0), Vec3::new(0.0, 0.0, 1.0004), 5, 1, 0xbb);
        assert_eq!(c.verts().len(), 4);
        assert_eq!(c.tris()[1].verts, [1, 3, 2]);
        assert_eq!(c.flags(), &[0xaa, 0xbb]);
        // outside the box still lands in a cell
        c.add_face(Vec3::splat(-5.0), Vec3::splat(50.0), Vec3::splat(-5.0), 0, 0, 0);
        assert_eq!(c.verts().len(), 6);
        assert_eq!(c.tris()[2].verts, [4, 5, 4]);
    }
}
