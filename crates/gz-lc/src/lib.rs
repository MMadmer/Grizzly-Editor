//! The level compiler (`XrLC`) and the detail compiler's pipeline (`xrDO_Light`), on the model
//! and the light of `gz-lc-light`.
//!
//! ```text
//! compile(&Input, &Options, &Ctl) -> Report        CBuild::Load + CBuild::Run: build.prj in,
//!                                                   level, level.geom, level.geomx, level.cform,
//!                                                   build.lights, build.cform, the lightmaps out
//! compile_details(&Input, &Ctl) -> Report          xrCompileDO: level.details out
//! compile_project(&BuildProject, Shaders, textures, &Options, &Ctl) -> Output
//!                                                   the same in memory, for a caller with its data
//! Options::parse(flags)                             the command line of XrLC
//! ```
//!
//! **Modules**: `load` (`CBuild::Load`), `geom` (`PreOptimize`, `CorrectTJunctions`,
//! `xrPhase_AdaptiveHT`, `xrPhase_TangentBasis`), `cform` (`BuildCForm`, `SimplifyCFORM`),
//! `qslim` (`XrQSlim`), `mender` (`NVMeshMenderTwo`), `split` (`ResolveMaterials`, `UVmap`,
//! `Subdivide`, `MergeGeometry`), `ogf` (`OGF_Face*`), `stripify` (`NvTriStrip`), `sphere`
//! (`CalcBounds`, the miniball, `Mgc::MinSphere`), `tree` (`Flex2OGF`, the MU visuals, the sectors,
//! the savers), `details` (`xrCompileDO`).
//!
//! Every phase runs on the caller's rayon pool and gives the same bytes on any number of
//! threads; nothing here makes a pool or holds a global.

mod cform;
mod details;
mod geom;
mod load;
pub mod mender;
mod ogf;
pub mod qslim;
pub mod sphere;
mod split;
pub mod stripify;
mod tree;
mod vcache;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::time::Instant;

use gz_fmt_level::BuildProject;
use gz_lc_light::{Ctl, LcError, Shaders, Smoothing, TextureUse};

pub use details::compile_details_in_memory;
pub use load::{Extra, Sources};

/// What can stop a compile.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Invalid(String),
    #[error("{0} invalid faces. Compilation aborted.")]
    InvalidFaces(u32),
    #[error("{0}")]
    Refused(String),
    #[error(transparent)]
    Light(#[from] LcError),
    #[error(transparent)]
    Level(#[from] gz_fmt_level::LevelError),
    #[error(transparent)]
    Tex(#[from] gz_fmt_tex::TexError),
}

impl Error {
    #[must_use]
    pub fn cancelled(&self) -> bool {
        matches!(self, Error::Light(LcError::Cancelled))
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// The command line of `XrLC` and `xrDO_Light`, as far as it reaches the compile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// `-nosun`: no sun light.
    pub nosun: bool,
    /// `-noise`: no progressive meshes (`b_noise`).
    pub noise: bool,
    /// `-nosmg`: smoothing by angle, the faces' smoothing groups ignored.
    pub nosmg: bool,
    /// `-no_optimize`: `PreOptimize` is skipped.
    pub no_optimize: bool,
    /// `-no_invalidefaces`: invalid faces do not stop the compile.
    pub no_invalid_faces: bool,
    /// `-att`: point lights fall off linearly.
    pub linear: bool,
}

impl Options {
    /// Reads the flags of a command line: the known ones are kept, `-gi` and `-net` are refused
    /// with a sentence, `-silent`, `-f`, `-o` and `-keep_temp_files` are the launcher's and
    /// accepted; anything else is named back.
    pub fn parse<'a>(args: impl IntoIterator<Item = &'a str>) -> Result<(Self, Vec<String>)> {
        let mut o = Self::default();
        let mut unknown = Vec::new();
        for a in args {
            match a.trim_start_matches('-').to_ascii_lowercase().as_str() {
                "nosun" => o.nosun = true,
                "noise" => o.noise = true,
                "nosmg" => o.nosmg = true,
                "no_optimize" => o.no_optimize = true,
                "no_invalidefaces" => o.no_invalid_faces = true,
                "att" => o.linear = true,
                "silent" | "o" | "keep_temp_files" => {}
                "gi" => {
                    return Err(Error::Refused(
                        "-gi (the radiosity pass) is not part of this compiler: the lightmaps are lit directly.".into(),
                    ));
                }
                "net" => {
                    return Err(Error::Refused(
                        "-net (lighting over a network of machines) is not part of this compiler: it lights on every core of this one.".into(),
                    ));
                }
                _ => unknown.push(a.to_owned()),
            }
        }
        Ok((o, unknown))
    }
}

/// The compile's log: the phases with their seconds, as the C++ log window lists them, and the
/// lines it prints.
#[derive(Default)]
pub struct Log {
    pub lines: Vec<String>,
    pub phases: Vec<(String, f64)>,
    started: Option<(String, Instant)>,
    echo: Option<Echo>,
}

/// Where a log line goes as it is written.
type Echo = Box<dyn Fn(&str) + Send + Sync>;

impl std::fmt::Debug for Log {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Log").field("lines", &self.lines.len()).field("phases", &self.phases).finish()
    }
}

impl Log {
    #[must_use]
    pub fn with_echo(echo: Box<dyn Fn(&str) + Send + Sync>) -> Self {
        Self { echo: Some(echo), ..Self::default() }
    }

    fn push(&mut self, s: String) {
        if let Some(e) = &self.echo {
            e(&s);
        }
        self.lines.push(s);
    }

    /// `clMsg`.
    pub fn msg(&mut self, s: String) {
        self.push(format!("    |    | {s}"));
    }

    /// `Status`.
    pub fn status(&mut self, s: String) {
        self.push(format!("    | {s}"));
    }

    /// `Phase`: the one before ends with its seconds.
    pub fn phase(&mut self, name: &str) {
        self.end_phase();
        self.push(format!("* New phase started: {name}"));
        self.started = Some((name.to_owned(), Instant::now()));
    }

    fn end_phase(&mut self) {
        if let Some((name, t)) = self.started.take() {
            let s = t.elapsed().as_secs_f64();
            self.push(format!("* Phase done: {name} ({s:.3} s)"));
            self.phases.push((name, s));
        }
    }
}

/// The files a compile writes, in the order it makes them.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub files: Vec<(String, Vec<u8>)>,
}

