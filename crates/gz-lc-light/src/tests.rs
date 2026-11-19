//! Scenes small enough to reason about, lit the way the compiler lights a level.

use glam::{Vec2, Vec3};
use gz_fmt_level::{BuildMaterial, DetailHeader, DetailSlot};
use gz_fmt_tex::{TextureFlags, TextureParams};

use crate::*;

const SUN_DOWN: Vec3 = Vec3::NEG_Y;

fn params() -> BuildParams {
    BuildParams {
        sm_angle: 75.0,
        weld_distance: 0.005,
        lm_pixels_per_meter: 4.0,
        lm_jitter_samples: 4,
        lm_rms_zero: 4,
        lm_rms: 4,
        quality: 1,
        reserved: 0,
        f_reserved: [0.0; 6],
    }
}

fn shader(flags: u32) -> ShaderXrlc {
    ShaderXrlc { flags, ..default_shader() }
}

/// Materials: 0 opaque, 1 alpha-tested, 2 vertex-lit, 3 implicitly lighted.
fn data() -> (Shaders, Vec<BuildMaterial>, Vec<BuildTexture>) {
    let base = ShaderXrlc::COLLISION | ShaderXrlc::RENDERING | ShaderXrlc::LIGHT_CAST_SHADOW;
    let shaders = Shaders::new(vec![shader(base), shader(base | ShaderXrlc::LIGHT_VERTEX)]);
    let m = |surface: u16, reserved: u16| BuildMaterial { surface, reserved, ..BuildMaterial::default() };
    let materials = vec![m(0, 0), m(1, 0), m(0, 1), m(2, 0)];
    let opaque = BuildTexture { name: "opaque".into(), width: 4, height: 4, ..BuildTexture::default() };
    // alpha 128 everywhere: a ray keeps 1 - (128 / 255)^2 of its light
    let alpha = BuildTexture {
        name: "alpha".into(),
        width: 2,
        height: 2,
        has_alpha: true,
        has_surface: true,
        surface: Some(vec![0x8080_8080; 4]),
        ..BuildTexture::default()
    };
    let mut thm = TextureParams::default();
    thm.flags |= TextureFlags::IMPLICIT_LIGHTED;
    let implicit = BuildTexture {
        name: "terrain".into(),
        width: 4,
        height: 4,
        has_surface: true,
        surface: Some(vec![0xff10_2030; 16]),
        thm,
        ..BuildTexture::default()
    };
    (shaders, materials, vec![opaque, alpha, implicit])
}

/// A quad of two faces over `c` (corners in order), its texture coordinates the unit square.
fn quad(mesh: &mut Mesh, c: [Vec3; 4], material: u16) -> [FaceId; 2] {
    let v: Vec<VertId> = c.iter().map(|&p| mesh.create_vertex(p)).collect();
    let uv = [Vec2::ZERO, Vec2::Y, Vec2::ONE, Vec2::X];
    let a = mesh.create_face([v[0], v[1], v[2]]);
    let b = mesh.create_face([v[0], v[2], v[3]]);
    for (f, k) in [(a, [0, 1, 2]), (b, [0, 2, 3])] {
        let face = mesh.face_mut(f).unwrap();
        face.material = material;
        face.add_channel(uv[k[0]], uv[k[1]], uv[k[2]]);
        mesh.calc_normal(f);
    }
    [a, b]
}

/// A horizontal quad at height `y` from `(x0, z0)` to `(x1, z1)`, facing up.
fn floor(mesh: &mut Mesh, y: f32, (x0, z0): (f32, f32), (x1, z1): (f32, f32), material: u16) -> [FaceId; 2] {
    quad(
        mesh,
        [Vec3::new(x0, y, z0), Vec3::new(x0, y, z1), Vec3::new(x1, y, z1), Vec3::new(x1, y, z0)],
        material,
    )
}

/// The same facing down.
fn ceiling(
    mesh: &mut Mesh,
    y: f32,
    (x0, z0): (f32, f32),
    (x1, z1): (f32, f32),
    material: u16,
) -> [FaceId; 2] {
    quad(
        mesh,
        [Vec3::new(x0, y, z0), Vec3::new(x1, y, z0), Vec3::new(x1, y, z1), Vec3::new(x0, y, z1)],
        material,
    )
}

fn direct(dir: Vec3, energy: f32) -> RLight {
    RLight { kind: LT_DIRECT, direction: dir, diffuse: Vec3::ONE, energy, ..RLight::default() }
}

fn point_light(at: Vec3, range: f32) -> RLight {
    RLight::from_flight(&gz_fmt_level::Light {
        kind: 1,
        diffuse: [1.0, 1.0, 1.0, 1.0],
        position: at,
        range,
        attenuation0: 1.0,
        ..Default::default()
    })
}

fn global(lights: LightSet) -> LcGlobalData {
    let (shaders, materials, textures) = data();
    LcGlobalData::new(params(), shaders, materials, textures, lights)
}

/// One deflector over the faces, as the UV mapping makes it.
fn deflect(g: &mut LcGlobalData, start: FaceId) -> DeflId {
    let id = g.deflectors.create();
    let n = g.mesh.face(start).unwrap().n;
    let d = g.deflectors.get_mut(id).unwrap();
    d.oa_set_normal(n);
    d.oa_unwarp(id, &mut g.mesh, start, g.params.sm_angle);
    d.oa_export(&mut g.mesh, &g.shaders, &g.materials, &g.params);
    id
}

/// What a test lights a point through: the level's model, its falloff.
fn tracer<'a>(env: &LightEnv<'a>) -> Tracer<'a> {
    Tracer { model: env.model, textures: env.textures, falloff: Falloff::Level { linear: env.linear } }
}

/// The detail light on the CPU.
fn details(scene: &DetailScene, slots: &mut [DetailSlot], only: Option<[f32; 3]>) -> Result<DetailStats> {
    let cpu = CpuLighter::default();
    let lit = cpu.scene(&scene.model, &scene.textures)?;
    light_details(scene, slots, only, lit.as_ref(), &Ctl::quiet())
}

fn finish_mesh(g: &mut LcGlobalData) {
    g.calc_normals(Smoothing::Angle, &Ctl::quiet()).unwrap();
    g.light_prepare();
}

