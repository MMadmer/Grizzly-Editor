//! The whole compile on a small level built in memory.

use glam::{Vec2, Vec3};
use gz_fmt_level::{
    BuildFace, BuildLight, BuildMaterial, BuildParams, BuildProject, BuildTexture, Light, LightControl,
    MuModel, MuReference, Portal, XRCL_CURRENT_VERSION,
};
use gz_fmt_xr::ShaderXrlc;
use gz_lc_light::{CpuLighter, Ctl, Shaders, TextureUse, default_shader, prepare_texture};

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
        compile_project(
            &prj,
            shaders,
            textures,
            &Options::default(),
            &CpuLighter::default(),
            &Ctl::quiet(),
            &mut log,
        )
        .unwrap()
        .files
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

/// A folder under the temp folder for one test, gone when the test ends.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("gzlc_test_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn listing(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                let rel = p.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/");
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// The level's files take their places together: the pages of an earlier compile go, the input
/// stays, no stage is left; a move that fails puts every file back as it was.
#[test]
fn the_level_files_appear_together_or_not_at_all() {
    let s = Scratch::new("write");
    let dir = &s.0;
    for (n, b) in
        [("build.prj", "input"), ("level", "old level"), ("lmap#2_1.dds", "old"), ("lmap#2_2.dds", "old")]
    {
        std::fs::write(dir.join(n), b).unwrap();
    }
    let mut out = crate::Output::default();
    for (n, b) in
        [("level", "new level"), ("lmap#1_1.dds", "a"), ("lmap#1_2.dds", "b"), ("terrain\\t.dds", "t")]
    {
        out.put(n, b.as_bytes().to_vec());
    }
    let mut report = crate::Report::default();
    crate::write_all(dir, &out, true, &mut report).unwrap();
    let names: Vec<String> = listing(dir).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["build.prj", "level", "lmap#1_1.dds", "lmap#1_2.dds", "terrain/t.dds"]);
    assert_eq!(std::fs::read(dir.join("level")).unwrap(), b"new level");
    assert_eq!(report.files.len(), 4);
    assert!(!dir.join(crate::STAGE_NEW).exists() && !dir.join(crate::STAGE_OLD).exists());

    // a folder where a file has to go: the move fails half way, the level stays the last one
    let before = listing(dir);
    std::fs::create_dir(dir.join("level.geom")).unwrap();
    let mut out2 = crate::Output::default();
    for (n, b) in [("level", "third"), ("lmap#1_1.dds", "c"), ("level.geom", "g")] {
        out2.put(n, b.as_bytes().to_vec());
    }
    assert!(crate::write_all(dir, &out2, true, &mut crate::Report::default()).is_err());
    std::fs::remove_dir(dir.join("level.geom")).unwrap();
    assert_eq!(listing(dir), before);

    // the detail compiler's one file leaves the level's pages where they are
    let mut details = crate::Output::default();
    details.put("level.details", b"d".to_vec());
    crate::write_all(dir, &details, false, &mut crate::Report::default()).unwrap();
    let mut with_details = before.clone();
    with_details.push(("level.details".into(), b"d".to_vec()));
    with_details.sort();
    assert_eq!(listing(dir), with_details);
}

#[test]
fn the_number_of_threads_is_an_option_of_the_test() {
    let (o, unknown) = Options::parse(["-threads=1", "-noise"]).unwrap();
    assert_eq!((o.threads, o.noise, unknown.len()), (Some(1), true, 0));
    assert!(Options::parse(["-threads=0"]).is_err());
    assert!(Options::parse(["-threads=x"]).is_err());
}

