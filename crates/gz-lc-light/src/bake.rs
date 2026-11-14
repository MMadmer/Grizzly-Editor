//! The baker: every light value of a build comes from one call - "light these sample points in
//! this scene". A lightmap texel's sample, a vertex, a static model's vertex, a texel of an
//! implicitly lit texture and a detail slot's point are all a [`Sample`]; what lights them is a
//! [`Lighter`].
//!
//! ```text
//! CpuLighter    light_point per sample on the caller's rayon pool: the reference, the same bytes on
//!               every machine and on any number of threads
//! AutoLighter   another lighter (the graphics card's) while it answers, the CPU from its first failure on
//! NullLighter   a constant light: the structure profiled without the light, and tests
//! ```
//!
//! A lighter is another way to compute [`light_point`], never another light: the formulas are
//! `point.rs`'s and `ray.rs`'s. A sample's light does not depend on the samples beside it, so a
//! caller may cut its points into batches as it likes.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use glam::Vec3;
use rayon::prelude::*;

use crate::color::ColorC;
use crate::data::BuildTexture;
use crate::light::LightSet;
use crate::mesh::FaceId;
use crate::point::{Falloff, Tracer, light_point};
use crate::ray::{LightCache, RayModel};
use crate::{Ctl, LcError, Result};

/// A sample that lies on no level face.
pub const NO_SKIP: u32 = u32::MAX;

/// The samples a CPU worker lights with one cache of shadowing triangles. Chunks are cut by the
/// samples' index, never by the scheduler, and a cache lives for one chunk: what a sample gets
/// does not depend on which thread lit it or what that thread lit before.
pub const CHUNK: usize = 256;

/// One point to light: what [`light_point`] takes. Plain data of 32 bytes - it goes into a
/// graphics card's buffer as it is.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Sample {
    pub p: [f32; 3],
    /// The `LP_*` bits.
    pub flags: u32,
    /// The normal, of unit length.
    pub n: [f32; 3],
    /// The level face the point lies on - a ray from it passes through that face - or [`NO_SKIP`].
    pub skip: u32,
}

impl Sample {
    #[must_use]
    pub fn new(p: Vec3, n: Vec3, flags: u32, skip: Option<FaceId>) -> Self {
        Self { p: p.to_array(), flags, n: n.to_array(), skip: skip.map_or(NO_SKIP, |f| f.0) }
    }

    #[must_use]
    pub fn skipped(&self) -> Option<FaceId> {
        (self.skip != NO_SKIP).then_some(FaceId(self.skip))
    }
}

/// What a batch is lit with.
#[derive(Clone, Copy, Debug)]
pub struct LightJob<'a> {
    pub lights: &'a LightSet,
    pub falloff: Falloff,
}

/// What lights sample points.
pub trait Lighter: Send + Sync {
    /// What the log and the report call it: "the CPU", a graphics card's name.
    fn name(&self) -> String;

    /// Makes a scene ready to be lit in: the ray model with the textures its transparent faces
    /// read. The model's opacity rule and back-face culling are the scene's.
    fn scene<'a>(
        &'a self,
        model: &'a RayModel,
        textures: &'a [BuildTexture],
    ) -> Result<Box<dyn LitScene + 'a>>;

    /// The samples lit so far.
    fn samples(&self) -> u64 {
        0
    }

    /// Sentences for the log that came up since the last ask (a graphics card given up on).
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }
}

/// A scene ready to be lit in.
pub trait LitScene: Send + Sync {
    /// `out[i]` becomes the light of `samples[i]`; the two are of one length. Stopped by `ctl`
    /// between chunks, with `Err(Cancelled)`.
    fn light(&self, job: &LightJob<'_>, samples: &[Sample], out: &mut [ColorC], ctl: &Ctl<'_>) -> Result<()>;
}

fn same_length(samples: &[Sample], out: &[ColorC]) -> Result<()> {
    if samples.len() == out.len() {
        Ok(())
    } else {
        Err(LcError::Invalid(format!(
            "A batch of {} samples was given room for {} lights.",
            samples.len(),
            out.len()
        )))
    }
}

