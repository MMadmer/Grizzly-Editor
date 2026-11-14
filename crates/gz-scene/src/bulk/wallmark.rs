//! The wallmarks: the static marks of a level - each a piece of
//! the geometry under it, clipped to a rectangle laid on the surface, with its shader and
//! texture - and the layer's chunk (an X-Ray format), read and written.
//!
//! A mark is made from the triangle a ray hits and the triangles round it that face the same way
//! ([`build_mark`]: a collector, an adjacency walk and a 3D ortho-frustum), and goes into the
//! slot of its shader and texture, replacing a mark there
//! whose centre is within 2 cm ([`Wallmarks::insert`]).

use std::sync::Arc;

use glam::Vec3;
use gz_base::SmolStr;
use gz_chunk::{Reader, Writer};
use gz_fmt_level::Collector;

use super::xr::{self, EPS, EPS_L, Frustum, M43, fsimilar};
use crate::SceneError;
use crate::class::ObjClass;
use crate::library::Library;
use crate::math::Aabb;
use crate::model::{ObjData, ObjId, SceneState};

/// `WM_VERSION`.
pub const WM_VERSION: u16 = 4;
/// The most vertices a wallmark may have.
pub const MAX_VERTICES: usize = 8192;
/// What a new tool's marks are drawn with.
pub const DEFAULT_SHADER: &str = "effects\\wallmarkmult";

/// The tool's flags.
pub mod flags {
    /// The flag that draws the wallmarks.
    pub const DRAW: u32 = 1 << 0;
    /// A mark is laid by the world's axes, not the camera's.
    pub const AXIS_ALIGN: u32 = 1 << 1;
}

/// The selected flag of a mark.
pub const SELECTED: u8 = 1 << 0;

const WM_CHUNK_VERSION: u32 = 1;
const WM_CHUNK_FLAGS: u32 = 2;
const WM_CHUNK_PARAMS: u32 = 3;
const WM_CHUNK_ITEMS: u32 = 4;
const WM_CHUNK_ITEMS2: u32 = 5;
const LIT: usize = 24;

/// Position, colour, texture coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LitVert {
    pub p: Vec3,
    pub color: u32,
    pub uv: [f32; 2],
}

/// One mark.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub flags: u8,
    pub bbox: Aabb,
    pub center: Vec3,
    pub radius: f32,
    /// The width, height and rotation it was made with.
    pub w: f32,
    pub h: f32,
    pub r: f32,
    /// Triangles.
    pub verts: Arc<[LitVert]>,
}

impl Mark {
    #[must_use]
    pub fn selected(&self) -> bool {
        self.flags & SELECTED != 0
    }
}

/// `wm_slot`: the marks of one shader and texture.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub shader: SmolStr,
    pub texture: SmolStr,
    pub items: Vec<Mark>,
}

/// The tool: its settings and its marks.
#[derive(Clone, Debug, PartialEq)]
pub struct Wallmarks {
    pub flags: u32,
    pub width: f32,
    pub height: f32,
    pub rotate: f32,
    pub shader: SmolStr,
    pub texture: SmolStr,
    pub slots: Vec<Slot>,
}

impl Default for Wallmarks {
    /// The constructor's settings.
    fn default() -> Self {
        Self {
            flags: flags::DRAW,
            width: 1.0,
            height: 1.0,
            rotate: 0.0,
            shader: SmolStr::new_static(DEFAULT_SHADER),
            texture: SmolStr::default(),
            slots: Vec::new(),
        }
    }
}

fn read_vec3(r: &mut Reader<'_>) -> Result<Vec3, SceneError> {
    Ok(r.r_vec3()?)
}