#[test]
fn a_spot_light_lights_what_is_in_its_cone_as_a_point_light_would() {
    // 4 m above the floor, looking straight down, a cone of 60 degrees: 2.31 m of floor around
    // the point under it
    let spot = RLight::from_flight(&gz_fmt_level::Light {
        kind: 2,
        diffuse: [1.0, 1.0, 1.0, 1.0],
        position: Vec3::new(5.0, 4.0, 5.0),
        direction: Vec3::NEG_Y,
        range: 10.0,
        attenuation0: 1.0,
        phi: 60.0f32.to_radians(),
        ..Default::default()
    });
    let lamp = point_light(Vec3::new(5.0, 4.0, 5.0), 10.0);
    let lit = |light: RLight, p: Vec3| {
        let mut g = global(LightSet { rgb: vec![light], hemi: vec![], sun: vec![] });
        floor(&mut g.mesh, 0.0, (0.0, 0.0), (10.0, 10.0), 0);
        finish_mesh(&mut g);
        g.build_rapid(&g.scene_bounds()).unwrap();
        let env = g.light_env().unwrap();
        let t = tracer(&env);
        let mut c = ColorC::default();
        light_point(&t, &mut LightCache::default(), &mut c, p, Vec3::Y, env.lights, LP_DEFAULT, None);
        c.rgb.x
    };
    // under the light: all of the point light
    let under = Vec3::new(5.0, 0.0, 5.0);
    assert_eq!(lit(spot, under), lit(lamp, under));
    assert!(lit(spot, under) > 0.0);
    // inside the cone, off its axis: a share of it; outside: nothing, where the lamp still shines
    let inside = Vec3::new(6.2, 0.0, 5.0);
    let share = lit(spot, inside) / lit(lamp, inside);
    assert!(share > 0.3 && share < 1.0, "{share}");
    let outside = Vec3::new(8.5, 0.0, 5.0);
    assert_eq!(lit(spot, outside), 0.0);
    assert!(lit(lamp, outside) > 0.0);
}

#[test]
fn a_point_with_a_free_sky_and_under_an_occluder() {
    // a point light 2 m above the point: from_flight makes range 11, energy 1, falloff 1/11
    let lights = LightSet {
        rgb: vec![point_light(Vec3::new(0.3, 2.0, 0.6), 10.0)],
        hemi: vec![],
        sun: vec![direct(SUN_DOWN, 1.0)],
    };
    let mut g = global(lights);
    floor(&mut g.mesh, 0.0, (0.0, 0.0), (1.0, 1.0), 0);
    let occ = ceiling(&mut g.mesh, 1.0, (0.0, 0.0), (1.0, 1.0), 1);
    finish_mesh(&mut g);
    g.build_rapid(&g.scene_bounds()).unwrap();
    let env = g.light_env().unwrap();
    let t = tracer(&env);
    let mut cache = LightCache::default();

    // through the alpha-tested occluder (alpha 128):
    //   rayTrace = 1 - (128/255)^2 = 0.748_097
    //   rgb: D = 1, R = 2: A = scale * (1 / (att0 + att1 R + att2 R^2) - R falloff)
    //        = 0.748_097 * (1 - 2/11)
    //   sun: energy * rayTrace = 0.748_097
    let a = 128.0f32 / 255.0;
    let through = 1.0 - a * a;
    let mut c = ColorC::default();
    // off the quads' diagonal, which a ray along it would meet twice
    let p = Vec3::new(0.3, 0.0, 0.6);
    light_point(&t, &mut cache, &mut c, p, Vec3::Y, env.lights, LP_DEFAULT, None);
    let want_rgb = through * (1.0 / 1.0 - 2.0 * (1.0 / 11.0));
    assert!((c.rgb.x - want_rgb).abs() < 1e-6, "{} {}", c.rgb.x, want_rgb);
    assert!((c.sun - through).abs() < 1e-6);

    // the occluder made opaque stops both
    let mut g2 = g.clone();
    for f in occ {
        g2.mesh.face_mut(f).unwrap().material = 0;
    }
    g2.light_prepare();
    g2.build_rapid(&g2.scene_bounds()).unwrap();
    let env2 = g2.light_env().unwrap();
    let mut c = ColorC::default();
    light_point(
        &tracer(&env2),
        &mut LightCache::default(),
        &mut c,
        p,
        Vec3::Y,
        env2.lights,
        LP_DEFAULT,
        None,
    );
    assert_eq!((c.rgb, c.sun), (Vec3::ZERO, 0.0));

    // beside the occluder: the attenuation to the light, D (1 - R / 11), and the sun
    let mut c = ColorC::default();
    let beside = Vec3::new(3.0, 0.0, 0.5);
    light_point(
        &tracer(&env2),
        &mut LightCache::default(),
        &mut c,
        beside,
        Vec3::Y,
        env2.lights,
        LP_DEFAULT,
        None,
    );
    let r = (Vec3::new(0.3, 2.0, 0.6) - beside).length();
    let d = (Vec3::new(0.3, 2.0, 0.6) - beside).normalize().y;
    let want = d * (1.0 / 1.0 - r * (1.0 / 11.0));
    assert!((c.rgb.x - want).abs() < 1e-5, "{} {want}", c.rgb.x);
    assert_eq!(c.sun, 1.0);
}

#[test]
fn the_hemisphere_counts_the_lights_above_the_surface() {
    // eight hemisphere lights of 1/8: five come from above (their direction points down)
    let dirs = [
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(0.6, -0.8, 0.0),
        Vec3::new(-0.6, -0.8, 0.0),
        Vec3::new(0.0, -0.8, 0.6),
        Vec3::new(0.0, -0.8, -0.6),
        Vec3::new(0.6, 0.8, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 0.8, -0.6),
    ];
    let hemi: Vec<RLight> = dirs.iter().map(|&d| direct(d, 1.0 / 8.0)).collect();
    let mut g = global(LightSet { rgb: vec![], hemi, sun: vec![] });
    floor(&mut g.mesh, 0.0, (0.0, 0.0), (1.0, 1.0), 0);
    finish_mesh(&mut g);
    g.build_rapid(&g.scene_bounds()).unwrap();
    let env = g.light_env().unwrap();
    let mut c = ColorC::default();
    light_point(
        &tracer(&env),
        &mut LightCache::default(),
        &mut c,
        Vec3::new(0.5, 0.0, 0.5),
        Vec3::Y,
        env.lights,
        LP_DEFAULT,
        None,
    );
    // the hemisphere's direct lights add their energy, not weighted by the cosine
    assert_eq!(c.hemi, 5.0 / 8.0);
    // and an MU model is lit at six points, one at the draft quality
    assert_eq!(c.sun, 0.0);
}

