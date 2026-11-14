//! The card against the CPU. Every test that needs a graphics card prints "no graphics adapter:
//! skipped" and passes without one; `GZ_LC_GPU_TEST_NO_ADAPTER=1` makes a machine with a card
//! take that path.

use std::time::{Duration, Instant};

use glam::{Vec2, Vec3};
use gz_lc_light::{
    BuildTexture, ColorC, CpuLighter, Ctl, FaceId, Falloff, LP_DEFAULT, LP_DONT_HEMI, LP_DONT_RGB,
    LP_DONT_SUN, LT_DIRECT, LT_POINT, LT_SECONDARY, LcError, LightChoice, LightJob, LightSet, Lighter,
    Opacity, RLight, RayFace, RayModel, Sample,
};
use gz_task::Cancel;

use crate::GpuLighter;

/// The card, or `None` where there is no adapter; a machine with one whose lighter fails fails the
/// test (a kernel that does not compile is no missing card).
fn card() -> Option<GpuLighter> {
    match GpuLighter::new() {
        Ok(g) => Some(g),
        Err(why) => {
            let forced = std::env::var_os(crate::gpu::NO_ADAPTER_VAR).is_some();
            assert!(forced || real_adapters().is_empty(), "the graphics card's lighter failed: {why}");
            println!("no graphics adapter: skipped ({why})");
            None
        }
    }
}

fn real_adapters() -> Vec<wgpu::AdapterInfo> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = desc_backends();
    let instance = wgpu::Instance::new(desc);
    futures::executor::block_on(instance.enumerate_adapters(desc_backends()))
        .iter()
        .map(|a| a.get_info())
        .filter(|i| i.device_type != wgpu::DeviceType::Cpu)
        .collect()
}

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn f(&mut self) -> f32 {
        (self.next() % 100_000) as f32 / 100_000.0
    }
}

/// A test level: a ground of quads, opaque boxes, alpha-textured foliage, a few faces that cast no
/// shadow. Answers the model's parts and the samples on the ground and on the boxes.
struct Level {
    verts: Vec<Vec3>,
    tris: Vec<[u32; 3]>,
    faces: Vec<RayFace>,
    samples: Vec<Sample>,
}

impl Level {
    fn tri(&mut self, p: [Vec3; 3], opaque: bool, cast_shadow: bool, texture: u32) -> u32 {
        let base = self.verts.len() as u32;
        let id = self.tris.len() as u32;
        self.verts.extend(p);
        self.tris.push([base, base + 1, base + 2]);
        self.faces.push(RayFace {
            face: Some(FaceId(id)),
            cast_shadow,
            opaque,
            texture,
            uv: [Vec2::new(0.1, 0.2), Vec2::new(1.7, -0.3), Vec2::new(-0.6, 2.4)],
        });
        id
    }

    fn quad(&mut self, o: Vec3, a: Vec3, b: Vec3, opaque: bool, cast_shadow: bool, texture: u32) -> [u32; 2] {
        [
            self.tri([o, o + a, o + a + b], opaque, cast_shadow, texture),
            self.tri([o, o + a + b, o + b], opaque, cast_shadow, texture),
        ]
    }