impl Wallmarks {
    /// Version 3 (no mark sizes, the multiply shader) or 4; a slot without marks is
    /// not kept, a mark of more than [`MAX_VERTICES`] vertices is dropped.
    pub fn read(bytes: &[u8]) -> Result<Self, SceneError> {
        let unsupported = || SceneError::load("Static Wallmark: Unsupported version.");
        let r = Reader::new(bytes);
        let version = r.open_chunk(WM_CHUNK_VERSION)?.ok_or_else(unsupported)?.reader().r_u16()?;
        if version != 3 && version != WM_VERSION {
            return Err(unsupported());
        }
        let mut out = Self { flags: r.need_chunk(WM_CHUNK_FLAGS)?.reader().r_u32()?, ..Self::default() };
        let params = r.need_chunk(WM_CHUNK_PARAMS)?;
        let mut p = params.reader();
        out.width = p.r_f32()?;
        out.height = p.r_f32()?;
        out.rotate = p.r_f32()?;
        out.shader = SmolStr::new(p.r_stringz()?);
        if version == 3 {
            out.shader = SmolStr::new_static(DEFAULT_SHADER);
        }
        out.texture = SmolStr::new(p.r_stringz()?);
        let (items, sized) = match r.open_chunk(WM_CHUNK_ITEMS)? {
            Some(c) => (Some(c), false),
            None => (r.open_chunk(WM_CHUNK_ITEMS2)?, true),
        };
        if let Some(items) = items {
            let items = items.reader();
            let mut id = 0u32;
            while let Some(c) = items.open_chunk(id)? {
                id += 1;
                let mut q = c.reader();
                let count = q.r_u32()?;
                if count == 0 {
                    continue;
                }
                let shader = SmolStr::new(q.r_stringz()?);
                let texture = SmolStr::new(q.r_stringz()?);
                let mut slot = Slot { shader, texture, items: Vec::new() };
                for _ in 0..count {
                    let flags = q.r_u8()?;
                    let min = read_vec3(&mut q)?;
                    let max = read_vec3(&mut q)?;
                    let center = read_vec3(&mut q)?;
                    let radius = q.r_f32()?;
                    let (w, h, rr) =
                        if sized { (q.r_f32()?, q.r_f32()?, q.r_f32()?) } else { (1.0, 1.0, 1.0) };
                    let n = q.r_u32()? as usize;
                    if n.saturating_mul(LIT) > q.elapsed() {
                        return Err(SceneError::load(
                            "Static Wallmark: a mark runs past the end of its chunk.",
                        ));
                    }
                    let mut verts = Vec::with_capacity(n);
                    for _ in 0..n {
                        let p = read_vec3(&mut q)?;
                        let color = q.r_u32()?;
                        let uv = [q.r_f32()?, q.r_f32()?];
                        verts.push(LitVert { p, color, uv });
                    }
                    // "Invalid wallmark (Contain more than 8192 vertices). Removed."
                    if n > MAX_VERTICES {
                        continue;
                    }
                    slot.items.push(Mark {
                        flags,
                        bbox: Aabb::new(min, max),
                        center,
                        radius,
                        w,
                        h,
                        r: rr,
                        verts: verts.into(),
                    });
                }
                out.slots.push(slot);
            }
        }
        Ok(out)
    }

