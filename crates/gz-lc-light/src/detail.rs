//! The light of the detail objects: every slot of
//! `level.details` that holds an object is lit at 15 x 15 points on the ground under it, the
//! mean quantised into its five colours.

use glam::Vec3;
use gz_fmt_level::{DETAIL_SLOT_SIZE, DetailHeader, DetailSlot};
use rayon::prelude::*;

use crate::bake::{LightJob, LitScene, Sample};
use crate::batch::{self, BATCH, Gathered, light_samples};
use crate::color::ColorC;
use crate::data::{BuildMaterial, BuildTexture, Shaders};
use crate::light::LightSet;
use crate::math::{Aabb, EPS, EPS_L, cross, dot, mknormal};
use crate::point::Falloff;
use crate::ray::{RayModel, RcastForm};
use crate::{Ctl, Result};

/// The points of a slot run from -7 to 7 on each axis.
pub const LIGHT_COUNT: i32 = 7;

/// `global_claculation_data`: what the detail compiler reads, less the slots.
#[derive(Clone, Debug)]
pub struct DetailScene {
    pub lights: LightSet,
    pub shaders: Shaders,
    pub materials: Vec<BuildMaterial>,
    pub textures: Vec<BuildTexture>,
    /// The ray model of `build.cform`: only front faces stop a ray, the alpha passes linearly.
    pub model: RayModel,
    /// The level's bounding box.
    pub bounds: Aabb,
    pub header: DetailHeader,
}

impl DetailScene {
    /// The scene of `build.cform`, the build's materials (`post_process_materials` done),
    /// its textures (prepared for the details: flipped) and `build.lights`.
    pub fn new(
        form: &RcastForm,
        shaders: Shaders,
        materials: Vec<BuildMaterial>,
        textures: Vec<BuildTexture>,
        lights: LightSet,
        header: DetailHeader,
    ) -> Result<Self> {
        let model = form.ray_model(&shaders, &materials, &textures)?;
        Ok(Self { lights, shaders, materials, textures, model, bounds: form.bounds, header })
    }

    /// `slot_min_x`.
    #[must_use]
    pub fn slot_min_x(&self, x: i32) -> f32 {
        (x - self.header.offs_x) as f32 * DETAIL_SLOT_SIZE
    }

    /// `slot_min_z`.
    #[must_use]
    pub fn slot_min_z(&self, z: i32) -> f32 {
        (z - self.header.offs_z) as f32 * DETAIL_SLOT_SIZE
    }

    /// `get_slot_box`: the slot's column from its base up its height, grown by 5 cm.
    #[must_use]
    pub fn slot_box(&self, x: i32, z: i32, slot: &DetailSlot) -> Aabb {
        let min = Vec3::new(self.slot_min_x(x), slot.base_height(), self.slot_min_z(z));
        let d = Vec3::new(DETAIL_SLOT_SIZE, slot.height(), DETAIL_SLOT_SIZE);
        let mut b = Aabb { min, max: Vec3::new(min.x + d.x, min.y + d.y, min.z + d.z) };
        b.grow(0.05);
        b
    }

    /// The slot's square on the ground, pulled in by `EPS_L`.
    #[must_use]
    pub fn slot_rect(&self, x: i32, z: i32) -> [f32; 4] {
        let h = DETAIL_SLOT_SIZE * 0.5;
        let cx = self.header.from_slot_x(x);
        let cz = self.header.from_slot_z(z);
        [cx - h + EPS_L, cz - h + EPS_L, cx + h - EPS_L, cz + h - EPS_L]
    }
}

/// `process_pallete`: a part whose four corner densities are all 0 holds no object.
pub fn process_palette(slot: &mut DetailSlot) {
    for part in 0..4 {
        if (0..4).all(|corner| slot.density(part, corner) == 0) {
            slot.set_id(part, DetailSlot::ID_EMPTY);
        }
    }
}

/// `get_CD`: the centre and the half size.
fn center_half(b: &Aabb) -> (Vec3, Vec3) {
    let d = Vec3::new((b.max.x - b.min.x) * 0.5, (b.max.y - b.min.y) * 0.5, (b.max.z - b.min.z) * 0.5);
    (Vec3::new(b.min.x + d.x, b.min.y + d.y, b.min.z + d.z), d)
}

/// A ray-triangle test with culling: `range` of a front face's hit.
fn test_ray_tri_cull(c: Vec3, d: Vec3, p: &[Vec3; 3]) -> Option<f32> {
    let e1 = p[1] - p[0];
    let e2 = p[2] - p[0];
    let pvec = cross(d, e2);
    let det = dot(e1, pvec);
    if det < EPS {
        return None;
    }
    let tvec = c - p[0];
    let u = dot(tvec, pvec);
    if u < 0.0 || u > det {
        return None;
    }
    let qvec = cross(tvec, e1);
    let v = dot(d, qvec);
    if v < 0.0 || u + v > det {
        return None;
    }
    Some(dot(e2, qvec) * (1.0 / det))
}

