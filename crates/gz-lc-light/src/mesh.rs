//! The compiler's mesh: the vertex and face types of the compiler over the level's
//! and an MU model's vertex and face data, and
//! the vertex and face lists of the global data.
//!
//! Vertices and faces live in arenas and name each other by id; an id is never reused, as a
//! pointer of X-Ray's compiler is unique while its object lives. The lists keep its order: a new
//! element goes to the end, an unregistered one is swapped with the last.
//! A vertex's adjacency is the list of its faces in the order they were added.

use glam::{DVec3, Vec2, Vec3};
use gz_chunk::Writer;
use smallvec::SmallVec;

use crate::color::BaseColor;
use crate::math::{
    EPS, EPS_S, cross, deg2rad, distance, distance2, dot, exact_normalize, i_floor, magnitude, normalize_safe,
};

/// A vertex of a mesh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VertId(pub u32);

/// A face of a mesh.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FaceId(pub u32);

/// A deflector of the build.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeflId(pub u32);

/// A vertex slot of a face that was not set yet.
pub const NO_VERT: VertId = VertId(u32::MAX);
/// `sm_group` of a face without a smoothing group.
pub const NO_SM_GROUP: u32 = u32::MAX;

/// `edge2idx`: the corners of a face's three edges.
pub const EDGE2IDX: [[usize; 2]; 3] = [[0, 1], [1, 2], [2, 0]];
/// `edge2idx3`.
pub const EDGE2IDX3: [[usize; 3]; 3] = [[0, 1, 2], [1, 2, 0], [2, 0, 1]];
/// `idx2edge`: the edge between two corners, -1 for a corner and itself.
pub const IDX2EDGE: [[i32; 3]; 3] = [[-1, 0, 2], [0, -1, 1], [2, 1, -1]];

/// `base_basis`: a unit vector in three bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Basis {
    pub x: u8,
    pub y: u8,
    pub z: u8,
}

impl Default for Basis {
    /// `set(0, 0, 0)`, as `base_Face()` leaves it.
    fn default() -> Self {
        Self::of(Vec3::ZERO)
    }
}

impl Basis {
    /// `set(N)`.
    #[must_use]
    pub fn of(n: Vec3) -> Self {
        let n = normalize_safe(n);
        let n = Vec3::new((n.x + 1.0) * 127.5, (n.y + 1.0) * 127.5, (n.z + 1.0) * 127.5);
        let b = |v: f32| i_floor(v).clamp(0, 255) as u8;
        Self { x: b(n.x), y: b(n.y), z: b(n.z) }
    }

    /// `similar`: no component more than one step away.
    #[must_use]
    pub fn similar(&self, o: &Basis) -> bool {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).abs() <= 1;
        d(self.x, o.x) && d(self.y, o.y) && d(self.z, o.z)
    }
}

/// `_TCF`: one set of texture coordinates of a face.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tcf {
    pub uv: [Vec2; 3],
}

impl Tcf {
    /// `barycentric`: `(u, v, w)` of a point, `w` of the third corner.
    #[must_use]
    pub fn barycentric(&self, p: Vec2) -> Vec3 {
        barycentric(&self.uv, p)
    }

    /// `isInside(P, B)`: the barycentric coordinates when the point lies on the triangle.
    #[must_use]
    pub fn inside(&self, p: Vec2) -> Option<Vec3> {
        let b = self.barycentric(p);
        is_inside(b).then_some(b)
    }

    /// `similar`.
    #[must_use]
    pub fn similar(&self, o: &Tcf, eps: f32) -> bool {
        self.uv.iter().zip(&o.uv).all(|(a, b)| (a.x - b.x).abs() < eps && (a.y - b.y).abs() < eps)
    }
}

/// The barycentric coordinates of a point in a triangle.
#[must_use]
pub fn barycentric(uv: &[Vec2; 3], p: Vec2) -> Vec3 {
    let v02 = uv[0] - uv[2];
    let v12 = uv[1] - uv[2];
    let pv2 = p - uv[2];
    let m00 = v02.x * v02.x + v02.y * v02.y;
    let m01 = v02.x * v12.x + v02.y * v12.y;
    let m11 = v12.x * v12.x + v12.y * v12.y;
    let r0 = v02.x * pv2.x + v02.y * pv2.y;
    let r1 = v12.x * pv2.x + v12.y * pv2.y;
    let det = m00 * m11 - m01 * m01;
    let u = (m11 * r0 - m01 * r1) / det;
    let v = (m00 * r1 - m01 * r0) / det;
    Vec3::new(u, v, 1.0 - u - v)
}