/// The compile of `prj` with a Stop that `stop_when` pulls, given every log line and every report
/// (before the Stop and after it): the answer, the lines, and how long the compile took after the
/// Stop was pulled.
fn stopped_compile(
    prj: &gz_fmt_level::BuildProject,
    shaders: Shaders,
    textures: Vec<gz_lc_light::BuildTexture>,
    stop_when: impl Fn(Option<&str>, Option<gz_lc_light::Report<'_>>) -> bool + Send + Sync + 'static,
) -> (crate::Result<crate::Output>, Vec<String>, Option<std::time::Duration>) {
    use std::sync::{Arc, Mutex};
    let cancel = gz_task::Cancel::new();
    let pulled: Arc<Mutex<Option<std::time::Instant>>> = Arc::default();
    let when = Arc::new(stop_when);
    let (c1, p1, w1) = (cancel.clone(), pulled.clone(), when.clone());
    let echo = move |line: &str| {
        if w1(Some(line), None) && !c1.is_cancelled() {
            *p1.lock().unwrap() = Some(std::time::Instant::now());
            c1.cancel();
        }
    };
    let (c2, p2) = (cancel.clone(), pulled.clone());
    let report = move |r: gz_lc_light::Report<'_>| {
        if when(None, Some(r)) && !c2.is_cancelled() {
            *p2.lock().unwrap() = Some(std::time::Instant::now());
            c2.cancel();
        }
    };
    let ctl = Ctl::new(&cancel, &report);
    let mut log = Log::with_echo(Box::new(echo));
    let out =
        compile_project(prj, shaders, textures, &Options::default(), &CpuLighter::default(), &ctl, &mut log);
    let after = pulled.lock().unwrap().map(|t| t.elapsed());
    (out, log.lines, after)
}

#[test]
fn a_stop_in_any_phase_ends_the_compile_in_that_phase() {
    for phase in crate::PHASES {
        let (prj, shaders, textures) = project();
        let at = format!("{}{phase}", crate::PHASE_STARTED);
        let (out, lines, _) =
            stopped_compile(&prj, shaders, textures, move |line, _| line == Some(at.as_str()));
        assert!(out.as_ref().err().is_some_and(crate::Error::cancelled), "{phase}: {:?}", out.err());
        let started: Vec<&str> = lines.iter().filter_map(|l| l.strip_prefix(crate::PHASE_STARTED)).collect();
        assert_eq!(started.last().copied(), Some(phase), "the compile went past {phase}");
    }
}

#[test]
fn a_stop_while_the_lightmaps_are_lit_ends_within_a_moment() {
    // the floor lit densely: the phase takes seconds, the Stop comes as its first deflector is done
    let (mut prj, shaders, textures) = project();
    prj.params.lm_pixels_per_meter = 24.0;
    prj.params.lm_jitter_samples = 9;
    let phase = format!("{}{}", crate::PHASE_STARTED, crate::PHASES[13]);
    let inside = std::sync::atomic::AtomicBool::new(false);
    let started = std::time::Instant::now();
    let (out, lines, after) = stopped_compile(&prj, shaders, textures, move |line, r| {
        if line == Some(phase.as_str()) {
            inside.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        inside.load(std::sync::atomic::Ordering::Relaxed)
            && matches!(r, Some(gz_lc_light::Report::Progress { .. }))
    });
    assert!(out.as_ref().err().is_some_and(crate::Error::cancelled), "{:?}", out.err());
    let after = after.expect("the Stop was pulled while the lightmaps were lit");
    assert!(after < std::time::Duration::from_secs(1), "the compile took {after:?} to stop");
    assert!(!lines.iter().any(|l| l.starts_with(crate::PHASE_DONE) && l.contains(crate::PHASES[13])));
    eprintln!("stopped {after:?} after the Stop, {:?} in all", started.elapsed());
}

#[test]
fn a_stop_while_the_references_are_lit_leaves_the_rest_unlit() {
    // many references of the model: the Stop at the first one lit, most never are
    let (mut prj, shaders, textures) = project();
    let first = prj.mu_refs[0];
    prj.mu_refs = (0..4000)
        .map(|i| {
            let mut r = first;
            r.transform[3][0] = -20.0 + (i % 64) as f32 * 0.6;
            r.transform[3][2] = -20.0 + (i / 64) as f32 * 0.6;
            r
        })
        .collect();
    let seen = std::sync::Arc::new(std::sync::Mutex::new((false, 0usize, 0usize)));
    let s2 = seen.clone();
    let (out, _, after) = stopped_compile(&prj, shaders, textures, move |_, r| {
        let mut s = s2.lock().unwrap();
        match r {
            Some(gz_lc_light::Report::Stage("the references")) => s.0 = true,
            Some(gz_lc_light::Report::Progress { done, total }) if s.0 => {
                s.1 = s.1.max(done);
                s.2 = total;
                return true;
            }
            _ => {}
        }
        false
    });
    assert!(out.as_ref().err().is_some_and(crate::Error::cancelled), "{:?}", out.err());
    assert!(after.is_some_and(|a| a < std::time::Duration::from_secs(1)), "{after:?}");
    let (_, done, total) = *seen.lock().unwrap();
    assert!(total > 4000 && done < total / 2, "{done} of {total} lit before the Stop took");
}

/// The CPU's light with every sample scaled by a factor between 0.9 and 1.1 drawn from the
/// sample's own position: another light, the same geometry.
#[derive(Default)]
struct Scaled(CpuLighter);

struct ScaledScene<'a>(Box<dyn gz_lc_light::LitScene + 'a>);