/// The reference: [`light_point`] per sample on the caller's rayon pool.
#[derive(Debug, Default)]
pub struct CpuLighter {
    lit: AtomicU64,
}

struct CpuScene<'a> {
    model: &'a RayModel,
    textures: &'a [BuildTexture],
    lit: &'a AtomicU64,
}

impl Lighter for CpuLighter {
    fn name(&self) -> String {
        "the CPU".to_owned()
    }

    fn scene<'a>(
        &'a self,
        model: &'a RayModel,
        textures: &'a [BuildTexture],
    ) -> Result<Box<dyn LitScene + 'a>> {
        Ok(Box::new(CpuScene { model, textures, lit: &self.lit }))
    }

    fn samples(&self) -> u64 {
        self.lit.load(Ordering::Relaxed)
    }
}

impl LitScene for CpuScene<'_> {
    fn light(&self, job: &LightJob<'_>, samples: &[Sample], out: &mut [ColorC], ctl: &Ctl<'_>) -> Result<()> {
        same_length(samples, out)?;
        let tracer = Tracer { model: self.model, textures: self.textures, falloff: job.falloff };
        samples.par_chunks(CHUNK).zip(out.par_chunks_mut(CHUNK)).for_each_init(
            LightCache::default,
            |cache, (samples, out)| {
                // a stopped bake drops what it lit: the chunks left are not worth a ray
                if ctl.cancelled() {
                    return;
                }
                cache.reset();
                for (s, o) in samples.iter().zip(out.iter_mut()) {
                    let mut c = ColorC::default();
                    light_point(
                        &tracer,
                        cache,
                        &mut c,
                        Vec3::from(s.p),
                        Vec3::from(s.n),
                        job.lights,
                        s.flags,
                        s.skipped(),
                    );
                    *o = c;
                }
            },
        );
        ctl.check()?;
        self.lit.fetch_add(samples.len() as u64, Ordering::Relaxed);
        Ok(())
    }
}

/// A constant light, without a ray: the structure of a compile measured apart from its light.
#[derive(Debug, Default)]
pub struct NullLighter {
    lit: AtomicU64,
}

/// What [`NullLighter`] gives every sample.
pub const NULL_LIGHT: ColorC = ColorC { rgb: Vec3::splat(0.5), hemi: 0.5, sun: 0.5, tmp: 0.0 };

struct NullScene<'a>(&'a AtomicU64);

impl Lighter for NullLighter {
    fn name(&self) -> String {
        "no light".to_owned()
    }

    fn scene<'a>(&'a self, _: &'a RayModel, _: &'a [BuildTexture]) -> Result<Box<dyn LitScene + 'a>> {
        Ok(Box::new(NullScene(&self.lit)))
    }

    fn samples(&self) -> u64 {
        self.lit.load(Ordering::Relaxed)
    }
}

impl LitScene for NullScene<'_> {
    fn light(&self, _: &LightJob<'_>, samples: &[Sample], out: &mut [ColorC], ctl: &Ctl<'_>) -> Result<()> {
        same_length(samples, out)?;
        ctl.check()?;
        out.iter_mut().for_each(|o| *o = NULL_LIGHT);
        self.0.fetch_add(samples.len() as u64, Ordering::Relaxed);
        Ok(())
    }
}

/// Another lighter while it answers, the CPU from its first failure on: a build never fails
/// because of a graphics card. What made it give up is a sentence of [`Lighter::notes`].
pub struct AutoLighter {
    first: Box<dyn Lighter>,
    cpu: CpuLighter,
    failed: AtomicBool,
    notes: Mutex<Vec<String>>,
}

impl std::fmt::Debug for AutoLighter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoLighter").field("lighter", &self.name()).finish()
    }
}

impl AutoLighter {
    #[must_use]
    pub fn new(first: Box<dyn Lighter>) -> Self {
        Self {
            first,
            cpu: CpuLighter::default(),
            failed: AtomicBool::new(false),
            notes: Mutex::new(Vec::new()),
        }
    }

