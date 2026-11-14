//! The visuals of the level as the compiler makes them: a model of one
//! material built face by face with its vertices welded, its texture coordinates moved near the
//! origin, made progressive, ordered for the vertex cache; the nodes of the tree above them.

use glam::{Vec2, Vec3};
use gz_fmt_ogf::{optimize_vertices, vipm_convert};
use gz_lc_light::math::{EPS_L, i_floor};
use gz_lc_light::{BaseColor, Basis};
use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::mender::{MVertex, mend};
use crate::sphere::Bounds;
use crate::stripify::stripify;

/// `c_vCacheSize`.
pub const VCACHE_SIZE: u32 = 24;
/// `c_PM_FaceLimit`.
pub const PM_FACE_LIMIT: usize = 128;
/// `c_PM_MetricLimit_static`.
pub const PM_METRIC_STATIC: f32 = 0.10;
/// `c_PM_MetricLimit_mu`.
pub const PM_METRIC_MU: f32 = 0.05;
/// `VIPM_Convert(u32(25), ...)`.
const PM_MAX_WINDOW: u32 = 25;

/// A name and the size of the surface it stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OgfTexture {
    pub name: SmolStr,
    pub width: u32,
    pub height: u32,
}

/// `OGF_Vertex`.
#[derive(Clone, Debug, PartialEq)]
pub struct OgfVertex {
    pub p: Vec3,
    pub n: Vec3,
    pub t: Basis,
    pub b: Basis,
    pub color: BaseColor,
    pub uv: SmallVec<[Vec2; 3]>,
}

/// A slide window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SlideWindow {
    pub offset: u32,
    pub num_tris: u16,
    pub num_verts: u16,
}

/// `ogf_container`: the vertices, the faces, the sliding windows and where the savers put them.
#[derive(Clone, Debug, PartialEq)]
pub struct Container<V> {
    pub vertices: Vec<V>,
    pub faces: Vec<[u16; 3]>,
    pub swi: Vec<SlideWindow>,
    pub vb_id: u32,
    pub vb_start: u32,
    pub ib_id: u32,
    pub ib_start: u32,
}

impl<V> Default for Container<V> {
    /// `ogf_container()`: no buffer yet.
    fn default() -> Self {
        Self {
            vertices: Vec::new(),
            faces: Vec::new(),
            swi: Vec::new(),
            vb_id: u32::MAX,
            vb_start: u32::MAX,
            ib_id: u32::MAX,
            ib_start: u32::MAX,
        }
    }
}

/// A model of one material.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ogf {
    pub material: u16,
    pub textures: Vec<OgfTexture>,
    pub sector: u16,
    pub data: Container<OgfVertex>,
    /// `fast_path_data`: positions only.
    pub fast: Container<Vec3>,
    pub bounds: Option<Bounds>,
    /// The points the bounds read, kept from when X-Ray's compiler calls it until the spheres are
    /// taken in its order.
    pub snapshot: Vec<Vec3>,
    /// `_subdiv::sw_id` of an MU model's visual: its sliding windows in the shared container.
    pub mu_sw_id: u32,
}

/// Vertices by cells twice `EPS_L` wide: a vertex within `EPS_L` of a point lies in one of the 27
/// cells around it. A cell lists its vertices in the order they were built.
#[derive(Default)]
struct Grid {
    cells: rustc_hash::FxHashMap<[i64; 3], SmallVec<[u32; 4]>>,
}

impl Grid {
    fn cell(p: Vec3) -> [i64; 3] {
        let size = 2.0 * f64::from(EPS_L);
        [
            (f64::from(p.x) / size).floor() as i64,
            (f64::from(p.y) / size).floor() as i64,
            (f64::from(p.z) / size).floor() as i64,
        ]
    }

    fn insert(&mut self, p: Vec3, i: usize) {
        self.cells.entry(Self::cell(p)).or_default().push(i as u32);
    }

    fn forget(&mut self, p: Vec3, i: usize) {
        if let Some(list) = self.cells.get_mut(&Self::cell(p))
            && let Some(at) = list.iter().rposition(|&x| x as usize == i)
        {
            list.remove(at);
        }
    }

