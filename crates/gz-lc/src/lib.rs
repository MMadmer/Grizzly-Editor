//! The level compiler (geometry, collision and light) and the detail compiler, on the model and
//! the light of `gz-lc-light`.
//!
//! ```text
//! compile(&Input, &Options, &Ctl) -> Report        a level folder's build.prj in; level, level.geom,
//!                                                   level.geomx, level.cform, build.lights,
//!                                                   build.cform and the light maps out
//! compile_details(&Input, &Ctl) -> Report          the detail slots lit: level.details out
//! compile_project(&BuildProject, Shaders, textures, &Options, &Ctl) -> Output
//!                                                   the same in memory, for a caller with its data
//! Options::parse(flags)                             the compiler's switches from a command line
//! ```
//!
//! **Modules**: `load` (the build project into the compiler's model), `geom` (optimising, the
//! T-junctions, the adaptive tessellation, the tangent basis), `cform` (the collision form and its
//! simplification), `qslim` (the progressive meshes), `mender` (the normal and tangent mender),
//! `split` (the materials resolved, the UV maps, the subdivision, the merge), `ogf` (the visuals'
//! faces), `stripify` (the triangle strips), `sphere` (the bounds and the minimal spheres), `tree`
//! (the visuals' tree, the multiple-usage models, the sectors, the savers), `details` (the detail
//! light).
//!
//! Every phase runs on the caller's rayon pool and gives the same bytes on any number of
//! threads and on every run; nothing here holds a global, and nothing makes a pool but the
//! test's option `-threads=<n>`. The files are written beside the level and moved into place
//! together at the end.

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
    /// The count and `build.err` with their corners, which [`compile`] writes before it answers.
    #[error("{0} invalid faces. Compilation aborted.")]
    InvalidFaces(u32, Vec<u8>),
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

/// The compiler's switches. `level.compile` and `level.build` take them as named parameters.
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
    /// `-threads=<n>`: the compile on a pool of its own of `n` threads instead of the caller's.
    /// The result does not depend on the number of threads: a test compiles on one to prove it.
    pub threads: Option<usize>,
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
                t if t.starts_with("threads=") => match t["threads=".len()..].parse::<usize>() {
                    Ok(n) if n > 0 => o.threads = Some(n),
                    _ => {
                        return Err(Error::Refused(format!(
                            "{a}: the number of threads is a whole number from 1."
                        )));
                    }
                },
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

/// The compile's log: the phases with their seconds and the lines it prints.
#[derive(Default)]
pub struct Log {
    pub lines: Vec<String>,
    pub phases: Vec<(String, f64)>,
    /// What the author should look at: the lines `warn` wrote, without the log's indent.
    pub warnings: Vec<String>,
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

    /// A line of a phase.
    pub fn msg(&mut self, s: String) {
        self.push(format!("    |    | {s}"));
    }

    /// A line the author should look at: in the log and in the report's warnings.
    pub fn warn(&mut self, s: String) {
        self.msg(s.clone());
        self.warnings.push(s);
    }

    /// What the phase works on now.
    pub fn status(&mut self, s: String) {
        self.push(format!("    | {s}"));
    }

    /// A new phase: the one before ends with its seconds.
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
    /// The faces the compiler refused as degenerate (kept under `-no_invalidefaces`; their corners are in
    /// `build.err`).
    pub invalid_faces: u32,
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
    pub warnings: Vec<String>,
    pub invalid_faces: u32,
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

/// The whole compile in memory: a read `build.prj` with the given shaders and prepared textures
/// loaded into the compiler's model, then every phase. `log` follows it phase by phase.
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
    out.invalid_faces = g.mesh.errors.invalid_faces;
    log.end_phase();
    Ok(out)
}

fn read(path: &Path) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => Error::Io(format!("{} is not there.", path.display())),
        _ => Error::Io(format!("{} could not be read: {e}", path.display())),
    })
}

/// A file of chunks is whole: its chunks end to end, the last one ending with the file. The chunk
/// reader stops at a header that does not fit, so a file cut short would otherwise read as one
/// that lacks its last chunks.
fn whole_chunks(name: &str, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Err(Error::Invalid(format!("{name} is empty.")));
    }
    let mut at = 0usize;
    while at < bytes.len() {
        let Some(head) = bytes.get(at..at + 8) else {
            return Err(Error::Invalid(format!(
                "{name} is cut short: it ends {} bytes into the header of a chunk.",
                bytes.len() - at
            )));
        };
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let left = bytes.len() - at - 8;
        if size > left {
            return Err(Error::Invalid(format!(
                "{name} is cut short: a chunk {at} bytes into it holds {size} bytes and the file ends {left} bytes into it."
            )));
        }
        at += 8 + size;
    }
    Ok(())
}

/// Where a compile writes its files before they take their places, beside them in the level
/// folder (a dot name: tooling to every walk of the editor), and where the files they replace
/// wait until the new ones are in.
const STAGE_NEW: &str = ".lc_new";
const STAGE_OLD: &str = ".lc_old";

fn io_err(what: &str, path: &Path, e: &std::io::Error) -> Error {
    Error::Io(format!("{} could not be {what}: {e}", path.display()))
}

fn rel_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(name.replace('\\', std::path::MAIN_SEPARATOR_STR))
}

fn move_file(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_err("made", parent, &e))?;
    }
    std::fs::rename(from, to).map_err(|e| io_err("moved into place", to, &e))
}