    /// Saves the marks to a stream.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.chunk(WM_CHUNK_VERSION, |w| w.w_u16(WM_VERSION));
        w.chunk(WM_CHUNK_FLAGS, |w| w.w_u32(self.flags));
        w.chunk(WM_CHUNK_PARAMS, |w| {
            w.w_f32(self.width);
            w.w_f32(self.height);
            w.w_f32(self.rotate);
            w.w_stringz(&self.shader);
            w.w_stringz(&self.texture);
        });
        w.chunk(WM_CHUNK_ITEMS2, |w| {
            for (i, slot) in self.slots.iter().enumerate() {
                w.chunk(i as u32, |w| {
                    w.w_u32(slot.items.len() as u32);
                    if slot.items.is_empty() {
                        return;
                    }
                    w.w_stringz(&slot.shader);
                    w.w_stringz(&slot.texture);
                    for m in &slot.items {
                        w.w_u8(m.flags);
                        w.w_vec3(m.bbox.min);
                        w.w_vec3(m.bbox.max);
                        w.w_vec3(m.center);
                        w.w_f32(m.radius);
                        w.w_f32(m.w);
                        w.w_f32(m.h);
                        w.w_f32(m.r);
                        w.w_u32(m.verts.len() as u32);
                        for v in m.verts.iter() {
                            w.w_vec3(v.p);
                            w.w_u32(v.color);
                            w.w_f32(v.uv[0]);
                            w.w_f32(v.uv[1]);
                        }
                    }
                });
            }
        });
        w.into_bytes()
    }

    /// The number of marks.
    #[must_use]
    pub fn count(&self) -> usize {
        self.slots.iter().map(|s| s.items.len()).sum()
    }

    #[must_use]
    pub fn selected_count(&self) -> usize {
        self.slots.iter().flat_map(|s| &s.items).filter(|m| m.selected()).count()
    }

    #[must_use]
    pub fn draws(&self) -> bool {
        self.flags & flags::DRAW != 0
    }

    /// `SelectObjects(flag)`.
    pub fn select_all(&mut self, on: bool) {
        for m in self.slots.iter_mut().flat_map(|s| &mut s.items) {
            if on {
                m.flags |= SELECTED;
            } else {
                m.flags &= !SELECTED;
            }
        }
    }

    /// Inverts the selection.
    pub fn invert_selection(&mut self) {
        for m in self.slots.iter_mut().flat_map(|s| &mut s.items) {
            m.flags ^= SELECTED;
        }
    }

    /// Sets the selection of marks by slot and index.
    pub fn select(&mut self, which: &[(usize, usize)], mode: crate::apply::SelectMode) -> usize {
        use crate::apply::SelectMode;
        if mode == SelectMode::Set {
            self.select_all(false);
        }
        let mut n = 0;
        for &(s, i) in which {
            let Some(m) = self.slots.get_mut(s).and_then(|s| s.items.get_mut(i)) else { continue };
            m.flags = match mode {
                SelectMode::Set | SelectMode::Add => m.flags | SELECTED,
                SelectMode::Remove => m.flags & !SELECTED,
                SelectMode::Toggle => m.flags ^ SELECTED,
            };
            n += 1;
        }
        n
    }

    /// Removes the selected marks: each replaced by its slot's last one.
    pub fn remove_selected(&mut self) -> usize {
        let mut n = 0;
        for slot in &mut self.slots {
            let mut i = 0;
            while i < slot.items.len() {
                if slot.items[i].selected() {
                    slot.items.swap_remove(i);
                    n += 1;
                } else {
                    i += 1;
                }
            }
        }
        n
    }

    /// The nearest mark whose box the ray enters from outside.
    #[must_use]
    pub fn pick(&self, start: Vec3, dir: Vec3, dist: f32) -> Option<(usize, usize, f32)> {
        let mut best = None;
        let mut reach = dist;
        for (si, slot) in self.slots.iter().enumerate() {
            for (mi, m) in slot.items.iter().enumerate() {
                if let Some((pt, false)) = xr::box_pick2(m.bbox.min, m.bbox.max, start, dir) {
                    let range = (pt - start).length();
                    if range < reach {
                        reach = range;
                        best = Some((si, mi, range));
                    }
                }
            }
        }
        best
    }

    /// The frustum test: the marks whose box meets every plane (`n . p + w >= 0` inside).
    #[must_use]
    pub fn in_frustum(&self, planes: &[glam::Vec4]) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (si, slot) in self.slots.iter().enumerate() {
            for (mi, m) in slot.items.iter().enumerate() {
                if crate::objtools::geom::box_in_frustum(planes, &m.bbox) {
                    out.push((si, mi));
                }
            }
        }
        out
    }

    /// The box of the marks, of the selected ones only with `selected`.
    #[must_use]
    pub fn bbox(&self, selected: bool) -> Option<Aabb> {
        let mut b = Aabb::INVALID;
        for m in self.slots.iter().flat_map(|s| &s.items).filter(|m| !selected || m.selected()) {
            b.merge(&m.bbox);
        }
        b.is_valid().then_some(b)
    }

    /// The registration of a new mark: into its slot (a mark there whose centre is within 2 cm
    /// is replaced), or into a new slot. Answers where it went and whether it replaced one.
    pub fn insert(&mut self, shader: &str, texture: &str, mark: Mark) -> (usize, usize, bool) {
        let si = match self.slots.iter().position(|s| s.shader == shader && s.texture == texture) {
            Some(si) => {
                if let Some(slot) = self.slots.get_mut(si)
                    && let Some(mi) = slot.items.iter().position(|m| xr::similar(m.center, mark.center, 0.02))
                {
                    slot.items[mi] = mark;
                    return (si, mi, true);
                }
                si
            }
            None => {
                self.slots.push(Slot {
                    shader: SmolStr::new(shader),
                    texture: SmolStr::new(texture),
                    items: Vec::new(),
                });
                self.slots.len() - 1
            }
        };
        let Some(slot) = self.slots.get_mut(si) else { return (0, 0, false) };
        slot.items.push(mark);
        (si, slot.items.len() - 1, false)
    }

    /// The one selected mark (moving two is refused).
    #[must_use]
    pub fn single_selected(&self) -> Option<(usize, usize)> {
        let mut found = None;
        for (si, slot) in self.slots.iter().enumerate() {
            for (mi, m) in slot.items.iter().enumerate() {
                if m.selected() {
                    if found.is_some() {
                        return None;
                    }
                    found = Some((si, mi));
                }
            }
        }
        found
    }
}