/// `detail_slot_calculate`'s points: where the slot is lit, `None` when no face of the level is in
/// its box. Each point is dropped onto the highest front face under it in the box; a point that
/// finds none below the slot's base is left out.
pub fn slot_points(
    scene: &DetailScene,
    x: i32,
    z: i32,
    slot: &DetailSlot,
    found: &mut Vec<u32>,
    out: &mut Vec<Sample>,
) -> Option<()> {
    let bb = scene.slot_box(x, z, slot);
    let (c, d) = center_half(&bb);
    let query = Aabb { min: c - (d + Vec3::splat(0.01)), max: c + (d + Vec3::splat(0.01)) };
    found.clear();
    scene.model.query_box(&query, found);
    if found.is_empty() {
        return None;
    }
    // X-Ray's compiler walks them in its tree's order; a fixed order here decides between equal heights
    found.sort_unstable();

    let coeff = (DETAIL_SLOT_SIZE * 0.5) / LIGHT_COUNT as f32;
    for px in -LIGHT_COUNT..=LIGHT_COUNT {
        let x_at = c.x + coeff * px as f32;
        for pz in -LIGHT_COUNT..=LIGHT_COUNT {
            let mut n = Vec3::Y;
            let mut p = Vec3::new(x_at, bb.min.y - 5.0, c.z + coeff * pz as f32);
            let dir = Vec3::NEG_Y;
            let start = Vec3::new(p.x, bb.max.y + EPS, p.z);
            for &t in found.iter() {
                let Some(tri) = scene.model.tri(t) else { continue };
                if let Some(range) = test_ray_tri_cull(start, dir, tri)
                    && range >= 0.0
                {
                    let y = start.y - range;
                    if y > p.y {
                        p.y = y + EPS;
                        n = mknormal(tri[0], tri[1], tri[2]);
                    }
                }
            }
            if p.y < bb.min.y {
                continue;
            }
            out.push(Sample::new(p, n, 0, None));
        }
    }
    Some(())
}

/// A slot's five colours from the sum of its points' light: the mean halved, quantised.
#[must_use]
pub fn slot_colors(mut amount: ColorC, count: u32) -> [u8; 5] {
    amount.scale(count);
    amount.mul(0.5);
    let q = |v: f32| DetailSlot::quantize(v, 15) as u8;
    [q(amount.sun), q(amount.hemi), q(amount.rgb.x), q(amount.rgb.y), q(amount.rgb.z)]
}

/// What [`light_details`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetailStats {
    /// Slots that got their light.
    pub lit: usize,
    /// Slots that hold no object.
    pub empty: usize,
    /// Slots with no face of the level in their box: their colours are left as they were.
    pub no_ground: usize,
}

/// What a row of slots wants lit: its points and, per slot, where they are.
#[derive(Default)]
struct RowPoints {
    stats: DetailStats,
    points: Gathered,
}

/// The detail light: every slot's palette processed and, unless it is empty (or outside
/// `only`, the square `do_light.ltx` asks for: centre x, centre z, radius), lit through `lit`,
/// the scene of the detail model - the points of the rows of a batch gathered in parallel, lit
/// in one call, the colours scattered back.
pub fn light_details(
    scene: &DetailScene,
    slots: &mut [DetailSlot],
    only: Option<[f32; 3]>,
    lit: &dyn LitScene,
    ctl: &Ctl<'_>,
) -> Result<DetailStats> {
    let sx = scene.header.size_x as usize;
    if sx == 0 {
        return Ok(DetailStats::default());
    }
    let rows = slots.len() / sx;
    let per_slot = ((2 * LIGHT_COUNT + 1) * (2 * LIGHT_COUNT + 1)) as usize;
    let rows_per = (BATCH / (sx * per_slot)).max(1);
    let job = LightJob { lights: &scene.lights, falloff: Falloff::Details };
    let mut stats = DetailStats::default();
    let mut z0 = 0usize;
    while z0 < rows {
        ctl.check()?;
        let z1 = (z0 + rows_per).min(rows);
        let block = &mut slots[z0 * sx..z1 * sx];
        let gathered: Vec<RowPoints> = block
            .par_chunks_mut(sx)
            .enumerate()
            .map_init(Vec::new, |found, (dz, row)| {
                let z = z0 + dz;
                let mut out = RowPoints::default();
                for (x, slot) in row.iter_mut().enumerate() {
                    process_palette(slot);
                    if slot.is_empty() {
                        out.stats.empty += 1;
                        continue;
                    }
                    if let Some([cx, cz, r]) = only {
                        let [x1, z1, x2, z2] = scene.slot_rect(x as i32, z as i32);
                        let inside = x2 >= cx - r && x1 <= cx + r && z2 >= cz - r && z1 <= cz + r;
                        if !inside {
                            continue;
                        }
                    }
                    let first = out.points.begin();
                    match slot_points(scene, x as i32, z as i32, slot, found, &mut out.points.samples) {
                        Some(()) => {
                            out.points.end(x as u32, first);
                            out.stats.lit += 1;
                        }
                        None => out.stats.no_ground += 1,
                    }
                }
                out
            })
            .collect();
        let mut bases = Vec::with_capacity(gathered.len());
        let mut samples = Vec::new();
        for g in &gathered {
            bases.push(samples.len());
            samples.extend_from_slice(&g.points.samples);
        }
        let light = light_samples(lit, &job, &samples, ctl)?;
        block.par_chunks_mut(sx).zip(gathered.par_iter().zip(&bases)).for_each(|(row, (g, &base))| {
            let mine = light.get(base..base + g.points.samples.len()).unwrap_or_default();
            for r in &g.points.runs {
                if let Some(slot) = row.get_mut(r.item as usize) {
                    slot.set_colors(slot_colors(batch::sum(mine, r), r.count));
                }
            }
        });
        for g in &gathered {
            stats.lit += g.stats.lit;
            stats.empty += g.stats.empty;
            stats.no_ground += g.stats.no_ground;
        }
        z0 = z1;
        ctl.progress(z0, rows);
    }
    ctl.check()?;
    Ok(stats)
}