/// A 2 x 2 m floor whose half `x < 1` lies under an opaque roof, lit by the sun and a sky
/// light from straight above.
fn half_shadowed() -> (LcGlobalData, DeflId) {
    let lights =
        LightSet { rgb: vec![], hemi: vec![direct(SUN_DOWN, 0.5)], sun: vec![direct(SUN_DOWN, 1.0)] };
    let mut g = global(lights);
    let f = floor(&mut g.mesh, 0.0, (0.0, 0.0), (2.0, 2.0), 0);
    ceiling(&mut g.mesh, 1.0, (-1.0, -1.0), (1.0, 3.0), 0);
    finish_mesh(&mut g);
    let d = deflect(&mut g, f[0]);
    g.build_rapid(&g.scene_bounds()).unwrap();
    (g, d)
}

#[test]
fn a_lit_and_a_shadowed_texel() {
    let (mut g, d) = half_shadowed();
    // OA_Export: projected along +y the floor is 2 x 2 in texture space: iCeil(2 * 4 + .5) = 9
    let layer = &g.deflectors.get(d).unwrap().layer;
    assert_eq!((layer.width, layer.height), (9, 9));
    g.light_deflectors(&CpuLighter::default(), &Ctl::quiet()).unwrap();
    let defl = g.deflectors.get(d).unwrap();
    let lm = &defl.layer;
    // the light changes along the floor's x, the projection's v (OA_Export: up = +x); the
    // texels on the faces' shared edge and at the ends of the shadow's edge differ, so neither
    // side passes compress_RMS here, and the layer is grown by its border
    assert_eq!((lm.width, lm.height), (9, 9));
    let (w, h) = (lm.width as usize, lm.height as usize);
    assert_eq!(lm.surface.len(), (w + 2) * (h + 2));
    // the border is filled: no texel is left unmarked
    assert!(lm.marker.iter().all(|&m| m > 0));

    // row v of the texels lies at x = v / 4 (RemapUV: the texel centres at the edges of 0..2).
    // A lit texel: four samples of sun 1 and sky 0.5, the mean halved (L_Direct):
    //   sun = 0.5 -> floor(0.5 * 32767 / 16) = 1023, hemi = 0.25 -> 511
    // a shadowed one (x + 0.111 of the jitter < 1): 0 and 0
    for v in 0..h {
        for u in 0..w {
            let c = lm.surface[(v + 1) * (w + 2) + u + 1];
            if v <= 3 {
                assert_eq!((c.s, c.h), (0, 0), "texel {u}, {v}");
            }
            if v >= 5 {
                assert_eq!((c.s, c.h), (1023, 511), "texel {u}, {v}");
            }
        }
    }
    // on the shadow's edge two samples of four are lit
    assert_eq!(lm.surface[5 * (w + 2) + 2].s, 511);
}

#[test]
fn a_free_floor_compresses_to_one_colour_and_packs_into_a_page() {
    let lights =
        LightSet { rgb: vec![], hemi: vec![direct(SUN_DOWN, 0.5)], sun: vec![direct(SUN_DOWN, 1.0)] };
    let mut g = global(lights);
    let f = floor(&mut g.mesh, 0.0, (0.0, 0.0), (2.0, 2.0), 0);
    finish_mesh(&mut g);
    let d = deflect(&mut g, f[0]);
    g.build_rapid(&g.scene_bounds()).unwrap();
    g.light_deflectors(&CpuLighter::default(), &Ctl::quiet()).unwrap();
    let lm = &g.deflectors.get(d).unwrap().layer;
    // compress_Zero: 2 x 2 texels of the mean, the size 0
    assert_eq!((lm.width, lm.height, lm.surface.len()), (0, 0, 4));
    assert!(lm.surface.iter().all(|c| c.s == 1023 && c.h == 511));

    let pages = g.merge_lightmaps(&Ctl::quiet()).unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].name, "lmap#1");
    assert_eq!(g.lightmaps.len(), 1);
    assert!(g.deflectors.is_empty());
    for face in f {
        let face = g.mesh.face(face).unwrap();
        assert_eq!(face.lmap, Some(0));
        assert_eq!(face.tc.len(), 2);
    }
    // the base page: the sun in alpha, u8_clr(0.49954) = 127, at the corner where it was placed
    let base = &pages[0].base;
    assert_eq!((base.width, base.height), (1024, 1024));
    assert_eq!(base.rgba[3], 127);
    // the hemisphere page: u8_clr(0.24951) = 63
    assert_eq!(pages[0].hemi.rgba[3], 63);
    // the border grows 16 rings around the 2 x 2 placed texels; far away nothing
    assert_eq!(base.rgba[(17 * 1024 + 17) * 4 + 3], 127);
    assert_eq!(base.rgba[(20 * 1024 + 20) * 4 + 3], 0);
    let (b1, b2) = pages[0].dds().unwrap();
    assert_eq!(&b1[..4], b"DDS ");
    assert_eq!(b2.len(), b1.len());
    assert!(pages[0].debug_tga().unwrap().len() > 1024 * 2048 * 4);
}

#[test]
fn vertex_light_on_a_two_triangle_floor() {
    let lights = LightSet {
        rgb: vec![point_light(Vec3::new(1.0, 2.0, 1.0), 10.0)],
        hemi: vec![],
        sun: vec![direct(SUN_DOWN, 1.0)],
    };
    let mut g = global(lights);
    floor(&mut g.mesh, 0.0, (0.0, 0.0), (2.0, 2.0), 2);
    finish_mesh(&mut g);
    // a hemisphere found before is kept
    for &v in g.mesh.vertices().to_vec().iter() {
        g.mesh.vertex_mut(v).unwrap().c = BaseColor::from_c(&ColorC { hemi: 0.3, ..ColorC::default() });
    }
    g.build_rapid(&g.scene_bounds()).unwrap();
    g.light_vertices(&CpuLighter::default(), &Ctl::quiet()).unwrap();
    for &v in g.mesh.vertices() {
        let vx = g.mesh.vertex(v).unwrap();
        // a corner: R = sqrt(1 + 4 + 1), D = 2 / R; A = D (1 - R / 11); the light halved
        let to = Vec3::new(1.0, 2.0, 1.0) - vx.p;
        let r = to.length();
        let d = to.y / r;
        let rgb = 0.5 * (d * (1.0 - r * (1.0 / 11.0)));
        let c = vx.c.get();
        assert!((c.rgb.x - rgb).abs() < 1e-3, "{} {rgb}", c.rgb.x);
        assert_eq!(vx.c.s, 1023);
        assert_eq!(vx.c.h, BaseColor::from_c(&ColorC { hemi: 0.3, ..ColorC::default() }).h);
        // the translucency was mixed away: the group of one point keeps its own light
        assert_eq!(vx.c.t, 0);
    }
}