    /// The first vertex near `p` that `accept` takes.
    fn first(&self, p: Vec3, accept: impl Fn(usize) -> bool) -> Option<usize> {
        let c = Self::cell(p);
        let mut best: Option<usize> = None;
        for dx in -1i64..=1 {
            for dy in -1i64..=1 {
                for dz in -1i64..=1 {
                    let key = [c[0].saturating_add(dx), c[1].saturating_add(dy), c[2].saturating_add(dz)];
                    let Some(list) = self.cells.get(&key) else { continue };
                    for &i in list {
                        let i = i as usize;
                        if best.is_some_and(|b| b <= i) {
                            break;
                        }
                        if accept(i) {
                            best = Some(i);
                            break;
                        }
                    }
                }
            }
        }
        best
    }
}

fn similar3(a: Vec3, b: Vec3, e: f32) -> bool {
    (a.x - b.x).abs() < e && (a.y - b.y).abs() < e && (a.z - b.z).abs() < e
}

#[cfg(test)]
fn face_equal(a: &[u16; 3], f: &[u16; 3]) -> bool {
    (a[0] == f[0] && a[1] == f[1] && a[2] == f[2])
        || (a[0] == f[0] && a[2] == f[1] && a[1] == f[2])
        || (a[2] == f[0] && a[0] == f[1] && a[1] == f[2])
        || (a[2] == f[0] && a[1] == f[1] && a[0] == f[2])
        || (a[1] == f[0] && a[0] == f[1] && a[2] == f[2])
        || (a[1] == f[0] && a[2] == f[1] && a[0] == f[2])
}

fn degenerate(f: &[u16; 3]) -> bool {
    f[0] == f[1] || f[0] == f[2] || f[1] == f[2]
}

impl Ogf {
    /// `OGF_Vertex::similar`.
    fn similar(&self, a: &OgfVertex, b: &OgfVertex) -> bool {
        if !similar3(a.p, b.p, EPS_L)
            || !similar3(a.n, b.n, EPS_L)
            || !a.t.similar(&b.t)
            || !a.b.similar(&b.b)
        {
            return false;
        }
        for (i, (x, y)) in a.uv.iter().zip(&b.uv).enumerate() {
            let (w, h) = self.textures.get(i).map_or((1, 1), |t| (t.width.max(1), t.height.max(1)));
            let eu = 1.0 / w as f32;
            let ev = 1.0 / h as f32;
            if !((x.x - y.x).abs() < eu && (x.y - y.y).abs() < ev) {
                return false;
            }
        }
        true
    }

    #[cfg(test)]
    fn build_vertex(&mut self, v: &OgfVertex) -> u16 {
        if let Some(i) = self.data.vertices.iter().position(|x| self.similar(x, v)) {
            return i as u16;
        }
        self.data.vertices.push(v.clone());
        (self.data.vertices.len() - 1) as u16
    }

    #[cfg(test)]
    fn x_build_vertex(&mut self, p: Vec3) -> u16 {
        if let Some(i) = self.fast.vertices.iter().position(|x| similar3(*x, p, EPS_L)) {
            return i as u16;
        }
        self.fast.vertices.push(p);
        (self.fast.vertices.len() - 1) as u16
    }

