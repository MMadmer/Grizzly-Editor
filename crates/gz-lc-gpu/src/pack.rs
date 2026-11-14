//! What the kernel reads, packed on the CPU: the scene (the tree, the triangles, the
//! transparent faces' texture coordinates, the alpha of their textures) and a job's lights
//! (the lights and the grid of the point lights).

use glam::{Vec2, Vec3};
use gz_lc_light::{
    BuildTexture, Falloff, LT_DIRECT, LT_POINT, LT_SECONDARY, LightSet, Opacity, RLight, RayModel, Sample,
};
use rayon::prelude::*;

use crate::bvh;

/// A triangle the kernel reads as opaque.
pub(crate) const OPAQUE: u32 = u32::MAX;
/// The point lights of a set are culled by a grid when there are more of them.
pub(crate) const GRID_FROM: usize = 16;

/// The scene as the kernel reads it.
#[derive(Debug, Default)]
pub(crate) struct PackedScene {
    pub nodes: Vec<bvh::GpuNode>,
    /// Three `vec4` a triangle: `v0` and the level face, `e1` and [`OPAQUE`] or the triangle's
    /// row of `transp`, `e2`.
    pub tris: Vec<[f32; 4]>,
    /// Two `vec4` a transparent triangle: its texture coordinates and its texture's slot.
    pub transp: Vec<[f32; 4]>,
    /// A table of four words a slot (the first texel's byte, width, height), then the alpha,
    /// four texels a word.
    pub alpha: Vec<u32>,
    /// The most entries the kernel's stack holds.
    pub stack: u32,
    /// Pictures made smaller to fit the card's binding limit.
    pub halved: usize,
}

struct Alpha {
    w: u32,
    h: u32,
    a: Vec<u8>,
}

impl Alpha {
    fn of(t: &BuildTexture) -> Option<Self> {
        let surface = t.surface.as_ref()?;
        let (w, h) = (t.width, t.height);
        if w == 0 || h == 0 {
            return None;
        }
        // a texel past the surface's end makes the CPU take the face as opaque there: alpha 255
        // lets nothing through under either rule
        let n = w as usize * h as usize;
        let a = (0..n).map(|i| surface.get(i).map_or(255, |p| (p >> 24) as u8)).collect();
        Some(Self { w, h, a })
    }

    fn bytes(&self) -> u64 {
        (self.a.len() as u64).div_ceil(4) * 4
    }

    /// Half the size, every texel the mean of the two by two it covers.
    fn halve(&self) -> Self {
        let (w, h) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let at = |x: u32, y: u32| {
            let (x, y) = (x.min(self.w - 1), y.min(self.h - 1));
            u32::from(self.a.get((y * self.w + x) as usize).copied().unwrap_or(255))
        };
        let mut a = Vec::with_capacity(w as usize * h as usize);
        for y in 0..h {
            for x in 0..w {
                let s =
                    at(2 * x, 2 * y) + at(2 * x + 1, 2 * y) + at(2 * x, 2 * y + 1) + at(2 * x + 1, 2 * y + 1);
                a.push(((s + 2) / 4) as u8);
            }
        }
        Self { w, h, a }
    }
}

fn v4(v: Vec3, w: u32) -> [f32; 4] {
    [v.x, v.y, v.z, f32::from_bits(w)]
}

