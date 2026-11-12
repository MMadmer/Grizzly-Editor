//! The whole compile on a small level built in memory.

use glam::{Vec2, Vec3};
use gz_fmt_level::{
    BuildFace, BuildLight, BuildMaterial, BuildParams, BuildProject, BuildTexture, Light, LightControl,
    MuModel, MuReference, Portal, XRCL_CURRENT_VERSION,
};
use gz_fmt_xr::ShaderXrlc;
use gz_lc_light::{Ctl, Shaders, TextureUse, default_shader, prepare_texture};

use crate::sphere::MsvcRand;
use crate::{Log, Options, compile_project};

fn shader(name: &str, flags: u32) -> ShaderXrlc {
    let mut s = default_shader();
    s.name_raw = [0; 128];
    s.name_raw[..name.len()].copy_from_slice(name.as_bytes());
    s.flags = flags;
    s
}

/// A floor of 8 x 8 quads 48 m wide (long edges to split), a box on it, a wall of vertex light,
/// and an MU model placed twice; two sectors and a portal between them.
fn project() -> (BuildProject, Shaders, Vec<gz_lc_light::BuildTexture>) {
    let mut verts = Vec::new();
    let mut faces = Vec::new();
    let mut groups = Vec::new();
    let n = 8;
    let step = 6.0f32;
    for z in 0..=n {
        for x in 0..=n {
            verts.push(Vec3::new(x as f32 * step - 24.0, 0.0, z as f32 * step - 24.0));
        }
    }
    let id = |x: u32, z: u32| z * (n + 1) + x;
    for z in 0..n {
        for x in 0..n {
            let uv = |x: u32, z: u32| Vec2::new(x as f32 * 0.5, z as f32 * 0.5);
            let mat = if x < n / 2 { 0 } else { 1 };
            faces.push(BuildFace {
                v: [id(x, z), id(x, z + 1), id(x + 1, z + 1)],
                t: [uv(x, z), uv(x, z + 1), uv(x + 1, z + 1)],
                material: mat,
                material_game: 3,
            });
            faces.push(BuildFace {
                v: [id(x, z), id(x + 1, z + 1), id(x + 1, z)],
                t: [uv(x, z), uv(x + 1, z + 1), uv(x + 1, z)],
                material: mat,
                material_game: 3,
            });
            groups.extend_from_slice(&[0, 0]);
        }
    }
    // a box: four walls and a roof, 2 m high
    let b = verts.len() as u32;
    for (x, z) in [(-2.0, -2.0), (2.0, -2.0), (2.0, 2.0), (-2.0, 2.0)] {
        verts.push(Vec3::new(x, 0.0, z));
        verts.push(Vec3::new(x, 2.0, z));
    }
    for k in 0..4u32 {
        let (a0, a1) = (b + k * 2, b + k * 2 + 1);
        let (c0, c1) = (b + ((k + 1) % 4) * 2, b + ((k + 1) % 4) * 2 + 1);
        let t = [Vec2::ZERO, Vec2::Y, Vec2::ONE];
        faces.push(BuildFace { v: [a0, a1, c1], t, material: 2, material_game: 4 });
        faces.push(BuildFace {
            v: [a0, c1, c0],
            t: [Vec2::ZERO, Vec2::ONE, Vec2::X],
            material: 2,
            material_game: 4,
        });
        groups.extend_from_slice(&[1 << 2, 1 << 2]);
    }
    let r = [b + 1, b + 3, b + 5, b + 7];
    faces.push(BuildFace {
        v: [r[0], r[3], r[2]],
        t: [Vec2::ZERO, Vec2::Y, Vec2::ONE],
        material: 2,
        material_game: 4,
    });
    faces.push(BuildFace {
        v: [r[0], r[2], r[1]],
        t: [Vec2::ZERO, Vec2::ONE, Vec2::X],
        material: 2,
        material_game: 4,
    });
    groups.extend_from_slice(&[0, 0]);

    let mu_verts = vec![
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(0.0, 3.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    let mu_faces = vec![
        BuildFace { v: [0, 2, 1], t: [Vec2::ZERO, Vec2::Y, Vec2::X], material: 3, material_game: 5 },
        BuildFace { v: [0, 3, 2], t: [Vec2::ZERO, Vec2::X, Vec2::Y], material: 3, material_game: 5 },
        BuildFace { v: [1, 2, 3], t: [Vec2::ZERO, Vec2::Y, Vec2::ONE], material: 3, material_game: 5 },
    ];
    let place =
        |x: f32, z: f32| [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [x, 0.0, z, 1.0]];
    let light = |kind: u32, dir: Vec3, pos: Vec3, range: f32| Light {
        kind,
        diffuse: [1.0, 0.9, 0.8, 1.0],
        position: pos,
        direction: dir,
        range,
        attenuation0: 1.0,
        attenuation1: 0.1,
        ..Light::default()
    };
    let prj = BuildProject {
        version: XRCL_CURRENT_VERSION,
        params: BuildParams { lm_pixels_per_meter: 2.0, lm_jitter_samples: 1, ..BuildParams::default() },
        verts: verts.into(),
        faces: faces.into(),
        smooth_groups: groups.into(),
        materials: vec![
            BuildMaterial {
                surface: 0,
                shader: 0,
                shader_xrlc: 0,
                sector: 0,
                reserved: 0,
                internal_max_area: 0,
            },
            BuildMaterial {
                surface: 0,
                shader: 0,
                shader_xrlc: 0,
                sector: 1,
                reserved: 0,
                internal_max_area: 0,
            },
            BuildMaterial {
                surface: 1,
                shader: 1,
                shader_xrlc: 1,
                sector: 0,
                reserved: 0,
                internal_max_area: 0,
            },
            BuildMaterial {
                surface: 1,
                shader: 0,
                shader_xrlc: 0,
                sector: 0,
                reserved: 0,
                internal_max_area: 0,
            },
        ],
        shaders: vec!["default".into(), "def_shaders\\def_vertex".into()],
        shaders_xrlc: vec!["default".into(), "vertex".into()],
        textures: vec![
            BuildTexture { name: "grnd\\floor".into(), width: 64, height: 64, has_alpha: 0, reserve: 0 },
            BuildTexture { name: "wall\\box".into(), width: 64, height: 64, has_alpha: 0, reserve: 0 },
        ],
        glows: vec![gz_fmt_level::BuildGlow { pos: Vec3::Y, size: 1.0, flags: 0, material: 2 }],
        portals: vec![Portal::new(
            0,
            1,
            &[Vec3::new(0.0, 0.0, -24.0), Vec3::new(0.0, 5.0, -24.0), Vec3::new(0.0, 5.0, 24.0)],
        )],
        light_controls: vec![
            LightControl { name: "$static".into(), data: vec![] },
            LightControl { name: "$hemi".into(), data: vec![] },
            LightControl { name: "$sun".into(), data: vec![] },
        ],
        lights_static: vec![
            BuildLight { controller: 0, light: light(1, Vec3::NEG_Y, Vec3::new(5.0, 3.0, 5.0), 10.0) },
            BuildLight { controller: 1, light: light(3, Vec3::new(0.3, -1.0, 0.2), Vec3::ZERO, 100.0) },
            BuildLight { controller: 1, light: light(3, Vec3::new(-0.3, -1.0, -0.2), Vec3::ZERO, 100.0) },
            BuildLight { controller: 2, light: light(3, Vec3::new(0.5, -1.0, 0.1), Vec3::ZERO, 100.0) },
        ],
        lights_dynamic: vec![],
        lods: vec![],
        mu_models: vec![MuModel {
            name: "tree".into(),
            verts: mu_verts,
            faces: mu_faces,
            lod_id: u16::MAX,
            smooth_groups: vec![0, 0, 0],
        }],
        mu_refs: vec![
            MuReference { model: 0, transform: place(10.0, 10.0), flags: 0, sector: 1, reserved: [0; 8] },
            MuReference { model: 0, transform: place(-10.0, 12.0), flags: 0, sector: 0, reserved: [0; 8] },
        ],
    };
    let rendering = ShaderXrlc::COLLISION
        | ShaderXrlc::RENDERING
        | ShaderXrlc::LIGHT_CAST_SHADOW
        | ShaderXrlc::OPTIMIZE_UV;
    let shaders = Shaders::new(vec![
        shader("default", rendering),
        shader("vertex", rendering | ShaderXrlc::LIGHT_VERTEX),
    ]);
    let thm = gz_fmt_tex::TextureParams { width: 64, height: 64, ..gz_fmt_tex::TextureParams::default() };
    let textures = prj
        .textures
        .iter()
        .map(|t| prepare_texture(t, Some(&thm), TextureUse::Level { radiosity: false }, |_| None))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    (prj, shaders, textures)
}

fn compile(threads: usize) -> Vec<(String, Vec<u8>)> {
    let (prj, shaders, textures) = project();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    pool.install(|| {
        let mut log = Log::default();
        compile_project(&prj, shaders, textures, &Options::default(), &Ctl::quiet(), &mut log).unwrap().files
    })
}

#[test]
fn a_small_level_compiles_into_its_files() {
    let files = compile(4);
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    for want in ["build.lights", "level.cform", "build.cform", "level", "level.geom", "level.geomx"] {
        assert!(names.contains(&want), "{want} missing: {names:?}");
    }
    assert!(names.iter().any(|n| n.starts_with("lmap#1_")), "{names:?}");
    let level = &files.iter().find(|(n, _)| n == "level").unwrap().1;
    let r = gz_chunk::Reader::new(level);
    let ids: Vec<u32> = r.chunks().map(|c| c.0).collect();
    assert_eq!(ids, vec![1, 4, 6, 7, 3, 2, 8]);
    let cform =
        gz_fmt_level::CForm::read(&files.iter().find(|(n, _)| n == "level.cform").unwrap().1).unwrap();
    assert!(!cform.tris.is_empty());
}

#[test]
fn the_same_bytes_on_one_thread_and_on_eight() {
    let a = compile(1);
    let b = compile(8);
    assert_eq!(a.len(), b.len());
    for ((na, ba), (nb, bb)) in a.iter().zip(&b) {
        assert_eq!(na, nb);
        assert!(ba == bb, "{na} differs between 1 and 8 threads");
    }
}

#[test]
fn the_command_line_refuses_the_network_and_radiosity() {
    let (o, unknown) = Options::parse(["-nosun", "-noise", "-silent", "-what"]).unwrap();
    assert!(o.nosun && o.noise && !o.nosmg);
    assert_eq!(unknown, vec!["-what".to_owned()]);
    assert!(Options::parse(["-net"]).is_err());
    assert!(Options::parse(["-gi"]).is_err());
}

#[test]
fn the_random_stream_jumps_ahead() {
    let mut a = MsvcRand::min_sphere();
    for _ in 0..1000 {
        a.rand();
    }
    assert_eq!(MsvcRand::min_sphere().advanced(1000), a);
    // the UCRT's first values without srand
    let mut r = MsvcRand::new(1);
    assert_eq!([r.rand(), r.rand(), r.rand(), r.rand()], [41, 18467, 6334, 26500]);
}

/// `std::random_shuffle` draws one value per place from the second on; every element stays.
#[test]
fn the_shuffle_is_a_permutation_drawn_from_rand() {
    let mut list: Vec<u32> = (0..100).collect();
    let mut r = MsvcRand::new(1);
    r.shuffle(&mut list);
    assert_ne!(list, (0..100).collect::<Vec<_>>());
    let mut sorted = list.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, (0..100).collect::<Vec<_>>());
    // place 1 takes rand() % 2 = 41 % 2 = 1: no swap; place 2 takes 18467 % 3 = 2: none; place 3 takes 6334 % 4 = 2
    let mut small = [0u32, 1, 2, 3];
    MsvcRand::new(1).shuffle(&mut small);
    assert_eq!(small, [0, 1, 3, 2]);
}