    fn gave_up(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    fn give_up(&self, why: &LcError) {
        if !self.failed.swap(true, Ordering::Relaxed)
            && let Ok(mut notes) = self.notes.lock()
        {
            notes.push(format!(
                "{} could not light the level ({why}): the light is baked on the CPU.",
                self.first.name()
            ));
        }
    }
}

struct AutoScene<'a> {
    owner: &'a AutoLighter,
    first: Option<Box<dyn LitScene + 'a>>,
    cpu: Box<dyn LitScene + 'a>,
}

impl Lighter for AutoLighter {
    fn name(&self) -> String {
        if self.gave_up() { self.cpu.name() } else { self.first.name() }
    }

    fn scene<'a>(
        &'a self,
        model: &'a RayModel,
        textures: &'a [BuildTexture],
    ) -> Result<Box<dyn LitScene + 'a>> {
        let cpu = self.cpu.scene(model, textures)?;
        let first = if self.gave_up() {
            None
        } else {
            match self.first.scene(model, textures) {
                Ok(s) => Some(s),
                Err(LcError::Cancelled) => return Err(LcError::Cancelled),
                Err(e) => {
                    self.give_up(&e);
                    None
                }
            }
        };
        Ok(Box::new(AutoScene { owner: self, first, cpu }))
    }

    fn samples(&self) -> u64 {
        self.first.samples() + self.cpu.samples()
    }

    fn notes(&self) -> Vec<String> {
        let mut out = self.first.notes();
        if let Ok(mut notes) = self.notes.lock() {
            out.append(&mut notes);
        }
        out
    }
}

impl LitScene for AutoScene<'_> {
    fn light(&self, job: &LightJob<'_>, samples: &[Sample], out: &mut [ColorC], ctl: &Ctl<'_>) -> Result<()> {
        if let Some(first) = &self.first
            && !self.owner.gave_up()
        {
            match first.light(job, samples, out, ctl) {
                Ok(()) => return Ok(()),
                Err(LcError::Cancelled) => return Err(LcError::Cancelled),
                // the batch is lit again, whole: what the first lighter wrote of it is written over
                Err(e) => self.owner.give_up(&e),
            }
        }
        self.cpu.light(job, samples, out, ctl)
    }
}

/// Where a build's light is baked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LightChoice {
    /// The graphics card when one is there and answers as the CPU does, the CPU otherwise.
    #[default]
    Auto,
    /// The graphics card; a build without one is refused.
    Gpu,
    /// The CPU: the reference.
    Cpu,
    /// A constant light ([`NullLighter`]).
    Off,
}