    fn new(seed: u32) -> Self {
        let mut l = Level { verts: Vec::new(), tris: Vec::new(), faces: Vec::new(), samples: Vec::new() };
        let mut r = Rng(seed);
        // the ground, faced up (its triangles turn their fronts to a ray coming down)
        for z in 0..24 {
            for x in 0..24 {
                let o = Vec3::new(x as f32, 0.0, z as f32);
                let [t0, t1] = l.quad(o, Vec3::Z, Vec3::X, true, true, 0);
                for (k, t) in [t0, t1].into_iter().enumerate() {
                    let p = o + Vec3::new(0.25 + 0.5 * k as f32, 0.0, 0.25 + 0.5 * (1 - k) as f32);
                    let flags = match (x + z) % 7 {
                        0 => LP_DONT_SUN,
                        1 => LP_DONT_HEMI,
                        2 => LP_DONT_RGB,
                        _ => LP_DEFAULT,
                    };
                    let skip = ((x * z) % 3 != 0).then_some(FaceId(t));
                    l.samples.push(Sample::new(p, Vec3::Y, flags, skip));
                }
            }
        }
        // boxes: their tops and sides, opaque
        for _ in 0..40 {
            let o = Vec3::new(r.f() * 22.0, 0.0, r.f() * 22.0);
            let s = Vec3::new(0.5 + r.f() * 1.5, 0.5 + r.f() * 3.0, 0.5 + r.f() * 1.5);
            let (x, y, z) = (Vec3::new(s.x, 0.0, 0.0), Vec3::new(0.0, s.y, 0.0), Vec3::new(0.0, 0.0, s.z));
            let top = l.quad(o + y, z, x, true, true, 0);
            l.quad(o, y, x, true, true, 0);
            l.quad(o + z, x, y, true, true, 0);
            l.quad(o, z, y, true, true, 0);
            l.quad(o + x, y, z, true, true, 0);
            l.samples.push(Sample::new(o + y + x * 0.4 + z * 0.6, Vec3::Y, LP_DEFAULT, Some(FaceId(top[0]))));
            l.samples.push(Sample::new(o + y * 0.5 + z * 0.5 + x, Vec3::X, LP_DEFAULT, None));
            l.samples.push(Sample::new(o + y * 0.3 + x * 0.5, Vec3::NEG_Z, LP_DEFAULT, None));
        }
        // foliage: tilted transparent cards of two textures
        for i in 0..300 {
            let o = Vec3::new(r.f() * 23.0, 0.3 + r.f() * 3.0, r.f() * 23.0);
            let a = Vec3::new(r.f() - 0.5, r.f() * 0.4, r.f() - 0.5).normalize_or_zero() * (0.4 + r.f());
            let b = Vec3::new(r.f() - 0.5, 0.5 + r.f(), r.f() - 0.5).normalize_or_zero() * (0.4 + r.f());
            l.quad(o, a, b, false, true, 1 + i % 2);
        }
        // canopies that cast no shadow, and a texture without pixels (opaque)
        for _ in 0..20 {
            let o = Vec3::new(r.f() * 20.0, 4.0 + r.f(), r.f() * 20.0);
            l.quad(o, Vec3::new(3.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 3.0), true, false, 0);
        }
        for _ in 0..20 {
            let o = Vec3::new(r.f() * 22.0, 2.0 + r.f(), r.f() * 22.0);
            l.quad(o, Vec3::new(0.6, 0.0, 0.0), Vec3::new(0.0, 0.2, 0.6), false, true, 3);
        }
        l
    }

    fn model(&self, opacity: Opacity, cull: bool) -> RayModel {
        RayModel::new(&self.verts, &self.tris, self.faces.clone(), opacity, cull).unwrap()
    }
}

fn textures(seed: u32) -> Vec<BuildTexture> {
    let mut r = Rng(seed);
    let mut pic = |w: u32, h: u32| BuildTexture {
        width: w,
        height: h,
        has_alpha: true,
        surface: Some((0..w * h).map(|_| (r.next() % 256) << 24 | 0x00ff_ffff).collect()),
        ..BuildTexture::default()
    };
    let (a, b) = (pic(16, 8), pic(5, 7));
    vec![
        BuildTexture::default(),
        a,
        b,
        // alpha, but its pixels are gone
        BuildTexture { width: 4, height: 4, has_alpha: true, surface: None, ..BuildTexture::default() },
    ]
}

fn point(p: Vec3, range: f32, colour: Vec3, energy: f32) -> RLight {
    let (a0, a1, a2) = (1.0, 0.2, 0.05);
    RLight {
        kind: LT_POINT,
        position: p,
        diffuse: colour,
        range,
        range2: range * range,
        attenuation0: a0,
        attenuation1: a1,
        attenuation2: a2,
        falloff: 1.0 / (range * (a0 + a1 * range + a2 * range * range)),
        energy,
        ..RLight::default()
    }
}

fn sky(d: Vec3, energy: f32) -> RLight {
    RLight { kind: LT_DIRECT, direction: -d.normalize(), diffuse: Vec3::ONE, energy, ..RLight::default() }
}

