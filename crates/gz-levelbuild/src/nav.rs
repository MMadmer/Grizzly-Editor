//! The engine's level graph lookups the graph and spawn
//! builders use, over a [`LevelGraph`].

use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::sync::Arc;

use glam::{Vec2, Vec3};
use gz_fmt_level::{AiNode, LevelGraph, LevelGraphHeader, i_floor, pv_decompress};

/// `MAX_NODE_BIT_COUNT` of the node layout the editor writes.
const MAX_NODE_BIT_COUNT: u32 = 24;
/// The straight-line search's default range.
pub const MAX_RANGE: f32 = 6000.0;
pub const INVALID: u32 = u32::MAX;

/// A level graph with its row and column lengths.
#[derive(Clone, Debug)]
pub struct LevelNav {
    pub header: LevelGraphHeader,
    pub nodes: Arc<[AiNode]>,
    row: u32,
    column: u32,
}

impl LevelNav {
    #[must_use]
    pub fn new(graph: &LevelGraph) -> Self {
        let row = graph.header.row_length().max(1);
        let column = graph.header.column_length().max(1);
        Self { header: graph.header, nodes: graph.nodes.clone(), row, column }
    }

    /// The side of an AI map cell.
    #[must_use]
    pub fn cell_size(&self) -> f32 {
        self.header.cell_size
    }

    #[must_use]
    pub fn count(&self) -> u32 {
        self.nodes.len() as u32
    }

    #[must_use]
    pub fn valid_id(&self, id: u32) -> bool {
        id < self.count()
    }

    fn node(&self, id: u32) -> Option<&AiNode> {
        self.nodes.get(id as usize)
    }

    /// The four links of a vertex.
    #[must_use]
    pub fn links(&self, id: u32) -> [u32; 4] {
        self.node(id).map_or([INVALID; 4], |n| n.links)
    }

    /// `vertex_position(u32)`.
    #[must_use]
    pub fn vertex_position(&self, id: u32) -> Vec3 {
        self.node(id).map_or(Vec3::ZERO, |n| self.header.unpack_position(n.xz, n.y))
    }

    fn xz(&self, id: u32) -> u32 {
        self.node(id).map_or(u32::MAX, |n| n.xz)
    }

    /// `unpack_xz` in cells.
    fn unpack_cells(&self, xz: u32) -> (u32, u32) {
        (xz / self.row, xz % self.row)
    }

    /// `unpack_xz` in metres.
    fn unpack_metres(&self, id: u32) -> Vec2 {
        let (x, z) = self.unpack_cells(self.xz(id));
        Vec2::new(
            x as f32 * self.header.cell_size + self.header.aabb.min.x,
            z as f32 * self.header.cell_size + self.header.aabb.min.z,
        )
    }

    fn pack_xz(&self, x: f32, z: f32) -> i32 {
        let h = &self.header;
        i_floor((x - h.aabb.min.x) / h.cell_size + 0.5)
            .wrapping_mul(self.row as i32)
            .wrapping_add(i_floor((z - h.aabb.min.z) / h.cell_size + 0.5))
    }

    /// `valid_vertex_position`.
    #[must_use]
    pub fn valid_position(&self, p: Vec3) -> bool {
        let h = &self.header;
        let half = h.cell_size * 0.5;
        if p.x < h.aabb.min.x - half
            || p.x > h.aabb.max.x + half
            || p.z < h.aabb.min.z - half
            || p.z > h.aabb.max.z + half
        {
            return false;
        }
        if i_floor((p.z - h.aabb.min.z) / h.cell_size + 0.5) >= self.row as i32 {
            return false;
        }
        if i_floor((p.x - h.aabb.min.x) / h.cell_size + 0.5) >= self.column as i32 {
            return false;
        }
        (self.header.pack_position(p).0) < (1 << MAX_NODE_BIT_COUNT) - 1
    }

    /// `vertex_plane_y`: the height of the vertex's plane under `(x, z)`.
    #[must_use]
    pub fn plane_y(&self, id: u32, x: f32, z: f32) -> f32 {
        let Some(n) = self.node(id) else { return 0.0 };
        let normal = pv_decompress(n.plane);
        let p = self.header.unpack_position(n.xz, n.y);
        let d = -normal.dot(p);
        let v = Vec3::new(x, p.y, z);
        // intersectRayPoint along +Y
        let t = -(normal.dot(v) + d) / normal.y;
        v.y + t
    }