/// `isInside(u, v, w)`.
#[must_use]
pub fn is_inside(b: Vec3) -> bool {
    (0.0..=1.0).contains(&b.x) && (0.0..=1.0).contains(&b.y) && (0.0..=1.0).contains(&b.z)
}

/// The flags of `base_Face`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FaceFlags {
    pub splitted: bool,
    pub processed: bool,
    /// For the ray tracer.
    pub opaque: bool,
    /// For the tesselation.
    pub locked: bool,
}

/// `Tvertex<DataVertex>` / `Tvertex<data_vertex>`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Vertex {
    pub p: Vec3,
    pub n: Vec3,
    /// All its light.
    pub c: BaseColor,
    /// Used by the mesh processing.
    pub handle: i32,
    pub adjacents: Vec<FaceId>,
}

/// `Tface<DataVertex>` / `Tface<data_vertex>`. An MU model's face has one set of texture
/// coordinates and never a deflector or a lightmap.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub v: [VertId; 3],
    pub basis_tangent: [Basis; 3],
    pub basis_binormal: [Basis; 3],
    /// The material's index.
    pub material: u16,
    /// The game material's id (kept up to the game collision form).
    pub material_game: u16,
    pub flags: FaceFlags,
    pub n: Vec3,
    pub tc: SmallVec<[Tcf; 2]>,
    /// The deflector the face is mapped by.
    pub deflector: Option<DeflId>,
    /// `lmap_layer`: the lightmap page that holds it.
    pub lmap: Option<u32>,
    pub sm_group: u32,
}

impl Default for Face {
    /// `Tface()`.
    fn default() -> Self {
        Self {
            v: [NO_VERT; 3],
            basis_tangent: [Basis::default(); 3],
            basis_binormal: [Basis::default(); 3],
            material: 0,
            material_game: 0,
            flags: FaceFlags::default(),
            n: Vec3::ZERO,
            tc: SmallVec::new(),
            deflector: None,
            lmap: None,
            sm_group: NO_SM_GROUP,
        }
    }
}

impl Face {
    /// The texture coordinates of channel 0.
    #[must_use]
    pub fn tc0(&self) -> [Vec2; 3] {
        self.tc.first().map_or([Vec2::ZERO; 3], |t| t.uv)
    }

    /// Adds a channel.
    pub fn add_channel(&mut self, p1: Vec2, p2: Vec2, p3: Vec2) {
        self.tc.push(Tcf { uv: [p1, p2, p3] });
    }

    /// Whether two vertices render alike.
    #[must_use]
    pub fn render_equal_to(&self, o: &Face) -> bool {
        self.material == o.material
    }

    /// The vertex index.
    #[must_use]
    pub fn v_index(&self, v: VertId) -> Option<usize> {
        self.v.iter().position(|&x| x == v)
    }

    /// Whether the face is degenerate.
    #[must_use]
    pub fn is_degenerated(&self) -> bool {
        self.v[0] == self.v[1] || self.v[0] == self.v[2] || self.v[1] == self.v[2]
    }

    /// The vertices of an edge.
    #[must_use]
    pub fn edge_verts(&self, e: usize) -> (VertId, VertId) {
        let [a, b] = EDGE2IDX[e % 3];
        (self.v[a], self.v[b])
    }

    /// The same three vertices in any order.
    #[must_use]
    pub fn is_equal(&self, f: &Face) -> bool {
        let (a, b) = (&self.v, &f.v);
        (a[0] == b[0] && a[1] == b[1] && a[2] == b[2])
            || (a[0] == b[0] && a[2] == b[1] && a[1] == b[2])
            || (a[2] == b[0] && a[0] == b[1] && a[1] == b[2])
            || (a[2] == b[0] && a[1] == b[1] && a[0] == b[2])
            || (a[1] == b[0] && a[0] == b[1] && a[2] == b[2])
            || (a[1] == b[0] && a[2] == b[1] && a[0] == b[2])
    }
}

/// The error records of a build (`err_invalid`, `err_multiedge`, `err_tjunction`) and the count
/// of invalid faces.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshErrors {
    /// Three points per invalid face.
    pub invalid: Vec<Vec3>,
    /// Two points per edge shared by more than two faces.
    pub multiedge: Vec<Vec3>,
    /// Points of T-junctions and of deflectors whose normal could not be found.
    pub tjunction: Vec<Vec3>,
    pub invalid_faces: u32,
}