/// How a mark is laid: its size, rotation and the axes it is aligned with.
#[derive(Clone, Copy, Debug)]
pub struct Lay {
    pub width: f32,
    pub height: f32,
    pub rotate: f32,
    pub axis_align: bool,
    /// Used unless the mark is axis-aligned.
    pub camera_top: Vec3,
    pub camera_right: Vec3,
}

/// The projection of a mark onto the surface at `from` with normal `n`.
#[must_use]
pub fn build_matrix(n: Vec3, from: Vec3, lay: &Lay) -> M43 {
    let (inv_w, inv_h) = (2.0 / lay.width, 2.0 / lay.height);
    let at = from - n;
    let y = if lay.axis_align {
        if n.y.abs() > 0.99 { Vec3::X } else { Vec3::Y }
    } else if fsimilar(xr::dot(lay.camera_top, n), 1.0, EPS) {
        lay.camera_right
    } else {
        lay.camera_top
    };
    let right = xr::cross(y, n);
    let up = xr::cross(n, right);
    let mut view = M43::camera(from, at, up);
    view = view.mul_a_43(&M43::rotate_z(lay.rotate));
    view.mul_a_43(&M43::scale(inv_w, inv_h, inv_w.max(inv_h)))
}

/// A mark once the geometry is gathered: `hit` is the triangle the ray met and
/// `contact` the point on it; `around` the triangles of the box round it. `Err` names why no
/// mark could be made.
pub fn build_mark(hit: [Vec3; 3], contact: Vec3, around: &[[Vec3; 3]], lay: &Lay) -> Result<Mark, String> {
    let normal = xr::mknormal(hit[0], hit[1], hit[2]);
    let mut col = Collector::new();
    col.add_face_packed_d(hit[0], hit[1], hit[2], 0, Collector::DEFAULT_EPS);
    for t in around {
        let tn = xr::mknormal(t[0], t[1], t[2]);
        if xr::dot(tn, normal) < 0.1 {
            continue;
        }
        col.add_face_packed_d(t[0], t[1], t[2], 0, Collector::DEFAULT_EPS);
    }
    col.remove_duplicate_tris();
    let adjacency = col.calc_adjacency();
    let view = build_matrix(normal, contact, lay);
    let clipper = Frustum::from_matrix_lrtb(&view);
    let verts = col.verts();
    let tris = col.tris();
    let tri = |t: usize| -> Option<[Vec3; 3]> {
        let v = tris.get(t)?.verts;
        Some([*verts.get(v[0] as usize)?, *verts.get(v[1] as usize)?, *verts.get(v[2] as usize)?])
    };
    let lit = |p: Vec3| {
        let uv = view.transform_tiny(p);
        LitVert { p, color: 0, uv: [(1.0 + uv.x) * 0.5, (1.0 - uv.y) * 0.5] }
    };
    let mut out: Vec<LitVert> = Vec::new();
    let mut visited = vec![false; tris.len()];
    // the recursion from triangle 0, as a stack of (triangle, next edge)
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let mut enter = |t: usize, out: &mut Vec<LitVert>, stack: &mut Vec<(usize, usize)>| {
        if visited.get(t).copied().unwrap_or(true) {
            return;
        }
        if let Some(v) = visited.get_mut(t) {
            *v = true;
        }
        let Some(src) = tri(t) else { return };
        let Some(poly) = clipper.clip_poly(&src) else { return };
        let (v0, mut v1) = (lit(poly[0]), lit(poly[1]));
        for p in poly.iter().skip(2) {
            let v2 = lit(*p);
            out.extend([v0, v1, v2]);
            v1 = v2;
        }
        stack.push((t, 0));
    };
    enter(0, &mut out, &mut stack);
    while let Some(top) = stack.last_mut() {
        let (t, i) = *top;
        if i >= 3 {
            stack.pop();
            continue;
        }
        top.1 += 1;
        let adj = adjacency.get(3 * t + i).copied().unwrap_or(u32::MAX);
        if adj == u32::MAX {
            continue;
        }
        let Some(a) = tri(adj as usize) else { continue };
        if xr::dot(xr::mknormal(a[0], a[1], a[2]), normal) < EPS {
            continue;
        }
        enter(adj as usize, &mut out, &mut stack);
    }
    if out.len() < 3 || out.len() > MAX_VERTICES {
        return Err(format!("Invalid wallmark vertex count. [Min: 3. Max: {MAX_VERTICES}]."));
    }
    let mut bbox = Aabb::INVALID;
    for v in &out {
        bbox.modify(v.p);
    }
    let center = bbox.center();
    let radius = xr::dist_sqr(center, bbox.max).sqrt();
    bbox.grow(EPS_L);
    Ok(Mark {
        flags: SELECTED,
        bbox,
        center,
        radius,
        w: lay.width,
        h: lay.height,
        r: lay.rotate,
        verts: out.into(),
    })
}

