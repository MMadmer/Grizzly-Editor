//! The height grid and the evaluation of the edits.

use glam::{Vec2, Vec3};

use crate::model::{Base, Edit, Terrain};

/// Heights over a terrain's grid: row by row from local -z, each row from local -x.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    pub columns: usize,
    pub rows: usize,
    pub heights: Vec<f32>,
}

impl Grid {
    #[must_use]
    pub fn flat(columns: usize, rows: usize, y: f32) -> Self {
        Self { columns, rows, heights: vec![y; columns * rows] }
    }

    /// The height of the point (i, j).
    #[must_use]
    pub fn at(&self, i: usize, j: usize) -> f32 {
        self.heights.get(j * self.columns + i).copied().unwrap_or(0.0)
    }

    fn set(&mut self, i: usize, j: usize, v: f32) {
        if let Some(h) = self.heights.get_mut(j * self.columns + i) {
            *h = v;
        }
    }

    /// The rows as lists: what `rows` of the file writes.
    #[must_use]
    pub fn as_rows(&self) -> Vec<Vec<f32>> {
        self.heights.chunks(self.columns.max(1)).map(<[f32]>::to_vec).collect()
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much an edit acts at a point `d` metres outside its core: all of it inside, nothing past
/// the soft border of `falloff` metres, a smooth step between.
#[must_use]
pub fn weight(d: f32, falloff: f32) -> f32 {
    if d <= 0.0 {
        1.0
    } else if falloff <= 0.0 || d >= falloff {
        0.0
    } else {
        1.0 - smoothstep(d / falloff)
    }
}

/// How far a point lies outside a rectangle (0 inside).
fn outside_rect(p: Vec2, min: Vec2, max: Vec2) -> f32 {
    let dx = (min.x - p.x).max(0.0).max(p.x - max.x);
    let dz = (min.y - p.y).max(0.0).max(p.y - max.y);
    (dx * dx + dz * dz).sqrt()
}

/// A number of `[-1, 1]` for a lattice point: an integer hash, the same on every machine.
fn lattice(x: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (z as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h >> 8) as f32 / ((1u32 << 24) - 1) as f32 * 2.0 - 1.0
}

/// Value noise of `[-1, 1]`: the lattice smoothly between its points.
fn noise(p: Vec2, seed: u32) -> f32 {
    let (fx, fz) = (p.x.floor(), p.y.floor());
    let (x0, z0) = (fx as i32, fz as i32);
    let (tx, tz) = (smoothstep(p.x - fx), smoothstep(p.y - fz));
    let a = lattice(x0, z0, seed) + (lattice(x0 + 1, z0, seed) - lattice(x0, z0, seed)) * tx;
    let b = lattice(x0, z0 + 1, seed) + (lattice(x0 + 1, z0 + 1, seed) - lattice(x0, z0 + 1, seed)) * tx;
    a + (b - a) * tz
}

impl Terrain {
    /// The terrain's surface over a world point, as its mesh has it (each cell's two faces): the
    /// height and the face's normal, of unit length and pointing up. `None` beside the rectangle.
    #[must_use]
    pub fn surface_at(&self, grid: &Grid, world: Vec3) -> Option<(f32, Vec3)> {
        if !self.covers(world) || grid.columns < 2 || grid.rows < 2 {
            return None;
        }
        let l = self.to_local(world);
        let h = self.half();
        let u = (l.x + h.x) / self.spec.size.x * (grid.columns - 1) as f32;
        let v = (l.y + h.y) / self.spec.size.y * (grid.rows - 1) as f32;
        let i = (u.floor().max(0.0) as usize).min(grid.columns - 2);
        let j = (v.floor().max(0.0) as usize).min(grid.rows - 2);
        let (fu, fv) = (u - i as f32, v - j as f32);
        let p = |a: usize, b: usize| self.to_world(self.point(a, b), grid.at(a, b));
        let (p00, p10, p01, p11) = (p(i, j), p(i + 1, j), p(i, j + 1), p(i + 1, j + 1));
        let (y, tri) = if fu + fv <= 1.0 {
            (p00.y + (p10.y - p00.y) * fu + (p01.y - p00.y) * fv, [p00, p01, p10])
        } else {
            (p11.y + (p01.y - p11.y) * (1.0 - fu) + (p10.y - p11.y) * (1.0 - fv), [p10, p01, p11])
        };
        let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or(Vec3::Y);
        Some((y, n))
    }

    /// The heights of the grid. `ground` is the level's ground at every grid point, the caller's
    /// sample (needed with `base = "ground"`, ignored with a number). With rows they are the
    /// heights; otherwise the base, the edits in order and, over the ground, the blend back to it
    /// within `edge` of the border, so the border ends at the ground's heights.
    pub fn heights(&self, ground: Option<&Grid>) -> Result<Grid, String> {
        self.check()?;
        let (cols, rows) = self.dims();
        if !self.spec.rows.is_empty() {
            return Ok(Grid {
                columns: cols,
                rows,
                heights: self.spec.rows.iter().flatten().copied().collect(),
            });
        }
        let base = match self.spec.base {
            Base::Height(h) => Grid::flat(cols, rows, self.centre.y + h),
            Base::Ground => match ground {
                Some(g) if g.columns == cols && g.rows == rows => g.clone(),
                Some(g) => {
                    return Err(format!(
                        "the ground was sampled on {} x {} points, the terrain has {cols} x {rows}",
                        g.columns, g.rows
                    ));
                }
                None => return Err("a terrain on the ground needs the ground's heights".into()),
            },
        };
        let mut g = base.clone();
        for e in &self.spec.edits {
            self.apply(e, &mut g);
        }
        if self.spec.base == Base::Ground && self.spec.edge > 0.0 {
            let h = self.half();
            for j in 0..rows {
                for i in 0..cols {
                    let p = self.point(i, j);
                    let border = (h.x - p.x.abs()).min(h.y - p.y.abs()).max(0.0);
                    if border < self.spec.edge {
                        let s = smoothstep(border / self.spec.edge);
                        let b = base.at(i, j);
                        g.set(i, j, b + (g.at(i, j) - b) * s);
                    }
                }
            }
        }
        Ok(g)
    }

    fn apply(&self, e: &Edit, g: &mut Grid) {
        let (cols, rows) = (g.columns, g.rows);
        let towards = |h: f32, target: f32, w: f32| h + (target - h) * w;
        match e {
            Edit::Smooth { at, radius, falloff, strength } => {
                let before = g.clone();
                for j in 0..rows {
                    for i in 0..cols {
                        let w = weight(self.point(i, j).distance(*at) - radius, *falloff) * strength;
                        if w <= 0.0 {
                            continue;
                        }
                        let mut sum = 0.0;
                        let mut n = 0.0;
                        for dj in -1i32..=1 {
                            for di in -1i32..=1 {
                                let (x, z) = (i as i32 + di, j as i32 + dj);
                                if x >= 0 && z >= 0 && (x as usize) < cols && (z as usize) < rows {
                                    sum += before.at(x as usize, z as usize);
                                    n += 1.0;
                                }
                            }
                        }
                        g.set(i, j, towards(before.at(i, j), sum / n, w));
                    }
                }
            }
            _ => {
                for j in 0..rows {
                    for i in 0..cols {
                        let p = self.point(i, j);
                        let h = g.at(i, j);
                        let v = match e {
                            Edit::Flatten { at, radius, falloff, height } => {
                                towards(h, *height, weight(p.distance(*at) - radius, *falloff))
                            }
                            Edit::Rect { min, max, falloff, height } => {
                                towards(h, *height, weight(outside_rect(p, *min, *max), *falloff))
                            }
                            Edit::Raise { at, radius, falloff, amount } => {
                                h + amount * weight(p.distance(*at) - radius, *falloff)
                            }
                            Edit::Ramp { from, to, width, falloff, heights } => {
                                let axis = *to - *from;
                                let len = axis.length();
                                let dir = axis / len;
                                let d = p - *from;
                                let t = d.dot(dir) / len;
                                let side = (d.x * dir.y - d.y * dir.x).abs() - width * 0.5;
                                let along = if t < 0.0 {
                                    -t * len
                                } else if t > 1.0 {
                                    (t - 1.0) * len
                                } else {
                                    0.0
                                };
                                let out = (side.max(0.0).powi(2) + along * along).sqrt();
                                let target = heights[0] + (heights[1] - heights[0]) * t.clamp(0.0, 1.0);
                                towards(h, target, weight(out, *falloff))
                            }
                            Edit::Noise { amount, scale, seed } => h + amount * noise(p / *scale, *seed),
                            Edit::Smooth { .. } => h,
                        };
                        g.set(i, j, v);
                    }
                }
            }
        }
    }
}