/// Forty lamps (a grid culls them), two secondary lights, a directional static light, a sun and
/// a point light in the sun's set, a sky of 64 directions and a point light in it.
fn lights(seed: u32) -> LightSet {
    let mut r = Rng(seed);
    let mut rgb: Vec<RLight> = (0..40)
        .map(|_| {
            let p = Vec3::new(r.f() * 24.0, 1.0 + r.f() * 4.0, r.f() * 24.0);
            point(p, 3.0 + r.f() * 6.0, Vec3::new(r.f(), r.f(), r.f()) + 0.2, 0.5 + r.f())
        })
        .collect();
    for _ in 0..2 {
        let p = Vec3::new(r.f() * 24.0, 3.0, r.f() * 24.0);
        rgb.push(RLight {
            kind: LT_SECONDARY,
            direction: Vec3::new(0.2, -1.0, 0.1).normalize(),
            ..point(p, 8.0, Vec3::ONE, 0.7)
        });
    }
    rgb.push(sky(Vec3::new(0.3, 1.0, 0.2), 0.3));
    let mut hemi: Vec<RLight> = (0..64)
        .map(|k| {
            let a = k as f32 * 0.7;
            let up = 0.2 + (k % 8) as f32 * 0.1;
            sky(Vec3::new(a.cos(), up, a.sin()), 1.0 / 64.0)
        })
        .collect();
    hemi.push(point(Vec3::new(12.0, 6.0, 12.0), 9.0, Vec3::ONE, 0.2));
    let sun = vec![sky(Vec3::new(-0.4, 0.8, 0.3), 0.9), point(Vec3::new(5.0, 2.0, 5.0), 6.0, Vec3::ONE, 0.4)];
    LightSet { rgb, hemi, sun }
}

fn cpu(model: &RayModel, textures: &[BuildTexture], job: &LightJob<'_>, samples: &[Sample]) -> Vec<ColorC> {
    let mut out = vec![ColorC::default(); samples.len()];
    CpuLighter::default()
        .scene(model, textures)
        .unwrap()
        .light(job, samples, &mut out, &Ctl::quiet())
        .unwrap();
    out
}

fn apart(a: &ColorC, b: &ColorC) -> f32 {
    (a.rgb - b.rgb).abs().max_element().max((a.hemi - b.hemi).abs()).max((a.sun - b.sun).abs())
}

/// The share of samples within 0.002 of the CPU's and the mean difference.
fn agreement(got: &[ColorC], want: &[ColorC]) -> (f64, f64) {
    let within = got.iter().zip(want).filter(|(a, b)| apart(a, b) <= 0.002).count();
    let mean =
        got.iter().zip(want).map(|(a, b)| f64::from(apart(a, b))).sum::<f64>() / want.len().max(1) as f64;
    (within as f64 / want.len().max(1) as f64, mean)
}

fn assert_agrees(got: &[ColorC], want: &[ColorC], what: &str) {
    let (within, mean) = agreement(got, want);
    let worst = got.iter().zip(want).map(|(a, b)| apart(a, b)).fold(0.0f32, f32::max);
    assert!(
        within >= 0.999 && mean < 0.0002,
        "{what}: {:.4} % within 0.002, mean {mean:.6}, worst {worst}",
        within * 100.0
    );
    let lit = want.iter().filter(|c| c.hemi > 0.0 || c.sun > 0.0 || c.rgb.max_element() > 0.0).count();
    assert!(lit > want.len() / 4, "{what}: {lit} of {} lit: the test proves little", want.len());
}

/// How many samples two answers light differently.
fn differ(a: &[ColorC], b: &[ColorC]) -> usize {
    a.iter().zip(b).filter(|(a, b)| apart(a, b) > 0.01).count()
}