    /// `inside(vertex, position)`.
    #[must_use]
    pub fn inside(&self, id: u32, p: Vec3) -> bool {
        self.valid_position(p) && self.node(id).is_some_and(|n| n.xz == self.header.pack_position(p).0)
    }

    fn inside_2d(&self, id: u32, p: Vec2) -> bool {
        self.node(id).is_some_and(|n| n.xz as i32 == self.pack_xz(p.x, p.y))
    }

    /// `vertex_id(position)`: the vertex of the position's cell nearest under it, by binary
    /// search over the cells.
    #[must_use]
    pub fn vertex_id(&self, p: Vec3) -> u32 {
        let want = self.header.pack_position(p).0;
        let start = self.nodes.partition_point(|n| n.xz < want);
        if start >= self.nodes.len() || self.nodes[start].xz != want {
            return INVALID;
        }
        let mut best = start as u32;
        let mut y = self.plane_y(best, p.x, p.z);
        for (i, n) in self.nodes.iter().enumerate().skip(start + 1) {
            if n.xz != want {
                break;
            }
            let id = i as u32;
            let ny = self.plane_y(id, p.x, p.z);
            if y <= p.y {
                if ny <= p.y && p.y - ny < p.y - y {
                    y = ny;
                    best = id;
                }
            } else if ny <= p.y || ny - p.y < y - p.y {
                y = ny;
                best = id;
            }
        }
        best
    }

    /// `contour`: the cell's corners projected onto the vertex's plane.
    fn contour(&self, id: u32) -> [Vec3; 4] {
        let Some(n) = self.node(id) else { return [Vec3::ZERO; 4] };
        let p = self.header.unpack_position(n.xz, n.y);
        let normal = pv_decompress(n.plane);
        let d = -normal.dot(p);
        let st = self.header.cell_size / 2.0;
        let project = |mut v: Vec3| {
            v.y -= (normal.dot(v) + d) / normal.y;
            v
        };
        [
            project(Vec3::new(p.x - st, p.y, p.z - st)),
            project(Vec3::new(p.x + st, p.y, p.z - st)),
            project(Vec3::new(p.x + st, p.y, p.z + st)),
            project(Vec3::new(p.x - st, p.y, p.z + st)),
        ]
    }

    /// The nearest point of a segment and its squared distance.
    fn nearest_seg(p: Vec3, a: Vec3, b: Vec3) -> (Vec3, f32) {
        let c = p - a;
        let v = b - a;
        let d = v.length();
        let v = v / d;
        let t = v.dot(c);
        if t <= 0.0 {
            return (a, p.distance_squared(a));
        }
        if t >= d {
            return (b, p.distance_squared(b));
        }
        let r = a + v * t;
        (r, p.distance_squared(r))
    }

    /// `nearest(dest, position, contour)`.
    fn nearest(&self, p: Vec3, id: u32) -> (Vec3, f32) {
        let c = self.contour(id);
        let mut best = Self::nearest_seg(p, c[0], c[1]);
        for (a, b) in [(c[1], c[2]), (c[2], c[3]), (c[3], c[0])] {
            let t = Self::nearest_seg(p, a, b);
            if t.1 < best.1 {
                best = t;
            }
        }
        best
    }

    /// `vertex(position)`: the vertex whose contour is nearest, by a full search.
    #[must_use]
    pub fn vertex_full(&self, p: Vec3) -> u32 {
        let mut best = (f32::MAX, INVALID);
        for id in 0..self.count() {
            let d = self.nearest(p, id).1;
            if d < best.0 {
                best = (d, id);
            }
        }
        best.1
    }

    /// `vertex(current, position)`.
    #[must_use]
    pub fn vertex(&self, current: u32, p: Vec3) -> u32 {
        if self.valid_position(p) {
            if self.valid_id(current) && self.inside(current, p) {
                return current;
            }
            let id = self.vertex_id(p);
            if self.valid_id(id) {
                let mut ok = true;
                if self.valid_id(current) {
                    if self.links(current).contains(&id) || self.links(id).contains(&current) {
                        return id;
                    }
                    let y0 = self.plane_y(current, p.x, p.z);
                    let y1 = self.plane_y(id, p.x, p.z);
                    let (d0, d1) = (p.y - y0, p.y - y1);
                    if p.y > y0 {
                        ok = if p.y > y1 { d1 - d0 <= 1.0 } else { d0 - d1 <= 1.0 };
                    }
                }
                if ok {
                    return id;
                }
            }
        }
        if !self.valid_id(current) {
            return self.vertex_full(p);
        }
        let guessed = self.guess_vertex_id(current, p);
        if guessed != current {
            return guessed;
        }
        let mut best = (self.nearest(p, current).1, current);
        for l in self.links(current) {
            if !self.valid_id(l) {
                continue;
            }
            let d = self.nearest(p, l).1;
            if best.0 > d {
                best = (d, l);
            }
        }
        best.1
    }

