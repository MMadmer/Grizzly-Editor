//! What can be worked out of a model: the facts a level designer needs before a thing is placed
//! or stood on - its box, where its origin lies, which of its faces stop a body, the flat faces
//! something can stand on, and for a large model the connected pieces it is made of.
//!
//! Everything is measured in the model's own triangles ([`LibraryObject`]) with the use of every
//! surface as the library tells it ([`crate::library::Library::uses`]); nothing is guessed from
//! a name. Pure and deterministic: the same model gives the same facts.

use rustc_hash::FxHashMap;
use serde_json::{Value, json};

use crate::library::{LibraryObject, NO_SURFACE, SurfaceUse};
use crate::math::{Aabb, Vec3};

/// A face is one a thing can stand on when its normal is within about 25 degrees of straight up.
pub const UP: f32 = 0.906;

/// A flat region of a model: connected faces that look up.
#[derive(Clone, Debug, PartialEq)]
pub struct Floor {
    /// The height most of it lies at (the mean, by area).
    pub y: f32,
    pub y_min: f32,
    pub y_max: f32,
    /// Its area seen from above, square metres.
    pub area: f32,
    /// The rectangle around it: `x0, z0, x1, z1`.
    pub rect: [f32; 4],
}

/// A connected piece of a model.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub bounds: Aabb,
    pub triangles: u32,
    /// The area of its faces, square metres.
    pub area: f32,
    /// The area under it: of its box seen from above.
    pub footprint: f32,
    /// Its flat regions, largest first.
    pub floors: Vec<Floor>,
    /// Flat regions left out of `floors`.
    pub more_floors: u32,
    /// The surfaces it wears, by area: indices into the model's surfaces.
    pub surfaces: Vec<u32>,
}

/// What a model's surface carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceFact {
    pub triangles: u32,
    pub area: f32,
    pub uses: SurfaceUse,
}

/// The faces of a model that stop a body.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Collision {
    pub triangles: u32,
    pub area: f32,
    /// The colliding area against the drawn one, at most 1: how much of what is seen is solid.
    pub share: f32,
    pub bounds: Aabb,
}

/// Where a model's origin lies in its box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pivot {
    /// The middle of the bottom: the model stands where it is placed.
    Base,
    /// The middle of the box.
    Centre,
    /// Elsewhere: the box's middle seen from the origin.
    Off(Vec3),
}

/// How much of a model is listed.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Flat regions listed for the model, and for each part.
    pub floors: usize,
    /// Parts listed.
    pub parts: usize,
    /// A model is told in parts when its box is at least this long, metres.
    pub parts_from: f32,
    /// A part at least this long (its box's longest side) is listed, metres.
    pub part_size: f32,
    /// A flat region that covers at least this share of the area under the model is ground: the
    /// pieces are told without it, so what stands on the ground is not one piece with it. 0: no
    /// region is ground (a model is not a level).
    pub ground_share: f32,
}

impl Default for Limits {
    fn default() -> Self {
        Self { floors: 12, parts: 48, parts_from: 20.0, part_size: 2.0, ground_share: 0.0 }
    }
}

/// Everything worked out of a model.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelFacts {
    pub bounds: Aabb,
    pub sphere: (Vec3, f32),
    pub pivot: Pivot,
    pub triangles: u32,
    pub vertices: u32,
    /// One per surface of the model, in its order.
    pub surfaces: Vec<SurfaceFact>,
    /// `None` when no face stops a body.
    pub collision: Option<Collision>,
    /// The flat regions of the whole model, largest first.
    pub floors: Vec<Floor>,
    pub more_floors: u32,
    /// The connected pieces, largest first; empty for a model told whole.
    pub parts: Vec<Part>,
    /// Pieces smaller than [`Limits::part_size`] or past [`Limits::parts`].
    pub more_parts: u32,
    /// The flat regions taken for ground ([`Limits::ground_share`]), largest first.
    pub ground: Vec<Floor>,
}

struct Dsu(Vec<u32>);

impl Dsu {
    fn new(n: usize) -> Self {
        Self((0..n as u32).collect())
    }