impl Output {
    fn put(&mut self, name: impl Into<String>, bytes: Vec<u8>) {
        self.files.push((name.into(), bytes));
    }
}

/// What a compile did.
#[derive(Debug, Default)]
pub struct Report {
    pub lines: Vec<String>,
    pub phases: Vec<(String, f64)>,
    pub files: Vec<(String, usize)>,
    pub seconds: f64,
}

/// Where a compile reads and writes.
#[derive(Clone, Debug)]
pub struct Input {
    /// The level folder: `build.prj` and the other build files in, the level out.
    pub level: PathBuf,
    pub sources: Sources,
}

fn adapt(e: LcError) -> Error {
    Error::Light(e)
}

/// The whole compile in memory: `CBuild::Load` of a read `build.prj` with the given shaders and
/// prepared textures, then `CBuild::Run`. `log` follows it phase by phase.
pub fn compile_project(
    prj: &BuildProject,
    shaders: Shaders,
    textures: Vec<gz_lc_light::BuildTexture>,
    opts: &Options,
    ctl: &Ctl<'_>,
    log: &mut Log,
) -> Result<Output> {
    log.phase("Converting data structures...");
    let (mut g, mut extra) = load::load(prj, shaders, textures, !opts.nosmg)?;
    g.nosun = opts.nosun;
    g.linear = opts.linear;
    let smoothing = if opts.nosmg { Smoothing::Angle } else { Smoothing::EdgeFlags };
    let mut out = Output::default();
    let mut tree = tree::Tree::default();
    tree.register_shader("");

    let mut level = gz_chunk::Writer::new();
    level.chunk(1, |w| {
        w.w_u16(14);
        w.w_u16(g.params.quality);
    });
    out.put("build.lights", g.lights.write_build_lights());

    log.phase("Optimizing...");
    if !opts.no_optimize {
        geom::pre_optimize(&mut g, &extra.scene_bb, log, opts.no_invalid_faces)?;
    }
    geom::correct_tjunctions(&g, log);
    ctl.check().map_err(adapt)?;

    log.phase("Adaptive HT...");
    geom::adaptive_ht(&mut g, &extra.scene_bb, log, ctl)?;

    log.phase("Building normals...");
    g.calc_normals(smoothing);
    log.status("Models...".into());
    g.mu_calc_normals(smoothing);
    ctl.check().map_err(adapt)?;

    log.phase("Building collision database...");
    out.put("level.cform", cform::build_cform(&mut g, log));
    tree::save_portals(&extra, &mut level);
    ctl.check().map_err(adapt)?;

    log.phase("Building tangent-basis...");
    geom::tangent_basis(&mut g, log);

    log.phase("Building rcast-CFORM model...");
    g.light_prepare();
    let form = g.build_rapid(&extra.scene_bb)?;
    out.put("build.cform", form.write());
    ctl.check().map_err(adapt)?;

    log.phase("LIGHT: Starting MU...");
    g.light_prepare();
    g.light_mu(ctl)?;

    log.phase("Resolving materials...");
    let mut splits = split::resolve_materials(&mut g, log);
    g.mesh.isolate_vertices();

    log.phase("Build UV mapping...");
    split::uv_map(&mut g, &mut splits, log, ctl)?;
    g.mesh.isolate_vertices();

    log.phase("Subdividing geometry...");
    split::subdivide(&mut g, &mut splits, log, ctl)?;
    g.mesh.isolate_vertices();
    g.mesh.reload_vertex_pool();

    log.phase("LIGHT: Implicit...");
    let implicit = g.implicit_lighting(ctl)?;
    for m in &implicit {
        let (base, lm) = m.dds()?;
        out.put(format!("{}.dds", m.name), base);
        out.put(format!("{}_lm.dds", m.name), lm);
    }
    log.phase("LIGHT: LMaps...");
    // LMapsLocal shuffles the deflectors with the main thread's rand(), never seeded before (srand(367)
    // comes with the first MinSphere, later); the pages' layout follows that order
    sphere::MsvcRand::new(1).shuffle(g.deflectors.list_mut());
    g.light_deflectors(ctl)?;
    log.phase("LIGHT: Vertex...");
    g.light_vertices(ctl)?;
    log.phase("LIGHT: Merging lightmaps...");
    let pages = g.merge_lightmaps(ctl)?;
    for p in &pages {
        let (a, b) = p.dds()?;
        out.put(format!("{}_1.dds", p.name), a);
        out.put(format!("{}_2.dds", p.name), b);
    }

    log.phase("Merging geometry...");
    split::merge_geometry(&g, &mut splits, log, ctl)?;

    log.phase("Converting to OGFs...");
    tree::flex2ogf(&g, splits, &mut tree, opts.noise, log);
    ctl.check().map_err(adapt)?;

    log.phase("Converting MU-models to OGFs...");
    log.status("MU : Models...".into());
    tree::mu_calc_ogf(&g, &mut tree, log);
    tree::mu_export_geometry(&g, &mut tree);
    log.status("MU : References...".into());
    tree::mu_export_refs(&g, &extra, &mut tree)?;

    log.phase("Building sectors...");
    let sectors = tree::build_sectors(&mut extra, &mut tree, log)?;
    let mut rng = sphere::MsvcRand::min_sphere();
    tree::take_bounds(&mut tree, &mut rng);
    ctl.check().map_err(adapt)?;

    log.phase("Saving...");
    tree::save_lights(&extra, &mut level);
    tree::save_glows(&g, &extra, &mut tree, &mut level);
    let (geom_main, geom_fast) = tree::save_tree(&g, &extra, &mut tree, &mut level, log);
    tree::save_sectors(&sectors, &mut level);
    out.put("level", level.into_bytes());
    out.put("level.geom", geom_main);
    out.put("level.geomx", geom_fast);
    out.put("build.err", g.mesh.errors.write_err());
    log.end_phase();
    Ok(out)
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| Error::Io(format!("{} could not be read: {e}", path.display())))
}