impl MeshErrors {
    /// The error file (`build_<user>.err`).
    #[must_use]
    pub fn write_err(&self) -> Vec<u8> {
        let mut w = Writer::new();
        for (id, list, per) in [(0, &self.tjunction, 1), (1, &self.multiedge, 2), (2, &self.invalid, 3)] {
            w.chunk(id, |w| {
                w.w_u32((list.len() / per) as u32);
                for p in list {
                    w.w_vec3(*p);
                }
            });
        }
        w.into_bytes()
    }
}

/// How `calculate_normals` decides that two faces of a vertex share its normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Smoothing {
    /// Their normals are closer than the build's smoothing angle (`g_using_smooth_groups` off).
    Angle,
    /// They are in one smoothing group (`g_smooth_groups_by_faces`).
    Groups,
    /// The shared edge is soft in both (`sm_group` holds edge flags): the compiler's default.
    EdgeFlags,
}

/// `face_smoth_flags.h`: the back-face bit of a face's edge flags.
pub const BACKFACE_FLAG: u32 = 1 << 3;

fn convert_edge_index(flags: u32, e: u16) -> u16 {
    if flags & BACKFACE_FLAG != 0 { (4 - e) % 3 } else { e }
}

/// `is_soft_edge`.
#[must_use]
pub fn is_soft_edge(flags: u32, e: u16) -> bool {
    let e = convert_edge_index(flags, e);
    flags & (1 << e) == 0
}

/// `do_connect_faces_by_faces_edge_flags`.
#[must_use]
pub fn connect_by_edge_flags(start: u32, test: u32, start_edge: u16, test_edge: u16) -> bool {
    if (start & BACKFACE_FLAG != 0) != (test & BACKFACE_FLAG != 0) {
        return false;
    }
    is_soft_edge(start, start_edge) && is_soft_edge(test, test_edge)
}

/// A pool of X-Ray's compiler as far as the order of its addresses goes: a freed object is the
/// first given out again (the free list is a stack), else the next slot after the last; blocks
/// come at rising addresses - but a block made after [`Self::mark_loaded`] lies below every block
/// made before it. Where X-Ray's compiler sorts objects by their pointers the compiler sorts by
/// [`Self::order`], and so meets them in X-Ray's order.
///
/// the blocks' places are the heap's. X-Ray's compiler reads `build.prj` into one buffer, loads
/// the level's vertices and faces into blocks above it, and frees it when the load returns
/// the blocks the compile makes later (the tesselation's faces) go into
/// that space. Seen on l03u_agr_underground against X-Ray's compiler's output: with the
/// tesselation's block in rising order its split order differs from 512137 faces on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PoolSim {
    free: Vec<u64>,
    next: u64,
    /// The blocks the load made.
    loaded: Option<u64>,
}

/// The pool's granularity: the objects of one block.
const POOL_BLOCK: u64 = 8 * 1024;

impl PoolSim {
    /// The end of the load: blocks made from here on lie below the load's.
    pub fn mark_loaded(&mut self) {
        self.loaded = Some(self.next.div_ceil(POOL_BLOCK));
    }

    /// The sort key of a slot: its address's order among the pool's.
    #[must_use]
    pub fn order(&self, a: u64) -> u64 {
        match self.loaded {
            Some(blocks) if a / POOL_BLOCK >= blocks => a - blocks * POOL_BLOCK,
            Some(_) => a | (1 << 48),
            None => a,
        }
    }

    /// `create`.
    pub fn create(&mut self) -> u64 {
        self.free.pop().unwrap_or_else(|| {
            let a = self.next;
            self.next += 1;
            a
        })
    }

    /// `destroy`.
    pub fn destroy(&mut self, a: u64) {
        self.free.push(a);
    }
}

/// A mesh: the level's (`g_vertices`, `g_faces`) or an MU model's (`m_vertices`, `m_faces`).
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    verts: Vec<Option<Vertex>>,
    faces: Vec<Option<Face>>,
    vert_list: Vec<VertId>,
    face_list: Vec<FaceId>,
    /// The address of every vertex and face as X-Ray's pools give them out.
    vaddr: Vec<u64>,
    faddr: Vec<u64>,
    /// The vertex pool (the MU models share one: the caller moves it from model to model).
    pub vpool: PoolSim,
    pub fpool: PoolSim,
    /// Whether an invalid face is recorded (`Face::Failure`); an MU model's face records nothing.
    pub log_failures: bool,
    pub errors: MeshErrors,
}

impl Mesh {
    /// The level's mesh.
    #[must_use]
    pub fn new() -> Self {
        Self { log_failures: true, ..Self::default() }
    }

    /// An MU model's mesh.
    #[must_use]
    pub fn new_model() -> Self {
        Self::default()
    }

    /// The registered vertices in list order.
    #[must_use]
    pub fn vertices(&self) -> &[VertId] {
        &self.vert_list
    }