#[test]
fn implicit_light_puts_the_sky_into_the_texture() {
    let lights =
        LightSet { rgb: vec![], hemi: vec![direct(SUN_DOWN, 0.5)], sun: vec![direct(SUN_DOWN, 1.0)] };
    let mut g = global(lights);
    let f = floor(&mut g.mesh, 0.0, (0.0, 0.0), (2.0, 2.0), 3);
    finish_mesh(&mut g);
    g.build_rapid(&g.scene_bounds()).unwrap();
    let maps = g.implicit_lighting(&CpuLighter::default(), &Ctl::quiet()).unwrap();
    assert_eq!(maps.len(), 1);
    let m = &maps[0];
    assert_eq!((m.texture, m.name.as_str(), m.base.width), (2, "terrain", 4));
    // every texel: sky 0.5 halved = 0.25, stored as 511, read back 0.249_52, u8_clr -> 63;
    // the colour of the texture is kept
    for px in m.base.rgba.as_chunks::<4>().0 {
        assert_eq!(*px, [0x10, 0x20, 0x30, 63]);
    }
    // the lightmap holds the sun in alpha: 0.5 -> 127
    assert!(m.lmap.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 127));
    assert_eq!(g.textures[2].surface.as_ref().unwrap()[0] >> 24, 63);
    for face in f {
        assert_eq!(g.mesh.face(face).unwrap().tc.len(), 2);
    }
    // at the draft quality nothing is done
    g.params.quality = QUALITY_DRAFT;
    assert!(g.implicit_lighting(&CpuLighter::default(), &Ctl::quiet()).unwrap().is_empty());
}

#[test]
fn a_face_that_keeps_the_levels_light_is_not_lit_again() {
    let lights =
        LightSet { rgb: vec![], hemi: vec![direct(SUN_DOWN, 0.5)], sun: vec![direct(SUN_DOWN, 1.0)] };
    let mut g = global(lights);
    let f = floor(&mut g.mesh, 0.0, (0.0, 0.0), (2.0, 2.0), 3);
    finish_mesh(&mut g);
    g.materials[3].internal_max_area |= crate::KEEPS_LEVEL_LIGHT;
    assert!(crate::keeps_level_light(3, &g.materials) && !crate::keeps_level_light(2, &g.materials));
    g.build_rapid(&g.scene_bounds()).unwrap();
    let before = g.textures[2].surface.clone();
    // no map, so no copy of the texture or its light; the faces read the level's pair
    assert!(g.implicit_lighting(&CpuLighter::default(), &Ctl::quiet()).unwrap().is_empty());
    assert_eq!(g.textures[2].surface, before);
    for face in f {
        let face = g.mesh.face(face).unwrap();
        assert_eq!(face.tc.len(), 2);
        assert_eq!(face.tc[1].uv, face.tc[0].uv);
        assert!(g.has_implicit_lighting(g.mesh.faces()[0]));
    }
}

fn mu_scene() -> LcGlobalData {
    let lights =
        LightSet { rgb: vec![], hemi: vec![direct(SUN_DOWN, 0.5)], sun: vec![direct(SUN_DOWN, 1.0)] };
    let mut g = global(lights);
    floor(&mut g.mesh, 0.0, (0.0, 0.0), (2.0, 2.0), 0);
    let src = gz_fmt_level::MuModel {
        name: "crate".into(),
        verts: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
        ],
        faces: vec![
            gz_fmt_level::BuildFace {
                v: [0, 1, 2],
                t: [Vec2::ZERO, Vec2::Y, Vec2::ONE],
                material: 0,
                material_game: 0,
            },
            gz_fmt_level::BuildFace {
                v: [0, 2, 3],
                t: [Vec2::ZERO, Vec2::ONE, Vec2::X],
                material: 0,
                material_game: 0,
            },
        ],
        lod_id: u16::MAX,
        smooth_groups: vec![0, 0],
    };
    let mut m = MuModel::load(&src).unwrap();
    m.calc_normals(75.0, Smoothing::Angle);
    g.mu_models.push(m);
    let mut r = MuReference::load(&gz_fmt_level::MuReference::default());
    r.xform = math::Matrix::from_rows([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [10.0, 0.0, 10.0, 1.0],
    ]);
    g.mu_refs.push(r);
    finish_mesh(&mut g);
    g
}

#[test]
fn an_mu_model_and_its_reference() {
    let mut g = mu_scene();
    g.build_rapid(&g.scene_bounds()).unwrap();
    // the reference's two faces joined the ray model after the floor's
    assert_eq!(g.ray.as_ref().unwrap().len(), 4);
    g.light_mu(&CpuLighter::default(), &Ctl::quiet()).unwrap();
    let m = &g.mu_models[0];
    assert_eq!(m.subdivs.len(), 1);
    // own light: the sky alone (0.5 at each of six samples), the translucency mix halves it
    for c in &m.color {
        let c = c.get();
        assert!((c.hemi - 0.25).abs() < 1e-3, "{c:?}");
        assert_eq!(c.sun, 0.0);
    }
    let r = &g.mu_refs[0];
    assert_eq!(r.color.len(), 4);
    for c in &r.color {
        let c = c.get();
        assert!((c.sun - 0.5).abs() < 1e-3 && (c.hemi - 0.25).abs() < 1e-3, "{c:?}");
    }
    // a flat model light leaves no slope to fit: the scale is 0 and the bias the mean
    assert!(r.c_bias.sun > 0.49 && r.c_bias.sun < 0.51, "{:?}", r.c_bias);
    assert!(r.c_bias.hemi > 0.24 && r.c_bias.hemi < 0.26);
    assert_eq!(r.c_scale.sun, 0.0);
}