/// `RayQuery(OPT_ONLYNEAREST | OPT_CULL)` over scene objects: the nearest front face, its range
/// and its world triangle.
#[must_use]
pub fn ray_triangle(
    state: &SceneState,
    lib: &Library,
    ids: &[ObjId],
    start: Vec3,
    dir: Vec3,
    range: f32,
) -> Option<(f32, [Vec3; 3])> {
    let mut best: Option<(f32, [Vec3; 3])> = None;
    for id in ids {
        let Some(o) = state.get(*id).filter(|o| o.class == ObjClass::SceneObject) else { continue };
        let ObjData::SceneObject(d) = &o.data else { continue };
        let Some(lo) = lib.cached(&d.reference) else { continue };
        let Some(bvh) = &lo.bvh else { continue };
        let world = o.world();
        let inv = gz_fmt_object::Mat4::invert_43(&world);
        let reach = best.map_or(range, |b| b.0);
        let mut ray = gz_fmt_level::Ray::new(inv.transform_tiny(start), inv.transform_dir(dir), reach);
        ray.cull_back = true;
        let Some(hit) = bvh.ray_nearest(&ray) else { continue };
        if hit.t >= reach {
            continue;
        }
        let Some(t) = lo.tri(hit.tri) else { continue };
        best = Some((
            hit.t,
            [world.transform_tiny(t[0]), world.transform_tiny(t[1]), world.transform_tiny(t[2])],
        ));
    }
    best
}

/// `BoxQuery(OPT_FULL_TEST)` over scene objects: the world triangles meeting a box, object by
/// object in the list's order, each object's in its own order.
#[must_use]
pub fn box_triangles(state: &SceneState, lib: &Library, ids: &[ObjId], b: &Aabb) -> Vec<[Vec3; 3]> {
    let mut out = Vec::new();
    let (c, e) = (b.center(), (b.max - b.min) * 0.5);
    for id in ids {
        let Some(o) = state.get(*id) else { continue };
        let ObjData::SceneObject(d) = &o.data else { continue };
        let Some(lo) = lib.cached(&d.reference) else { continue };
        let Some(bvh) = &lo.bvh else { continue };
        let world = o.world();
        let inv = gz_fmt_object::Mat4::invert_43(&world);
        let local = crate::math::xform_box(b, &inv);
        let mut found = Vec::new();
        bvh.query_box(&local, &mut found);
        found.sort_unstable();
        for t in found {
            let Some(tri) = lo.tri(t) else { continue };
            let w =
                [world.transform_tiny(tri[0]), world.transform_tiny(tri[1]), world.transform_tiny(tri[2])];
            if xr::tri_box(c, e, &w, true) {
                out.push(w);
            }
        }
    }
    out
}

