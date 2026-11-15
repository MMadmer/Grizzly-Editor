//! What a terrain's text says, and the checks of its values.

use glam::{Vec2, Vec3};

use crate::clip::{CutKind, Region};
use crate::grid::Grid;

/// The grid step, metres.
pub const CELL_MIN: f32 = 0.5;
pub const CELL_MAX: f32 = 4.0;
/// The most points a side of the grid has.
pub const MAX_POINTS: usize = 513;
/// The longest side, metres.
pub const SIZE_MAX: f32 = 2048.0;
/// The widest blend back to the ground, metres.
pub const EDGE_MAX: f32 = 1024.0;
/// The skirt round the border: how far out it goes and how far down.
pub const SKIRT_WIDTH: f32 = 0.3;
pub const SKIRT_DROP: f32 = 0.2;
/// How near the old ground a triangle of the cut ground lies to be removed, metres.
pub const TOLERANCE: f32 = 0.5;

/// Where every point of the grid starts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Base {
    /// The level's ground under it.
    Ground,
    /// Flat, this many metres over the centre's height.
    Height(f32),
}

/// One edit of the heights. `at`, `min`, `max`, `from`, `to` are metres from the centre along the
/// terrain's own axes; `height` and `heights` are world heights, `amount` is relative. `falloff`
/// is the width of the soft border.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    Flatten { at: Vec2, radius: f32, falloff: f32, height: f32 },
    Rect { min: Vec2, max: Vec2, falloff: f32, height: f32 },
    Raise { at: Vec2, radius: f32, falloff: f32, amount: f32 },
    Ramp { from: Vec2, to: Vec2, width: f32, falloff: f32, heights: [f32; 2] },
    Smooth { at: Vec2, radius: f32, falloff: f32, strength: f32 },
    Noise { amount: f32, scale: f32, seed: u32 },
}

impl Edit {
    /// The `op` of the file.
    #[must_use]
    pub fn op(&self) -> &'static str {
        match self {
            Self::Flatten { .. } => "flatten",
            Self::Rect { .. } => "rect",
            Self::Raise { .. } => "raise",
            Self::Ramp { .. } => "ramp",
            Self::Smooth { .. } => "smooth",
            Self::Noise { .. } => "noise",
        }
    }

    /// Every `op` there is.
    pub const OPS: &'static [&'static str] = &["flatten", "rect", "raise", "ramp", "smooth", "noise"];

    /// What is wrong with its values, in a sentence.
    pub fn check(&self) -> Result<(), String> {
        let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
        let not_negative = |what: &str, v: f32| {
            if v >= 0.0 { Ok(()) } else { Err(format!("its {what} is below 0")) }
        };
        let ok = match self {
            Self::Flatten { at, radius, falloff, height }
            | Self::Raise { at, radius, falloff, amount: height } => {
                finite(&[at.x, at.y, *radius, *falloff, *height])
                    .then_some(())
                    .ok_or_else(|| "takes finite numbers".to_owned())
                    .and_then(|()| not_negative("radius", *radius))
                    .and_then(|()| not_negative("falloff", *falloff))
            }
            Self::Rect { min, max, falloff, height } => {
                if !finite(&[min.x, min.y, max.x, max.y, *falloff, *height]) {
                    Err("takes finite numbers".to_owned())
                } else if max.x < min.x || max.y < min.y {
                    Err("its max is below its min".to_owned())
                } else {
                    not_negative("falloff", *falloff)
                }
            }
            Self::Ramp { from, to, width, falloff, heights } => {
                if !finite(&[from.x, from.y, to.x, to.y, *width, *falloff, heights[0], heights[1]]) {
                    Err("takes finite numbers".to_owned())
                } else if from.distance(*to) < 1e-3 {
                    Err("its from and to are the same point".to_owned())
                } else {
                    not_negative("width", *width).and_then(|()| not_negative("falloff", *falloff))
                }
            }
            Self::Smooth { at, radius, falloff, strength } => {
                if !finite(&[at.x, at.y, *radius, *falloff, *strength]) {
                    Err("takes finite numbers".to_owned())
                } else if !(0.0..=1.0).contains(strength) {
                    Err("its strength is 0 to 1".to_owned())
                } else {
                    not_negative("radius", *radius).and_then(|()| not_negative("falloff", *falloff))
                }
            }
            Self::Noise { amount, scale, .. } => {
                if !finite(&[*amount, *scale]) {
                    Err("takes finite numbers".to_owned())
                } else if *scale <= 0.0 {
                    Err("its scale is above 0 metres".to_owned())
                } else {
                    Ok(())
                }
            }
        };
        ok.map_err(|why| format!("a {} edit {why}", self.op()))
    }
}