/// A 2 x 2 slot grid over a floor, the slot (1, 1) under a roof.
fn detail_scene() -> (DetailScene, Vec<DetailSlot>) {
    let lights =
        LightSet { rgb: vec![], hemi: vec![direct(SUN_DOWN, 1.0)], sun: vec![direct(SUN_DOWN, 1.0)] };
    let mut mesh = Mesh::new();
    floor(&mut mesh, 0.0, (0.0, 0.0), (4.0, 4.0), 0);
    ceiling(&mut mesh, 10.0, (2.0, 2.0), (4.0, 4.0), 0);
    let (shaders, materials, textures) = data();
    let mut form = RcastForm::default();
    for &f in mesh.faces() {
        let p = mesh.positions(f).unwrap();
        let base = form.verts.len() as u32;
        form.verts.extend_from_slice(&p);
        form.tris.push([base, base + 1, base + 2]);
        form.faces.push(RcFace { material: 0, ..RcFace::default() });
    }
    let header = DetailHeader { version: 3, object_count: 1, offs_x: 0, offs_z: 0, size_x: 2, size_z: 2 };
    let scene = DetailScene::new(&form, shaders, materials, textures, lights, header).unwrap();
    let mut slot = DetailSlot::default();
    slot.set_heights(0.0, 1.0);
    for part in 0..4 {
        slot.set_id(part, DetailSlot::ID_EMPTY);
    }
    slot.set_id(0, 0);
    for corner in 0..4 {
        slot.set_density(0, corner, 8);
    }
    (scene, vec![slot; 4])
}

#[test]
fn a_detail_slot_is_lit_at_its_points_and_its_corners_decide_its_objects() {
    let (scene, mut slots) = detail_scene();
    // a part with no density in any of its four corners holds no object
    let mut s = slots[0];
    s.set_id(1, 5);
    s.set_density(1, 2, 0);
    process_palette(&mut s);
    assert_eq!(s.id(1), DetailSlot::ID_EMPTY);
    s.set_id(1, 5);
    s.set_density(1, 3, 1);
    process_palette(&mut s);
    assert_eq!(s.id(1), 5);
    assert_eq!(s.id(0), 0);

    let stats = details(&scene, &mut slots, None).unwrap();
    assert_eq!(stats, DetailStats { lit: 4, empty: 0, no_ground: 0 });
    // the free slots: sun 1 and sky 1 at each of the 225 points, the mean halved: 0.5 is
    // floor(7.5) = 7 of 15; the roofed one: 0. The two beside the roof have their row of 15
    // points on the roof's edge: a vertical ray there meets the edge (its origin on the roof's
    // box face along a still axis, as X-Ray's slab test takes it), 210 of 225 lit: 6
    assert_eq!(slots[0].colors(), [7, 7, 0, 0, 0]);
    assert_eq!(slots[1].colors(), [6, 6, 0, 0, 0]);
    assert_eq!(slots[2].colors(), [6, 6, 0, 0, 0]);
    assert_eq!(slots[3].colors(), [0, 0, 0, 0, 0]);
    // a square that holds only the first slot lights only that one
    let (scene, mut slots) = detail_scene();
    let stats = details(&scene, &mut slots, Some([1.0, 1.0, 0.5])).unwrap();
    assert_eq!(stats.lit, 1);
}

/// A scene with every kind of light and occluder, compiled whole.
fn full_scene() -> LcGlobalData {
    let mut hemi = Vec::new();
    for i in 0..12 {
        let a = i as f32 * 0.52;
        hemi.push(direct(Vec3::new(a.cos() * 0.6, -0.8, a.sin() * 0.6), 1.0 / 12.0));
    }
    let lights = LightSet {
        rgb: vec![point_light(Vec3::new(1.5, 1.5, 1.0), 6.0), point_light(Vec3::new(-1.0, 0.5, 3.0), 4.0)],
        hemi,
        sun: vec![direct(Vec3::new(0.3, -0.9, 0.3).normalize(), 1.0)],
    };
    let mut g = global(lights);
    let f = floor(&mut g.mesh, 0.0, (0.0, 0.0), (3.0, 3.0), 0);
    let wall = quad(
        &mut g.mesh,
        [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 1.5, 0.0),
            Vec3::new(0.0, 1.5, 3.0),
            Vec3::new(0.0, 0.0, 3.0),
        ],
        0,
    );
    ceiling(&mut g.mesh, 1.0, (0.5, 0.5), (1.5, 2.0), 0);
    ceiling(&mut g.mesh, 0.7, (1.8, 0.2), (2.6, 1.5), 1);
    floor(&mut g.mesh, 0.0, (4.0, 0.0), (5.0, 1.0), 2);
    floor(&mut g.mesh, 0.0, (6.0, 0.0), (7.0, 1.0), 3);
    let mu = mu_scene();
    g.mu_models = mu.mu_models;
    g.mu_refs = mu.mu_refs;
    g.mu_refs[0].xform.m[3] = [2.0, 0.2, 2.2, 1.0];
    finish_mesh(&mut g);
    deflect(&mut g, f[0]);
    deflect(&mut g, wall[0]);
    g
}

/// Everything a compile makes, as bytes.
fn compile(threads: usize) -> Vec<u8> {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    pool.install(|| {
        let mut g = full_scene();
        let form = g.build_rapid(&g.scene_bounds()).unwrap();
        g.light_mu(&CpuLighter::default(), &Ctl::quiet()).unwrap();
        let out = g.light(&CpuLighter::default(), &Ctl::quiet()).unwrap();
        let mut bytes = form.write();
        for p in &out.pages {
            bytes.extend_from_slice(&p.base.rgba);
            bytes.extend_from_slice(&p.hemi.rgba);
        }
        for m in &out.implicit {
            bytes.extend_from_slice(&m.base.rgba);
            bytes.extend_from_slice(&m.lmap.rgba);
        }
        for &v in g.mesh.vertices() {
            bytes.extend_from_slice(&g.mesh.vertex(v).unwrap().c.to_bytes());
        }
        for &f in g.mesh.faces() {
            for t in &g.mesh.face(f).unwrap().tc {
                for p in t.uv {
                    bytes.extend_from_slice(&p.x.to_le_bytes());
                    bytes.extend_from_slice(&p.y.to_le_bytes());
                }
            }
        }
        for m in &g.mu_models {
            m.color.iter().for_each(|c| bytes.extend_from_slice(&c.to_bytes()));
        }
        for r in &g.mu_refs {
            r.color.iter().for_each(|c| bytes.extend_from_slice(&c.to_bytes()));
            for i in 0..5 {
                bytes.extend_from_slice(&r.c_scale.channel(i).to_le_bytes());
                bytes.extend_from_slice(&r.c_bias.channel(i).to_le_bytes());
            }
        }
        let (scene, mut slots) = detail_scene();
        details(&scene, &mut slots, None).unwrap();
        slots.iter().for_each(|s| bytes.extend_from_slice(&s.word1.to_le_bytes()));
        bytes
    })
}