    fn find(&mut self, mut x: u32) -> u32 {
        while self.0[x as usize] != x {
            let up = self.0[self.0[x as usize] as usize];
            self.0[x as usize] = up;
            x = up;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b) as usize] = a.min(b);
        }
    }
}

struct Region {
    area: f32,
    y_sum: f32,
    y_min: f32,
    y_max: f32,
    rect: [f32; 4],
    /// Some triangle of it.
    tri: u32,
}

fn floor_of(r: &Region) -> Floor {
    Floor { y: r.y_sum / r.area.max(1e-9), y_min: r.y_min, y_max: r.y_max, area: r.area, rect: r.rect }
}

/// The smallest flat region worth a line: a fiftieth of the area under the model, never under a
/// square decimetre or over four square metres.
fn smallest_floor(bounds: &Aabb) -> f32 {
    let size = bounds.max - bounds.min;
    (size.x * size.z * 0.02).clamp(0.01, 4.0)
}

/// Triangles to measure: a soup with the surface of every triangle and the use of every surface.
pub struct Soup<'a> {
    pub verts: &'a [Vec3],
    pub tris: &'a [[u32; 3]],
    /// Per triangle: its surface ([`NO_SURFACE`] for none).
    pub tri_surface: &'a [u32],
    /// Per surface; a surface past its end is drawn and solid.
    pub uses: &'a [SurfaceUse],
    /// How many surfaces there are.
    pub surfaces: usize,
}

/// The facts of a model. `uses` is the use of every surface ([`crate::library::Library::uses`]).
/// A model with a collision mesh of its own ([`crate::library::SolidMesh`]) is measured as its
/// drawn faces and, after its own surfaces, one surface a material of that mesh.
#[must_use]
pub fn facts(o: &LibraryObject, uses: &[SurfaceUse], limits: &Limits) -> ModelFacts {
    let n = o.object.surfaces.len();
    let Some(solid) = &o.solid else {
        return measure(
            &Soup { verts: &o.verts, tris: &o.tris, tri_surface: &o.tri_surface, uses, surfaces: n },
            limits,
        );
    };
    let base = o.verts.len() as u32;
    let mut verts: Vec<Vec3> = o.verts.to_vec();
    verts.extend(solid.verts.iter().copied());
    let mut tris: Vec<[u32; 3]> = o.tris.to_vec();
    tris.extend(solid.tris.iter().map(|t| t.map(|v| v + base)));
    let mut tri_surface: Vec<u32> = o.tri_surface.to_vec();
    tri_surface.extend(solid.tri_material.iter().map(|m| n as u32 + u32::from(*m)));
    let mut all: Vec<SurfaceUse> = vec![SurfaceUse { draws: true, collides: false }; n];
    all.extend(solid.materials.iter().map(|_| SurfaceUse { draws: false, collides: true }));
    measure(
        &Soup { verts: &verts, tris: &tris, tri_surface: &tri_surface, uses: &all, surfaces: all.len() },
        limits,
    )
}