#[test]
fn the_card_lights_a_level_as_the_cpu_does() {
    let Some(gpu) = card() else { return };
    println!("lit on {}", gpu.name());
    let level = Level::new(0x2545_F491);
    let tex = textures(7);
    let model = level.model(Opacity::Squared, false);
    let set = lights(11);
    for falloff in [Falloff::Level { linear: false }, Falloff::Level { linear: true }] {
        let job = LightJob { lights: &set, falloff };
        let want = cpu(&model, &tex, &job, &level.samples);
        let mut got = vec![ColorC::default(); level.samples.len()];
        gpu.scene(&model, &tex).unwrap().light(&job, &level.samples, &mut got, &Ctl::quiet()).unwrap();
        assert_agrees(&got, &want, &format!("{falloff:?}"));
        // the test proves something: the walls shadow, and the foliage's alpha is read
        let n = want.len();
        let empty = RayModel::new(&[], &[], Vec::new(), Opacity::Squared, false).unwrap();
        assert!(differ(&want, &cpu(&empty, &tex, &job, &level.samples)) > n / 10, "few shadows");
        let mut clear = tex.clone();
        clear.iter_mut().filter_map(|t| t.surface.as_mut()).flatten().for_each(|p| *p &= 0x00ff_ffff);
        assert!(
            differ(&want, &cpu(&model, &clear, &job, &level.samples)) > n / 50,
            "the alpha changes little"
        );
    }
    assert_eq!(gpu.samples(), 2 * level.samples.len() as u64);
}

#[test]
fn the_details_rule_is_linear_and_sees_front_faces() {
    let Some(gpu) = card() else { return };
    let level = Level::new(0x0000_BEEF);
    let tex = textures(3);
    let model = level.model(Opacity::Linear, true);
    let set = lights(5);
    let job = LightJob { lights: &set, falloff: Falloff::Details };
    let want = cpu(&model, &tex, &job, &level.samples);
    let mut got = vec![ColorC::default(); level.samples.len()];
    gpu.scene(&model, &tex).unwrap().light(&job, &level.samples, &mut got, &Ctl::quiet()).unwrap();
    assert_agrees(&got, &want, "details");
}

/// A floor under a slab: a sample on the slab's underside, its face skipped, sees the sky
/// through it; the same sample with no face skipped does not. A slab that casts no shadow lets
/// the light through either way.
#[test]
fn the_skipped_face_and_one_that_casts_no_shadow_are_passed_through() {
    let Some(gpu) = card() else { return };
    let verts = vec![
        Vec3::new(-5.0, 2.0, -5.0),
        Vec3::new(-5.0, 2.0, 5.0),
        Vec3::new(5.0, 2.0, 0.0),
        Vec3::new(-5.0, 4.0, -5.0),
        Vec3::new(-5.0, 4.0, 5.0),
        Vec3::new(5.0, 4.0, 0.0),
    ];
    let face = |i: u32, cast_shadow: bool| RayFace {
        face: Some(FaceId(i)),
        cast_shadow,
        opaque: true,
        texture: 0,
        uv: [Vec2::ZERO; 3],
    };
    let model = RayModel::new(
        &verts,
        &[[0, 1, 2], [3, 4, 5]],
        vec![face(0, true), face(1, false)],
        Opacity::Squared,
        false,
    )
    .unwrap();
    let set = LightSet { rgb: vec![], hemi: vec![sky(Vec3::Y, 1.0)], sun: vec![sky(Vec3::Y, 1.0)] };
    let job = LightJob { lights: &set, falloff: Falloff::Level { linear: false } };
    // under the lower slab, the normal up: the offset leaves the origin under it
    let p = Vec3::new(0.0, 1.98, 0.0);
    let samples =
        [Sample::new(p, Vec3::Y, LP_DEFAULT, Some(FaceId(0))), Sample::new(p, Vec3::Y, LP_DEFAULT, None)];
    let want = cpu(&model, &[], &job, &samples);
    let mut got = [ColorC::default(); 2];
    gpu.scene(&model, &[]).unwrap().light(&job, &samples, &mut got, &Ctl::quiet()).unwrap();
    assert!((got[0].sun - 1.0).abs() < 1e-5 && (got[0].hemi - 1.0).abs() < 1e-5, "{:?}", got[0]);
    assert_eq!((got[1].sun, got[1].hemi), (0.0, 0.0));
    assert!(apart(&got[0], &want[0]) < 1e-5 && apart(&got[1], &want[1]) < 1e-5);
}