/// A surface of its own: laid flat every `tile` metres.
#[derive(Clone, Debug, PartialEq)]
pub struct Look {
    pub texture: String,
    /// The engine shader.
    pub shader: String,
    /// The compiler shader.
    pub compile: String,
    /// The game material.
    pub material: String,
    pub tile: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self {
            texture: String::new(),
            shader: "default".into(),
            compile: "default".into(),
            material: "materials\\earth".into(),
            tile: 4.0,
        }
    }
}

/// What the faces wear.
#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    /// The surface and the texture coordinates of the ground under each face.
    Ground,
    Look(Look),
}

/// What the file says beside the transform.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    /// Metres along its own x and z.
    pub size: Vec2,
    /// The grid step, metres.
    pub cell: f32,
    pub base: Base,
    /// With the ground as the base: metres along the border over which it meets the ground.
    pub edge: f32,
    pub edits: Vec<Edit>,
    /// Instead of the edits: every height written out, rows from -z to +z, each from -x.
    pub rows: Vec<Vec<f32>>,
    pub surface: Surface,
    /// On a level of the game the ground under it is cut out of the level.
    pub cut: bool,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            size: Vec2::new(20.0, 20.0),
            cell: 1.0,
            base: Base::Ground,
            edge: 4.0,
            edits: Vec::new(),
            rows: Vec::new(),
            surface: Surface::Ground,
            cut: false,
        }
    }
}

/// A terrain where it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Terrain {
    /// The centre; its `y` is what a numeric base counts from.
    pub centre: Vec3,
    /// The heading, radians, as the editor's transform turns about the vertical.
    pub heading: f32,
    pub spec: Spec,
}

impl Terrain {
    /// Half the size.
    #[must_use]
    pub fn half(&self) -> Vec2 {
        self.spec.size * 0.5
    }

    /// The points of the grid along x and z.
    #[must_use]
    pub fn dims(&self) -> (usize, usize) {
        let n = |len: f32| ((len / self.spec.cell).round().max(1.0) as usize).saturating_add(1);
        (n(self.spec.size.x), n(self.spec.size.y))
    }

    /// What is wrong with the values, in a sentence; `Ok` when the terrain can be made.
    pub fn check(&self) -> Result<(), String> {
        let s = &self.spec;
        if !self.centre.is_finite() || !self.heading.is_finite() {
            return Err("the position and the rotation take finite numbers".into());
        }
        if !(s.size.x.is_finite() && s.size.y.is_finite()) || s.size.x <= 0.0 || s.size.y <= 0.0 {
            return Err("the size takes two lengths above 0 metres".into());
        }
        if s.size.x > SIZE_MAX || s.size.y > SIZE_MAX {
            return Err(format!("a side is at most {SIZE_MAX} m long"));
        }
        if !(CELL_MIN..=CELL_MAX).contains(&s.cell) {
            return Err(format!("the cell takes {CELL_MIN} to {CELL_MAX} metres"));
        }
        let (cols, rows) = self.dims();
        if cols > MAX_POINTS || rows > MAX_POINTS {
            let need = s.size.x.max(s.size.y) / (MAX_POINTS - 1) as f32;
            return Err(format!(
                "a {} x {} m terrain at a cell of {} m has {cols} x {rows} points, more than {MAX_POINTS} on a side: a cell of {:.2} m or a smaller size fits",
                s.size.x, s.size.y, s.cell, need
            ));
        }
        if let Base::Height(h) = s.base
            && !h.is_finite()
        {
            return Err("the base takes \"ground\" or a finite number".into());
        }
        if !s.edge.is_finite() || !(0.0..=EDGE_MAX).contains(&s.edge) {
            return Err(format!("the edge takes 0 to {EDGE_MAX} metres"));
        }
        if let Surface::Look(l) = &s.surface
            && !(l.tile.is_finite() && l.tile > 0.0)
        {
            return Err("the surface's tile takes metres above 0".into());
        }
        if !s.rows.is_empty() {
            if s.rows.len() != rows || s.rows.iter().any(|r| r.len() != cols) {
                return Err(format!(
                    "rows holds {} rows of {} heights; this terrain has {rows} rows of {cols}",
                    s.rows.len(),
                    s.rows.first().map_or(0, Vec::len)
                ));
            }
            if s.rows.iter().flatten().any(|h| !h.is_finite()) {
                return Err("rows takes finite heights".into());
            }
        }
        for e in &s.edits {
            e.check()?;
        }
        Ok(())
    }