/// The lightmap pages of an earlier compile this one does not write: they would be a level of
/// two compiles.
fn stale_pages(dir: &Path, out: &Output) -> Vec<String> {
    let Ok(listing) = std::fs::read_dir(dir) else { return Vec::new() };
    listing
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            let l = n.to_ascii_lowercase();
            l.starts_with("lmap#") && (l.ends_with("_1.dds") || l.ends_with("_2.dds"))
        })
        .filter(|n| !out.files.iter().any(|(f, _)| f.eq_ignore_ascii_case(n)))
        .collect()
}

/// The level's files appear together or not at all: written beside the level into
/// [`STAGE_NEW`], then the files they replace (and, for the level compile - `pages` -, the
/// lightmap pages of an earlier compile this one does not write) moved aside into
/// [`STAGE_OLD`], the new ones moved in, the old ones removed. A compile that fails or is
/// stopped before this leaves the last level whole; one whose move fails puts the old files
/// back. The caller's write guard has judged the level folder.
fn write_all(dir: &Path, out: &Output, pages: bool, report: &mut Report) -> Result<()> {
    let (new, old) = (dir.join(STAGE_NEW), dir.join(STAGE_OLD));
    // what a compile that crashed while it moved left behind
    for d in [&new, &old] {
        if d.exists() {
            std::fs::remove_dir_all(d).map_err(|e| io_err("removed", d, &e))?;
        }
    }
    std::fs::create_dir_all(&new).map_err(|e| io_err("made", &new, &e))?;
    let staged = (|| {
        for (name, bytes) in &out.files {
            let path = rel_path(&new, name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_err("made", parent, &e))?;
            }
            std::fs::write(&path, bytes).map_err(|e| io_err("written", &path, &e))?;
        }
        Ok(())
    })();
    if let Err(e) = staged {
        let _ = std::fs::remove_dir_all(&new);
        return Err(e);
    }

    // the swap: every step that went through is undone when a later one fails
    let mut aside: Vec<String> = Vec::new();
    let mut placed: Vec<String> = Vec::new();
    let stale = if pages { stale_pages(dir, out) } else { Vec::new() };
    let names = out.files.iter().map(|(n, _)| n.clone()).chain(stale);
    let swapped = (|| {
        for name in names {
            let at = rel_path(dir, &name);
            if at.is_file() {
                move_file(&at, &rel_path(&old, &name))?;
                aside.push(name);
            }
        }
        for (name, _) in &out.files {
            move_file(&rel_path(&new, name), &rel_path(dir, name))?;
            placed.push(name.clone());
        }
        Ok(())
    })();
    if let Err(e) = swapped {
        for name in placed.iter().rev() {
            let _ = std::fs::remove_file(rel_path(dir, name));
        }
        for name in aside.iter().rev() {
            let _ = move_file(&rel_path(&old, name), &rel_path(dir, name));
        }
        let _ = std::fs::remove_dir_all(&new);
        let _ = std::fs::remove_dir_all(&old);
        return Err(e);
    }
    // the level is in place: what is left of the stage goes with the next compile if not now
    let _ = std::fs::remove_dir_all(&new);
    let _ = std::fs::remove_dir_all(&old);
    for (name, bytes) in &out.files {
        report.files.push((name.clone(), bytes.len()));
    }
    Ok(())
}

/// A level folder's `build.prj` compiled, the files written beside it, `build.err` (the faces the
/// compiler refused, which the viewport draws) among them.
pub fn compile(input: &Input, opts: &Options, ctl: &Ctl<'_>, log: &mut Log) -> Result<Report> {
    if let Some(n) = opts.threads {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .map_err(|e| Error::Invalid(format!("The compiler's {n} threads did not start: {e}")))?;
        let opts = Options { threads: None, ..opts.clone() };
        log.msg(format!("* {n} worker threads of its own (-threads={n})"));
        return pool.install(|| compile(input, &opts, ctl, log));
    }
    let t = Instant::now();
    log.phase(&format!("Reading project [{}]...", input.level.display()));
    let prj = BuildProject::read(&read(&input.level.join("build.prj"))?)?;
    log.msg(format!("version: {}", prj.version));
    let shaders = input.sources.shaders()?;
    log.status("Processing textures...".into());
    let textures = input.sources.textures(&prj.textures, TextureUse::Level { radiosity: false })?;
    let out = match compile_project(&prj, shaders, textures, opts, ctl, log) {
        Ok(out) => out,
        Err(Error::InvalidFaces(n, err)) => {
            // written before the refusal: the author finds the faces in the viewport from it
            let path = input.level.join("build.err");
            std::fs::write(&path, &err)
                .map_err(|e| Error::Io(format!("{} could not be written: {e}", path.display())))?;
            return Err(Error::InvalidFaces(n, Vec::new()));
        }
        Err(e) => return Err(e),
    };
    let mut report = Report { invalid_faces: out.invalid_faces, ..Report::default() };
    write_all(&input.level, &out, true, &mut report)?;
    report.seconds = t.elapsed().as_secs_f64();
    report.lines = std::mem::take(&mut log.lines);
    report.phases = log.phases.clone();
    report.warnings = log.warnings.clone();
    Ok(report)
}

/// The detail slots of a level folder lit: `build.details` into `level.details` from `build.cform`,
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
    // the detail compiler writes level.details alone: the level's pages are not its own
    write_all(dir, &out, false, &mut report)?;
    log.end_phase();
    report.seconds = t.elapsed().as_secs_f64();
    report.lines = std::mem::take(&mut log.lines);
    report.phases = log.phases.clone();
    report.warnings = log.warnings.clone();
    Ok(report)
}