#[test]
fn the_same_light_on_one_thread_and_on_eight() {
    let one = compile(1);
    let eight = compile(8);
    assert!(one.len() > 2 * 1024 * 1024 * 4);
    assert!(one == eight, "the compile differs between 1 and 8 threads");
}

#[test]
fn a_stopped_compile_says_so() {
    let mut g = full_scene();
    g.build_rapid(&g.scene_bounds()).unwrap();
    let cancel = gz_task::Cancel::new();
    cancel.cancel();
    let ctl = Ctl::with_cancel(&cancel);
    assert!(matches!(g.light_deflectors(&CpuLighter::default(), &ctl), Err(LcError::Cancelled)));
    // the phase's name is the compiler's: the deflectors say only how far they are
    let seen = std::sync::Mutex::new((Vec::new(), 0usize, 0usize));
    let report = |r: Report<'_>| {
        let mut s = seen.lock().unwrap();
        match r {
            Report::Stage(t) => s.0.push(t.to_string()),
            Report::Progress { done, total } => {
                s.1 = s.1.max(done);
                s.2 = total;
            }
        }
    };
    let quiet = gz_task::Cancel::new();
    g.light_deflectors(&CpuLighter::default(), &Ctl::new(&quiet, &report)).unwrap();
    let (stages, done, total) = seen.into_inner().unwrap();
    assert!(stages.is_empty(), "{stages:?}");
    assert!(total > 0 && done == total, "{done} of {total}");
}

// ---- the baker against light_point -----------------------------------------------------------------
//
// What each consumer lit before it went through the baker, kept here as the reference: the loops
// as they were, `light_point` called into one colour sample after sample, the lights of a sphere
// selected. The baker lights every sample from nothing and the consumer sums them in their order,
// so a value of several samples may differ by the rounding of the sums - one step of the 16-bit
// fixed point at most -, and a value of one sample (a vertex, an edge texel, a texel at one
// jitter sample) is the same bits.

/// The largest difference of two packed colours, in steps of the fixed point.
fn apart(a: &BaseColor, b: &BaseColor) -> i32 {
    [(a.r, b.r), (a.g, b.g), (a.b, b.b), (a.h, b.h), (a.s, b.s), (a.t, b.t)]
        .iter()
        .map(|&(x, y)| (i32::from(x) - i32::from(y)).abs())
        .max()
        .unwrap_or(0)
}

/// The old `l_calculate` + `l_direct` of one deflector, rows and edges, before the borders.
fn old_chart(env: &LightEnv<'_>, d: &Deflector) -> LmLayer {
    let mut d = d.clone();
    d.take_sphere(env.mesh);
    let lights = env.lights.select(d.sphere_p, d.sphere_r);
    let (w, h) = (d.layer.width, d.layer.height);
    d.remap_uv(0, 0, w, h, w, h, false);
    let boxes: Vec<math::Box2> = d.uv_polys.iter().map(|t| math::Box2::of_tri(&t.uv)).collect();
    let hash = Hash2D::build(128, 128, d.bounds_summary(), &boxes);
    let mut layer = LmLayer::new(w, h);
    let (w, h) = (w as usize, h as usize);
    let t = tracer(env);
    let mut cache = LightCache::default();
    let flags = env.sun_flag() | LP_USE_FACE_DISABLE;
    let dim = Vec2::new(w as f32, h as f32);
    let half = Vec2::new(0.5 / dim.x, 0.5 / dim.y);
    let js = Vec2::new(0.4999 / dim.x, 0.4999 / dim.y);
    let world = |f: FaceId| {
        let face = env.mesh.face(f)?;
        Some((env.mesh.positions(f)?, env.mesh.normals(f)?, face.n))
    };
    for v in 0..h {
        for u in 0..w {
            let mut c = ColorC::default();
            let mut count = 0u32;
            for j in jitter(env.params.lm_jitter_samples) {
                let p =
                    Vec2::new(u as f32 / dim.x + half.x + j.x * js.x, v as f32 / dim.y + half.y + j.y * js.y);
                for &i in hash.query(p.x, p.y) {
                    let tri = &d.uv_polys[i as usize];
                    let Some(b) = (Tcf { uv: tri.uv }).inside(p) else { continue };
                    let Some((wp, wn, fnrm)) = world(tri.owner) else { break };
                    let wp = math::from_bary(wp[0], wp[1], wp[2], b);
                    let (wn, _) = math::exact_normalize(math::from_bary(wn[0], wn[1], wn[2], b));
                    let (wn, _) = math::exact_normalize(wn + fnrm);
                    light_point(&t, &mut cache, &mut c, wp, wn, &lights, flags, Some(tri.owner));
                    count += 1;
                    break;
                }
            }
            if count > 0 {
                c.scale(count);
                c.mul(0.5);
                layer.marker[v * w + u] = 255;
            }
            layer.surface[v * w + u] = BaseColor::from_c(&c);
        }
    }
    let texel = (1.0 / w.max(h) as f32) / 8.0;
    for tri in &d.uv_polys {
        let Some((p, _, fnrm)) = world(tri.owner) else { continue };
        for e in 0..3 {
            let (a, b) = (e, (e + 1) % 3);
            let (p1, p2) = (tri.uv[a], tri.uv[b]);
            let (v1, v2) = (p[a], p[b]);
            let size = p2 - p1;
            let steps = math::i_ceil(size.x.abs() / texel).max(math::i_ceil(size.y.abs() / texel));
            if steps <= 0 {
                continue;
            }
            for i in 0..=steps {
                let time = i as f32 / steps as f32;
                let uv = Vec2::new(size.x * time + p1.x, size.y * time + p1.y);
                let (x, y) = (math::i_floor(uv.x * w as f32), math::i_floor(uv.y * h as f32));
                if x < 0 || x >= w as i32 || y < 0 || y >= h as i32 {
                    continue;
                }
                let id = y as usize * w + x as usize;
                if layer.marker[id] != 0 {
                    continue;
                }
                let mut c = ColorC::default();
                light_point(
                    &t,
                    &mut cache,
                    &mut c,
                    math::mad(v1, v2 - v1, time),
                    fnrm,
                    &lights,
                    env.sun_flag(),
                    Some(tri.owner),
                );
                c.mul(0.5);
                layer.surface[id] = BaseColor::from_c(&c);
                layer.marker[id] = 255;
            }
        }
    }
    layer
}