    /// `guess_vertex_id`: the nearest vertex within four cells.
    fn guess_vertex_id(&self, current: u32, p: Vec3) -> u32 {
        let xz = if self.valid_position(p) { self.header.pack_position(p).0 } else { self.xz(current) };
        let (x, z) = self.unpack_cells(xz);
        let (mut result_distance, mut result) = (self.nearest(p, current).1, current);
        let max_x = self.column.saturating_sub(1);
        let max_z = self.row.saturating_sub(1);
        let (sx, ex) = (x.saturating_sub(4), (x + 4).min(max_x));
        let (sz, ez) = (z.saturating_sub(4), (z + 4).min(max_z));
        for i in sx..=ex {
            for j in sz..=ez {
                let test = i * self.row + j;
                let start = self.nodes.partition_point(|n| n.xz < test);
                if start >= self.nodes.len() || self.nodes[start].xz != test {
                    continue;
                }
                let mut best_id = start as u32;
                let (mut best_point, mut best_distance) = self.nearest(p, best_id);
                for (k, n) in self.nodes.iter().enumerate().skip(start + 1) {
                    if n.xz != test {
                        break;
                    }
                    let (pt, d) = self.nearest(p, k as u32);
                    if d >= best_distance {
                        continue;
                    }
                    best_point = pt;
                    best_distance = d;
                    best_id = k as u32;
                }
                if (best_point.y - p.y).abs() >= 3.0 || result_distance <= best_distance {
                    continue;
                }
                result_distance = best_distance;
                result = best_id;
            }
        }
        result
    }

    /// `check_position_in_direction`: the vertex a straight walk from `start` (at `from`)
    /// reaches `to` on, `INVALID` when the walk leaves the graph.
    #[must_use]
    pub fn check_position_in_direction(&self, start: u32, from: Vec3, to: Vec3) -> u32 {
        let (s, d) = (Vec2::new(from.x, from.z), Vec2::new(to.x, to.z));
        if self.inside_2d(start, d) {
            return start;
        }
        if !self.valid_position(Vec3::new(d.x, 0.0, d.y)) {
            return INVALID;
        }
        let half = Vec2::splat(self.header.cell_size * 0.5);
        let dir = d - s;
        let dest_xz = self.header.pack_position(Vec3::new(d.x, 0.0, d.y)).0;
        let (mut cur, mut prev) = (start, INVALID);
        let t = self.unpack_metres(start);
        let mut cur_sqr = (t - d).length_squared();
        loop {
            let mut found = false;
            for next in self.links(cur) {
                if next == prev || !self.valid_id(next) {
                    continue;
                }
                let c = self.unpack_metres(next);
                if pick_exact(c - half, c + half, s, dir) {
                    if dest_xz == self.xz(next) {
                        return next;
                    }
                    let dist = (c - d).length_squared();
                    if dist > cur_sqr {
                        continue;
                    }
                    cur_sqr = dist;
                    found = true;
                    prev = cur;
                    cur = next;
                    break;
                }
            }
            if !found {
                return INVALID;
            }
        }
    }

    /// The A* over the level graph: a step costs a cell, the estimate is
    /// twice the Manhattan distance in cells. The path from `start` to `goal`, both included.
    fn search(&self, start: u32, goal: u32) -> Option<Vec<u32>> {
        let cell = self.header.cell_size;
        let (gx, gz) = self.unpack_cells(self.xz(goal));
        let estimate = |id: u32| {
            let (x, z) = self.unpack_cells(self.xz(id));
            2.0 * cell * (x.abs_diff(gx) + z.abs_diff(gz)) as f32
        };
        let n = self.nodes.len();
        let mut g = vec![f32::MAX; n];
        let mut parent = vec![INVALID; n];
        let mut closed = vec![false; n];
        let mut open = BinaryHeap::new();
        *g.get_mut(start as usize)? = 0.0;
        open.push(Open { f: estimate(start), id: start });
        while let Some(Open { f, id }) = open.pop() {
            if f >= MAX_RANGE {
                return None;
            }
            if closed[id as usize] {
                continue;
            }
            closed[id as usize] = true;
            if id == goal {
                let mut path = vec![goal];
                let mut c = goal;
                while c != start {
                    c = parent[c as usize];
                    if c == INVALID {
                        return None;
                    }
                    path.push(c);
                }
                path.reverse();
                return Some(path);
            }
            for next in self.links(id) {
                if !self.valid_id(next) || closed[next as usize] {
                    continue;
                }
                let ng = g[id as usize] + cell;
                if ng < g[next as usize] {
                    g[next as usize] = ng;
                    parent[next as usize] = id;
                    open.push(Open { f: ng + estimate(next), id: next });
                }
            }
        }
        None
    }