    /// The local x and z of the grid point (i, j): from -half to +half.
    #[must_use]
    pub fn point(&self, i: usize, j: usize) -> Vec2 {
        let (cols, rows) = self.dims();
        let h = self.half();
        let t = |k: usize, n: usize| if n < 2 { 0.0 } else { k as f32 / (n - 1) as f32 };
        Vec2::new(-h.x + t(i, cols) * 2.0 * h.x, -h.y + t(j, rows) * 2.0 * h.y)
    }

    /// A local point in the world, at a height.
    #[must_use]
    pub fn to_world(&self, local: Vec2, y: f32) -> Vec3 {
        // the editor's heading turns x to (cos, 0, sin) and z to (-sin, 0, cos)
        let (s, c) = self.heading.sin_cos();
        Vec3::new(self.centre.x + local.x * c - local.y * s, y, self.centre.z + local.x * s + local.y * c)
    }

    /// A world point along the terrain's axes, from its centre.
    #[must_use]
    pub fn to_local(&self, world: Vec3) -> Vec2 {
        let (s, c) = self.heading.sin_cos();
        let (dx, dz) = (world.x - self.centre.x, world.z - self.centre.z);
        Vec2::new(dx * c + dz * s, -dx * s + dz * c)
    }

    /// Whether a world point lies over the rectangle.
    #[must_use]
    pub fn covers(&self, world: Vec3) -> bool {
        let l = self.to_local(world);
        let h = self.half();
        l.x.abs() <= h.x && l.y.abs() <= h.y
    }

    /// The rectangle's corners in the world, at a height.
    #[must_use]
    pub fn corners(&self, y: f32) -> [Vec3; 4] {
        let h = self.half();
        [
            self.to_world(Vec2::new(-h.x, -h.y), y),
            self.to_world(Vec2::new(h.x, -h.y), y),
            self.to_world(Vec2::new(h.x, h.y), y),
            self.to_world(Vec2::new(-h.x, h.y), y),
        ]
    }

    /// The region of `overlay.cut` that cuts the ground this terrain replaces: the old ground's
    /// heights over the rectangle on the terrain's own grid, [`TOLERANCE`] round them.
    #[must_use]
    pub fn region(&self, old_ground: &Grid) -> Region {
        let (lo, hi) =
            old_ground.heights.iter().fold((f32::MAX, f32::MIN), |(a, b), h| (a.min(*h), b.max(*h)));
        let (lo, hi) = if lo <= hi { (lo, hi) } else { (self.centre.y, self.centre.y) };
        let h = self.half();
        Region {
            kind: CutKind::Ground,
            centre: Vec3::new(self.centre.x, (lo + hi) * 0.5, self.centre.z),
            half: Vec3::new(h.x, (hi - lo) * 0.5 + TOLERANCE, h.y),
            // the region turns a point by -yaw into its own axes: the heading the other way round
            yaw: -self.heading,
            tolerance: TOLERANCE,
            columns: old_ground.columns as u32,
            rows: old_ground.rows as u32,
            heights: old_ground.heights.clone(),
        }
    }
}