    /// `_BuildFace` of every face in turn, as [`Self::build_face`] builds them: a vertex is the first
    /// similar one built before it, found through a grid of the positions (similar vertices lie
    /// within `EPS_L` of each other), and a face that repeats one through a set of the faces.
    pub fn build_faces<'a>(&mut self, faces: impl IntoIterator<Item = &'a [OgfVertex; 3]>, tc: bool) {
        let mut grid = Grid::default();
        for (i, v) in self.data.vertices.iter().enumerate() {
            grid.insert(v.p, i);
        }
        let mut xgrid = Grid::default();
        for (i, &p) in self.fast.vertices.iter().enumerate() {
            xgrid.insert(p, i);
        }
        let key = |f: &[u16; 3]| {
            let mut k = *f;
            k.sort_unstable();
            k
        };
        let mut seen: rustc_hash::FxHashSet<[u16; 3]> = self.data.faces.iter().map(key).collect();
        for v in faces {
            let count = self.data.vertices.len();
            let mut f = [0u16; 3];
            for (slot, x) in f.iter_mut().zip(v) {
                let found =
                    grid.first(x.p, |i| self.data.vertices.get(i).is_some_and(|y| self.similar(y, x)));
                *slot = match found {
                    Some(i) => i as u16,
                    None => {
                        self.data.vertices.push(x.clone());
                        grid.insert(x.p, self.data.vertices.len() - 1);
                        (self.data.vertices.len() - 1) as u16
                    }
                };
            }
            if degenerate(&f) {
                for (i, y) in self.data.vertices.iter().enumerate().skip(count) {
                    grid.forget(y.p, i);
                }
                self.data.vertices.truncate(count);
                continue;
            }
            // a repeated face is the same three corners in any order
            if !seen.insert(key(&f)) {
                continue;
            }
            self.data.faces.push(f);
            if tc {
                continue;
            }
            let xcount = self.fast.vertices.len();
            let mut x = [0u16; 3];
            for (slot, corner) in x.iter_mut().zip(v) {
                let p = corner.p;
                let found =
                    xgrid.first(p, |i| self.fast.vertices.get(i).is_some_and(|&q| similar3(q, p, EPS_L)));
                *slot = match found {
                    Some(i) => i as u16,
                    None => {
                        self.fast.vertices.push(p);
                        xgrid.insert(p, self.fast.vertices.len() - 1);
                        (self.fast.vertices.len() - 1) as u16
                    }
                };
            }
            if degenerate(&x) {
                for (i, &q) in self.fast.vertices.iter().enumerate().skip(xcount) {
                    xgrid.forget(q, i);
                }
                self.fast.vertices.truncate(xcount);
            } else {
                self.fast.faces.push(x);
            }
        }
    }

    /// `_BuildFace` (with `x_BuildFace` when `tc` is off: the fast path holds opaque faces).
    #[cfg(test)]
    pub fn build_face(&mut self, v: &[OgfVertex; 3], tc: bool) {
        let count = self.data.vertices.len();
        let f = [self.build_vertex(&v[0]), self.build_vertex(&v[1]), self.build_vertex(&v[2])];
        if degenerate(&f) {
            self.data.vertices.truncate(count);
            return;
        }
        if self.data.faces.iter().any(|x| face_equal(x, &f)) {
            return;
        }
        self.data.faces.push(f);
        if tc {
            return;
        }
        let xcount = self.fast.vertices.len();
        let x = [self.x_build_vertex(v[0].p), self.x_build_vertex(v[1].p), self.x_build_vertex(v[2].p)];
        if degenerate(&x) {
            self.fast.vertices.truncate(xcount);
        } else {
            self.fast.faces.push(x);
        }
    }

    /// Optimizes: with the UV optimisation, every group of faces joined by their vertices gets its
    /// first texture coordinates moved by whole tiles near the origin.
    pub fn optimize(&mut self, optimize_uv: bool) {
        if self.data.vertices.is_empty() || !optimize_uv {
            return;
        }
        // the groups `adjacent_select` grows are the components joined by shared vertices
        let n = self.data.vertices.len();
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], mut x: usize) -> usize {
            while p[x] != x {
                p[x] = p[p[x]];
                x = p[x];
            }
            x
        }
        let mut used = vec![false; n];
        for f in &self.data.faces {
            let [a, b, c] = f.map(usize::from);
            for &v in &[a, b, c] {
                if let Some(u) = used.get_mut(v) {
                    *u = true;
                }
            }
            if a < n && b < n && c < n {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                parent[rb] = ra;
                let (ra, rc) = (find(&mut parent, a), find(&mut parent, c));
                parent[rc] = ra;
            }
        }
        let mut lo: Vec<Vec2> = vec![Vec2::splat(f32::MAX); n];
        let mut hi: Vec<Vec2> = vec![Vec2::splat(-f32::MAX); n];
        for v in (0..n).filter(|&v| used[v]) {
            let r = find(&mut parent, v);
            let uv = self.data.vertices[v].uv.first().copied().unwrap_or(Vec2::ZERO);
            lo[r] = Vec2::new(
                if uv.x < lo[r].x { uv.x } else { lo[r].x },
                if uv.y < lo[r].y { uv.y } else { lo[r].y },
            );
            hi[r] = Vec2::new(
                if uv.x > hi[r].x { uv.x } else { hi[r].x },
                if uv.y > hi[r].y { uv.y } else { hi[r].y },
            );
        }
        for v in (0..n).filter(|&v| used[v]) {
            let r = find(&mut parent, v);
            let d = Vec2::new(
                ((hi[r].x - lo[r].x) / 2.0 + lo[r].x).floor(),
                ((hi[r].y - lo[r].y) / 2.0 + lo[r].y).floor(),
            );
            if let Some(uv) = self.data.vertices[v].uv.first_mut() {
                *uv = Vec2::new(uv.x - d.x, uv.y - d.y);
            }
        }
    }

    /// The positions the geometry gives.
    #[must_use]
    pub fn geometry(&self) -> Vec<Vec3> {
        self.data.vertices.iter().map(|v| v.p).collect()
    }

    #[must_use]
    pub fn progressive(&self) -> bool {
        !self.data.swi.is_empty()
    }

    /// The VIPM collapses of the model, kept when they remove at least
    /// `metric` of its vertices; then the same for the fast path. Answers the log lines.
    pub fn make_progressive(&mut self, metric: f32, draft: bool, log: &mut Vec<String>) {
        if self.data.faces.len() < PM_FACE_LIMIT || draft {
            return;
        }
        let saved_v = self.data.vertices.clone();
        let saved_f = self.data.faces.clone();
        let verts: Vec<(Vec3, Vec2)> =
            self.data.vertices.iter().map(|v| (v.p, v.uv.first().copied().unwrap_or(Vec2::ZERO))).collect();
        match vipm_convert(&verts, &self.data.faces, PM_MAX_WINDOW) {
            None => {
                self.data.swi.clear();
                log.push("* mesh simplification failed".to_owned());
            }
            Some(vr) if !vr.swr.is_empty() => {
                let full = self.data.vertices.len();
                let remove = vr.swr.len();
                let simple = full.wrapping_sub(remove);
                let m = remove as f32 / full as f32;
                if m < metric {
                    self.data.swi.clear();
                    log.push(format!(
                        "* mesh simplified from [{full:4}v] to [{simple:4}v], nf[{:4}] ==> em[{metric:0.2}]-discarded",
                        vr.indices.len() / 3
                    ));
                } else {
                    log.push(format!(
                        "* mesh simplified from [{full:4}v] to [{simple:4}v], nf[{:4}] ==> em[{metric:0.2}]-accepted",
                        vr.indices.len() / 3
                    ));
                    for (i, &to) in vr.permute_verts.iter().enumerate() {
                        if let (Some(dst), Some(src)) =
                            (self.data.vertices.get_mut(usize::from(to)), saved_v.get(i))
                        {
                            *dst = src.clone();
                        }
                    }
                    self.data.faces =
                        vr.indices.as_chunks::<3>().0.iter().map(|t| [t[0], t[1], t[2]]).collect();
                    self.data.swi = vr
                        .swr
                        .iter()
                        .map(|s| SlideWindow {
                            offset: s.offset,
                            num_tris: s.num_tris,
                            num_verts: s.num_verts,
                        })
                        .collect();
                }
            }
            Some(_) => {}
        }
        if !self.progressive() || self.fast.vertices.is_empty() || self.fast.faces.is_empty() {
            return;
        }
        let verts: Vec<(Vec3, Vec2)> = self.fast.vertices.iter().map(|p| (*p, Vec2::ZERO)).collect();
        match vipm_convert(&verts, &self.fast.faces, PM_MAX_WINDOW) {
            None => {
                self.data.faces = saved_f;
                self.data.vertices = saved_v;
                self.data.swi.clear();
                log.push("* X-mesh simplification failed".to_owned());
            }
            Some(vr) => {
                let full = self.data.vertices.len();
                let remove = vr.swr.len();
                log.push(format!(
                    "X mesh simplified from [{full:4}v] to [{:4}v], nf[{:4}]",
                    full.wrapping_sub(remove),
                    vr.indices.len() / 3
                ));
                let saved = self.fast.vertices.clone();
                for (i, &to) in vr.permute_verts.iter().enumerate() {
                    if let (Some(dst), Some(src)) =
                        (self.fast.vertices.get_mut(usize::from(to)), saved.get(i))
                    {
                        *dst = *src;
                    }
                }
                self.fast.faces = vr.indices.as_chunks::<3>().0.iter().map(|t| [t[0], t[1], t[2]]).collect();
                self.fast.swi = vr
                    .swr
                    .iter()
                    .map(|s| SlideWindow { offset: s.offset, num_tris: s.num_tris, num_verts: s.num_verts })
                    .collect();
            }
        }
    }

    /// A progressive model is left as it is; the fast path through the D3DX
    /// orders, the model through NvTriStrip and the first-use renumbering.
    pub fn stripify(&mut self, log: &mut Vec<String>) {
        if self.progressive() {
            return;
        }
        if !self.fast.vertices.is_empty() && !self.fast.faces.is_empty() {
            let flat: Vec<u16> = self.fast.faces.iter().flatten().copied().collect();
            let remap = crate::vcache::optimize_faces(&flat, self.fast.vertices.len());
            let source = self.fast.faces.clone();
            for (slot, r) in self.fast.faces.iter_mut().zip(&remap) {
                if let Some(f) = source.get(*r as usize) {
                    *slot = *f;
                }
            }
            let flat: Vec<u16> = self.fast.faces.iter().flatten().copied().collect();
            let remap = optimize_vertices(&flat, self.fast.vertices.len());
            let source = self.fast.vertices.clone();
            for (old, &new) in remap.iter().enumerate() {
                if let (Some(dst), Some(src)) = (self.fast.vertices.get_mut(new as usize), source.get(old)) {
                    *dst = *src;
                }
            }
            for f in &mut self.fast.faces {
                for v in f.iter_mut() {
                    *v = remap.get(usize::from(*v)).map_or(*v, |&x| x as u16);
                }
            }
        }
        let flat: Vec<u16> = self.data.faces.iter().flatten().copied().collect();
        match stripify(&flat, self.data.vertices.len(), VCACHE_SIZE) {
            Some((indices, permute)) => {
                self.data.faces = indices.as_chunks::<3>().0.iter().map(|t| [t[0], t[1], t[2]]).collect();
                let temp = self.data.vertices.clone();
                for (i, &p) in permute.iter().enumerate() {
                    if let (Some(dst), Some(src)) = (self.data.vertices.get_mut(i), temp.get(usize::from(p)))
                    {
                        *dst = src.clone();
                    }
                }
            }
            None => log.push("ERROR: [slow-vert] Stripifying failed. Dump below.".to_owned()),
        }
    }

    /// `remove_isolated_verts`: the vertices renumbered in first use by the faces, the unused
    /// ones dropped.
    pub fn remove_isolated(&mut self) {
        let mut remap = vec![u32::MAX; self.data.vertices.len()];
        let mut verts = Vec::with_capacity(self.data.vertices.len());
        for f in &mut self.data.faces {
            for v in f.iter_mut() {
                let old = usize::from(*v);
                let Some(slot) = remap.get_mut(old) else { continue };
                if *slot == u32::MAX {
                    if let Some(src) = self.data.vertices.get(old) {
                        verts.push(src.clone());
                    }
                    *slot = (verts.len() - 1) as u32;
                }
                *v = *slot as u16;
            }
        }
        self.data.vertices = verts;
    }

    /// The tangents and binormals through MeshMender, the vertices it splits
    /// added.
    pub fn calculate_tb(&mut self) {
        self.remove_isolated();
        let mut mv: Vec<MVertex> = self
            .data
            .vertices
            .iter()
            .map(|v| {
                let uv = v.uv.first().copied().unwrap_or(Vec2::ZERO);
                MVertex {
                    pos: v.p.to_array(),
                    normal: v.n.to_array(),
                    s: uv.x,
                    t: uv.y,
                    ..MVertex::default()
                }
            })
            .collect();
        let mut idx: Vec<u32> = self.data.faces.iter().flatten().map(|&v| u32::from(v)).collect();
        let mapping = mend(&mut mv, &mut idx, 0.5, 0.5);
        let old = std::mem::take(&mut self.data.vertices);
        for t in idx.as_chunks::<3>().0.iter().zip(self.data.faces.iter_mut()) {
            *t.1 = [t.0[0] as u16, t.0[1] as u16, t.0[2] as u16];
        }
        self.data.vertices = mv
            .iter()
            .zip(&mapping)
            .map(|(m, &from)| {
                let mut v = old.get(from as usize).cloned().unwrap_or_else(|| {
                    old.first().cloned().unwrap_or(OgfVertex {
                        p: Vec3::ZERO,
                        n: Vec3::ZERO,
                        t: Basis::default(),
                        b: Basis::default(),
                        color: BaseColor::default(),
                        uv: SmallVec::new(),
                    })
                });
                v.p = Vec3::from_array(m.pos);
                v.n = Vec3::from_array(m.normal);
                if let Some(uv) = v.uv.first_mut() {
                    *uv = Vec2::new(m.s, m.t);
                }
                v.t = Basis::of(Vec3::from_array(m.tangent));
                v.b = Basis::of(Vec3::from_array(m.binormal));
                v
            })
            .collect();
        self.remove_isolated();
    }
}

