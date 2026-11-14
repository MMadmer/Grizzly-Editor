//! The light a point with a normal gets from a light set, in three layers - the
//! static lights into `rgb`, the hemisphere into `hemi`, the sun into `sun`. The level
//! compiler's and the detail compiler's
//! differ only in how a point light of the static set falls off.

use glam::Vec3;

use crate::color::ColorC;
use crate::data::BuildTexture;
use crate::light::{LT_DIRECT, LT_POINT, LT_SECONDARY, LightSet, RLight};
use crate::math::{distance_sqr, dot, mad, normalize_safe};
use crate::mesh::FaceId;
use crate::ray::{LightCache, NO_TRI, RayModel};

/// `LP_DEFAULT`.
pub const LP_DEFAULT: u32 = 0;
/// Asks for nothing the tracer does.
pub const LP_USE_FACE_DISABLE: u32 = 1 << 0;
/// Leaves the static (rgb) light out.
pub const LP_DONT_RGB: u32 = 1 << 1;
/// Leaves the hemisphere out.
pub const LP_DONT_HEMI: u32 = 1 << 2;
/// Leaves the sun out.
pub const LP_DONT_SUN: u32 = 1 << 3;

/// How a point light of the static set falls off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Falloff {
    /// The level compiler: the attenuation, made 0 at the range (`-att`: linearly by distance).
    Level { linear: bool },
    /// The detail compiler: the attenuation alone; a secondary light is a point light.
    Details,
}

/// What a point is lit through: the model, the textures its transparent faces read, the falloff.
#[derive(Clone, Copy, Debug)]
pub struct Tracer<'a> {
    pub model: &'a RayModel,
    pub textures: &'a [BuildTexture],
    pub falloff: Falloff,
}

impl Tracer<'_> {
    fn trace(
        &self,
        cache_slot: &mut u32,
        hits: &mut Vec<gz_fmt_level::RayHit>,
        p: Vec3,
        d: Vec3,
        r: f32,
        skip: Option<FaceId>,
    ) -> f32 {
        self.model.trace(self.textures, hits, cache_slot, p, d, r, skip)
    }
}

fn slot(cache: &mut [u32], i: usize) -> u32 {
    cache.get(i).copied().unwrap_or(NO_TRI)
}

fn store(cache: &mut [u32], i: usize, v: u32) {
    if let Some(s) = cache.get_mut(i) {
        *s = v;
    }
}

/// The distance, the direction and its cosine of a point light seen from `p`; `None` out of
/// range or behind the surface.
fn towards(l: &RLight, p: Vec3, n: Vec3) -> Option<(f32, Vec3, f32)> {
    let sq_d = distance_sqr(p, l.position);
    if sq_d > l.range2 {
        return None;
    }
    let ldir = normalize_safe(l.position - p);
    let d = dot(ldir, n);
    (d > 0.0).then_some((sq_d, ldir, d))
}

/// Adds the light of `lights` at `p` (normal `n`) to `c`. `skip` is the face the
/// point lies on.
#[allow(clippy::too_many_arguments)]
pub fn light_point(
    t: &Tracer<'_>,
    cache: &mut LightCache,
    c: &mut ColorC,
    p: Vec3,
    n: Vec3,
    lights: &LightSet,
    flags: u32,
    skip: Option<FaceId>,
) {
    cache.fit(lights);
    let pnew = mad(p, n, 0.01);
    let LightCache { rgb, hemi, sun, hits } = cache;

    if flags & LP_DONT_RGB == 0 {
        for (i, l) in lights.rgb.iter().enumerate() {
            let mut cached = slot(rgb, i);
            let kind = if t.falloff == Falloff::Details && l.kind != LT_DIRECT { LT_POINT } else { l.kind };
            match kind {
                LT_DIRECT => {
                    let ldir = -l.direction;
                    let d = dot(ldir, n);
                    if d <= 0.0 {
                        continue;
                    }
                    let scale = d * l.energy * t.trace(&mut cached, hits, pnew, ldir, 1000.0, skip);
                    c.rgb.x += scale * l.diffuse.x;
                    c.rgb.y += scale * l.diffuse.y;
                    c.rgb.z += scale * l.diffuse.z;
                }
                LT_POINT => {
                    let Some((sq_d, ldir, d)) = towards(l, p, n) else { continue };
                    let r = sq_d.sqrt();
                    let scale = d * l.energy * t.trace(&mut cached, hits, pnew, ldir, r, skip);
                    let a = match t.falloff {
                        Falloff::Level { linear: true } => 1.0 - r / l.range,
                        Falloff::Level { linear: false } => {
                            // the attenuation, made 0 at the light's range
                            scale
                                * (1.0 / (l.attenuation0 + l.attenuation1 * r + l.attenuation2 * sq_d)
                                    - r * l.falloff)
                        }
                        Falloff::Details => {
                            scale / (l.attenuation0 + l.attenuation1 * r + l.attenuation2 * sq_d)
                        }
                    };
                    c.rgb.x += a * l.diffuse.x;
                    c.rgb.y += a * l.diffuse.y;
                    c.rgb.z += a * l.diffuse.z;
                }
                LT_SECONDARY => {
                    let Some((sq_d, ldir, d)) = towards(l, p, n) else { continue };
                    let d = d * -dot(ldir, l.direction);
                    if d <= 0.0 {
                        continue;
                    }
                    // X-Ray's compiler jitters the light's position here and then traces from the point
                    // along the direction it took before: the jitter changes nothing
                    let r = sq_d.sqrt();
                    let scale =
                        d.powf(1.0 / 8.0) * l.energy * t.trace(&mut cached, hits, pnew, ldir, r, skip);
                    let a = scale * (1.0 - r / l.range);
                    c.rgb.x += a * l.diffuse.x;
                    c.rgb.y += a * l.diffuse.y;
                    c.rgb.z += a * l.diffuse.z;
                }
                _ => {}
            }
            store(rgb, i, cached);
        }
    }
    if flags & LP_DONT_SUN == 0 {
        for (i, l) in lights.sun.iter().enumerate() {
            let mut cached = slot(sun, i);
            if l.kind == LT_DIRECT {
                let ldir = -l.direction;
                if dot(ldir, n) <= 0.0 {
                    continue;
                }
                c.sun += l.energy * t.trace(&mut cached, hits, pnew, ldir, 1000.0, skip);
            } else {
                let Some((sq_d, ldir, d)) = towards(l, p, n) else { continue };
                let r = sq_d.sqrt();
                let scale = d * l.energy * t.trace(&mut cached, hits, pnew, ldir, r, skip);
                c.sun += scale / (l.attenuation0 + l.attenuation1 * r + l.attenuation2 * sq_d);
            }
            store(sun, i, cached);
        }
    }
    if flags & LP_DONT_HEMI == 0 {
        for (i, l) in lights.hemi.iter().enumerate() {
            let mut cached = slot(hemi, i);
            if l.kind == LT_DIRECT {
                let ldir = -l.direction;
                if dot(ldir, n) <= 0.0 {
                    continue;
                }
                let moved = mad(pnew, ldir, 0.001);
                c.hemi += l.energy * t.trace(&mut cached, hits, moved, ldir, 1000.0, skip);
            } else {
                let Some((sq_d, ldir, d)) = towards(l, p, n) else { continue };
                let r = sq_d.sqrt();
                let scale = d * l.energy * t.trace(&mut cached, hits, pnew, ldir, r, skip);
                c.hemi += scale / (l.attenuation0 + l.attenuation1 * r + l.attenuation2 * sq_d);
            }
            store(hemi, i, cached);
        }
    }
}