/// Packs a ray model. `alpha_limit` is the largest alpha buffer the card binds, in bytes.
pub(crate) fn scene(model: &RayModel, textures: &[BuildTexture], alpha_limit: u64) -> PackedScene {
    let (tris, faces) = (model.tris(), model.faces());
    // a face that casts no shadow is passed by every ray: it is left out of the tree
    let kept: Vec<u32> = (0..tris.len().min(faces.len()) as u32)
        .filter(|&i| faces.get(i as usize).is_some_and(|f| f.cast_shadow))
        .collect();
    let corners: Vec<[Vec3; 3]> =
        kept.iter().map(|&i| tris.get(i as usize).copied().unwrap_or_default()).collect();
    let tree = bvh::build(&corners);

    // the alpha of every texture a transparent face reads; a face whose texture has no pixels is
    // opaque, as the CPU takes it
    let mut slot_of = vec![u32::MAX; textures.len()];
    let mut pictures: Vec<Alpha> = Vec::new();
    for &i in &kept {
        let Some(f) = faces.get(i as usize) else { continue };
        if f.opaque {
            continue;
        }
        let t = f.texture as usize;
        if slot_of.get(t).is_some_and(|&s| s == u32::MAX)
            && let Some(a) = textures.get(t).and_then(Alpha::of)
        {
            slot_of[t] = pictures.len() as u32;
            pictures.push(a);
        }
    }
    let table = 16 * pictures.len() as u64;
    let mut halved = 0;
    while table + pictures.iter().map(Alpha::bytes).sum::<u64>() > alpha_limit {
        let Some(largest) = pictures.iter_mut().filter(|p| p.a.len() > 1).max_by_key(|p| p.a.len()) else {
            break;
        };
        *largest = largest.halve();
        halved += 1;
    }
    let mut alpha = Vec::new();
    let mut offset = table;
    let mut data: Vec<u8> = Vec::new();
    for p in &pictures {
        alpha.extend([offset as u32, p.w, p.h, 0]);
        data.extend_from_slice(&p.a);
        data.resize(data.len().div_ceil(4) * 4, 0);
        offset = table + data.len() as u64;
    }
    alpha.extend(data.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)));
    if alpha.is_empty() {
        alpha.push(0);
    }

    let mut transp = Vec::new();
    let mut row_of = vec![OPAQUE; tree.order.len()];
    for (k, &o) in tree.order.iter().enumerate() {
        let Some(f) = kept.get(o as usize).and_then(|&i| faces.get(i as usize)) else { continue };
        let slot = slot_of.get(f.texture as usize).copied().unwrap_or(u32::MAX);
        if f.opaque || slot == u32::MAX {
            continue;
        }
        row_of[k] = (transp.len() / 2) as u32;
        let [a, b, c]: [Vec2; 3] = f.uv;
        transp.push([a.x, a.y, b.x, b.y]);
        transp.push([c.x, c.y, f32::from_bits(slot), 0.0]);
    }
    if transp.is_empty() {
        transp.push([0.0; 4]);
    }
    let tris: Vec<[f32; 4]> = tree
        .order
        .par_iter()
        .zip(row_of.par_iter())
        .flat_map_iter(|(&o, &row)| {
            let i = kept.get(o as usize).copied().unwrap_or(0) as usize;
            let t = tris.get(i).copied().unwrap_or_default();
            let face = faces.get(i).and_then(|f| f.face).map_or(gz_lc_light::NO_SKIP, |f| f.0);
            [v4(t[0], face), v4(t[1] - t[0], row), v4(t[2] - t[0], 0)]
        })
        .collect();
    let tris = if tris.is_empty() { vec![[0.0; 4]; 3] } else { tris };
    PackedScene { nodes: tree.nodes, tris, transp, alpha, stack: tree.stack, halved }
}

/// A sample as the kernel reads it: with its rays' origin, rounded as the CPU rounds it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuSample {
    pub sample: Sample,
    pub o: [f32; 3],
    pub pad: u32,
}

impl GpuSample {
    pub fn of(s: &Sample) -> Self {
        let (p, n) = (s.p, s.n);
        // `mad(p, n, 0.01)` of `light_point`
        let o = [p[0] + n[0] * 0.01, p[1] + n[1] * 0.01, p[2] + n[2] * 0.01];
        Self { sample: *s, o, pad: 0 }
    }
}

/// The kernel's uniform block.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Params {
    /// The samples, the mode bits ([`MODE_CULL`], [`MODE_LINEAR`]), the falloff, whether a grid
    /// culls the point lights.
    pub a: [u32; 4],
    /// The static directional lights (first, count) and the static point lights.
    pub b: [u32; 4],
    /// The sun's and the hemisphere's lights.
    pub c: [u32; 4],
    /// The grid's cells along each axis and where its entries start.
    pub d: [u32; 4],
    /// The grid's corner.
    pub e: [f32; 4],
    /// The grid's cells a metre.
    pub f: [f32; 4],
    /// -, the most lights a sample has (the rays of the trace pass a sample).
    pub g: [u32; 4],
}

pub(crate) const MODE_CULL: u32 = 1;
pub(crate) const MODE_LINEAR: u32 = 2;
pub(crate) const FALLOFF_LEVEL: u32 = 0;
pub(crate) const FALLOFF_LEVEL_LINEAR: u32 = 1;
pub(crate) const FALLOFF_DETAILS: u32 = 2;

/// One light: position and kind, direction and range, colour and squared range, the
/// attenuations and the energy, the falloff.
pub(crate) type GpuLight = [[f32; 4]; 5];

/// A job's lights.
#[derive(Debug, Default)]
pub(crate) struct PackedLights {
    pub lights: Vec<GpuLight>,
    pub grid: Vec<u32>,
    pub params: Params,
}

