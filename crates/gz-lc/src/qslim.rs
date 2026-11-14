//! QSlim as the collision-form simplification runs it: edge contractions ordered by the quadric
//! error, endpoint placement, uniform weighting, the boundary and manual constraints, the
//! meshing penalties.
//!
//! Vertices, faces and edges are numbered as X-Ray's compiler allocates them; an edge's id stands for
//! its address where X-Ray's compiler sorts edges by pointer (`apply_contraction`). Lists keep X-Ray's
//! order, a removal from one is "copy the last over it".

use rayon::prelude::*;
use smallvec::SmallVec;

/// The placement modes (endpoints and friends).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Placement {
    Endpoints = 0,
    EndOrMid = 1,
    Line = 2,
    Optimal = 3,
}

/// The weighting modes: uniform and by area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weighting {
    Uniform,
    Area,
}

const EDGE_BASE_ERROR: f32 = 1.0;
const NOT_IN_HEAP: i32 = -47;

type V3 = [f64; 3];

fn sub3(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot3(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross3(u: V3, v: V3) -> V3 {
    [u[1] * v[2] - v[1] * u[2], -u[0] * v[2] + v[0] * u[2], u[0] * v[1] - v[0] * u[1]]
}
fn norm2(v: V3) -> f64 {
    dot3(v, v)
}
/// `unitize(TVec3<double>&)`.
fn unitize(v: V3) -> V3 {
    let l = norm2(v);
    if l != 1.0 && l != 0.0 {
        let s = l.sqrt();
        [v[0] / s, v[1] / s, v[2] / s]
    } else {
        v
    }
}
fn widen(v: [f32; 3]) -> V3 {
    [f64::from(v[0]), f64::from(v[1]), f64::from(v[2])]
}

/// `mxv_*` over floats.
fn fsub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn fdot(a: [f32; 3], b: [f32; 3]) -> f32 {
    let mut d = 0.0f32;
    for i in 0..3 {
        d += a[i] * b[i];
    }
    d
}
fn fcross(u: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    [u[1] * v[2] - v[1] * u[2], -u[0] * v[2] + v[0] * u[2], u[0] * v[1] - v[0] * u[1]]
}
fn funitize(v: [f32; 3]) -> [f32; 3] {
    let l = fdot(v, v);
    if l != 1.0 && l != 0.0 {
        let s = l.sqrt();
        [v[0] / s, v[1] / s, v[2] / s]
    } else {
        v
    }
}

/// `triangle_normal`.
fn triangle_normal(v1: V3, v2: V3, v3: V3) -> V3 {
    unitize(cross3(sub3(v2, v1), sub3(v3, v1)))
}

/// `triangle_compactness`.
fn triangle_compactness(v1: V3, v2: V3, v3: V3) -> f64 {
    const FOUR_ROOT3: f64 = 6.928_203_230_275_509;
    let area = 0.5 * norm2(cross3(sub3(v2, v1), sub3(v3, v1))).sqrt();
    FOUR_ROOT3 * area / (norm2(sub3(v2, v1)) + norm2(sub3(v3, v2)) + norm2(sub3(v1, v3)))
}

/// A quadric of three dimensions.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Quadric {
    a2: f64,
    ab: f64,
    ac: f64,
    ad: f64,
    b2: f64,
    bc: f64,
    bd: f64,
    c2: f64,
    cd: f64,
    d2: f64,
    r: f64,
}

impl Quadric {
    fn new(a: f64, b: f64, c: f64, d: f64, area: f64) -> Self {
        Self {
            a2: a * a,
            ab: a * b,
            ac: a * c,
            ad: a * d,
            b2: b * b,
            bc: b * c,
            bd: b * d,
            c2: c * c,
            cd: c * d,
            d2: d * d,
            r: area,
        }
    }

    fn add(&mut self, q: &Quadric) {
        self.r += q.r;
        self.a2 += q.a2;
        self.ab += q.ab;
        self.ac += q.ac;
        self.ad += q.ad;
        self.b2 += q.b2;
        self.bc += q.bc;
        self.bd += q.bd;
        self.c2 += q.c2;
        self.cd += q.cd;
        self.d2 += q.d2;
    }

    fn scale(&mut self, s: f64) {
        self.a2 *= s;
        self.ab *= s;
        self.ac *= s;
        self.ad *= s;
        self.b2 *= s;
        self.bc *= s;
        self.bd *= s;
        self.c2 *= s;
        self.cd *= s;
        self.d2 *= s;
    }

    fn eval(&self, v: V3) -> f64 {
        let (x, y, z) = (v[0], v[1], v[2]);
        x * x * self.a2
            + 2.0 * x * y * self.ab
            + 2.0 * x * z * self.ac
            + 2.0 * x * self.ad
            + y * y * self.b2
            + 2.0 * y * z * self.bc
            + 2.0 * y * self.bd
            + z * z * self.c2
            + 2.0 * z * self.cd
            + self.d2
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Tags {
    mark: u8,
    tag: u8,
}

const VALID: u8 = 1;

/// A pair contraction.
#[derive(Clone, Debug, Default)]
struct Contraction {
    v1: u32,
    v2: u32,
    dv1: [f32; 3],
    delta_pivot: usize,
    delta_faces: Vec<u32>,
    dead_faces: Vec<u32>,
}

/// What the simplification builds and reads back.
#[derive(Clone, Debug, Default)]
pub struct Model {
    verts: Vec<[f32; 3]>,
    faces: Vec<[u32; 3]>,
    v_data: Vec<Tags>,
    f_data: Vec<Tags>,
    links: Vec<Vec<u32>>,
}

fn remove_at<T: Copy>(list: &mut Vec<T>, i: usize) {
    if i < list.len() {
        list.swap_remove(i);
    }
}

impl Model {
    #[must_use]
    pub fn with_capacity(verts: usize, faces: usize) -> Self {
        Self {
            verts: Vec::with_capacity(verts),
            faces: Vec::with_capacity(faces),
            v_data: Vec::with_capacity(verts),
            f_data: Vec::with_capacity(faces),
            links: Vec::with_capacity(verts),
        }
    }

    /// `add_vertex`.
    pub fn add_vertex(&mut self, p: [f32; 3]) -> u32 {
        self.verts.push(p);
        self.v_data.push(Tags { tag: VALID, ..Tags::default() });
        self.links.push(Vec::with_capacity(6));
        (self.verts.len() - 1) as u32
    }

    /// `add_face` (linked into its corners' neighbour lists).
    pub fn add_face(&mut self, f: [u32; 3]) -> u32 {
        let id = self.faces.len() as u32;
        self.faces.push(f);
        self.f_data.push(Tags { tag: VALID, ..Tags::default() });
        for v in f {
            if let Some(l) = self.links.get_mut(v as usize) {
                l.push(id);
            }
        }
        id
    }

    #[must_use]
    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    #[must_use]
    pub fn face(&self, f: u32) -> [u32; 3] {
        self.faces.get(f as usize).copied().unwrap_or([0; 3])
    }

    #[must_use]
    pub fn vertex(&self, v: u32) -> [f32; 3] {
        self.verts.get(v as usize).copied().unwrap_or([0.0; 3])
    }

    #[must_use]
    pub fn face_is_valid(&self, f: u32) -> bool {
        self.f_data.get(f as usize).is_some_and(|t| t.tag & VALID != 0)
    }

    fn vertex_is_valid(&self, v: u32) -> bool {
        self.v_data.get(v as usize).is_some_and(|t| t.tag & VALID != 0)
    }

    #[must_use]
    pub fn neighbors(&self, v: u32) -> &[u32] {
        self.links.get(v as usize).map_or(&[], Vec::as_slice)
    }

    fn fmark(&self, f: u32) -> u8 {
        self.f_data.get(f as usize).map_or(0, |t| t.mark)
    }
    fn set_fmark(&mut self, f: u32, m: u8) {
        if let Some(t) = self.f_data.get_mut(f as usize) {
            t.mark = m;
        }
    }
    fn vmark(&self, v: u32) -> u8 {
        self.v_data.get(v as usize).map_or(0, |t| t.mark)
    }
    fn set_vmark(&mut self, v: u32, m: u8) {
        if let Some(t) = self.v_data.get_mut(v as usize) {
            t.mark = m;
        }
    }

    fn mark_neighborhood(&mut self, v: u32, mark: u8) {
        for i in 0..self.neighbors(v).len() {
            let f = self.neighbors(v)[i];
            self.set_fmark(f, mark);
        }
    }

    fn mark_neighborhood_delta(&mut self, v: u32, delta: u8) {
        for i in 0..self.neighbors(v).len() {
            let f = self.neighbors(v)[i];
            let m = self.fmark(f).wrapping_add(delta);
            self.set_fmark(f, m);
        }
    }

    fn collect_unmarked_neighbors(&mut self, v: u32, out: &mut Vec<u32>) {
        for i in 0..self.neighbors(v).len() {
            let f = self.neighbors(v)[i];
            if self.fmark(f) == 0 {
                out.push(f);
                self.set_fmark(f, 1);
            }
        }
    }

    fn partition_marked_neighbors(&mut self, v: u32, pivot: u8, lo: &mut Vec<u32>, hi: &mut Vec<u32>) {
        for i in 0..self.neighbors(v).len() {
            let f = self.neighbors(v)[i];
            let m = self.fmark(f);
            if m != 0 {
                if m < pivot {
                    lo.push(f);
                } else {
                    hi.push(f);
                }
                self.set_fmark(f, 0);
            }
        }
    }

    /// `collect_edge_neighbors`.
    fn collect_edge_neighbors(&mut self, v1: u32, v2: u32, out: &mut Vec<u32>) {
        self.mark_neighborhood(v1, 1);
        self.mark_neighborhood(v2, 0);
        self.collect_unmarked_neighbors(v1, out);
    }

    /// `collect_vertex_star`.
    fn collect_vertex_star(&mut self, v: u32, out: &mut Vec<u32>) {
        let n = self.neighbors(v).to_vec();
        for &f in &n {
            for c in self.face(f) {
                self.set_vmark(c, 0);
            }
        }
        self.set_vmark(v, 1);
        for &f in &n {
            for c in self.face(f) {
                if self.vmark(c) == 0 {
                    out.push(c);
                    self.set_vmark(c, 1);
                }
            }
        }
    }

    /// `compute_face_normal(f, double*, true)`.
    fn face_normal_d(&self, f: u32) -> V3 {
        let [a, b, c] = self.face(f).map(|v| self.vertex(v));
        let mut x = [0.0f64; 3];
        let mut y = [0.0f64; 3];
        for i in 0..3 {
            x[i] = f64::from(b[i] - a[i]);
            y[i] = f64::from(c[i] - a[i]);
        }
        unitize(cross3(x, y))
    }

    /// `compute_face_area`.
    fn face_area(&self, f: u32) -> f64 {
        let [a, b, c] = self.face(f).map(|v| self.vertex(v));
        let mut x = [0.0f64; 3];
        let mut y = [0.0f64; 3];
        for i in 0..3 {
            x[i] = f64::from(b[i] - a[i]);
            y[i] = f64::from(c[i] - a[i]);
        }
        0.5 * norm2(cross3(x, y)).sqrt()
    }

    fn unlink_face(&mut self, f: u32) {
        if let Some(t) = self.f_data.get_mut(f as usize) {
            t.tag &= !VALID;
        }
        let corners = self.face(f);
        for v in corners {
            if let Some(l) = self.links.get_mut(v as usize)
                && let Some(j) = l.iter().position(|&x| x == f)
            {
                remove_at(l, j);
            }
        }
    }

    fn remap_face(&mut self, f: u32, from: u32, to: u32) {
        if let Some(face) = self.faces.get_mut(f as usize) {
            for v in face.iter_mut() {
                if *v == from {
                    *v = to;
                }
            }
        }
    }

    /// `compute_contraction(v1, v2, conx, vnew)`.
    fn compute_contraction(&mut self, v1: u32, v2: u32, vnew: [f32; 3]) -> Contraction {
        let mut c = Contraction {
            v1,
            v2,
            dv1: fsub(vnew, self.vertex(v1)),
            delta_pivot: 0,
            ..Contraction::default()
        };
        self.mark_neighborhood(v2, 0);
        self.mark_neighborhood(v1, 1);
        self.mark_neighborhood_delta(v2, 1);
        let (mut delta, mut dead) = (Vec::new(), Vec::new());
        self.partition_marked_neighbors(v1, 2, &mut delta, &mut dead);
        c.delta_pivot = delta.len();
        self.partition_marked_neighbors(v2, 2, &mut delta, &mut dead);
        c.delta_faces = delta;
        c.dead_faces = dead;
        c
    }

    /// Applies a contraction to the model.
    fn apply_contraction(&mut self, c: &Contraction) {
        if let Some(p) = self.verts.get_mut(c.v1 as usize) {
            for (x, d) in p.iter_mut().zip(c.dv1) {
                *x += d;
            }
        }
        for &f in &c.dead_faces {
            self.unlink_face(f);
        }
        for &f in c.delta_faces.iter().skip(c.delta_pivot) {
            self.remap_face(f, c.v2, c.v1);
            if let Some(l) = self.links.get_mut(c.v1 as usize) {
                l.push(f);
            }
        }
        if let Some(t) = self.v_data.get_mut(c.v2 as usize) {
            t.tag &= !VALID;
        }
        if let Some(l) = self.links.get_mut(c.v2 as usize) {
            l.clear();
        }
    }

    /// `compact_vertices`: the valid vertices moved down in order, the faces renumbered.
    pub fn compact_vertices(&mut self) {
        let mut new_id = 0u32;
        for old in 0..self.verts.len() as u32 {
            if !self.vertex_is_valid(old) {
                continue;
            }
            if new_id != old {
                let p = self.vertex(old);
                if let Some(slot) = self.verts.get_mut(new_id as usize) {
                    *slot = p;
                }
                self.links.swap(new_id as usize, old as usize);
                if let Some(t) = self.v_data.get_mut(new_id as usize) {
                    t.tag |= VALID;
                }
                let n = self.neighbors(new_id).to_vec();
                for f in n {
                    self.remap_face(f, old, new_id);
                }
            }
            new_id += 1;
        }
        self.verts.truncate(new_id as usize);
        self.v_data.truncate(new_id as usize);
        self.links.truncate(new_id as usize);
    }
}

#[derive(Clone, Copy, Debug)]
struct Edge {
    v1: u32,
    v2: u32,
    vnew: [f32; 3],
    key: f32,
    pos: i32,
}

/// The edge simplification with the settings of its caller.
pub struct EdgeQSlim {
    pub model: Model,
    quadrics: Vec<Quadric>,
    edge_links: Vec<Vec<usize>>,
    edges: Vec<Edge>,
    heap: Vec<usize>,
    pub valid_verts: u32,
    pub valid_faces: u32,
    pub placement: Placement,
    pub weighting: Weighting,
    pub boundary_weight: f64,
    pub compactness_ratio: f64,
    pub meshing_penalty: f64,
    local_validity_threshold: f32,
    vertex_degree_limit: usize,
}

impl EdgeQSlim {
    /// The edge simplification with the defaults.
    #[must_use]
    pub fn new(model: Model) -> Self {
        let valid_faces = (0..model.faces.len() as u32).filter(|&f| model.face_is_valid(f)).count() as u32;
        let valid_verts = (0..model.verts.len() as u32).filter(|&v| model.vertex_is_valid(v)).count() as u32;
        let n = model.verts.len();
        Self {
            model,
            quadrics: vec![Quadric::default(); n],
            edge_links: vec![Vec::with_capacity(6); n],
            edges: Vec::new(),
            heap: Vec::with_capacity(64),
            valid_verts,
            valid_faces,
            placement: Placement::Endpoints,
            weighting: Weighting::Area,
            boundary_weight: 1000.0,
            compactness_ratio: 1.0,
            meshing_penalty: 1000.0,
            local_validity_threshold: 0.0,
            vertex_degree_limit: 24,
        }
    }

    /// `initialize`: the quadrics of the faces and of the boundaries.
    pub fn initialize(&mut self) {
        self.collect_quadrics();
        if self.boundary_weight > 0.0 {
            self.constrain_boundaries();
        }
    }

    fn collect_quadrics(&mut self) {
        for q in &mut self.quadrics {
            *q = Quadric::default();
        }
        for f in 0..self.model.faces.len() as u32 {
            let face = self.model.face(f);
            let [v1, v2, v3] = face.map(|v| widen(self.model.vertex(v)));
            let n = triangle_normal(v1, v2, v3);
            let d = -dot3(n, v1);
            let mut q = Quadric::new(n[0], n[1], n[2], d, self.model.face_area(f));
            if self.weighting == Weighting::Area {
                let a = q.r;
                q.scale(a);
            }
            for v in face {
                if let Some(slot) = self.quadrics.get_mut(v as usize) {
                    slot.add(&q);
                }
            }
        }
    }

    fn discontinuity_constraint(&mut self, i: u32, j: u32, f: u32) {
        let org = widen(self.model.vertex(i));
        let dest = widen(self.model.vertex(j));
        let e = sub3(dest, org);
        let n = self.model.face_normal_d(f);
        let n2 = unitize(cross3(e, n));
        let mut q = Quadric::new(n2[0], n2[1], n2[2], -dot3(n2, org), 1.0);
        q.scale(self.boundary_weight);
        if self.weighting == Weighting::Area {
            q.r = norm2(e);
            let a = q.r;
            q.scale(a);
        }
        for v in [i, j] {
            if let Some(slot) = self.quadrics.get_mut(v as usize) {
                slot.add(&q);
            }
        }
    }

    fn constrain_boundaries(&mut self) {
        let mut star = Vec::new();
        let mut faces = Vec::new();
        for i in 0..self.model.verts.len() as u32 {
            star.clear();
            self.model.collect_vertex_star(i, &mut star);
            for &s in &star.clone() {
                if i < s {
                    faces.clear();
                    self.model.collect_edge_neighbors(i, s, &mut faces);
                    if faces.len() == 1 {
                        for &f in &faces.clone() {
                            self.discontinuity_constraint(i, s, f);
                        }
                    }
                }
            }
        }
    }

    /// `constraint_manual`.
    pub fn constraint_manual(&mut self, v0: u32, v1: u32, f: u32) {
        self.discontinuity_constraint(v0, v1, f);
    }

    // ---------------------------------------------------------------- the heap (MxHeap)

    fn key(&self, e: usize) -> f32 {
        self.edges.get(e).map_or(0.0, |x| x.key)
    }
    fn place(&mut self, e: usize, i: usize) {
        if let Some(slot) = self.heap.get_mut(i) {
            *slot = e;
        }
        if let Some(x) = self.edges.get_mut(e) {
            x.pos = i as i32;
        }
    }
    fn at(&self, i: usize) -> usize {
        self.heap.get(i).copied().unwrap_or(0)
    }
    fn upheap(&mut self, i: usize) {
        let moving = self.at(i);
        let mut index = i;
        let mut p = i.wrapping_sub(1) / 2;
        while index > 0 && self.key(moving) > self.key(self.at(p)) {
            let ep = self.at(p);
            self.place(ep, index);
            index = p;
            p = p.wrapping_sub(1) / 2;
        }
        if index != i {
            self.place(moving, index);
        }
    }
    fn downheap(&mut self, i: usize) {
        let moving = self.at(i);
        let mut index = i;
        let mut l = 2 * i + 1;
        let mut r = 2 * i + 2;
        let len = self.heap.len();
        while l < len {
            let largest = if r < len && self.key(self.at(l)) < self.key(self.at(r)) { r } else { l };
            if self.key(moving) < self.key(self.at(largest)) {
                let e = self.at(largest);
                self.place(e, index);
                index = largest;
                l = 2 * index + 1;
                r = 2 * index + 2;
            } else {
                break;
            }
        }
        if index != i {
            self.place(moving, index);
        }
    }
    fn heap_insert(&mut self, e: usize) {
        self.heap.push(e);
        let i = self.heap.len() - 1;
        if let Some(x) = self.edges.get_mut(e) {
            x.pos = i as i32;
        }
        self.upheap(i);
    }
    fn heap_update(&mut self, e: usize) {
        let i = self.edges.get(e).map_or(0, |x| x.pos.max(0) as usize);
        let v = self.key(e);
        if i > 0 && v > self.key(self.at((i - 1) / 2)) {
            self.upheap(i);
        } else {
            self.downheap(i);
        }
    }
    fn heap_swap(&mut self, i: usize, j: usize) {
        let a = self.at(i);
        let b = self.at(j);
        self.place(b, i);
        self.place(a, j);
    }
    fn heap_extract(&mut self) -> Option<usize> {
        if self.heap.is_empty() {
            return None;
        }
        let last = self.heap.len() - 1;
        self.heap_swap(0, last);
        let dead = self.heap.pop()?;
        if !self.heap.is_empty() {
            self.downheap(0);
        }
        if let Some(x) = self.edges.get_mut(dead) {
            x.pos = NOT_IN_HEAP;
        }
        Some(dead)
    }
    fn heap_remove(&mut self, e: usize) {
        let Some(pos) = self.edges.get(e).map(|x| x.pos) else { return };
        if pos == NOT_IN_HEAP {
            return;
        }
        let i = pos as usize;
        let last = self.heap.len() - 1;
        self.heap_swap(i, last);
        self.heap.pop();
        if let Some(x) = self.edges.get_mut(e) {
            x.pos = NOT_IN_HEAP;
        }
        // X-Ray's compiler compares against the slot `i` also when it was the last one, reading
        // past the heap's length; that slot then holds the removed edge itself (equal keys)
        if i < self.heap.len() {
            if self.key(self.at(i)) < self.key(e) {
                self.downheap(i);
            } else {
                self.upheap(i);
            }
        }
    }

    // ---------------------------------------------------------------- the edges

    fn new_edge(&mut self, i: u32, j: u32) -> usize {
        let id = self.edges.len();
        self.edges.push(Edge { v1: i, v2: j, vnew: [0.0; 3], key: 0.0, pos: NOT_IN_HEAP });
        for v in [i, j] {
            if let Some(l) = self.edge_links.get_mut(v as usize) {
                l.push(id);
            }
        }
        id
    }

    /// Collects the edges: every edge of the model once, in X-Ray's order. `create_edge` rates
    /// each edge as it is made; a rating reads the quadrics and the model only, so all are
    /// rated at once and then go into the heap in that order.
    pub fn collect_edges(&mut self) {
        let mut star = Vec::new();
        let first = self.edges.len();
        for i in 0..self.model.verts.len() as u32 {
            star.clear();
            self.model.collect_vertex_star(i, &mut star);
            for &s in &star {
                if i < s {
                    self.new_edge(i, s);
                }
            }
        }
        let this = &*self;
        let infos: Vec<Option<([f32; 3], f32)>> =
            (first..self.edges.len()).into_par_iter().map(|e| this.edge_info(e)).collect();
        for (e, info) in (first..).zip(infos) {
            self.set_edge_info(e, info);
        }
    }

    fn set_edge_info(&mut self, e: usize, info: Option<([f32; 3], f32)>) {
        if let (Some((vnew, key)), Some(x)) = (info, self.edges.get_mut(e)) {
            x.vnew = vnew;
            x.key = key;
        }
        let in_heap = self.edges.get(e).is_some_and(|x| x.pos != NOT_IN_HEAP);
        if in_heap {
            self.heap_update(e);
        } else {
            self.heap_insert(e);
        }
    }

    /// `compute_target_placement` and the penalties: the edge's new point and its key.
    fn edge_info(&self, e: usize) -> Option<([f32; 3], f32)> {
        let &Edge { v1: i, v2: j, .. } = self.edges.get(e)?;
        let mut q = self.quadrics.get(i as usize).copied().unwrap_or_default();
        q.add(&self.quadrics.get(j as usize).copied().unwrap_or_default());
        let vi = widen(self.model.vertex(i));
        let vj = widen(self.model.vertex(j));
        let (ei, ej) = (q.eval(vi), q.eval(vj));
        let (mut e_min, mut best) = if ei < ej { (ei, vi) } else { (ej, vj) };
        if self.placement >= Placement::EndOrMid {
            let mid = [(vi[0] + vj[0]) / 2.0, (vi[1] + vj[1]) / 2.0, (vi[2] + vj[2]) / 2.0];
            let e_mid = q.eval(mid);
            if e_mid < e_min {
                e_min = e_mid;
                best = mid;
            }
        }
        let vnew = [best[0] as f32, best[1] as f32, best[2] as f32];
        let mut key = (-e_min) as f32;
        if self.meshing_penalty > 1.0 {
            key = self.penalized_key(i, j, vnew, key);
        }
        Some((vnew, key))
    }

    fn after(&self, f: u32, v1: u32, vnew: [f32; 3]) -> [V3; 3] {
        self.model.face(f).map(|v| if v == v1 { widen(vnew) } else { widen(self.model.vertex(v)) })
    }

    fn check_local_compactness(&self, v1: u32, vnew: [f32; 3], marked: &impl Fn(u32) -> bool) -> f64 {
        let mut c_min = 1.0;
        for &f in self.model.neighbors(v1) {
            if marked(f) {
                let [a, b, c] = self.after(f, v1, vnew);
                let c = triangle_compactness(a, b, c);
                if c < c_min {
                    c_min = c;
                }
            }
        }
        c_min
    }

    fn check_local_inversion(&self, v1: u32, vnew: [f32; 3], marked: &impl Fn(u32) -> bool) -> f64 {
        let mut n_min = 1.0;
        for &f in self.model.neighbors(v1) {
            if marked(f) {
                let before = self.model.face_normal_d(f);
                let [a, b, c] = self.after(f, v1, vnew);
                let after = triangle_normal(a, b, c);
                let d = dot3(before, after);
                if d < n_min {
                    n_min = d;
                }
            }
        }
        n_min
    }

    fn check_local_validity(&self, v1: u32, vnew: [f32; 3], marked: &impl Fn(u32) -> bool) -> u32 {
        let mut failed = 0;
        for &f in self.model.neighbors(v1) {
            if !marked(f) {
                continue;
            }
            let face = self.model.face(f);
            let k = face.iter().position(|&v| v == v1).unwrap_or(2);
            let x = face[(k + 1) % 3];
            let y = face[(k + 2) % 3];
            let px = self.model.vertex(x);
            let d_yx = fsub(self.model.vertex(y), px);
            let d_vx = fsub(self.model.vertex(v1), px);
            let d_vnew = fsub(vnew, px);
            let f_n = fcross(d_yx, d_vx);
            let n = funitize(fcross(f_n, d_yx));
            if fdot(d_vnew, n) < self.local_validity_threshold * fdot(d_vx, n) {
                failed += 1;
            }
        }
        failed
    }

    /// Applies the mesh penalties: the key of an edge whose placement is `vnew`, `key`. X-Ray's compiler
    /// marks the faces around both ends (0 for those of v2, 1 for those of v1, then +1 for each
    /// time a face is v2's) and looks at those marked 1: a face of v1 that v2 lacks, and a face
    /// of v2 once over that v1 lacks - counted here without the marks, so edges can be rated at
    /// once.
    fn penalized_key(&self, v1: u32, v2: u32, vnew: [f32; 3], key: f32) -> f32 {
        let n1 = self.model.neighbors(v1);
        let n2 = self.model.neighbors(v2);
        let sorted = |n: &[u32]| {
            let mut s: SmallVec<[u32; 32]> = SmallVec::from_slice(n);
            s.sort_unstable();
            s
        };
        let (s1, s2) = (sorted(n1), sorted(n2));
        let count = |s: &[u32], f: u32| s.partition_point(|&x| x <= f) - s.partition_point(|&x| x < f);
        let of_v1 = |f: u32| count(&s2, f) == 0;
        let of_v2 = |f: u32| count(&s1, f) == 0 && count(&s2, f) == 1;
        let base_error = f64::from(key);
        let mut bias = 0.0f64;
        let max_degree = n1.len().max(n2.len());
        if max_degree > self.vertex_degree_limit {
            bias += (max_degree - self.vertex_degree_limit) as f64 * self.meshing_penalty * 0.001;
        }
        let failed =
            self.check_local_validity(v1, vnew, &of_v1) + self.check_local_validity(v2, vnew, &of_v2);
        if failed > 0 {
            bias += f64::from(failed) * self.meshing_penalty;
        }
        let m1 = self.check_local_inversion(v1, vnew, &of_v1);
        let m2 = self.check_local_inversion(v2, vnew, &of_v2);
        if m1.min(m2) < 0.0 {
            bias += self.meshing_penalty;
        }
        let mut scale = 1.0f32;
        if self.compactness_ratio > 0.0 {
            let c1 = self.check_local_compactness(v1, vnew, &of_v1);
            let c2 = self.check_local_compactness(v2, vnew, &of_v2);
            let c_min = c1.min(c2);
            if c_min < self.compactness_ratio {
                scale += ((self.compactness_ratio - c_min) / self.compactness_ratio) as f32;
            }
        }
        ((base_error - f64::from(EDGE_BASE_ERROR)) * f64::from(scale) - bias) as f32
    }

    fn update_pre_contract(&mut self, c: &Contraction) {
        let (v1, v2) = (c.v1, c.v2);
        let mut star: Vec<u32> = Vec::new();
        for &e in self.edge_links.get(v1 as usize).map_or(&[][..], Vec::as_slice) {
            if let Some(x) = self.edges.get(e) {
                star.push(if x.v1 == v1 { x.v2 } else { x.v1 });
            }
        }
        let links = self.edge_links.get(v2 as usize).cloned().unwrap_or_default();
        for e in links {
            let Some(&x) = self.edges.get(e) else { continue };
            let u = if x.v1 == v2 { x.v2 } else { x.v1 };
            if u == v1 || star.contains(&u) {
                if let Some(l) = self.edge_links.get_mut(u as usize)
                    && let Some(j) = l.iter().position(|&y| y == e)
                {
                    remove_at(l, j);
                }
                self.heap_remove(e);
            } else {
                if let Some(x) = self.edges.get_mut(e) {
                    x.v1 = v1;
                    x.v2 = u;
                }
                if let Some(l) = self.edge_links.get_mut(v1 as usize) {
                    l.push(e);
                }
            }
        }
        if let Some(l) = self.edge_links.get_mut(v2 as usize) {
            l.clear();
        }
    }

    fn apply_contraction(&mut self, c: &Contraction) {
        self.valid_verts -= 1;
        self.valid_faces -= c.dead_faces.len() as u32;
        let q2 = self.quadrics.get(c.v2 as usize).copied().unwrap_or_default();
        if let Some(q1) = self.quadrics.get_mut(c.v1 as usize) {
            q1.add(&q2);
        }
        self.update_pre_contract(c);
        self.model.apply_contraction(c);

        let mut star = Vec::new();
        self.model.collect_vertex_star(c.v1, &mut star);
        star.push(c.v1);
        let mut edges: Vec<usize> = Vec::new();
        for &s in &star {
            if let Some(l) = self.edge_links.get(s as usize) {
                edges.extend_from_slice(l);
            }
        }
        // X-Ray's compiler sorts the edges by their addresses: here by the order they were made
        edges.sort_unstable();
        edges.dedup();
        // rated at once (a rating reads the model and the quadrics only), put in the heap in order
        let this = &*self;
        let infos: Vec<Option<([f32; 3], f32)>> = if edges.len() >= 16 {
            edges.par_iter().map(|&e| this.edge_info(e)).collect()
        } else {
            edges.iter().map(|&e| this.edge_info(e)).collect()
        };
        for (e, info) in edges.into_iter().zip(infos) {
            self.set_edge_info(e, info);
        }
    }

    /// `decimate(target, max_error)`: contractions until no more than `target` faces are
    /// left or the cheapest one costs more than `max_error`.
    pub fn decimate(&mut self, target: u32, max_error: f32) -> bool {
        self.decimate_until(target, max_error, &|| false)
    }

    /// [`Self::decimate`] that gives up, answering `false`, once `stop` says so - asked every few
    /// thousand contractions.
    pub fn decimate_until(&mut self, target: u32, max_error: f32, stop: &(dyn Fn() -> bool + Sync)) -> bool {
        let max_error = max_error + EDGE_BASE_ERROR;
        let mut n = 0usize;
        while self.valid_faces > target {
            n += 1;
            if n.is_multiple_of(4096) && stop() {
                return false;
            }
            let Some(&top) = self.heap.first() else { return false };
            if -self.key(top) > max_error {
                return true;
            }
            let Some(e) = self.heap_extract() else { return false };
            let Some(&Edge { v1, v2, vnew, .. }) = self.edges.get(e) else { continue };
            if self.model.vertex_is_valid(v1) && self.model.vertex_is_valid(v2) {
                let c = self.model.compute_contraction(v1, v2, vnew);
                self.apply_contraction(&c);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat grid of `n` x `n` quads split into triangles.
    fn grid(n: u32) -> Model {
        let mut m = Model::with_capacity(((n + 1) * (n + 1)) as usize, (n * n * 2) as usize);
        for z in 0..=n {
            for x in 0..=n {
                m.add_vertex([x as f32, 0.0, z as f32]);
            }
        }
        let id = |x: u32, z: u32| z * (n + 1) + x;
        for z in 0..n {
            for x in 0..n {
                m.add_face([id(x, z), id(x, z + 1), id(x + 1, z + 1)]);
                m.add_face([id(x, z), id(x + 1, z + 1), id(x + 1, z)]);
            }
        }
        m
    }

    #[test]
    fn a_flat_grid_collapses_and_keeps_its_border() {
        let mut slim = EdgeQSlim::new(grid(6));
        slim.boundary_weight = 1_000_000.0;
        slim.compactness_ratio = 0.001;
        slim.meshing_penalty = 1_000_000.0;
        slim.weighting = Weighting::Uniform;
        slim.initialize();
        slim.collect_edges();
        assert!(slim.decimate(0, 0.0005));
        let before = 72;
        assert!(slim.valid_faces < before, "{} faces left", slim.valid_faces);
        let mut m = slim.model;
        m.compact_vertices();
        // every corner of the grid survives: the boundary quadrics hold them
        for corner in [[0.0, 0.0, 0.0], [6.0, 0.0, 0.0], [0.0, 0.0, 6.0], [6.0, 0.0, 6.0]] {
            assert!(m.verts.contains(&corner), "{corner:?} was lost");
        }
        // the surviving faces still cover the square
        let area: f64 =
            (0..m.face_count() as u32).filter(|&f| m.face_is_valid(f)).map(|f| m.face_area(f)).sum();
        assert!((area - 36.0).abs() < 1e-3, "{area}");
    }

    #[test]
    fn the_heap_gives_the_largest_key_first() {
        let mut slim = EdgeQSlim::new(Model::default());
        for k in [3.0f32, -1.0, 7.0, 0.5, 7.0, -4.0] {
            let id = slim.edges.len();
            slim.edges.push(Edge { v1: 0, v2: 0, vnew: [0.0; 3], key: k, pos: NOT_IN_HEAP });
            slim.heap_insert(id);
        }
        slim.heap_remove(3);
        let mut got = Vec::new();
        while let Some(e) = slim.heap_extract() {
            got.push(slim.key(e));
        }
        assert_eq!(got, vec![7.0, 7.0, 3.0, -1.0, -4.0]);
    }
}