/// The charts of the full scene, lit the old way and through the baker: the texels and their
/// markers. Answers (texels, texels that differ, the largest difference).
fn charts_apart(jitter_samples: u32) -> (usize, usize, i32) {
    let mut g = full_scene();
    g.params.lm_jitter_samples = jitter_samples;
    g.build_rapid(&g.scene_bounds()).unwrap();
    let env = g.light_env().unwrap();
    let cpu = CpuLighter::default();
    let scene = cpu.scene(env.model, env.textures).unwrap();
    let (mut texels, mut differ, mut worst) = (0, 0, 0);
    for &id in g.deflectors.list() {
        let d = g.deflectors.get(id).unwrap();
        let old = old_chart(&env, d);
        let mut new = d.clone();
        new.take_sphere(env.mesh);
        let got = new.gather(env.mesh, env.params, env.sun_flag());
        let lit = light_samples(scene.as_ref(), &env.job(), &got.samples, &Ctl::quiet()).unwrap();
        new.scatter(&lit, &got.runs);
        assert_eq!((new.layer.width, new.layer.height), (old.width, old.height));
        assert_eq!(new.layer.marker, old.marker, "the markers are geometry's");
        for (a, b) in new.layer.surface.iter().zip(&old.surface) {
            texels += 1;
            let e = apart(a, b);
            differ += usize::from(e != 0);
            worst = worst.max(e);
        }
    }
    (texels, differ, worst)
}

#[test]
fn the_charts_through_the_baker_are_the_charts_light_point_gave() {
    let (texels, differ, worst) = charts_apart(1);
    assert!(texels > 200, "{texels}");
    assert_eq!((differ, worst), (0, 0), "one sample a texel: the same bits");
    let (texels, differ, worst) = charts_apart(4);
    assert!(worst <= 1 && differ * 100 <= texels, "{differ} of {texels} apart, by {worst}");
}

#[test]
fn the_vertices_the_models_the_textures_and_the_slots_through_the_baker() {
    let cpu = CpuLighter::default();
    // the vertices: one sample each, the same bits
    let mut g = full_scene();
    g.build_rapid(&g.scene_bounds()).unwrap();
    let mut old = Vec::new();
    {
        let env = g.light_env().unwrap();
        let t = tracer(&env);
        for &v in env.mesh.vertices() {
            let vx = env.mesh.vertex(v).unwrap();
            let lit = vx
                .adjacents
                .iter()
                .any(|&f| g.shader(env.mesh.face(f).unwrap().material).flags & ShaderXrlc::LIGHT_VERTEX != 0);
            old.push(lit.then(|| {
                let mut c = ColorC::default();
                light_point(
                    &t,
                    &mut LightCache::default(),
                    &mut c,
                    vx.p,
                    vx.n,
                    env.lights,
                    LP_DONT_HEMI,
                    None,
                );
                c.mul(0.5);
                c.hemi = vx.c.get().hemi;
                BaseColor::from_c(&c)
            }));
        }
    }
    g.light_vertices(&cpu, &Ctl::quiet()).unwrap();
    let mut seen = 0;
    for (&v, want) in g.mesh.vertices().iter().zip(&old) {
        if let Some(want) = want {
            assert_eq!(g.mesh.vertex(v).unwrap().c, *want);
            seen += 1;
        }
    }
    assert!(seen >= 4, "{seen}");

    // the models' own light and the references': six samples a vertex
    let mut g = full_scene();
    g.build_rapid(&g.scene_bounds()).unwrap();
    let mut models = g.mu_models.clone();
    let env = g.mu_env();
    let mut stream = math::XrRandom::new(1);
    let mut old_models = Vec::new();
    for m in &mut models {
        let start = stream;
        stream = stream.advanced(m.light_draws(env.params));
        m.calc_materials(env.shaders, env.materials);
        let own = m.own_ray_model(env.shaders, env.materials).unwrap();
        old_models.push(old_mu(
            m,
            &env,
            &math::Matrix::IDENTITY,
            &own,
            LP_DONT_RGB + LP_DONT_SUN,
            Some(start),
        ));
    }
    let level = g.ray.clone().unwrap();
    let old_refs: Vec<Vec<BaseColor>> = g
        .mu_refs
        .iter()
        .map(|r| old_mu(&models[r.model as usize], &env, &r.xform, &level, LP_DEFAULT, None))
        .collect();
    g.light_mu(&cpu, &Ctl::quiet()).unwrap();
    let close = |a: &[BaseColor], b: &[BaseColor]| {
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(b).all(|(x, y)| apart(x, y) <= 1), "{a:?} {b:?}");
    };
    for (m, want) in g.mu_models.iter().zip(&old_models) {
        close(&m.color, want);
    }
    for (r, want) in g.mu_refs.iter().zip(&old_refs) {
        close(&r.color, want);
    }

    // the implicit texture
    let mut g = full_scene();
    g.build_rapid(&g.scene_bounds()).unwrap();
    let faces: Vec<FaceId> =
        g.mesh.faces().iter().copied().filter(|&f| g.mesh.face(f).unwrap().material == 3).collect();
    let env = g.light_env().unwrap();
    let old = old_implicit(&env, 4, 4, &faces);
    let scene = cpu.scene(env.model, env.textures).unwrap();
    let new = crate::implicit::light_texture(&env, 4, 4, &faces, scene.as_ref(), &Ctl::quiet()).unwrap();
    assert_eq!(new.marker, old.marker);
    assert!(new.surface.iter().zip(&old.surface).all(|(a, b)| apart(a, b) <= 1));
    assert!(old.marker.iter().filter(|&&m| m == 255).count() >= 8);

    // the slots: 225 points each
    let (scene, slots) = detail_scene();
    let mut new = slots.clone();
    details(&scene, &mut new, None).unwrap();
    for (z, row) in slots.chunks(2).enumerate() {
        for (x, s) in row.iter().enumerate() {
            let want = old_slot(&scene, x as i32, z as i32, s).unwrap();
            let got = new[z * 2 + x].colors();
            assert!(want.iter().zip(&got).all(|(a, b)| a.abs_diff(*b) <= 1), "{want:?} {got:?}");
        }
    }
}