fn gpu_light(l: &RLight, kind: u16) -> GpuLight {
    [
        [l.position.x, l.position.y, l.position.z, f32::from(kind)],
        [l.direction.x, l.direction.y, l.direction.z, l.range],
        [l.diffuse.x, l.diffuse.y, l.diffuse.z, l.range2],
        [l.attenuation0, l.attenuation1, l.attenuation2, l.energy],
        [l.falloff, -l.direction.x * 0.001, -l.direction.y * 0.001, -l.direction.z * 0.001],
    ]
}

/// The lights of a job for a scene of the given rules.
pub(crate) fn lights(set: &LightSet, falloff: Falloff, opacity: Opacity, cull: bool) -> PackedLights {
    let mut lights = Vec::new();
    let mut points: Vec<(Vec3, f32)> = Vec::new();
    // the static lights: the directional ones, then those that fall off (the detail compiler
    // takes a secondary light as a point light); a kind neither knows lights nothing
    let kind_of = |l: &RLight| {
        if falloff == Falloff::Details && l.kind != LT_DIRECT { LT_POINT } else { l.kind }
    };
    for l in set.rgb.iter().filter(|l| kind_of(l) == LT_DIRECT) {
        lights.push(gpu_light(l, LT_DIRECT));
    }
    let rgb_pt = lights.len();
    for l in set.rgb.iter() {
        let k = kind_of(l);
        if k == LT_POINT || k == LT_SECONDARY {
            lights.push(gpu_light(l, k));
            points.push((l.position, l.range2.max(0.0).sqrt()));
        }
    }
    let sun = lights.len();
    lights.extend(set.sun.iter().map(|l| gpu_light(l, l.kind)));
    let hemi = lights.len();
    lights.extend(set.hemi.iter().map(|l| gpu_light(l, l.kind)));
    let end = lights.len();
    if lights.is_empty() {
        lights.push([[0.0; 4]; 5]);
    }

    let mode = if cull { MODE_CULL } else { 0 } | if opacity == Opacity::Linear { MODE_LINEAR } else { 0 };
    let fall = match falloff {
        Falloff::Level { linear: false } => FALLOFF_LEVEL,
        Falloff::Level { linear: true } => FALLOFF_LEVEL_LINEAR,
        Falloff::Details => FALLOFF_DETAILS,
    };
    let mut params = Params {
        a: [0, mode, fall, 0],
        b: [0, rgb_pt as u32, rgb_pt as u32, (sun - rgb_pt) as u32],
        c: [sun as u32, (hemi - sun) as u32, hemi as u32, (end - hemi) as u32],
        ..Params::default()
    };
    let mut grid = vec![0u32; 4];
    // the most point lights a sample may have
    let mut most_points = points.len();
    if points.len() > GRID_FROM
        && let Some(g) = Grid::of(&points)
    {
        params.a[3] = 1;
        params.d = [g.dims[0], g.dims[1], g.dims[2], g.cells() + 1];
        params.e = [g.min.x, g.min.y, g.min.z, 0.0];
        params.f = [g.inv.x, g.inv.y, g.inv.z, 0.0];
        grid = g.lists(&points, rgb_pt as u32);
        let cells = g.cells() as usize;
        most_points = (0..cells)
            .map(|c| grid.get(c + 1).zip(grid.get(c)).map_or(0, |(b, a)| b.saturating_sub(*a) as usize))
            .max()
            .unwrap_or(0);
    }
    let most = (rgb_pt + most_points + (end - sun)) as u32;
    params.g = [0, most.max(1), 0, 0];
    PackedLights { lights, grid, params }
}

/// A uniform grid over the point lights' spheres.
struct Grid {
    min: Vec3,
    inv: Vec3,
    dims: [u32; 3],
}

/// The grid's cells at most.
const MAX_CELLS: u32 = 1 << 18;
/// The grid's entries at most; a finer grid with more is made coarser.
const MAX_ENTRIES: u64 = 1 << 24;