    /// The registered faces in list order.
    #[must_use]
    pub fn faces(&self) -> &[FaceId] {
        &self.face_list
    }

    /// The vertex list, for a phase that reorders it.
    pub fn vertices_mut(&mut self) -> &mut Vec<VertId> {
        &mut self.vert_list
    }

    /// The face list, for a phase that reorders it.
    pub fn faces_mut(&mut self) -> &mut Vec<FaceId> {
        &mut self.face_list
    }

    #[must_use]
    pub fn vertex(&self, v: VertId) -> Option<&Vertex> {
        self.verts.get(v.0 as usize)?.as_ref()
    }

    pub fn vertex_mut(&mut self, v: VertId) -> Option<&mut Vertex> {
        self.verts.get_mut(v.0 as usize)?.as_mut()
    }

    #[must_use]
    pub fn face(&self, f: FaceId) -> Option<&Face> {
        self.faces.get(f.0 as usize)?.as_ref()
    }

    pub fn face_mut(&mut self, f: FaceId) -> Option<&mut Face> {
        self.faces.get_mut(f.0 as usize)?.as_mut()
    }

    /// A vertex that is not on the list (`b_vert_not_register`).
    pub fn create_vertex_unlisted(&mut self, p: Vec3) -> VertId {
        let id = VertId(self.verts.len() as u32);
        self.verts.push(Some(Vertex { p, adjacents: Vec::with_capacity(4), ..Vertex::default() }));
        let a = self.vpool.create();
        self.vaddr.push(a);
        id
    }

    /// Every address moved up by a base: an MU model loaded after others in the pools they share.
    pub fn rebase_pools(&mut self, vbase: u64, fbase: u64) {
        for a in &mut self.vaddr {
            *a += vbase;
        }
        for a in &mut self.faddr {
            *a += fbase;
        }
        self.vpool.next += vbase;
        self.fpool.next += fbase;
        for a in &mut self.vpool.free {
            *a += vbase;
        }
        for a in &mut self.fpool.free {
            *a += fbase;
        }
    }

    /// The next fresh slots of the two pools.
    #[must_use]
    pub fn pool_ends(&self) -> (u64, u64) {
        (self.vpool.next, self.fpool.next)
    }

    /// Where X-Ray's pool put a vertex (see [`PoolSim`]).
    #[must_use]
    pub fn vertex_addr(&self, v: VertId) -> u64 {
        self.vaddr.get(v.0 as usize).map_or(u64::MAX, |&a| self.vpool.order(a))
    }

    /// Where X-Ray's pool put a face.
    #[must_use]
    pub fn face_addr(&self, f: FaceId) -> u64 {
        self.faddr.get(f.0 as usize).map_or(u64::MAX, |&a| self.fpool.order(a))
    }

    /// The end of the load: the face pool's later blocks lie below the load's (see
    /// [`PoolSim`]). The vertex pool's do not: the tesselation's faces take the space first.
    pub fn mark_loaded(&mut self) {
        self.fpool.mark_loaded();
    }

    /// `vertices_isolate_and_pool_reload`'s pool: every listed vertex made again, in list order,
    /// in a fresh pool.
    pub fn reload_vertex_pool(&mut self) {
        self.vpool = PoolSim::default();
        for i in 0..self.vert_list.len() {
            let v = self.vert_list[i];
            let a = self.vpool.create();
            if let Some(slot) = self.vaddr.get_mut(v.0 as usize) {
                *slot = a;
            }
        }
    }

    /// `create_vertex`: a vertex at `p` with no normal, on the list.
    pub fn create_vertex(&mut self, p: Vec3) -> VertId {
        let id = self.create_vertex_unlisted(p);
        self.vert_list.push(id);
        id
    }

    /// A face that is not on the list (`Face::read_create`).
    pub fn create_face_unlisted(&mut self) -> FaceId {
        let id = FaceId(self.faces.len() as u32);
        self.faces.push(Some(Face::default()));
        let a = self.fpool.create();
        self.faddr.push(a);
        id
    }

    /// `create_face`: a face on the list whose vertices are set later.
    pub fn create_face_raw(&mut self) -> FaceId {
        let id = self.create_face_unlisted();
        self.face_list.push(id);
        id
    }

    /// A face on the list with its three vertices.
    pub fn create_face(&mut self, v: [VertId; 3]) -> FaceId {
        let f = self.create_face_raw();
        for (i, v) in v.into_iter().enumerate() {
            self.set_vertex(f, i, v);
        }
        f
    }