/// The old `calc_lighting_with`: six samples a vertex into one colour, then the groups.
fn old_mu(
    m: &MuModel,
    env: &MuEnv<'_>,
    xform: &math::Matrix,
    model: &RayModel,
    flags: u32,
    stream: Option<math::XrRandom>,
) -> Vec<BaseColor> {
    let rot = xform.without_translation().transposed().inverted_43();
    let samples = if env.params.quality == QUALITY_DRAFT { 1u32 } else { 6 };
    let t = Tracer { model, textures: env.textures, falloff: Falloff::Level { linear: env.linear } };
    let verts = m.mesh.vertices();
    let mut colors: Vec<BaseColor> = verts
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let vx = m.mesh.vertex(v).unwrap();
            let (mut amb, mut trans) = (0.0f32, 0.0f32);
            for f in &vx.adjacents {
                let s = shader_of(m.mesh.face(*f).unwrap().material, env.shaders, env.materials);
                amb += s.vert_ambient;
                trans += s.vert_translucency;
            }
            let n_adj = vx.adjacents.len() as f32;
            amb /= n_adj;
            trans /= n_adj;
            let vp = xform.transform_tiny(vx.p);
            let (vn, _) = math::exact_normalize(rot.transform_dir(vx.n));
            let mut rnd = match stream {
                Some(s) => s.advanced(i as u64 * u64::from(samples) * 3),
                None => math::XrRandom::from_words(&math::point_words(vp)),
            };
            let mut c = ColorC::default();
            for sample in 0..samples {
                let a = 0.2 * sample as f32 / samples as f32;
                let n = rnd.dir_cone(vn, math::deg2rad(30.0));
                let p = math::mad(vp, n, a);
                light_point(&t, &mut LightCache::default(), &mut c, p, n, env.lights, flags, None);
            }
            c.scale(samples);
            c.tmp = trans;
            if flags & LP_DONT_HEMI == 0 {
                c.hemi += amb;
            }
            BaseColor::from_c(&c)
        })
        .collect();
    let mut groups = TransGroups::default();
    for (i, &v) in verts.iter().enumerate() {
        groups.register(m.mesh.vertex(v).unwrap().p, i as u32, false);
    }
    for list in groups.in_order() {
        let mut list = list.to_vec();
        list.sort_unstable();
        list.dedup();
        let cs: Vec<ColorC> = list.iter().map(|&i| colors[i as usize].get()).collect();
        for (&i, c) in list.iter().zip(mix_group(&cs, 0.5)) {
            colors[i as usize] = BaseColor::from_c(&c);
        }
    }
    colors
}

/// The old `light_texture`.
fn old_implicit(env: &LightEnv<'_>, w: u32, h: u32, faces: &[FaceId]) -> LmLayer {
    let mut lmap = LmLayer::new(w, h);
    let tcs: Vec<[Vec2; 3]> = faces.iter().map(|&f| env.mesh.face(f).unwrap().tc0()).collect();
    let mut bounds = math::Box2::INVALID;
    let boxes: Vec<math::Box2> = tcs.iter().map(math::Box2::of_tri).collect();
    boxes.iter().for_each(|b| bounds.merge(b));
    let hash = Hash2D::build(384, 384, bounds, &boxes);
    let t = tracer(env);
    let dim = Vec2::new(w as f32, h as f32);
    let half = Vec2::new(0.5 / dim.x, 0.5 / dim.y);
    let js = Vec2::new(0.499 / dim.x, 0.499 / dim.y);
    for v in 0..h as usize {
        for u in 0..w as usize {
            let mut c = ColorC::default();
            let mut count = 0u32;
            for j in jitter(env.params.lm_jitter_samples) {
                let p =
                    Vec2::new(u as f32 / dim.x + half.x + j.x * js.x, v as f32 / dim.y + half.y + j.y * js.y);
                for &i in hash.query(p.x, p.y) {
                    let i = i as usize;
                    let Some(b) = (Tcf { uv: tcs[i] }).inside(p) else { continue };
                    let pos = env.mesh.positions(faces[i]).unwrap();
                    let nrm = env.mesh.normals(faces[i]).unwrap();
                    let wp = math::from_bary(pos[0], pos[1], pos[2], b);
                    let wn = math::normalize(math::from_bary(nrm[0], nrm[1], nrm[2], b));
                    let skip = Some(faces[i]);
                    light_point(
                        &t,
                        &mut LightCache::default(),
                        &mut c,
                        wp,
                        wn,
                        env.lights,
                        env.sun_flag(),
                        skip,
                    );
                    count += 1;
                }
            }
            if count > 0 {
                c.scale(count);
                c.mul(0.5);
                lmap.surface[v * w as usize + u] = BaseColor::from_c(&c);
                lmap.marker[v * w as usize + u] = 255;
            }
        }
    }
    lmap
}

/// The old `slot_light`: the points into one colour, the lights of the slot's sphere.
fn old_slot(scene: &DetailScene, x: i32, z: i32, slot: &DetailSlot) -> Option<[u8; 5]> {
    let mut slot = *slot;
    process_palette(&mut slot);
    let bb = scene.slot_box(x, z, &slot);
    let (sp, sr) = math::sphere_of(&bb);
    let selected = scene.lights.select(sp, sr);
    let mut points = Vec::new();
    slot_points(scene, x, z, &slot, &mut Vec::new(), &mut points)?;
    let t = Tracer { model: &scene.model, textures: &scene.textures, falloff: Falloff::Details };
    let mut amount = ColorC::default();
    for s in &points {
        let (p, n) = (Vec3::from(s.p), Vec3::from(s.n));
        light_point(&t, &mut LightCache::default(), &mut amount, p, n, &selected, 0, None);
    }
    Some(slot_colors(amount, points.len() as u32))
}