/// Everything a mark needs of the scene for a ray: the mark, or why there is none
/// (`Ok(None)`: the ray hit nothing).
pub fn mark_at(
    state: &SceneState,
    lib: &Library,
    start: Vec3,
    dir: Vec3,
    range: f32,
    lay: &Lay,
) -> Result<Option<Mark>, String> {
    let ids = super::ground::objects(state, lib, None, None);
    let Some((t, hit)) = ray_triangle(state, lib, &ids, start, dir, range) else { return Ok(None) };
    let contact = xr::mad(start, dir, t);
    let mut bbox = Aabb::new(contact, contact);
    bbox.grow(lay.height.max(lay.width) * 2.0);
    let around = box_triangles(state, lib, &ids, &bbox);
    build_mark(hit, contact, &around, lay).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lay() -> Lay {
        Lay {
            width: 1.0,
            height: 1.0,
            rotate: 0.0,
            axis_align: true,
            camera_top: Vec3::Y,
            camera_right: Vec3::X,
        }
    }

    /// Two faces of a box corner at the origin: the floor (y = 0, x and z > 0) and a wall
    /// (x = 0, facing +x).
    fn corner() -> Vec<[Vec3; 3]> {
        let s = 4.0;
        vec![
            [Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, s), Vec3::new(s, 0.0, s)],
            [Vec3::new(0.0, 0.0, 0.0), Vec3::new(s, 0.0, s), Vec3::new(s, 0.0, 0.0)],
            [Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, s, 0.0), Vec3::new(0.0, s, s)],
            [Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, s, s), Vec3::new(0.0, 0.0, s)],
        ]
    }

    #[test]
    fn a_mark_is_clipped_against_a_box_corner() {
        let tris = corner();
        // on the floor 0.2 m from the wall: the mark (1 m wide) is cut at the wall, and the wall
        // (at right angles) is not painted
        let contact = Vec3::new(0.2, 0.0, 2.0);
        let m = build_mark(tris[0], contact, &tris, &lay()).unwrap();
        assert!(m.verts.len() >= 3 && m.verts.len().is_multiple_of(3));
        for v in m.verts.iter() {
            assert!(v.p.y.abs() < 1e-5, "only the floor is painted: {:?}", v.p);
            assert!(v.p.x >= -1e-5, "nothing behind the wall: {:?}", v.p);
            assert!((v.p.x - contact.x).abs() <= 0.5 + 1e-4 && (v.p.z - contact.z).abs() <= 0.5 + 1e-4);
            assert!((0.0..=1.0).contains(&v.uv[0]) && (0.0..=1.0).contains(&v.uv[1]));
        }
        assert!(m.bbox.min.x >= -EPS_L * 2.0 && m.selected());
    }

    #[test]
    fn the_chunk_round_trips_and_a_close_mark_replaces() {
        let tris = corner();
        let m = build_mark(tris[0], Vec3::new(2.0, 0.0, 2.0), &tris, &lay()).unwrap();
        let mut w = Wallmarks { texture: "wm\\wm_blood".into(), ..Wallmarks::default() };
        let tex = w.texture.clone();
        assert!(!w.insert(DEFAULT_SHADER, &tex, m.clone()).2);
        assert!(w.insert(DEFAULT_SHADER, &tex, m.clone()).2);
        assert_eq!(w.count(), 1);
        let back = Wallmarks::read(&w.write()).unwrap();
        assert_eq!(back, w);
        w.select_all(true);
        assert_eq!(w.remove_selected(), 1);
        assert_eq!(w.count(), 0);
        // an empty slot is written and not read back
        assert!(Wallmarks::read(&w.write()).unwrap().slots.is_empty());
    }
}