    /// The length of the A* path from `v0` (at `p0`) to `v1`
    /// (at `p1`), its corners cut where a straight walk stays on the graph. `None` when there
    /// is no path.
    #[must_use]
    pub fn path_distance(&self, v0: u32, p0: Vec3, v1: u32, p1: Vec3) -> Option<f32> {
        let path = self.search(v0, v1)?;
        let (mut cumulative, mut last_direct) = (0.0f32, 0.0f32);
        let mut pos = p0;
        let mut node_idx = 0usize;
        let mut i = 1usize;
        while i < path.len() {
            let node = path[node_idx];
            let target = self.vertex_position(path[i]);
            let direct = if self.valid_id(self.check_position_in_direction(node, pos, target)) {
                pos.distance(target)
            } else {
                MAX_RANGE
            };
            if direct == MAX_RANGE {
                if last_direct == 0.0 {
                    cumulative += self.vertex_position(node).distance(target);
                    node_idx = i;
                } else {
                    cumulative += last_direct;
                    last_direct = 0.0;
                    i -= 1;
                    node_idx = i;
                }
                pos = self.vertex_position(path[node_idx]);
            } else {
                last_direct = direct;
            }
            if cumulative + last_direct >= MAX_RANGE {
                return Some(MAX_RANGE);
            }
            i += 1;
        }
        let node = path[node_idx];
        let direct = if self.valid_id(self.check_position_in_direction(node, pos, p1)) {
            pos.distance(p1)
        } else {
            MAX_RANGE
        };
        Some(if direct == MAX_RANGE {
            cumulative + last_direct + p1.distance(self.vertex_position(path[path.len() - 1]))
        } else {
            cumulative + direct
        })
    }

    /// Every vertex a walk of less than `radius` reaches from `start` within the
    /// circle of `radius` cells, in the order the walk meets them.
    #[must_use]
    pub fn flood(&self, start: u32, radius: f32) -> Vec<u32> {
        if !self.valid_id(start) {
            return Vec::new();
        }
        let cell = self.header.cell_size;
        let max_sqr = i_floor(radius * radius / (cell * cell) + 0.5).max(0) as u64;
        let (x0, z0) = self.unpack_cells(self.xz(start));
        let accessible = |id: u32| {
            let (x, z) = self.unpack_cells(self.xz(id));
            let (dx, dz) = (u64::from(x.abs_diff(x0)), u64::from(z.abs_diff(z0)));
            dx * dx + dz * dz <= max_sqr
        };
        let mut seen = vec![false; self.nodes.len()];
        let mut out = Vec::new();
        let mut queue = VecDeque::new();
        seen[start as usize] = true;
        queue.push_back((start, 0.0f32));
        while let Some((id, g)) = queue.pop_front() {
            if g >= radius {
                break;
            }
            out.push(id);
            for next in self.links(id) {
                if self.valid_id(next) && !seen[next as usize] && accessible(next) {
                    seen[next as usize] = true;
                    queue.push_back((next, g + cell));
                }
            }
        }
        out
    }
}

/// The ray from `start` along `dir` meets the box.
fn pick_exact(min: Vec2, max: Vec2, start: Vec2, dir: Vec2) -> bool {
    let (mut t0, mut t1) = (0.0f32, f32::MAX);
    for a in 0..2 {
        if dir[a].abs() < 1e-20 {
            if start[a] < min[a] || start[a] > max[a] {
                return false;
            }
            continue;
        }
        let inv = 1.0 / dir[a];
        let (mut near, mut far) = ((min[a] - start[a]) * inv, (max[a] - start[a]) * inv);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return false;
        }
    }
    true
}

#[derive(Clone, Copy, Debug)]
struct Open {
    f: f32,
    id: u32,
}

impl PartialEq for Open {
    fn eq(&self, other: &Self) -> bool {
        self.f == other.f && self.id == other.id
    }
}

impl Eq for Open {}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Open {
    /// The smallest `f` first; equal ones by id, so a search is reproducible.
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.total_cmp(&self.f).then_with(|| other.id.cmp(&self.id))
    }
}