    /// `prep_add`: the face joins the vertex's adjacency once.
    pub fn prep_add(&mut self, v: VertId, f: FaceId) {
        if let Some(vx) = self.vertex_mut(v)
            && !vx.adjacents.contains(&f)
        {
            vx.adjacents.push(f);
        }
    }

    /// `prep_remove`.
    pub fn prep_remove(&mut self, v: VertId, f: FaceId) {
        if let Some(vx) = self.vertex_mut(v)
            && let Some(at) = vx.adjacents.iter().position(|&x| x == f)
        {
            vx.adjacents.remove(at);
        }
    }

    /// `raw_set_vertex`: the slot alone, the adjacency untouched.
    pub fn raw_set_vertex(&mut self, f: FaceId, idx: usize, v: VertId) {
        if let Some(face) = self.face_mut(f)
            && let Some(slot) = face.v.get_mut(idx)
        {
            *slot = v;
        }
    }

    /// Sets a vertex.
    pub fn set_vertex(&mut self, f: FaceId, idx: usize, v: VertId) {
        self.raw_set_vertex(f, idx, v);
        self.prep_add(v, f);
    }

    /// Every slot holding `what` takes `to`, the adjacency follows.
    pub fn v_replace(&mut self, f: FaceId, what: VertId, to: VertId) {
        self.replace(f, what, to, true);
    }

    /// `what` keeps the face in its adjacency.
    pub fn v_replace_not_remove(&mut self, f: FaceId, what: VertId, to: VertId) {
        self.replace(f, what, to, false);
    }

    fn replace(&mut self, f: FaceId, what: VertId, to: VertId, remove: bool) {
        for i in 0..3 {
            if self.face(f).is_some_and(|x| x.v[i] == what) {
                self.raw_set_vertex(f, i, to);
                if remove {
                    self.prep_remove(what, f);
                }
                self.prep_add(to, f);
            }
        }
    }

    /// Destroys a face: off the list when `unregister` (swapped with the last), out of
    /// its vertices' adjacency, gone.
    pub fn destroy_face(&mut self, f: FaceId, unregister: bool) {
        if unregister && let Some(at) = self.face_list.iter().position(|&x| x == f) {
            self.face_list.swap_remove(at);
        }
        let Some(face) = self.faces.get_mut(f.0 as usize).and_then(Option::take) else { return };
        if let Some(&a) = self.faddr.get(f.0 as usize) {
            self.fpool.destroy(a);
        }
        for v in face.v {
            self.prep_remove(v, f);
        }
    }

    /// Destroys a vertex.
    pub fn destroy_vertex(&mut self, v: VertId, unregister: bool) {
        if unregister && let Some(at) = self.vert_list.iter().position(|&x| x == v) {
            self.vert_list.swap_remove(at);
        }
        if let Some(slot) = self.verts.get_mut(v.0 as usize)
            && slot.take().is_some()
            && let Some(&a) = self.vaddr.get(v.0 as usize)
        {
            self.vpool.destroy(a);
        }
    }

    /// The corners of a face.
    #[must_use]
    pub fn positions(&self, f: FaceId) -> Option<[Vec3; 3]> {
        let face = self.face(f)?;
        Some([self.vertex(face.v[0])?.p, self.vertex(face.v[1])?.p, self.vertex(face.v[2])?.p])
    }

    /// The normals of a face's corners.
    #[must_use]
    pub fn normals(&self, f: FaceId) -> Option<[Vec3; 3]> {
        let face = self.face(f)?;
        Some([self.vertex(face.v[0])?.n, self.vertex(face.v[1])?.n, self.vertex(face.v[2])?.n])
    }

    /// The length of an edge.
    #[must_use]
    pub fn edge_len(&self, f: FaceId, e: usize) -> f32 {
        let Some(p) = self.positions(f) else { return 0.0 };
        let [a, b] = EDGE2IDX[e % 3];
        distance(p[a], p[b])
    }

    /// The area: Heron's formula.
    #[must_use]
    pub fn calc_area(&self, f: FaceId) -> f32 {
        let Some(p) = self.positions(f) else { return 0.0 };
        area_of(&p)
    }

    /// The longest edge.
    #[must_use]
    pub fn calc_max_edge(&self, f: FaceId) -> f32 {
        let Some(p) = self.positions(f) else { return 0.0 };
        let (e1, e2, e3) = (distance(p[0], p[1]), distance(p[0], p[2]), distance(p[1], p[2]));
        if e1 > e2 && e1 > e3 {
            e1
        } else if e2 > e1 && e2 > e3 {
            e2
        } else {
            e3
        }
    }