impl Grid {
    fn of(points: &[(Vec3, f32)]) -> Option<Self> {
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(-f32::MAX);
        let mut radii: Vec<f32> = Vec::with_capacity(points.len());
        for &(p, r) in points {
            if !p.is_finite() || !r.is_finite() {
                return None;
            }
            lo = lo.min(p - Vec3::splat(r));
            hi = hi.max(p + Vec3::splat(r));
            radii.push(r);
        }
        // a sample on the very border of a sphere is inside it
        let pad = (hi - lo).max_element() * 1e-4 + 1e-3;
        let (lo, hi) = (lo - Vec3::splat(pad), hi + Vec3::splat(pad));
        let ext = (hi - lo).max(Vec3::splat(1e-3));
        radii.sort_unstable_by(f32::total_cmp);
        let median = radii.get(radii.len() / 2).copied().unwrap_or(1.0).max(1e-3);
        let mut cell = median;
        loop {
            let dims = [0, 1, 2].map(|a| ((ext[a] / cell).ceil() as u32).clamp(1, 256));
            let g =
                Grid { min: lo, inv: Vec3::new(dims[0] as f32, dims[1] as f32, dims[2] as f32) / ext, dims };
            if g.cells() <= MAX_CELLS && g.entries(points) <= MAX_ENTRIES {
                return Some(g);
            }
            cell *= 1.5;
        }
    }

    fn cells(&self) -> u32 {
        self.dims[0] * self.dims[1] * self.dims[2]
    }

    /// The cells a sphere's box covers.
    fn span(&self, p: Vec3, r: f32) -> [[u32; 2]; 3] {
        [0, 1, 2].map(|a| {
            let at =
                |x: f32| (((x - self.min[a]) * self.inv[a]).floor().max(0.0) as u32).min(self.dims[a] - 1);
            [at(p[a] - r), at(p[a] + r)]
        })
    }

    fn touches(&self, (p, r): (Vec3, f32), c: [u32; 3]) -> bool {
        let lo = self.min + Vec3::new(c[0] as f32, c[1] as f32, c[2] as f32) / self.inv;
        let hi = self.min + Vec3::new(c[0] as f32 + 1.0, c[1] as f32 + 1.0, c[2] as f32 + 1.0) / self.inv;
        let q = p.clamp(lo, hi);
        let r = r * (1.0 + 1e-4) + 1e-3;
        (q - p).length_squared() <= r * r
    }

    fn entries(&self, points: &[(Vec3, f32)]) -> u64 {
        points
            .iter()
            .map(|&(p, r)| self.span(p, r).iter().map(|s| u64::from(s[1] - s[0] + 1)).product::<u64>())
            .sum()
    }

    /// The cells' starts and, per cell, the lights (by their index in the light buffer) that may
    /// reach it, in their set's order.
    fn lists(&self, points: &[(Vec3, f32)], first: u32) -> Vec<u32> {
        let cells = self.cells() as usize;
        let id = |c: [u32; 3]| ((c[2] * self.dims[1] + c[1]) * self.dims[0] + c[0]) as usize;
        let mut per: Vec<Vec<u32>> = vec![Vec::new(); cells];
        for (i, &(p, r)) in points.iter().enumerate() {
            let s = self.span(p, r);
            for z in s[2][0]..=s[2][1] {
                for y in s[1][0]..=s[1][1] {
                    for x in s[0][0]..=s[0][1] {
                        if self.touches((p, r), [x, y, z])
                            && let Some(list) = per.get_mut(id([x, y, z]))
                        {
                            list.push(first + i as u32);
                        }
                    }
                }
            }
        }
        let mut out = Vec::with_capacity(cells + 1 + per.iter().map(Vec::len).sum::<usize>());
        let mut at = (cells + 1) as u32;
        for list in &per {
            out.push(at);
            at += list.len() as u32;
        }
        out.push(at);
        for list in per {
            out.extend(list);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_holds_every_light_that_reaches_a_point() {
        let mut s = 0x0BAD_5EEDu32;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s % 10_000) as f32 * 0.01
        };
        let points: Vec<(Vec3, f32)> =
            (0..200).map(|_| (Vec3::new(next(), next() * 0.2, next()), 1.0 + next() * 0.1)).collect();
        let g = Grid::of(&points).unwrap();
        let lists = g.lists(&points, 0);
        for _ in 0..5000 {
            let p = Vec3::new(next(), next() * 0.2, next());
            let c = ((p - g.min) * g.inv).floor();
            let inside = (0..3).all(|a| c[a] >= 0.0 && (c[a] as u32) < g.dims[a]);
            let want: Vec<u32> = points
                .iter()
                .enumerate()
                .filter(|(_, (q, r))| (p - *q).length_squared() <= r * r)
                .map(|(i, _)| i as u32)
                .collect();
            if !inside {
                assert!(want.is_empty());
                continue;
            }
            let cell = ((c.z as u32 * g.dims[1] + c.y as u32) * g.dims[0] + c.x as u32) as usize;
            let list = &lists[lists[cell] as usize..lists[cell + 1] as usize];
            for w in want {
                assert!(list.contains(&w), "light {w} missing at {p}");
            }
        }
    }
}