fn write_all(dir: &Path, out: &Output, report: &mut Report) -> Result<()> {
    for (name, bytes) in &out.files {
        let path = dir.join(name.replace('\\', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Io(format!("{} could not be made: {e}", parent.display())))?;
        }
        std::fs::write(&path, bytes)
            .map_err(|e| Error::Io(format!("{} could not be written: {e}", path.display())))?;
        report.files.push((name.clone(), bytes.len()));
    }
    Ok(())
}

/// `XrLC -f <level>`: the level folder's `build.prj` compiled, the files written beside it
/// (`build.err` too: the C++ writes it to its logs).
pub fn compile(input: &Input, opts: &Options, ctl: &Ctl<'_>, log: &mut Log) -> Result<Report> {
    let t = Instant::now();
    log.phase(&format!("Reading project [{}]...", input.level.display()));
    let prj = BuildProject::read(&read(&input.level.join("build.prj"))?)?;
    log.msg(format!("version: {}", prj.version));
    let shaders = input.sources.shaders()?;
    log.status("Processing textures...".into());
    let textures = input.sources.textures(&prj.textures, TextureUse::Level { radiosity: false })?;
    let out = compile_project(&prj, shaders, textures, opts, ctl, log)?;
    let mut report = Report::default();
    write_all(&input.level, &out, &mut report)?;
    report.seconds = t.elapsed().as_secs_f64();
    report.lines = std::mem::take(&mut log.lines);
    report.phases = log.phases.clone();
    Ok(report)
}

/// `xrDO_Light -f <level>`: `build.details` lit into `level.details` from `build.cform`,
/// `build.lights` and `build.prj` of the level folder.
pub fn compile_details(input: &Input, ctl: &Ctl<'_>, log: &mut Log) -> Result<Report> {
    let t = Instant::now();
    log.phase("Loading level...");
    let dir = &input.level;
    let prj = BuildProject::read(&read(&dir.join("build.prj"))?)?;
    let shaders = input.sources.shaders()?;
    let textures = input.sources.textures(&prj.textures, TextureUse::Details)?;
    let details = read(&dir.join("build.details"))?;
    let cform = read(&dir.join("build.cform"))?;
    let lights = read(&dir.join("build.lights"))?;
    log.phase("Lighting nodes...");
    let bytes = compile_details_in_memory(&prj, shaders, textures, &details, &cform, &lights, ctl, log)?;
    let mut out = Output::default();
    out.put("level.details", bytes);
    let mut report = Report::default();
    write_all(dir, &out, &mut report)?;
    log.end_phase();
    report.seconds = t.elapsed().as_secs_f64();
    report.lines = std::mem::take(&mut log.lines);
    report.phases = log.phases.clone();
    Ok(report)
}