    /// The centre.
    #[must_use]
    pub fn calc_center(&self, f: FaceId) -> Vec3 {
        let Some(p) = self.positions(f) else { return Vec3::ZERO };
        let s = p[0] + p[1] + p[2];
        Vec3::new(s.x / 3.0, s.y / 3.0, s.z / 3.0)
    }

    /// The face's normal from its corners, with the double precision fallback.
    pub fn calc_normal(&mut self, f: FaceId) {
        let Some(p) = self.positions(f) else { return };
        let n = cross(p[1] - p[0], p[2] - p[1]);
        let mag = magnitude(n);
        if mag < EPS_S {
            let (saved, ok) = exact_normalize(n);
            if ok {
                if let Some(face) = self.face_mut(f) {
                    face.n = saved;
                }
            } else {
                if let Some(face) = self.face_mut(f) {
                    face.n = n;
                }
                self.calc_normal2(f);
            }
        } else if let Some(face) = self.face_mut(f) {
            face.n = crate::math::normalize(Vec3::new(n.x / mag, n.y / mag, n.z / mag));
        }
    }

    /// In double; a face with no area gets the axis of its largest component and
    /// is recorded as invalid.
    pub fn calc_normal2(&mut self, f: FaceId) {
        let Some(p) = self.positions(f) else { return };
        let [v0, v1, v2] = p.map(DVec3::from);
        let (t1, t2) = (v1 - v0, v2 - v1);
        let dn = DVec3::new(t1.y * t2.z - t1.z * t2.y, t1.z * t2.x - t1.x * t2.z, t1.x * t2.y - t1.y * t2.x);
        let mag = (dn.x * dn.x + dn.y * dn.y + dn.z * dn.z).sqrt();
        if mag < f64::MIN_POSITIVE {
            self.failure(f);
            let a = dn.abs();
            let Some(face) = self.face_mut(f) else { return };
            let sign = |v: f32| if v >= 0.0 { 1.0 } else { -1.0 };
            let n = face.n;
            face.n = if a.x > a.y && a.x > a.z {
                Vec3::new(sign(n.x), 0.0, 0.0)
            } else if a.y > a.x && a.y > a.z {
                Vec3::new(0.0, sign(n.y), 0.0)
            } else if a.z > a.x && a.z > a.y {
                Vec3::new(0.0, 0.0, sign(n.z))
            } else {
                Vec3::Y
            };
        } else if let Some(face) = self.face_mut(f) {
            face.n = Vec3::new((dn.x / mag) as f32, (dn.y / mag) as f32, (dn.z / mag) as f32);
        }
    }

    /// `Failure`: one more invalid face, its corners recorded.
    pub fn failure(&mut self, f: FaceId) {
        if !self.log_failures {
            return;
        }
        self.errors.invalid_faces += 1;
        if let Some(p) = self.positions(f) {
            self.errors.invalid.extend_from_slice(&p);
        }
    }

    /// An area, texture coordinates that span half a texel of a 4096 texture
    /// and a normal; `false` (and a recorded failure) for a face without them.
    pub fn verify_face(&mut self, f: FaceId) -> bool {
        let a = self.calc_area(f);
        if !a.is_finite() || a < EPS {
            self.failure(f);
            return false;
        }
        let Some(tc) = self.face(f).map(Face::tc0) else { return false };
        let eps = 0.5 / 4096.0;
        let p = distance2(tc[0], tc[1]) + distance2(tc[1], tc[2]) + distance2(tc[2], tc[0]);
        // X-Ray's compiler tests the area's validity a second time where the perimeter's is meant
        if !a.is_finite() || p < eps {
            self.failure(f);
            return false;
        }
        self.calc_normal(f);
        if !self.face(f).is_some_and(|x| x.n.is_finite()) {
            self.failure(f);
            return false;
        }
        true
    }

    /// A listed vertex with the position, normal and light of `v`.
    pub fn create_copy_noadj(&mut self, v: VertId) -> VertId {
        let (p, n, c) =
            self.vertex(v).map_or((Vec3::ZERO, Vec3::ZERO, BaseColor::default()), |x| (x.p, x.n, x.c));
        let id = self.create_vertex(p);
        if let Some(nv) = self.vertex_mut(id) {
            nv.n = n;
            nv.c = c;
        }
        id
    }

    /// The same with the adjacency.
    pub fn create_copy(&mut self, v: VertId) -> VertId {
        let adj = self.vertex(v).map(|x| x.adjacents.clone()).unwrap_or_default();
        let id = self.create_copy_noadj(v);
        if let Some(nv) = self.vertex_mut(id) {
            nv.adjacents = adj;
        }
        id
    }