/// A ray along an axis whose origin lies on a box's face is inside that slab (Cordon's floors
/// leaked the sky through their corners until the card's box test said so too).
#[test]
fn a_ray_along_an_axis_through_a_corner_is_stopped() {
    let Some(gpu) = card() else { return };
    // flat triangles at height 1, each with a corner right above a sample
    let mut verts = Vec::new();
    let mut tris = Vec::new();
    let mut samples = Vec::new();
    for i in 0..16u32 {
        let c = Vec3::new(i as f32 * 3.0 + 0.25, 1.0, 2.5);
        let base = verts.len() as u32;
        verts.extend([c, c - Vec3::X, c + Vec3::Z]);
        tris.push([base, base + 1, base + 2]);
        // the offset along the normal keeps x and z: the ray up starts on the box's faces
        samples.push(Sample::new(Vec3::new(c.x, 0.0, c.z), Vec3::Y, LP_DEFAULT, None));
    }
    let faces = (0..16)
        .map(|i| RayFace {
            face: Some(FaceId(i)),
            cast_shadow: true,
            opaque: true,
            texture: 0,
            uv: [Vec2::ZERO; 3],
        })
        .collect();
    let model = RayModel::new(&verts, &tris, faces, Opacity::Squared, false).unwrap();
    let set = LightSet {
        rgb: vec![],
        hemi: vec![sky(Vec3::Y, 1.0)],
        sun: vec![sky(Vec3::new(0.0, 1.0, 0.0), 1.0)],
    };
    let job = LightJob { lights: &set, falloff: Falloff::Level { linear: false } };
    let want = cpu(&model, &[], &job, &samples);
    assert!(want.iter().all(|c| c.hemi == 0.0 && c.sun == 0.0), "the CPU sees the corners");
    let mut got = vec![ColorC::default(); samples.len()];
    gpu.scene(&model, &[]).unwrap().light(&job, &samples, &mut got, &Ctl::quiet()).unwrap();
    assert!(got.iter().all(|c| c.hemi == 0.0 && c.sun == 0.0), "{got:?}");
}

/// Rays straight up through the corners and edges of a sloped, skewed floor far from the origin
/// meet it as the CPU's do. (The kernel's unfused products, which keep such rays from slipping
/// between two triangles on Cordon, are not caught here: this card does not fuse these cases.)
#[test]
fn rays_through_shared_corners_and_edges_meet_the_floor() {
    let Some(gpu) = card() else { return };
    let (mut verts, mut tris, mut samples) = (Vec::new(), Vec::new(), Vec::new());
    let at = |x: u32, z: u32| {
        let (fx, fz) = (-317.13 + x as f32 * 0.731 + z as f32 * 0.0137, 512.71 + z as f32 * 0.677);
        Vec3::new(fx, 3.3 + fx * 0.013 - fz * 0.007, fz)
    };
    for z in 0..40 {
        for x in 0..40 {
            let base = verts.len() as u32;
            verts.extend([at(x, z), at(x + 1, z), at(x + 1, z + 1), at(x, z + 1)]);
            tris.push([base, base + 1, base + 2]);
            tris.push([base, base + 2, base + 3]);
            // under a corner, and under the middle of the diagonal the two triangles share
            let (c, m) = (at(x, z), (at(x, z) + at(x + 1, z + 1)) * 0.5);
            for q in [c, m] {
                samples.push(Sample::new(Vec3::new(q.x, q.y - 1.0, q.z), Vec3::Y, LP_DEFAULT, None));
            }
        }
    }
    let faces = (0..tris.len() as u32)
        .map(|i| RayFace {
            face: Some(FaceId(i)),
            cast_shadow: true,
            opaque: true,
            texture: 0,
            uv: [Vec2::ZERO; 3],
        })
        .collect();
    let model = RayModel::new(&verts, &tris, faces, Opacity::Squared, false).unwrap();
    let set = LightSet { rgb: vec![], hemi: vec![sky(Vec3::Y, 1.0)], sun: vec![sky(Vec3::Y, 1.0)] };
    let job = LightJob { lights: &set, falloff: Falloff::Level { linear: false } };
    let want = cpu(&model, &[], &job, &samples);
    let mut got = vec![ColorC::default(); samples.len()];
    gpu.scene(&model, &[]).unwrap().light(&job, &samples, &mut got, &Ctl::quiet()).unwrap();
    let parted = got.iter().zip(&want).filter(|(a, b)| apart(a, b) > 1e-5).count();
    assert_eq!(parted, 0, "{parted} of {} rays part from the CPU's", samples.len());
}