impl gz_lc_light::Lighter for Scaled {
    fn name(&self) -> String {
        "a scaled light".into()
    }

    fn scene<'a>(
        &'a self,
        model: &'a gz_lc_light::RayModel,
        textures: &'a [gz_lc_light::BuildTexture],
    ) -> gz_lc_light::Result<Box<dyn gz_lc_light::LitScene + 'a>> {
        Ok(Box::new(ScaledScene(self.0.scene(model, textures)?)))
    }
}

impl gz_lc_light::LitScene for ScaledScene<'_> {
    fn light(
        &self,
        job: &gz_lc_light::LightJob<'_>,
        samples: &[gz_lc_light::Sample],
        out: &mut [gz_lc_light::ColorC],
        ctl: &Ctl<'_>,
    ) -> gz_lc_light::Result<()> {
        self.0.light(job, samples, out, ctl)?;
        for (s, o) in samples.iter().zip(out.iter_mut()) {
            let mut h = 0x9e37_79b9u32;
            for v in s.p {
                h = (h ^ v.to_bits()).wrapping_mul(0x0100_0193);
                h ^= h >> 15;
            }
            o.mul(0.9 + 0.2 * (h % 1000) as f32 / 999.0);
        }
        Ok(())
    }
}

fn compile_with(lighter: &dyn gz_lc_light::Lighter, threads: usize, opts: &Options) -> (crate::Output, Log) {
    let (mut prj, shaders, textures) = project();
    // charts big enough that the light decides their sizes
    prj.params.lm_pixels_per_meter = 6.0;
    let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
    pool.install(|| {
        let mut log = Log::default();
        let out = compile_project(&prj, shaders, textures, opts, lighter, &Ctl::quiet(), &mut log).unwrap();
        (out, log)
    })
}

/// Section 2 of the level build's contract: one scene, one set of settings - the same structure
/// whatever the light. A light scaled sample by sample shrinks, collapses and packs the charts
/// differently and lands them elsewhere on the pages, and the visuals, their faces, vertices and
/// bounds, the sectors and the collision stay the same; so does the structure on one thread and
/// on eight, with no light at all, and with every chart at its geometry's size.
#[test]
fn the_structure_does_not_follow_the_light() {
    let opts = Options::default();
    let (cpu, log) = compile_with(&CpuLighter::default(), 4, &opts);
    let (scaled, _) = compile_with(&Scaled::default(), 4, &opts);
    let (one, _) = compile_with(&CpuLighter::default(), 1, &opts);
    let (none, _) = compile_with(&gz_lc_light::NullLighter::default(), 8, &opts);
    let (kept, _) =
        compile_with(&CpuLighter::default(), 4, &Options { no_lm_shrink: true, ..Options::default() });
    let file = |o: &crate::Output, n: &str| o.files.iter().find(|(f, _)| f == n).map(|(_, b)| b.clone());
    assert_ne!(cpu.structure, 0);
    assert_eq!(cpu.structure, scaled.structure, "a scaled light changed the structure");
    assert_eq!(cpu.structure, one.structure, "the number of threads changed the structure");
    assert_eq!(cpu.structure, none.structure, "no light changed the structure");
    assert_eq!(cpu.structure, kept.structure, "the charts' sizes changed the structure");
    for n in ["level.cform", "build.cform"] {
        assert_eq!(file(&cpu, n), file(&scaled, n), "{n}");
    }
    // the light did differ: the pages are not the same
    let pages = |o: &crate::Output| -> Vec<Vec<u8>> {
        o.files.iter().filter(|(n, _)| n.starts_with("lmap#")).map(|(_, b)| b.clone()).collect()
    };
    assert!(!pages(&cpu).is_empty());
    assert_ne!(pages(&cpu), pages(&scaled), "the scaled light lit the same pages: the test proves nothing");
    // the log says what was packed and where the light was baked
    assert!(log.lines.iter().any(|l| l.contains("lightmap pages,")), "{:?}", log.lines);
    assert!(log.lines.iter().any(|l| l.contains("The light was baked on the CPU:")), "{:?}", log.lines);
}
