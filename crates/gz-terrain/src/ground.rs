//! The level's ground under a terrain: the first hit from above at every grid point and under the
//! middle of every face, from the ground's triangles the caller gathers.

use std::collections::VecDeque;

use glam::{Vec2, Vec3};

use crate::grid::Grid;
use crate::model::Terrain;

/// A triangle of the ground, in the world, with its texture coordinates and the index of its look
/// in the caller's list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundTri {
    pub p: [Vec3; 3],
    pub uv: [Vec2; 3],
    pub look: u32,
}

/// The ground found at a point: its height and the triangle (an index of the caller's list).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub height: f32,
    pub tri: u32,
}

/// The ground under a terrain.
#[derive(Clone, Debug, PartialEq)]
pub struct GroundSample {
    /// Per grid point, row by row from local -z.
    pub points: Vec<Option<Hit>>,
    /// Per face of the grid: two per cell, cell by cell row by row, the lower-left first.
    pub faces: Vec<Option<Hit>>,
}

/// The lattice of points at `(i + ox) * step`, from the rectangle's lowest corner.
struct Lattice {
    origin: Vec2,
    step: Vec2,
    columns: usize,
    rows: usize,
}

fn raster(l: &Lattice, q: [Vec2; 3], heights: [f32; 3], mut hit: impl FnMut(usize, f32)) {
    let (a, b, c) = (q[0], q[1], q[2]);
    let det = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
    if det.abs() < 1e-9 {
        return;
    }
    let lo = a.min(b).min(c);
    let hi = a.max(b).max(c);
    let first = |v: f32, o: f32, s: f32| ((v - o) / s).ceil().max(0.0);
    let last = |v: f32, o: f32, s: f32, n: usize| ((v - o) / s).floor().min(n as f32 - 1.0);
    let (i0, i1) = (first(lo.x, l.origin.x, l.step.x), last(hi.x, l.origin.x, l.step.x, l.columns));
    let (j0, j1) = (first(lo.y, l.origin.y, l.step.y), last(hi.y, l.origin.y, l.step.y, l.rows));
    if i1 < i0 || j1 < j0 {
        return;
    }
    const EDGE: f32 = -1e-6;
    for j in j0 as usize..=j1 as usize {
        for i in i0 as usize..=i1 as usize {
            let p = l.origin + Vec2::new(i as f32 * l.step.x, j as f32 * l.step.y);
            let u = ((p.x - a.x) * (c.y - a.y) - (c.x - a.x) * (p.y - a.y)) / det;
            let v = ((b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y)) / det;
            if u < EDGE || v < EDGE || u + v > 1.0 - EDGE {
                continue;
            }
            hit(
                j * l.columns + i,
                heights[0] + (heights[1] - heights[0]) * u + (heights[2] - heights[0]) * v,
            );
        }
    }
}

/// The ground under a terrain: of the triangles that face up, the highest at every point.
#[must_use]
pub fn sample_ground(t: &Terrain, tris: &[GroundTri]) -> GroundSample {
    let (cols, rows) = t.dims();
    let half = t.half();
    let step = Vec2::new(t.spec.size.x / (cols - 1) as f32, t.spec.size.y / (rows - 1) as f32);
    let corner = -half;
    let points = Lattice { origin: corner, step, columns: cols, rows };
    // the middles of the two faces of a cell: a third and two thirds across it
    let lower = Lattice { origin: corner + step / 3.0, step, columns: cols - 1, rows: rows - 1 };
    let upper = Lattice { origin: corner + step * (2.0 / 3.0), step, columns: cols - 1, rows: rows - 1 };
    let mut at_points: Vec<Option<Hit>> = vec![None; cols * rows];
    let mut at_faces: Vec<Option<Hit>> = vec![None; (cols - 1) * (rows - 1) * 2];
    let keep = |slot: &mut Option<Hit>, height: f32, tri: u32| {
        if slot.is_none_or(|h| height > h.height) {
            *slot = Some(Hit { height, tri });
        }
    };
    for (k, tri) in tris.iter().enumerate() {
        let n = (tri.p[1] - tri.p[0]).cross(tri.p[2] - tri.p[0]);
        if !tri.p.iter().all(|p| p.is_finite()) || n.y <= 0.0 {
            continue;
        }
        let q = tri.p.map(|p| t.to_local(p));
        let lo = q[0].min(q[1]).min(q[2]);
        let hi = q[0].max(q[1]).max(q[2]);
        if hi.x < -half.x || lo.x > half.x || hi.y < -half.y || lo.y > half.y {
            continue;
        }
        let h = tri.p.map(|p| p.y);
        let k = k as u32;
        raster(&points, q, h, |i, y| keep(&mut at_points[i], y, k));
        raster(&lower, q, h, |i, y| keep(&mut at_faces[i * 2], y, k));
        raster(&upper, q, h, |i, y| keep(&mut at_faces[i * 2 + 1], y, k));
    }
    GroundSample { points: at_points, faces: at_faces }
}

/// Fills the gaps of a grid from the nearest neighbours that have a value, breadth first.
pub(crate) fn fill<T: Copy>(values: &mut [Option<T>], columns: usize, rows: usize) {
    let mut queue: VecDeque<usize> = (0..values.len()).filter(|i| values[*i].is_some()).collect();
    while let Some(i) = queue.pop_front() {
        let Some(v) = values[i] else { continue };
        let (x, z) = (i % columns, i / columns);
        let mut near = Vec::with_capacity(4);
        if x > 0 {
            near.push(i - 1);
        }
        if x + 1 < columns {
            near.push(i + 1);
        }
        if z > 0 {
            near.push(i - columns);
        }
        if z + 1 < rows {
            near.push(i + columns);
        }
        for n in near {
            if values[n].is_none() {
                values[n] = Some(v);
                queue.push_back(n);
            }
        }
    }
}

impl GroundSample {
    /// The ground's heights at the grid points: a point over nothing takes the nearest found,
    /// and with no ground at all the centre's height.
    #[must_use]
    pub fn grid(&self, t: &Terrain) -> Grid {
        let (cols, rows) = t.dims();
        let mut h: Vec<Option<f32>> = self.points.iter().map(|p| p.map(|x| x.height)).collect();
        fill(&mut h, cols, rows);
        Grid { columns: cols, rows, heights: h.into_iter().map(|v| v.unwrap_or(t.centre.y)).collect() }
    }

    /// The ground triangle under every face: a face over nothing takes its neighbour's.
    #[must_use]
    pub fn face_tris(&self, t: &Terrain) -> Vec<Option<u32>> {
        let (cols, rows) = t.dims();
        // the two faces of a cell side by side: a grid twice as wide
        let mut f: Vec<Option<u32>> = self.faces.iter().map(|h| h.map(|x| x.tri)).collect();
        fill(&mut f, (cols - 1) * 2, rows - 1);
        f
    }
}
