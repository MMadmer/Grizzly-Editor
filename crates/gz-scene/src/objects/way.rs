//! Way paths, sectors and portals.

use gz_base::SmolStr;

use crate::math::{Aabb, Vec3};

/// `WAYPOINT_SIZE` and its half.
pub const WAYPOINT_SIZE: f32 = 1.5;
pub const WAYPOINT_RADIUS: f32 = WAYPOINT_SIZE * 0.5;

#[derive(Clone, Debug, PartialEq)]
pub struct WayPoint {
    pub name: SmolStr,
    pub position: Vec3,
    pub flags: u32,
    pub selected: bool,
    /// The points it links to, by index, with the link's probability.
    pub links: Vec<(u32, f32)>,
}

impl WayPoint {
    #[must_use]
    pub fn new(name: &str, position: Vec3) -> Self {
        Self { name: SmolStr::new(name), position, flags: 0, selected: false, links: Vec::new() }
    }

    /// The box of one point.
    #[must_use]
    pub fn bounds(&self) -> Aabb {
        let p = self.position;
        Aabb::new(
            Vec3::new(p.x - WAYPOINT_RADIUS, p.y, p.z - WAYPOINT_RADIUS),
            Vec3::new(p.x + WAYPOINT_RADIUS, p.y + WAYPOINT_SIZE, p.z + WAYPOINT_RADIUS),
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WayData {
    /// The way kinds: patrol path 0.
    pub kind: u32,
    pub points: Vec<WayPoint>,
}

impl Default for WayData {
    /// One point named `wp`.
    fn default() -> Self {
        let mut p = WayPoint::new("wp", Vec3::ZERO);
        p.selected = true;
        Self { kind: 0, points: vec![p] }
    }
}

impl WayData {
    /// A point never links to itself.
    pub fn create_link(&mut self, from: usize, to: usize, prob: f32) {
        if from != to
            && to < self.points.len()
            && let Some(p) = self.points.get_mut(from)
        {
            p.links.push((u32::try_from(to).unwrap_or(u32::MAX), prob));
        }
    }

    /// Appends a link: only once.
    pub fn append_link(&mut self, from: usize, to: usize, prob: f32) -> bool {
        let to32 = u32::try_from(to).unwrap_or(u32::MAX);
        if self.points.get(from).is_some_and(|p| p.links.iter().any(|(t, _)| *t == to32)) {
            return false;
        }
        self.create_link(from, to, prob);
        true
    }

    /// The name of a new point: `wp`, `wp_00`, `wp_01` ...
    #[must_use]
    pub fn next_point_name(&self) -> String {
        for i in 0u32.. {
            let n = if i == 0 { "wp".to_owned() } else { format!("wp_{:02}", i - 1) };
            if !self.points.iter().any(|p| p.name == n) {
                return n;
            }
        }
        String::new()
    }

    /// The points, each linked to the next, the last back to the first when
    /// `looped` and there are more than two.
    #[must_use]
    pub fn build(positions: &[Vec3], looped: bool) -> Self {
        let mut w = Self { kind: 0, points: Vec::with_capacity(positions.len()) };
        for p in positions {
            let name = w.next_point_name();
            for q in &mut w.points {
                q.selected = false;
            }
            let mut wp = WayPoint::new(&name, *p);
            wp.selected = true;
            w.points.push(wp);
        }
        for i in 0..w.points.len().saturating_sub(1) {
            w.append_link(i, i + 1, 1.0);
        }
        if looped && w.points.len() > 2 {
            let last = w.points.len() - 1;
            w.append_link(last, 0, 1.0);
        }
        w
    }

    /// The way's box.
    #[must_use]
    pub fn bounds(&self) -> Aabb {
        let mut b = Aabb::INVALID;
        for p in &self.points {
            b.modify(p.position);
        }
        b.max.x += WAYPOINT_RADIUS;
        b.max.z += WAYPOINT_RADIUS;
        b.max.y += WAYPOINT_SIZE;
        b.min.x -= WAYPOINT_RADIUS;
        b.min.z -= WAYPOINT_RADIUS;
        b
    }

    /// `MoveTo` (not in point mode): every point moves with the first.
    pub fn move_to(&mut self, pos: Vec3) {
        if let Some(first) = self.points.first().map(|p| p.position) {
            let diff = pos - first;
            for p in &mut self.points {
                p.position += diff;
            }
        }
    }

    /// The nearest point's box.
    #[must_use]
    pub fn ray_pick(&self, start: Vec3, dir: Vec3, distance: f32) -> Option<f32> {
        let mut best = distance;
        for p in &self.points {
            if let Some((t, false)) = crate::math::ray_box(start, dir, &p.bounds())
                && t < best
            {
                best = t;
            }
        }
        (best < distance).then_some(best)
    }
}

/// Sets a colour from a `u32`.
#[must_use]
pub fn fcolor_from(argb: u32) -> [f32; 4] {
    let f = |s: u32| ((argb >> s) & 0xff) as f32 / 255.0;
    [f(16), f(8), f(0), f(24)]
}

/// Gets the colour as a `u32`.
#[must_use]
pub fn fcolor_get(c: [f32; 4]) -> u32 {
    let q = |v: f32| ((v * 255.0).floor() as i32).clamp(0, 255) as u32;
    (q(c[3]) << 24) | (q(c[0]) << 16) | (q(c[1]) << 8) | q(c[2])
}

#[derive(Clone, Debug, PartialEq)]
pub struct SectorData {
    pub color: [f32; 4],
    pub default: bool,
    /// The scene object and mesh of every item.
    pub items: Vec<(SmolStr, SmolStr)>,
    /// The index of the path's map: 255 for the default map.
    pub map_idx: u8,
}

impl Default for SectorData {
    fn default() -> Self {
        Self { color: [1.0, 1.0, 1.0, 0.0], default: false, items: Vec::new(), map_idx: u8::MAX }
    }
}

impl SectorData {}

/// A polygon between two sectors, in world coordinates.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PortalData {
    pub front: SmolStr,
    pub back: SmolStr,
    pub vertices: Vec<Vec3>,
}

impl PortalData {
    #[must_use]
    pub fn bounds(&self) -> Option<Aabb> {
        (!self.vertices.is_empty()).then(|| Aabb::from_points(self.vertices.iter()))
    }
}