    /// The sum of the adjacent faces' normals, `exact_normalize`d.
    pub fn normal_from_adj(&mut self, v: VertId) {
        let Some(vx) = self.vertex(v) else { return };
        let mut n = Vec3::ZERO;
        for f in &vx.adjacents {
            if let Some(face) = self.face(*f) {
                n = Vec3::new(n.x + face.n.x, n.y + face.n.y, n.z + face.n.z);
            }
        }
        let (n, _) = exact_normalize(n);
        if let Some(vx) = self.vertex_mut(v) {
            vx.n = n;
        }
    }

    /// `isolate_vertices`: the vertices no face uses leave, the rest keep their order. Answers
    /// how many left. `vertices_isolate_and_pool_reload` is the same in memory.
    pub fn isolate_vertices(&mut self) -> usize {
        let before = self.vert_list.len();
        let verts = &mut self.verts;
        let mut gone = Vec::new();
        self.vert_list.retain(|v| {
            let keep =
                verts.get(v.0 as usize).and_then(Option::as_ref).is_some_and(|x| !x.adjacents.is_empty());
            if !keep
                && let Some(slot) = verts.get_mut(v.0 as usize)
                && slot.take().is_some()
            {
                gone.push(*v);
            }
            keep
        });
        for v in gone {
            if let Some(&a) = self.vaddr.get(v.0 as usize) {
                self.vpool.destroy(a);
            }
        }
        before - self.vert_list.len()
    }

    fn has_same_edge(&self, f1: FaceId, f2: FaceId) -> Option<(u16, u16)> {
        let (a, b) = (self.face(f1)?, self.face(f2)?);
        for e in 0..3 {
            let (x, y) = a.edge_verts(e);
            let (x, y) = if x > y { (y, x) } else { (x, y) };
            for r in 0..3 {
                let (u, w) = b.edge_verts(r);
                let (u, w) = if u > w { (w, u) } else { (u, w) };
                if x == u && y == w {
                    return Some((e as u16, r as u16));
                }
            }
        }
        None
    }

    fn connect(
        &self,
        start: FaceId,
        test: FaceId,
        se: u16,
        te: u16,
        sm_cos: f32,
        smoothing: Smoothing,
    ) -> bool {
        let (Some(a), Some(b)) = (self.face(start), self.face(test)) else { return false };
        match smoothing {
            Smoothing::Angle => dot(a.n, b.n) > sm_cos,
            Smoothing::Groups => a.sm_group != NO_SM_GROUP && a.sm_group == b.sm_group,
            Smoothing::EdgeFlags => connect_by_edge_flags(a.sm_group, b.sm_group, se, te),
        }
    }

    /// The faces around the first face of `adj`'s vertex that share its normal with it, in the order the recursion meets them.
    fn smoothing_group(&mut self, adj: &[FaceId], sm_cos: f32, smoothing: Smoothing) -> Vec<FaceId> {
        let mut out = Vec::new();
        // (start face, the next test face) - the recursion without the call stack
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        while let Some(top) = stack.last_mut() {
            let (start, test) = *top;
            if test >= adj.len() {
                stack.pop();
                continue;
            }
            top.1 += 1;
            let (Some(&sf), Some(&tf)) = (adj.get(start), adj.get(test)) else { continue };
            if self.face(tf).is_none_or(|f| f.flags.splitted) {
                continue;
            }
            let Some((se, te)) = self.has_same_edge(sf, tf) else { continue };
            if sf == tf || self.connect(sf, tf, se, te, sm_cos, smoothing) {
                out.push(tf);
                if let Some(f) = self.face_mut(tf) {
                    f.flags.splitted = true;
                }
                stack.push((test, 0));
            }
        }
        out
    }

