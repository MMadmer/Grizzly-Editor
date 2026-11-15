//! The level's ground under a terrain: at every grid point and under the middle of every face,
//! the surface the terrain's centre stands on, followed outward through the ground's triangles
//! the caller gathers.

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

/// Every hit of a lattice, by point: the hits of point `i` are `hits[starts[i]..starts[i + 1]]`,
/// in the order of the triangles.
struct Layers {
    starts: Vec<usize>,
    hits: Vec<Hit>,
}

impl Layers {
    fn new(points: usize, mut found: Vec<(usize, Hit)>) -> Self {
        // stable: the hits of a point stay in the triangles' order
        found.sort_by_key(|(i, _)| *i);
        let mut starts = vec![0; points + 1];
        for (i, _) in &found {
            starts[*i + 1] += 1;
        }
        for i in 0..points {
            starts[i + 1] += starts[i];
        }
        Self { starts, hits: found.into_iter().map(|(_, h)| h).collect() }
    }

    /// The hit of point `i` nearest the height `near`; of two as near, the earlier triangle's.
    fn nearest(&self, i: usize, near: f32) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        for h in &self.hits[self.starts[i]..self.starts[i + 1]] {
            if best.is_none_or(|b| (h.height - near).abs() < (b.height - near).abs()) {
                best = Some(*h);
            }
        }
        best
    }
}

/// The ground under a terrain: the surface its centre stands on, followed outward. Of the
/// triangles that face up, the middle of the grid takes the one nearest the terrain's own height
/// and every other point the one nearest the height its neighbour took, so what stands over the
/// ground - a roof, a tree's crown, a bridge - is not the ground, and a terrain written at a
/// bridge's height lies on the bridge. A face takes the triangle nearest its corners' heights.
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
    let mut over_points: Vec<(usize, Hit)> = Vec::new();
    let mut over_faces: Vec<(usize, Hit)> = Vec::new();
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
        raster(&points, q, h, |i, y| over_points.push((i, Hit { height: y, tri: k })));
        raster(&lower, q, h, |i, y| over_faces.push((i * 2, Hit { height: y, tri: k })));
        raster(&upper, q, h, |i, y| over_faces.push((i * 2 + 1, Hit { height: y, tri: k })));
    }
    let over_points = Layers::new(cols * rows, over_points);
    let over_faces = Layers::new((cols - 1) * (rows - 1) * 2, over_faces);

    // outward from the middle, each point by the height the point it was reached from took (a
    // point over nothing hands on the height it was given)
    let mut at_points: Vec<Option<Hit>> = vec![None; cols * rows];
    let mut level: Vec<f32> = vec![t.centre.y; cols * rows];
    let mut seen = vec![false; cols * rows];
    let middle = (rows / 2) * cols + cols / 2;
    let mut queue = VecDeque::from([middle]);
    seen[middle] = true;
    while let Some(i) = queue.pop_front() {
        if let Some(hit) = over_points.nearest(i, level[i]) {
            at_points[i] = Some(hit);
            level[i] = hit.height;
        }
        let (x, z) = (i % cols, i / cols);
        let near = [
            (x > 0).then(|| i - 1),
            (x + 1 < cols).then(|| i + 1),
            (z > 0).then(|| i - cols),
            (z + 1 < rows).then(|| i + cols),
        ];
        for n in near.into_iter().flatten() {
            if !seen[n] {
                seen[n] = true;
                level[n] = level[i];
                queue.push_back(n);
            }
        }
    }
    let mut at_faces: Vec<Option<Hit>> = vec![None; (cols - 1) * (rows - 1) * 2];
    for z in 0..rows - 1 {
        for x in 0..cols - 1 {
            let i = z * cols + x;
            let corners = (level[i] + level[i + 1] + level[i + cols] + level[i + cols + 1]) / 4.0;
            let cell = (z * (cols - 1) + x) * 2;
            at_faces[cell] = over_faces.nearest(cell, corners);
            at_faces[cell + 1] = over_faces.nearest(cell + 1, corners);
        }
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