#[test]
fn an_empty_scene_is_lit_without_shadows() {
    let Some(gpu) = card() else { return };
    let model = RayModel::new(&[], &[], Vec::new(), Opacity::Squared, false).unwrap();
    let set = lights(9);
    let job = LightJob { lights: &set, falloff: Falloff::Level { linear: false } };
    let samples: Vec<Sample> = (0..300)
        .map(|i| Sample::new(Vec3::new((i % 20) as f32, 0.5, (i / 20) as f32), Vec3::Y, LP_DEFAULT, None))
        .collect();
    let want = cpu(&model, &[], &job, &samples);
    let mut got = vec![ColorC::default(); samples.len()];
    let scene = gpu.scene(&model, &[]).unwrap();
    scene.light(&job, &samples, &mut got, &Ctl::quiet()).unwrap();
    assert!(got.iter().zip(&want).all(|(a, b)| apart(a, b) < 1e-4));
    // no samples, no work
    scene.light(&job, &[], &mut [], &Ctl::quiet()).unwrap();
    assert!(scene.light(&job, &samples, &mut got[..3], &Ctl::quiet()).is_err());
}

#[test]
fn a_batch_larger_than_a_dispatch_is_cut_and_put_back_in_order() {
    let Some(gpu) = card() else { return };
    let level = Level::new(0x1357_9BDF);
    let tex = textures(1);
    let model = level.model(Opacity::Squared, false);
    let set = lights(2);
    let job = LightJob { lights: &set, falloff: Falloff::Level { linear: false } };
    // the level's samples many times over, each moved a little: some 40 000 points
    let mut r = Rng(77);
    let samples: Vec<Sample> = (0..40)
        .flat_map(|_| level.samples.to_vec())
        .map(|mut s| {
            s.p[0] += (r.f() - 0.5) * 0.2;
            s.p[2] += (r.f() - 0.5) * 0.2;
            s
        })
        .collect();
    assert!(samples.len() > 8 * 4096);
    let want = cpu(&model, &tex, &job, &samples);
    let mut got = vec![ColorC::default(); samples.len()];
    gpu.scene(&model, &tex).unwrap().light(&job, &samples, &mut got, &Ctl::quiet()).unwrap();
    assert_agrees(&got, &want, "many batches");
}

#[test]
fn a_stop_ends_the_light_within_a_second() {
    let Some(gpu) = card() else { return };
    let level = Level::new(0x0F0F_1234);
    let tex = textures(4);
    let model = level.model(Opacity::Squared, false);
    let mut set = lights(6);
    // a heavy sky: some 2 000 rays a sample
    set.hemi = (0..2000)
        .map(|k| sky(Vec3::new((k as f32).cos(), 0.3 + (k % 5) as f32 * 0.1, (k as f32).sin()), 1e-3))
        .collect();
    let job = LightJob { lights: &set, falloff: Falloff::Level { linear: false } };
    let samples: Vec<Sample> = level.samples.iter().copied().cycle().take(4_000_000).collect();
    let mut out = vec![ColorC::default(); samples.len()];
    let scene = gpu.scene(&model, &tex).unwrap();
    // stopped before it starts
    let stopped = Cancel::new();
    stopped.cancel();
    assert!(matches!(
        scene.light(&job, &samples, &mut out, &Ctl::with_cancel(&stopped)),
        Err(LcError::Cancelled)
    ));
    // stopped in the middle
    let cancel = Cancel::new();
    let stopper = cancel.clone();
    let t = Instant::now();
    let after = Duration::from_millis(300);
    let waiter = std::thread::spawn(move || {
        std::thread::sleep(after);
        stopper.cancel();
        Instant::now()
    });
    let got = scene.light(&job, &samples, &mut out, &Ctl::with_cancel(&cancel));
    let ended = Instant::now();
    let asked = waiter.join().unwrap();
    match got {
        Err(LcError::Cancelled) => {
            assert!(
                ended.duration_since(asked) < Duration::from_secs(1),
                "{:?} after the Stop",
                ended.duration_since(asked)
            );
        }
        other => panic!("{other:?} after {:?}: the light was not stopped", t.elapsed()),
    }
}