/// The facts of a soup of triangles.
#[must_use]
pub fn measure(o: &Soup<'_>, limits: &Limits) -> ModelFacts {
    let uses = o.uses;
    let use_of = |tri: usize| -> SurfaceUse {
        match o.tri_surface.get(tri).copied() {
            Some(s) if s != NO_SURFACE => uses.get(s as usize).copied().unwrap_or(SurfaceUse::BOTH),
            _ => SurfaceUse::BOTH,
        }
    };
    // positions welded to the millimetre: two meshes that meet are one piece
    let mut ids: FxHashMap<[i32; 3], u32> = FxHashMap::default();
    let weld: Vec<u32> = o
        .verts
        .iter()
        .map(|p| {
            let key = [p.x, p.y, p.z].map(|v| (v * 1000.0).round() as i32);
            let next = ids.len() as u32;
            *ids.entry(key).or_insert(next)
        })
        .collect();
    let points = ids.len();
    drop(ids);

    let n = o.tris.len();
    let mut surfaces: Vec<SurfaceFact> = uses
        .iter()
        .map(|u| SurfaceFact { triangles: 0, area: 0.0, uses: *u })
        .chain(std::iter::repeat(SurfaceFact { triangles: 0, area: 0.0, uses: SurfaceUse::BOTH }))
        .take(o.surfaces)
        .collect();
    let mut area = vec![0.0f32; n];
    let mut normal_y = vec![0.0f32; n];
    let (mut drawn_area, mut solid_area, mut solid_tris) = (0.0f32, 0.0f32, 0u32);
    let mut solid_box = Aabb::INVALID;
    let mut bounds = Aabb::INVALID;
    for (i, t) in o.tris.iter().enumerate() {
        let [a, b, c] = t.map(|v| o.verts.get(v as usize).copied().unwrap_or(Vec3::ZERO));
        let cross = (b - a).cross(c - a);
        let len = cross.length();
        if !len.is_finite() {
            continue;
        }
        area[i] = len * 0.5;
        normal_y[i] = if len > 1e-12 { cross.y / len } else { 0.0 };
        for p in [a, b, c] {
            bounds.modify(p);
        }
        let u = use_of(i);
        if let Some(s) = o.tri_surface.get(i).and_then(|s| surfaces.get_mut(*s as usize)) {
            s.triangles += 1;
            s.area += area[i];
        }
        if u.draws {
            drawn_area += area[i];
        }
        if u.collides {
            solid_area += area[i];
            solid_tris += 1;
            for p in [a, b, c] {
                solid_box.modify(p);
            }
        }
    }
    if !bounds.is_valid() {
        bounds = Aabb::new(Vec3::ZERO, Vec3::ZERO);
    }
    let collision = (solid_tris > 0).then(|| Collision {
        triangles: solid_tris,
        area: solid_area,
        share: if drawn_area > 1e-9 { (solid_area / drawn_area).min(1.0) } else { 1.0 },
        bounds: solid_box,
    });
    // the pieces and the floors are told of what stops a body; of a model nothing of which does,
    // of what is drawn
    let counts = |i: usize| if solid_tris > 0 { use_of(i).collides } else { true };

    let mut pieces = Dsu::new(points);
    let mut flats = Dsu::new(n);
    // the first face that looks up at every welded point
    let mut up_at: Vec<u32> = vec![u32::MAX; points];
    for (i, t) in o.tris.iter().enumerate() {
        if !counts(i) || area[i] <= 0.0 {
            continue;
        }
        let w = t.map(|v| weld.get(v as usize).copied().unwrap_or(0));
        if normal_y[i] >= UP {
            for p in w {
                let first = &mut up_at[p as usize];
                if *first == u32::MAX {
                    *first = i as u32;
                } else {
                    flats.union(*first, i as u32);
                }
            }
        }
    }

    // the flat regions
    let mut regions: FxHashMap<u32, Region> = FxHashMap::default();
    for (i, t) in o.tris.iter().enumerate() {
        if !counts(i) || area[i] <= 0.0 || normal_y[i] < UP {
            continue;
        }
        let root = flats.find(i as u32);
        let r = regions.entry(root).or_insert(Region {
            area: 0.0,
            y_sum: 0.0,
            y_min: f32::MAX,
            y_max: f32::MIN,
            rect: [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
            tri: i as u32,
        });
        // seen from above
        let flat = area[i] * normal_y[i];
        let mut y = 0.0;
        for v in t {
            let p = o.verts.get(*v as usize).copied().unwrap_or(Vec3::ZERO);
            y += p.y / 3.0;
            r.y_min = r.y_min.min(p.y);
            r.y_max = r.y_max.max(p.y);
            r.rect = [r.rect[0].min(p.x), r.rect[1].min(p.z), r.rect[2].max(p.x), r.rect[3].max(p.z)];
        }
        r.area += flat;
        r.y_sum += y * flat;
    }
    let least = smallest_floor(&bounds);
    // the ground: the flat regions that cover a share of everything under the model
    let under = (bounds.max.x - bounds.min.x) * (bounds.max.z - bounds.min.z);
    let ground_roots: rustc_hash::FxHashSet<u32> = if limits.ground_share > 0.0 {
        regions.iter().filter(|(_, r)| r.area >= under * limits.ground_share).map(|(root, _)| *root).collect()
    } else {
        rustc_hash::FxHashSet::default()
    };
    let is_ground =
        |flats: &mut Dsu, i: usize| normal_y[i] >= UP && ground_roots.contains(&flats.find(i as u32));
    // the pieces: what is joined through shared points, the ground left out, so that what
    // stands on it is not one piece with it
    for (i, t) in o.tris.iter().enumerate() {
        if !counts(i) || area[i] <= 0.0 || is_ground(&mut flats, i) {
            continue;
        }
        let w = t.map(|v| weld.get(v as usize).copied().unwrap_or(0));
        pieces.union(w[0], w[1]);
        pieces.union(w[0], w[2]);
    }
    let mut ground: Vec<Region> = Vec::new();
    let mut flat_list: Vec<Region> = Vec::new();
    for (root, r) in regions {
        if ground_roots.contains(&root) {
            ground.push(r);
        } else if r.area >= least {
            flat_list.push(r);
        }
    }
    ground.sort_by(|a, b| b.area.total_cmp(&a.area).then(a.tri.cmp(&b.tri)));
    flat_list.sort_by(|a, b| b.area.total_cmp(&a.area).then(a.tri.cmp(&b.tri)));

    // the pieces
    let size = bounds.max - bounds.min;
    let in_parts = size.max_element() >= limits.parts_from;
    let (mut parts, mut more_parts) = (Vec::new(), 0u32);
    if in_parts {
        struct Piece {
            bounds: Aabb,
            triangles: u32,
            area: f32,
            by_surface: FxHashMap<u32, f32>,
            first: u32,
        }
        let mut found: FxHashMap<u32, Piece> = FxHashMap::default();
        for (i, t) in o.tris.iter().enumerate() {
            if !counts(i) || area[i] <= 0.0 || is_ground(&mut flats, i) {
                continue;
            }
            let root = pieces.find(weld.get(t[0] as usize).copied().unwrap_or(0));
            let p = found.entry(root).or_insert(Piece {
                bounds: Aabb::INVALID,
                triangles: 0,
                area: 0.0,
                by_surface: FxHashMap::default(),
                first: i as u32,
            });
            for v in t {
                p.bounds.modify(o.verts.get(*v as usize).copied().unwrap_or(Vec3::ZERO));
            }
            p.triangles += 1;
            p.area += area[i];
            if let Some(s) = o.tri_surface.get(i).filter(|s| **s != NO_SURFACE) {
                *p.by_surface.entry(*s).or_default() += area[i];
            }
        }
        let mut list: Vec<(u32, Piece)> = found.into_iter().collect();
        if list.len() > 1 || !ground_roots.is_empty() {
            let longest = |b: &Aabb| (b.max - b.min).max_element();
            let before = list.len();
            list.retain(|(_, p)| longest(&p.bounds) >= limits.part_size);
            more_parts = (before - list.len()) as u32;
            // the largest under it first; the order of the model's faces between equals
            let foot = |b: &Aabb| (b.max.x - b.min.x) * (b.max.z - b.min.z);
            list.sort_by(|a, b| {
                foot(&b.1.bounds).total_cmp(&foot(&a.1.bounds)).then(a.1.first.cmp(&b.1.first))
            });
            if list.len() > limits.parts {
                more_parts += (list.len() - limits.parts) as u32;
                list.truncate(limits.parts);
            }
            for (root, p) in list {
                let mine: Vec<&Region> = flat_list
                    .iter()
                    .filter(|r| {
                        let t = o.tris.get(r.tri as usize).map_or(0, |t| t[0]);
                        pieces.find(weld.get(t as usize).copied().unwrap_or(0)) == root
                    })
                    .collect();
                let mut by_surface: Vec<(u32, f32)> = p.by_surface.into_iter().collect();
                by_surface.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                parts.push(Part {
                    footprint: foot(&p.bounds),
                    bounds: p.bounds,
                    triangles: p.triangles,
                    area: p.area,
                    more_floors: mine.len().saturating_sub(limits.floors) as u32,
                    floors: mine.iter().take(limits.floors).map(|r| floor_of(r)).collect(),
                    surfaces: by_surface.into_iter().take(4).map(|(s, _)| s).collect(),
                });
            }
        }
    }

    let centre = bounds.center();
    let radius = o.verts.iter().map(|p| p.distance(centre)).fold(0.0f32, f32::max);
    let near = |v: f32, span: f32| v.abs() <= (span * 0.1).max(0.02);
    let pivot = if near(centre.x, size.x) && near(centre.z, size.z) {
        if bounds.min.y.abs() <= (size.y * 0.02).max(0.02) {
            Pivot::Base
        } else if near(centre.y, size.y) {
            Pivot::Centre
        } else {
            Pivot::Off(centre)
        }
    } else {
        Pivot::Off(centre)
    };
    ModelFacts {
        bounds,
        sphere: (centre, radius),
        pivot,
        triangles: n as u32,
        vertices: o.verts.len() as u32,
        surfaces,
        collision,
        more_floors: flat_list.len().saturating_sub(limits.floors) as u32,
        floors: flat_list.iter().take(limits.floors).map(floor_of).collect(),
        parts,
        more_parts,
        ground: ground.iter().map(floor_of).collect(),
    }
}

/// A length for a reader: millimetres are exact enough and read back as written.
#[must_use]
pub fn mm(v: f32) -> f64 {
    let r = (f64::from(v) * 1000.0).round() / 1000.0;
    if r == 0.0 { 0.0 } else { r }
}

/// A point as `[x, y, z]`.
#[must_use]
pub fn v3(p: Vec3) -> Value {
    json!([mm(p.x), mm(p.y), mm(p.z)])
}

/// A box as `{ min, max }`.
#[must_use]
pub fn box_json(b: &Aabb) -> Value {
    json!({ "min": v3(b.min), "max": v3(b.max) })
}

impl Floor {
    /// `{ y, area, box = [x0, z0, x1, z1] }`, with `slope = [y_min, y_max]` where it is no level floor.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = json!({
            "y": mm(self.y),
            "area": mm(self.area),
            "box": self.rect.map(mm),
        });
        if self.y_max - self.y_min > 0.05
            && let Some(m) = out.as_object_mut()
        {
            m.insert("slope".into(), json!([mm(self.y_min), mm(self.y_max)]));
        }
        out
    }
}