impl LightChoice {
    pub const ALL: [LightChoice; 4] = [Self::Auto, Self::Gpu, Self::Cpu, Self::Off];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Gpu => "gpu",
            Self::Cpu => "cpu",
            Self::Off => "off",
        }
    }

    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name().eq_ignore_ascii_case(word))
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec2;

    use super::*;
    use crate::light::{LT_DIRECT, LT_POINT, RLight};
    use crate::point::{LP_DEFAULT, LP_DONT_SUN};
    use crate::ray::{Opacity, RayFace};

    /// Some hundred triangles on a half-metre grid, every fourth transparent, with a texture of
    /// four alphas.
    fn scene() -> (RayModel, Vec<BuildTexture>, Vec<Vec3>) {
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        let mut faces = Vec::new();
        let mut s = 0x2545_F491u32;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        for i in 0..400u32 {
            let mut g = || (next() % 17) as f32 * 0.5;
            let (x, y, z) = (g(), g(), g());
            let (a, b) = (g() * 0.25 + 0.5, g() * 0.25 + 0.5);
            let base = verts.len() as u32;
            match i % 3 {
                0 => verts.extend([Vec3::new(x, y, z), Vec3::new(x + a, y, z), Vec3::new(x, y, z + b)]),
                1 => verts.extend([Vec3::new(x, y, z), Vec3::new(x, y + a, z), Vec3::new(x + b, y, z)]),
                _ => verts.extend([Vec3::new(x, y, z), Vec3::new(x, y, z + a), Vec3::new(x, y + b, z)]),
            }
            tris.push([base, base + 1, base + 2]);
            faces.push(RayFace {
                face: Some(FaceId(i)),
                cast_shadow: i % 11 != 0,
                opaque: i % 4 != 0,
                texture: 0,
                uv: [Vec2::ZERO, Vec2::X, Vec2::Y],
            });
        }
        let tex = BuildTexture {
            width: 2,
            height: 2,
            has_alpha: true,
            surface: Some(vec![0x8000_0000, 0x4000_0000, 0xC000_0000, 0x1000_0000]),
            ..BuildTexture::default()
        };
        let model = RayModel::new(&verts, &tris, faces, Opacity::Squared, false).unwrap();
        (model, vec![tex], verts)
    }

    fn lights() -> LightSet {
        let sky =
            |d: Vec3| RLight { kind: LT_DIRECT, direction: -d.normalize(), energy: 0.1, ..RLight::default() };
        let lamp = RLight {
            kind: LT_POINT,
            position: Vec3::new(4.0, 6.0, 4.0),
            diffuse: Vec3::new(1.0, 0.8, 0.6),
            range: 12.0,
            range2: 144.0,
            attenuation0: 1.0,
            falloff: 1.0 / 12.0,
            energy: 1.5,
            ..RLight::default()
        };
        LightSet {
            rgb: vec![lamp, sky(Vec3::new(0.2, 1.0, 0.1))],
            hemi: vec![sky(Vec3::Y), sky(Vec3::new(1.0, 1.0, 0.0)), sky(Vec3::new(-0.5, 0.7, 0.5))],
            sun: vec![sky(Vec3::new(0.3, 0.8, -0.4))],
        }
    }

    fn samples(verts: &[Vec3]) -> Vec<Sample> {
        verts
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let n = [Vec3::Y, Vec3::X, Vec3::new(0.0, 0.6, 0.8), Vec3::NEG_Z][i % 4];
                let flags = if i % 5 == 0 { LP_DONT_SUN } else { LP_DEFAULT };
                Sample::new(p, n, flags, (i % 3 == 0).then_some(FaceId((i / 3) as u32)))
            })
            .collect()
    }

    fn bits(c: &ColorC) -> [u32; 5] {
        [c.rgb.x.to_bits(), c.rgb.y.to_bits(), c.rgb.z.to_bits(), c.hemi.to_bits(), c.sun.to_bits()]
    }

    #[test]
    fn the_cpu_lighter_gives_what_light_point_gives_point_by_point() {
        let (model, textures, verts) = scene();
        let (lights, samples) = (lights(), samples(&verts));
        let job = LightJob { lights: &lights, falloff: Falloff::Level { linear: false } };
        let cpu = CpuLighter::default();
        let mut out = vec![ColorC::default(); samples.len()];
        cpu.scene(&model, &textures).unwrap().light(&job, &samples, &mut out, &Ctl::quiet()).unwrap();
        let tracer = Tracer { model: &model, textures: &textures, falloff: job.falloff };
        let mut lit = 0;
        for (s, got) in samples.iter().zip(&out) {
            // a cold cache for every point: the lighter's warm one may only save a walk
            let (mut cache, mut c) = (LightCache::default(), ColorC::default());
            light_point(
                &tracer,
                &mut cache,
                &mut c,
                Vec3::from(s.p),
                Vec3::from(s.n),
                &lights,
                s.flags,
                s.skipped(),
            );
            assert_eq!(bits(got), bits(&c));
            lit += usize::from(c.hemi > 0.0 || c.sun > 0.0 || c.rgb.x > 0.0);
        }
        assert!(lit > samples.len() / 10, "{lit} of {} lit: the test proves little", samples.len());
        assert_eq!(cpu.samples(), samples.len() as u64);
        // one thread or many: the same bytes
        let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let mut one = vec![ColorC::default(); samples.len()];
        pool.install(|| {
            cpu.scene(&model, &textures).unwrap().light(&job, &samples, &mut one, &Ctl::quiet()).unwrap();
        });
        assert!(one.iter().zip(&out).all(|(a, b)| bits(a) == bits(b)));
    }

    /// A lighter whose scene is made and whose second batch fails.
    struct Fails(AtomicU64);
    struct FailsScene<'a>(&'a AtomicU64);

    impl Lighter for Fails {
        fn name(&self) -> String {
            "A card".to_owned()
        }

        fn scene<'a>(&'a self, _: &'a RayModel, _: &'a [BuildTexture]) -> Result<Box<dyn LitScene + 'a>> {
            Ok(Box::new(FailsScene(&self.0)))
        }
    }

    impl LitScene for FailsScene<'_> {
        fn light(&self, _: &LightJob<'_>, _: &[Sample], out: &mut [ColorC], _: &Ctl<'_>) -> Result<()> {
            if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
                out.iter_mut().for_each(|o| *o = NULL_LIGHT);
                Ok(())
            } else {
                // half written, then lost
                out.iter_mut().take(3).for_each(|o| o.sun = 99.0);
                Err(LcError::Invalid("the device was lost".into()))
            }
        }
    }

    #[test]
    fn a_lighter_that_fails_hands_the_batch_and_the_rest_to_the_cpu() {
        let (model, textures, verts) = scene();
        let (lights, samples) = (lights(), samples(&verts));
        let job = LightJob { lights: &lights, falloff: Falloff::Level { linear: false } };
        let auto = AutoLighter::new(Box::new(Fails(AtomicU64::new(0))));
        assert_eq!(auto.name(), "A card");
        let scene = auto.scene(&model, &textures).unwrap();
        let mut first = vec![ColorC::default(); 8];
        scene.light(&job, &samples[..8], &mut first, &Ctl::quiet()).unwrap();
        assert!(first.iter().all(|c| *c == NULL_LIGHT), "the first batch is the card's");
        let mut cpu = vec![ColorC::default(); samples.len()];
        CpuLighter::default()
            .scene(&model, &textures)
            .unwrap()
            .light(&job, &samples, &mut cpu, &Ctl::quiet())
            .unwrap();
        let mut second = vec![ColorC::default(); samples.len()];
        scene.light(&job, &samples, &mut second, &Ctl::quiet()).unwrap();
        assert!(
            second.iter().zip(&cpu).all(|(a, b)| bits(a) == bits(b)),
            "the failed batch is the CPU's, whole"
        );
        assert_eq!(auto.name(), "the CPU");
        let notes = auto.notes();
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0].starts_with("A card could not light the level (the device was lost)"),
            "{}",
            notes[0]
        );
        assert!(auto.notes().is_empty(), "a note is said once");
        // a scene made after the failure never asks the card
        let mut third = vec![ColorC::default(); samples.len()];
        auto.scene(&model, &textures).unwrap().light(&job, &samples, &mut third, &Ctl::quiet()).unwrap();
        assert!(third.iter().zip(&cpu).all(|(a, b)| bits(a) == bits(b)));
    }

    #[test]
    fn the_choice_has_its_words_and_no_light_is_one_colour() {
        for c in LightChoice::ALL {
            assert_eq!(LightChoice::parse(c.name()), Some(c));
        }
        assert_eq!(LightChoice::parse("GPU"), Some(LightChoice::Gpu));
        assert_eq!(LightChoice::parse("fast"), None);
        let (model, textures, verts) = scene();
        let (lights, samples) = (lights(), samples(&verts));
        let job = LightJob { lights: &lights, falloff: Falloff::Details };
        let null = NullLighter::default();
        let mut out = vec![ColorC::default(); samples.len()];
        null.scene(&model, &textures).unwrap().light(&job, &samples, &mut out, &Ctl::quiet()).unwrap();
        assert!(out.iter().all(|c| *c == NULL_LIGHT));
        let mut short = vec![ColorC::default(); 2];
        assert!(
            null.scene(&model, &textures).unwrap().light(&job, &samples, &mut short, &Ctl::quiet()).is_err()
        );
    }
}