#[test]
fn alpha_larger_than_the_limit_is_made_smaller_with_a_note() {
    let Some(gpu) = card() else { return };
    let level = Level::new(0x00C0_FFEE);
    // pictures of one alpha each: made smaller they let the same light through
    let flat = |w: u32, h: u32, a: u32| BuildTexture {
        width: w,
        height: h,
        has_alpha: true,
        surface: Some(vec![a << 24; (w * h) as usize]),
        ..BuildTexture::default()
    };
    let tex = vec![BuildTexture::default(), flat(256, 256, 100), flat(128, 64, 200), BuildTexture::default()];
    let gpu = gpu.with_alpha_limit(16 * 1024);
    let model = level.model(Opacity::Squared, false);
    let set = lights(8);
    let job = LightJob { lights: &set, falloff: Falloff::Level { linear: false } };
    let want = cpu(&model, &tex, &job, &level.samples);
    let mut got = vec![ColorC::default(); level.samples.len()];
    gpu.scene(&model, &tex).unwrap().light(&job, &level.samples, &mut got, &Ctl::quiet()).unwrap();
    assert_agrees(&got, &want, "halved alpha");
    let notes = gpu.notes();
    assert!(notes.iter().any(|n| n.contains("made smaller")), "{notes:?}");
    assert!(gpu.notes().is_empty(), "a note is said once");
}

#[test]
fn the_choice_falls_back_to_the_cpu_without_a_card_and_takes_the_card_with_one() {
    let none = || Err("no graphics adapter.".to_owned());
    let auto = crate::choose(LightChoice::Auto, none).unwrap();
    assert_eq!(auto.lighter.name(), "the CPU");
    assert_eq!(auto.note.as_deref(), Some("The light is baked on the CPU: no graphics adapter."));
    assert_eq!(crate::choose(LightChoice::Gpu, none).err().as_deref(), Some("no graphics adapter."));
    assert_eq!(crate::lighter(LightChoice::Cpu).map(|c| c.lighter.name()), Ok("the CPU".to_owned()));
    assert_eq!(crate::lighter(LightChoice::Off).map(|c| c.lighter.name()), Ok("no light".to_owned()));
    let Some(gpu) = card() else {
        assert_eq!(crate::lighter(LightChoice::Auto).map(|c| c.lighter.name()), Ok("the CPU".to_owned()));
        return;
    };
    let name = gpu.name();
    drop(gpu);
    let auto = crate::lighter(LightChoice::Auto).unwrap();
    assert_eq!(auto.lighter.name(), name, "auto takes the card");
    assert!(auto.note.is_none());
    // a discrete card is taken before an integrated one
    let discrete: Vec<String> = real_adapters()
        .into_iter()
        .filter(|i| i.device_type == wgpu::DeviceType::DiscreteGpu)
        .map(|i| i.name)
        .collect();
    println!("the card: {name}; discrete adapters: {discrete:?}");
    if !discrete.is_empty() {
        assert!(discrete.contains(&name), "{name} is not one of {discrete:?}");
    }
}

fn desc_backends() -> wgpu::Backends {
    wgpu::Backends::DX12 | wgpu::Backends::VULKAN
}

/// The self-check takes the CPU's own answers and refuses answers that are off: it needs no card.
#[test]
fn a_card_that_answers_otherwise_is_refused_by_the_self_check() {
    let level = Level::new(0x5151_5151);
    let tex = textures(2);
    let model = level.model(Opacity::Squared, false);
    let set = lights(3);
    let job = LightJob { lights: &set, falloff: Falloff::Level { linear: false } };
    let right = cpu(&model, &tex, &job, &level.samples);
    crate::gpu::check_against_cpu(&model, &tex, &job, &level.samples, &right, &Ctl::quiet()).unwrap();
    // one sample in fifty off: still 98 % - refused
    let mut wrong = right.clone();
    wrong.iter_mut().step_by(50).for_each(|c| c.sun += 0.01);
    let refused = crate::gpu::check_against_cpu(&model, &tex, &job, &level.samples, &wrong, &Ctl::quiet());
    assert!(
        matches!(&refused, Err(LcError::Invalid(why)) if why.starts_with("the graphics card's answers differ")),
        "{refused:?}"
    );
}