    /// `calculate_normals::calc_normals`: every face's normal, then every vertex split into one
    /// copy per group of its faces that share a normal, the originals dropped. Answers how many
    /// vertices were added.
    pub fn calc_normals(&mut self, sm_angle: f32, smoothing: Smoothing) -> isize {
        let vcount = self.vert_list.len();
        let sm_cos = deg2rad(sm_angle).cos();
        let faces = self.face_list.clone();
        for &f in &faces {
            if let Some(face) = self.face_mut(f) {
                face.flags.splitted = true;
            }
            self.calc_normal(f);
        }
        for i in 0..vcount {
            let Some(&v) = self.vert_list.get(i) else { break };
            // X-Ray's compiler sorts a vertex's faces by their addresses
            let (faddr, fpool) = (&self.faddr, &self.fpool);
            let Some(vx) = self.verts.get_mut(v.0 as usize).and_then(Option::as_mut) else { continue };
            vx.adjacents
                .sort_unstable_by_key(|f| faddr.get(f.0 as usize).map_or(u64::MAX, |&a| fpool.order(a)));
            let adj = vx.adjacents.clone();
            for f in &adj {
                if let Some(face) = self.face_mut(*f) {
                    face.flags.splitted = false;
                }
            }
            loop {
                let adj = self.vertex(v).map(|x| x.adjacents.clone()).unwrap_or_default();
                if adj.is_empty() {
                    break;
                }
                let group = self.smoothing_group(&adj, sm_cos, smoothing);
                if group.is_empty() {
                    // cannot happen: the first face always starts a group; stops a loop if it did
                    break;
                }
                let nv = self.create_copy_noadj(v);
                for f in group {
                    self.v_replace(f, v, nv);
                }
                self.normal_from_adj(nv);
            }
        }
        self.isolate_vertices();
        let verts = self.vert_list.clone();
        for v in verts {
            self.normal_from_adj(v);
        }
        for &f in &faces {
            if let Some(face) = self.face_mut(f) {
                face.flags.splitted = false;
            }
        }
        self.vert_list.len() as isize - vcount as isize
    }
}

/// Heron's formula over three corners, as the area takes the edges.
#[must_use]
pub fn area_of(p: &[Vec3; 3]) -> f32 {
    let (e1, e2, e3) = (distance(p[0], p[1]), distance(p[0], p[2]), distance(p[1], p[2]));
    let s = (e1 + e2 + e3) / 2.0;
    (s * (s - e1) * (s - e2) * (s - e3)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit square of two faces in the plane y = 0, both facing up.
    fn square() -> (Mesh, [FaceId; 2]) {
        let mut m = Mesh::new();
        let v: Vec<VertId> = [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)]
            .iter()
            .map(|&(x, z)| m.create_vertex(Vec3::new(x, 0.0, z)))
            .collect();
        let a = m.create_face([v[0], v[1], v[2]]);
        let b = m.create_face([v[0], v[2], v[3]]);
        for f in [a, b] {
            let face = m.face_mut(f).unwrap();
            face.add_channel(Vec2::ZERO, Vec2::Y, Vec2::ONE);
            face.sm_group = 0;
        }
        (m, [a, b])
    }

    #[test]
    fn faces_of_a_square_face_up_and_share_their_corner_normals() {
        let (mut m, [a, b]) = square();
        assert!(m.verify_face(a) && m.verify_face(b));
        assert_eq!(m.face(a).unwrap().n, Vec3::Y);
        assert!((m.calc_area(a) - 0.5).abs() < 1e-6);
        let added = m.calc_normals(75.0, Smoothing::EdgeFlags);
        // every corner is copied once and the originals go: the count stays
        assert_eq!(added, 0);
        assert_eq!(m.vertices().len(), 4);
        for v in m.vertices() {
            assert_eq!(m.vertex(*v).unwrap().n, Vec3::Y);
        }
    }

    #[test]
    fn a_hard_edge_splits_the_shared_corners() {
        let (mut m, [a, _]) = square();
        // edge 2 of face a (v2 - v0) is hard
        m.face_mut(a).unwrap().sm_group = 1 << 2;
        m.calc_normals(75.0, Smoothing::EdgeFlags);
        // the two corners on the hard edge are split in two
        assert_eq!(m.vertices().len(), 6);
    }

    #[test]
    fn destroying_a_face_takes_it_off_the_list_and_the_corners() {
        let (mut m, [a, b]) = square();
        m.destroy_face(a, true);
        assert_eq!(m.faces(), &[b]);
        assert!(m.face(a).is_none());
        let v1 = VertId(1);
        assert!(m.vertex(v1).unwrap().adjacents.is_empty());
        assert_eq!(m.isolate_vertices(), 1);
        assert_eq!(m.vertices().len(), 3);
    }

    #[test]
    fn a_face_without_area_is_recorded() {
        let mut m = Mesh::new();
        let v = m.create_vertex(Vec3::ZERO);
        let w = m.create_vertex(Vec3::X);
        let f = m.create_face([v, w, v]);
        m.face_mut(f).unwrap().add_channel(Vec2::ZERO, Vec2::X, Vec2::Y);
        assert!(!m.verify_face(f));
        assert_eq!(m.errors.invalid_faces, 1);
        assert_eq!(m.errors.invalid.len(), 3);
        let err = m.errors.write_err();
        // three chunks with their counts: 0 t-junction points, 0 edges, 1 face
        assert_eq!(err.len(), 3 * 8 + 3 * 4 + 3 * 12);
    }
}