/// A normal in a byte per axis.
#[must_use]
pub fn u8_vec4(n: Vec3, a: u8) -> [u8; 4] {
    let x = (n.x + 1.0) * (0.5 * 255.0);
    let y = (n.y + 1.0) * (0.5 * 255.0);
    let z = (n.z + 1.0) * (0.5 * 255.0);
    let b = |v: f32| i_floor(v).clamp(0, 255) as u8;
    // D3DCOLOR in memory: b, g, r, a
    [b(z), b(y), b(x), a]
}

/// `u8_vec4(base_basis, A)`.
#[must_use]
pub fn basis_vec4(n: &Basis, a: u8) -> [u8; 4] {
    [n.z, n.y, n.x, a]
}

/// `s24_tc_base`: a texture coordinate of `[-32, 32]` in 16 bits and a fraction.
#[must_use]
pub fn s24_tc_base(uv: f32) -> (i16, u8) {
    const QUANT: i32 = 32768 / 32;
    let rebased = uv * QUANT as f32;
    let primary = i_floor(rebased).clamp(-32768, 32767);
    let secondary = i_floor(255.5 * (rebased - primary as f32)).clamp(0, 255);
    (primary as i16, secondary as u8)
}

/// `s16_tc_lmap`.
#[must_use]
pub fn s16_tc_lmap(uv: f32) -> i16 {
    i_floor(uv * 32768.0).clamp(-32768, 32767) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vert(p: Vec3, uv: Vec2) -> OgfVertex {
        OgfVertex {
            p,
            n: Vec3::Y,
            t: Basis::default(),
            b: Basis::default(),
            color: BaseColor::default(),
            uv: SmallVec::from_slice(&[uv]),
        }
    }

    /// The grid and the set build what the scans built: corners a hair apart and a hair more, on the
    /// borders of the grid's cells, with texture coordinates within a texel and past it, normals that
    /// differ, faces repeated in any order and degenerate ones, the fast path too.
    #[test]
    fn the_faces_are_built_as_the_scans_built_them() {
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let mut corners: Vec<OgfVertex> = Vec::new();
        for _ in 0..600 {
            let lattice = |n: &mut dyn FnMut() -> u64| (n() % 8) as f32 * 0.002 - 0.004;
            let nudge = |n: &mut dyn FnMut() -> u64| (n() % 5) as f32 * 0.0004 - 0.0008;
            let p = Vec3::new(
                lattice(&mut next) + nudge(&mut next),
                lattice(&mut next),
                lattice(&mut next) + nudge(&mut next),
            );
            let uv = Vec2::new((next() % 4) as f32 * 0.002, (next() % 3) as f32 * 0.003);
            let mut v = vert(p, uv);
            if next() % 5 == 0 {
                v.n = Vec3::new(0.0, 0.9995, 0.0316);
            }
            corners.push(v);
        }
        let mut faces: Vec<[OgfVertex; 3]> = Vec::new();
        for i in 0..3000 {
            let mut pick = || corners[(next() % corners.len() as u64) as usize].clone();
            let f = [pick(), pick(), pick()];
            if i % 9 == 0
                && let Some(last) = faces.last().cloned()
            {
                faces.push([last[2].clone(), last[0].clone(), last[1].clone()]);
            }
            faces.push(f);
        }
        for tc in [false, true] {
            let blank = Ogf {
                textures: vec![OgfTexture { name: "t".into(), width: 256, height: 128 }],
                ..Ogf::default()
            };
            let mut old = blank.clone();
            for f in &faces {
                old.build_face(f, tc);
            }
            let mut new = blank;
            new.build_faces(&faces, tc);
            assert!(old == new, "tc {tc}");
            assert!(
                old.data.faces.len() > 500 && old.data.faces.len() + 300 < faces.len(),
                "{} faces",
                old.data.faces.len()
            );
        }
    }

    #[test]
    fn faces_weld_their_vertices_and_skip_repeats() {
        let mut o = Ogf {
            textures: vec![OgfTexture { name: "t".into(), width: 256, height: 256 }],
            ..Ogf::default()
        };
        let a = vert(Vec3::ZERO, Vec2::ZERO);
        let b = vert(Vec3::X, Vec2::X);
        let c = vert(Vec3::Z, Vec2::Y);
        let d = vert(Vec3::new(1.0, 0.0, 1.0), Vec2::ONE);
        o.build_face(&[a.clone(), b.clone(), c.clone()], false);
        o.build_face(&[b.clone(), d.clone(), c.clone()], false);
        o.build_face(&[c.clone(), a.clone(), b.clone()], false);
        assert_eq!(o.data.vertices.len(), 4);
        assert_eq!(o.data.faces, vec![[0, 1, 2], [1, 3, 2]]);
        assert_eq!(o.fast.faces.len(), 2);
        // a degenerate face leaves no vertex behind
        o.build_face(&[a.clone(), a.clone(), vert(Vec3::splat(5.0), Vec2::ZERO)], false);
        assert_eq!(o.data.vertices.len(), 4);
    }

    #[test]
    fn texture_coordinates_move_by_whole_tiles() {
        let mut o = Ogf {
            textures: vec![OgfTexture { name: "t".into(), width: 256, height: 256 }],
            ..Ogf::default()
        };
        o.build_face(
            &[
                vert(Vec3::ZERO, Vec2::new(5.2, -3.1)),
                vert(Vec3::X, Vec2::new(6.4, -3.0)),
                vert(Vec3::Z, Vec2::new(5.0, -2.5)),
            ],
            true,
        );
        o.optimize(true);
        // the middle of (5.0..6.4, -3.1..-2.5) floors to (5, -3)
        assert!((o.data.vertices[0].uv[0] - Vec2::new(0.2, -0.1)).length() < 1e-5);
    }

    #[test]
    fn the_packed_coordinates() {
        assert_eq!(s24_tc_base(1.0), (1024, 0));
        assert_eq!(s24_tc_base(-0.5), (-512, 0));
        assert_eq!(s16_tc_lmap(0.5), 16384);
        assert_eq!(u8_vec4(Vec3::Y, 7), [127, 255, 127, 7]);
    }
}