impl ModelFacts {
    /// The facts as the cards and the commands state them (`docs/GAME_LIBRARY.md`, 3.1): the
    /// box, the size, the sphere, the pivot, the counts, the collision, the floors, the parts.
    /// `surface` gives a surface's own words (its name, texture, shaders, material) by index.
    #[must_use]
    pub fn to_json(&self, surface: &dyn Fn(u32) -> Value) -> Value {
        let size = self.bounds.max - self.bounds.min;
        let pivot = match self.pivot {
            Pivot::Base => json!("base"),
            Pivot::Centre => json!("centre"),
            Pivot::Off(c) => json!({ "box_centre": v3(c) }),
        };
        let surfaces: Vec<Value> = self
            .surfaces
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let mut row = surface(i as u32);
                if let Some(m) = row.as_object_mut() {
                    m.insert("triangles".into(), json!(s.triangles));
                    m.insert("area".into(), json!(mm(s.area)));
                    m.insert("draws".into(), json!(s.uses.draws));
                    m.insert("collides".into(), json!(s.uses.collides));
                }
                row
            })
            .collect();
        let collision = match &self.collision {
            Some(c) => json!({
                "triangles": c.triangles,
                "area": mm(c.area),
                "share": mm(c.share),
                "box": box_json(&c.bounds),
            }),
            None => json!(false),
        };
        let mut out = json!({
            "box": box_json(&self.bounds),
            "size": v3(size),
            "sphere": { "center": v3(self.sphere.0), "radius": mm(self.sphere.1) },
            "pivot": pivot,
            "triangles": self.triangles,
            "vertices": self.vertices,
            "surfaces": surfaces,
            "collision": collision,
            "floors": self.floors.iter().map(Floor::to_json).collect::<Vec<_>>(),
        });
        let Some(m) = out.as_object_mut() else { return out };
        if self.more_floors > 0 {
            m.insert("more_floors".into(), json!(self.more_floors));
        }
        if !self.ground.is_empty() {
            m.insert("ground".into(), json!(self.ground.iter().map(Floor::to_json).collect::<Vec<_>>()));
        }
        if !self.parts.is_empty() {
            let parts: Vec<Value> = self
                .parts
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let mut row = json!({
                        "id": i + 1,
                        "box": box_json(&p.bounds),
                        "size": v3(p.bounds.max - p.bounds.min),
                        "footprint": mm(p.footprint),
                        "triangles": p.triangles,
                        "floors": p.floors.iter().map(Floor::to_json).collect::<Vec<_>>(),
                        "surfaces": p.surfaces.iter().map(|s| surface(*s)).collect::<Vec<_>>(),
                    });
                    if p.more_floors > 0
                        && let Some(r) = row.as_object_mut()
                    {
                        r.insert("more_floors".into(), json!(p.more_floors));
                    }
                    row
                })
                .collect();
            m.insert("parts".into(), json!(parts));
            if self.more_parts > 0 {
                m.insert("more_parts".into(), json!(self.more_parts));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_fmt_object::Object as EditObject;

    /// A model of boxes: `(min, max)` each, six faces looking out.
    fn boxes(list: &[(Vec3, Vec3)]) -> LibraryObject {
        let mut o = EditObject::default();
        let mut mesh = gz_fmt_object::Mesh::default();
        for (min, max) in list {
            let base = mesh.points.len() as u32;
            let c = |x: bool, y: bool, z: bool| {
                Vec3::new(
                    if x { max.x } else { min.x },
                    if y { max.y } else { min.y },
                    if z { max.z } else { min.z },
                )
            };
            let mut points: Vec<Vec3> = mesh.points.to_vec();
            for i in 0..8u32 {
                points.push(c(i & 1 != 0, i & 2 != 0, i & 4 != 0));
            }
            mesh.points = points.into();
            // corner bits: x 1, y 2, z 4; every face wound to look out (left-handed)
            let quads: [[u32; 4]; 6] = [
                [2, 6, 7, 3], // +y
                [0, 1, 5, 4], // -y
                [1, 3, 7, 5], // +x
                [0, 4, 6, 2], // -x
                [4, 5, 7, 6], // +z
                [0, 2, 3, 1], // -z
            ];
            let mut faces = mesh.faces.to_vec();
            for q in quads {
                for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                    faces.push(gz_fmt_object::Face {
                        v: tri.map(|p| gz_fmt_object::FaceVert { point: base + p, vmref: 0 }),
                    });
                }
            }
            mesh.faces = faces.into();
        }
        o.meshes.push(mesh);
        LibraryObject::provided("test", o)
    }

    fn up_is_up(o: &LibraryObject) -> bool {
        // the first two triangles of a box are its top
        let t = o.tri(0).unwrap();
        (t[1] - t[0]).cross(t[2] - t[0]).y > 0.0
    }

    #[test]
    fn a_table_has_its_top_and_its_pivot_at_the_base() {
        // a top 1.4 x 1.0 at 0.8 on one thick leg
        let o = boxes(&[
            (Vec3::new(-0.7, 0.75, -0.5), Vec3::new(0.7, 0.8, 0.5)),
            (Vec3::new(-0.1, 0.0, -0.1), Vec3::new(0.1, 0.75, 0.1)),
        ]);
        assert!(up_is_up(&o));
        let f = facts(&o, &[], &Limits::default());
        assert_eq!(f.pivot, Pivot::Base);
        assert_eq!(f.triangles, 24);
        let top = &f.floors[0];
        assert!((top.y - 0.8).abs() < 1e-4 && (top.area - 1.4).abs() < 1e-3, "{top:?}");
        assert_eq!(top.rect, [-0.7, -0.5, 0.7, 0.5]);
        // every face stops a body: nothing says otherwise
        let c = f.collision.unwrap();
        assert_eq!(c.triangles, 24);
        assert!((c.share - 1.0).abs() < 1e-6);
        // a small model is told whole
        assert!(f.parts.is_empty());
        let j = f.to_json(&|_| json!({}));
        assert_eq!(j["pivot"], "base");
        assert_eq!(j["size"], json!([1.4, 0.8, 1.0]));
    }

    #[test]
    fn a_large_model_is_told_in_its_pieces_with_their_floors() {
        // a house 10 x 6 with a roof at 3 and a platform inside at 1.5, and a tower 30 m away;
        // a pebble between them is too small for a line
        let o = boxes(&[
            (Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 3.0, 6.0)),
            (Vec3::new(40.0, 0.0, 0.0), Vec3::new(42.0, 12.0, 2.0)),
            (Vec3::new(20.0, 0.0, 0.0), Vec3::new(20.2, 0.2, 0.2)),
        ]);
        let f = facts(&o, &[], &Limits::default());
        assert_eq!(f.parts.len(), 2);
        assert_eq!(f.more_parts, 1);
        let house = &f.parts[0];
        assert_eq!(house.bounds.max, Vec3::new(10.0, 3.0, 6.0));
        assert!((house.floors[0].y - 3.0).abs() < 1e-4 && (house.floors[0].area - 60.0).abs() < 1e-2);
        let tower = &f.parts[1];
        assert!((tower.floors[0].y - 12.0).abs() < 1e-4);
        assert_eq!(f.pivot, Pivot::Off(Vec3::new(21.0, 6.0, 3.0)));
    }

    #[test]
    fn what_stands_on_the_ground_is_told_apart_from_it() {
        // a ground slab 100 x 100 whose top is at 0, a house and a shed standing in it
        let o = boxes(&[
            (Vec3::new(-50.0, -1.0, -50.0), Vec3::new(50.0, 0.0, 50.0)),
            (Vec3::new(0.0, 0.0, 0.0), Vec3::new(10.0, 3.0, 6.0)),
            (Vec3::new(-30.0, 0.0, -30.0), Vec3::new(-26.0, 2.5, -27.0)),
        ]);
        let limits = Limits { ground_share: 0.05, parts_from: 0.0, ..Limits::default() };
        let f = facts(&o, &[], &limits);
        assert_eq!(f.ground.len(), 1);
        assert!((f.ground[0].area - 10_000.0).abs() < 1.0 && f.ground[0].y.abs() < 1e-4);
        // the slab's sides and bottom are a piece too, with no floor of their own
        let house = f.parts.iter().find(|p| (p.bounds.max.y - 3.0).abs() < 1e-4).expect("the house");
        assert_eq!(house.bounds.min, Vec3::new(0.0, 0.0, 0.0));
        assert!((house.floors[0].y - 3.0).abs() < 1e-4);
        assert!(f.parts.iter().any(|p| (p.bounds.max.y - 2.5).abs() < 1e-4));
        // without the rule a model is told as before
        assert!(facts(&o, &[], &Limits::default()).ground.is_empty());
    }

    #[test]
    fn what_does_not_collide_is_no_floor_and_no_collision() {
        let mut o = boxes(&[(Vec3::new(-1.0, 0.0, -1.0), Vec3::new(1.0, 1.0, 1.0))]);
        // one surface over every face, and it is a ghost
        let mut object = o.object.clone();
        object.surfaces.push(gz_fmt_object::Surface::default());
        let all: Vec<u32> = (0..12).collect();
        object.meshes[0].surf_faces = vec![gz_fmt_object::SurfFaces { surface: 0, faces: all.into() }];
        o = LibraryObject::provided("ghost", object);
        let ghost = [SurfaceUse { draws: true, collides: false }];
        let f = facts(&o, &ghost, &Limits::default());
        assert!(f.collision.is_none());
        assert!(!f.surfaces[0].uses.collides);
        assert_eq!(f.surfaces[0].triangles, 12);
        // the floors of a model nothing of which collides are told of what is drawn
        assert_eq!(f.floors.len(), 1);
        let solid = [SurfaceUse::BOTH];
        assert_eq!(facts(&o, &solid, &Limits::default()).collision.unwrap().triangles, 12);
    }
}
