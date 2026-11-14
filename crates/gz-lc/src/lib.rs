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
use gz_lc_light::{Ctl, LcError, LcGlobalData, LightChoice, Lighter, Shaders, Smoothing, TextureUse};
use rayon::prelude::*;

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
    /// `-no_optimize`: the pre-optimisation is skipped.
    pub no_optimize: bool,
    /// `-no_invalidefaces`: invalid faces do not stop the compile.
    pub no_invalid_faces: bool,
    /// `-att`: point lights fall off linearly.
    pub linear: bool,
    /// `-threads=<n>`: the compile on a pool of its own of `n` threads instead of the caller's.
    /// The result does not depend on the number of threads: a test compiles on one to prove it.
    pub threads: Option<usize>,
    /// `-light=auto|gpu|cpu|off`: where the light is baked. The compiler lights with the lighter it
    /// is given; this is the word its caller makes that lighter from.
    pub light: LightChoice,
    /// `-no_lm_shrink`: every lightmap keeps the size its geometry gives it, flat or smooth as its
    /// light may be - the charts, the merge and the packing measured at their real sizes under a
    /// constant light.
    pub no_lm_shrink: bool,
    /// `-lm_second_page`: for a game that reads a lightmap page's second texture only (the sun
    /// and the hemisphere). The lightmaps take no static light and every page's first texture is a
    /// 4 x 4 placeholder: half of the lightmaps' bytes and the static lights' rays are not made.
    pub lm_second_page: bool,
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
                "no_lm_shrink" => o.no_lm_shrink = true,
                "lm_second_page" => o.lm_second_page = true,
                "silent" | "o" | "keep_temp_files" => {}
                t if t.starts_with("light=") => match LightChoice::parse(&t["light=".len()..]) {
                    Some(c) => o.light = c,
                    None => {
                        return Err(Error::Refused(format!(
                            "{a}: the light is baked on auto, gpu, cpu or off."
                        )));
                    }
                },
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

/// How a phase's first line starts: `* Phase: Lighting the lightmaps`. Whoever follows the log
/// tells the phases from the other lines by it.
pub const PHASE_STARTED: &str = "* Phase: ";
/// How a phase's last line starts: `* Phase done: Lighting the lightmaps (12.500 s)`.
pub const PHASE_DONE: &str = "* Phase done: ";

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

    /// A new phase, by the plain name of what it does: the one before ends with its seconds.
    pub fn phase(&mut self, name: &str) {
        self.end_phase();
        self.push(format!("{PHASE_STARTED}{name}"));
        self.started = Some((name.to_owned(), Instant::now()));
    }

    fn end_phase(&mut self) {
        if let Some((name, t)) = self.started.take() {
            let s = t.elapsed().as_secs_f64();
            self.push(format!("{PHASE_DONE}{name} ({s:.3} s)"));
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
    /// A hash of the structure the compile made - the visuals' faces, vertices, bounds and sectors,
    /// the static models' references, the collision - and nothing of its light: two compiles of one
    /// project with any lighter on any number of threads answer the same.
    pub structure: u64,
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
    /// Where the light was baked: the lighter's name.
    pub lighter: String,
    /// The samples it lit.
    pub samples: u64,
    /// [`Output::structure`]: the same on every lighter, machine and number of threads.
    pub structure: u64,
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

/// The phases of the level compile in their order, by the plain names the log and the job's stage
/// show ([`Log::phase`]); the compile of a level folder reads its project first ([`READING`]).
pub const PHASES: [&str; 20] = [
    "Loading the build project into the compiler",
    "Optimizing the geometry",
    "Tessellating the geometry for its vertex light",
    "Building the normals",
    "Building the collision",
    "Building the tangent basis",
    "Building the ray model of the light",
    "Resolving the materials",
    "Mapping the lightmaps",
    "Subdividing the geometry",
    "Merging the geometry into visuals",
    "Lighting the static models",
    "Lighting the implicit textures",
    "Lighting the lightmaps",
    "Lighting the vertices",
    "Packing the lightmaps",
    "Converting the geometry to visuals",
    "Converting the static models to visuals",
    "Building the sectors",
    "Writing the level",
];
/// The first phase of a level folder's compile.
pub const READING: &str = "Reading the build project";
/// The phases of the detail light.
pub const DETAIL_PHASES: [&str; 2] =
    ["Reading the level for the detail light", "Lighting the detail objects"];

/// The scene the light is traced in, for a measurement: a read `build.prj` taken through the
/// phases up to the ray model (the load, the optimisation, the tessellation, the normals, the
/// collision, the tangent basis), with [`LcGlobalData::ray`] built. Writes nothing.
pub fn light_scene(
    prj: &BuildProject,
    shaders: Shaders,
    textures: Vec<gz_lc_light::BuildTexture>,
    opts: &Options,
    ctl: &Ctl<'_>,
) -> Result<LcGlobalData> {
    let mut log = Log::default();
    let (mut g, extra) = load::load(prj, shaders, textures, !opts.nosmg)?;
    g.nosun = opts.nosun;
    g.linear = opts.linear;
    let smoothing = if opts.nosmg { Smoothing::Angle } else { Smoothing::EdgeFlags };
    if !opts.no_optimize {
        geom::pre_optimize(&mut g, &extra.scene_bb, &mut log, opts.no_invalid_faces, ctl)?;
    }
    geom::correct_tjunctions(&g, &mut log, ctl)?;
    geom::adaptive_ht(&mut g, &extra.scene_bb, &mut log, ctl)?;
    g.calc_normals(smoothing, ctl)?;
    g.mu_calc_normals(smoothing, ctl)?;
    cform::build_cform(&mut g, &mut log, ctl)?;
    geom::tangent_basis(&mut g, &mut log, ctl)?;
    g.light_prepare();
    g.build_rapid_with(&extra.scene_bb, ctl)?;
    Ok(g)
}

/// The whole compile in memory: a read `build.prj` with the given shaders and prepared textures
/// loaded into the compiler's model, then every phase. `log` follows it phase by phase; `ctl`
/// stops it within a moment in any phase (every long loop checks it per item or per chunk).
/// `lighter` is what bakes the light.
pub fn compile_project(
    prj: &BuildProject,
    shaders: Shaders,
    textures: Vec<gz_lc_light::BuildTexture>,
    opts: &Options,
    lighter: &dyn Lighter,
    ctl: &Ctl<'_>,
    log: &mut Log,
) -> Result<Output> {
    log.phase(PHASES[0]);
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
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[1]);
    if !opts.no_optimize {
        geom::pre_optimize(&mut g, &extra.scene_bb, log, opts.no_invalid_faces, ctl)?;
    }
    geom::correct_tjunctions(&g, log, ctl)?;
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[2]);
    geom::adaptive_ht(&mut g, &extra.scene_bb, log, ctl)?;

    log.phase(PHASES[3]);
    g.calc_normals(smoothing, ctl)?;
    log.status("The normals of the static models".into());
    g.mu_calc_normals(smoothing, ctl)?;
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[4]);
    out.put("level.cform", cform::build_cform(&mut g, log, ctl)?);
    tree::save_portals(&extra, &mut level);
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[5]);
    geom::tangent_basis(&mut g, log, ctl)?;

    log.phase(PHASES[6]);
    g.light_prepare();
    let form = g.build_rapid_with(&extra.scene_bb, ctl)?;
    out.put("build.cform", form.write());
    ctl.check().map_err(adapt)?;

    // STRUCTURE: the visuals are made from the geometry alone, before any light
    log.phase(PHASES[7]);
    let mut splits = split::resolve_materials(&mut g, log);
    g.mesh.isolate_vertices();
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[8]);
    split::uv_map(&mut g, &mut splits, log, ctl)?;
    g.mesh.isolate_vertices();

    log.phase(PHASES[9]);
    split::subdivide(&mut g, &mut splits, log, ctl)?;
    g.mesh.isolate_vertices();
    g.mesh.reload_vertex_pool();
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[10]);
    split::merge_geometry(&g, &mut splits, log, ctl)?;
    let charts = split::visual_charts(&g, &splits);
    ctl.check().map_err(adapt)?;

    // LIGHT: every light value through the lighter
    g.shrink = !opts.no_lm_shrink;
    g.second_page_only = opts.lm_second_page;
    let implicit = {
        let mut phase = |i: usize| {
            for n in lighter.notes() {
                log.warn(n);
            }
            log.phase(PHASES[11 + i]);
        };
        g.bake(lighter, ctl, &mut phase)?
    };
    for n in lighter.notes() {
        log.warn(n);
    }
    log.msg(format!("The light was baked on {}: {} samples lit.", lighter.name(), lighter.samples()));
    let implicit_dds: Vec<_> = implicit.par_iter().map(|m| m.dds().map(|d| (m.name.clone(), d))).collect();
    for r in implicit_dds {
        let (name, (base, lm)) = r?;
        out.put(format!("{name}.dds"), base);
        out.put(format!("{name}_lm.dds"), lm);
    }
    ctl.check().map_err(adapt)?;

    // ATLAS: what was lit packed, the charts of one visual on one page
    log.phase(PHASES[15]);
    let (pages, packed) = g.pack_lightmaps(&charts, ctl)?;
    log.msg(format!(
        "{} lightmap pages, {:.1} % of their texels taken by the charts.",
        packed.pages,
        packed.fill * 100.0
    ));
    if split::split_by_page(&g, &mut splits, log) > 0 {
        log.warn(format!(
            "{} visuals were cut by lightmap page: their charts did not fit one page.",
            packed.split.len()
        ));
    }
    let second_only = opts.lm_second_page;
    if second_only && !pages.is_empty() {
        log.msg(
            "The lightmaps hold the sun and the hemisphere: a page's first texture is a placeholder.".into(),
        );
    }
    let encoded: Vec<_> = pages
        .par_iter()
        .map(|p| if second_only { p.dds_second_only() } else { p.dds() }.map(|d| (p.name.clone(), d)))
        .collect();
    drop(pages);
    for r in encoded {
        let (name, (a, b)) = r?;
        out.put(format!("{name}_1.dds"), a);
        out.put(format!("{name}_2.dds"), b);
    }
    ctl.check().map_err(adapt)?;

    // OUTPUT
    log.phase(PHASES[16]);
    tree::flex2ogf(&g, splits, &mut tree, opts.noise, log, ctl)?;
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[17]);
    log.status("The models".into());
    tree::mu_calc_ogf(&g, &mut tree, log, ctl)?;
    tree::mu_export_geometry(&g, &mut tree);
    log.status("The references".into());
    tree::mu_export_refs(&g, &extra, &mut tree)?;
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[18]);
    let sectors = tree::build_sectors(&mut extra, &mut tree, log)?;
    let mut rng = sphere::MsvcRand::min_sphere();
    tree::take_bounds(&mut tree, &mut rng);
    out.structure = structure_digest(&tree, &sectors, &out);
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[19]);
    ctl.check().map_err(adapt)?;
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

/// What a compile that was stopped, failed or crashed left of its stage beside the level.
fn clean_stage(dir: &Path) {
    for d in [STAGE_NEW, STAGE_OLD] {
        let p = dir.join(d);
        if p.is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        }
    }
}

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
pub fn compile(
    input: &Input,
    opts: &Options,
    lighter: &dyn Lighter,
    ctl: &Ctl<'_>,
    log: &mut Log,
) -> Result<Report> {
    if let Some(n) = opts.threads {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .map_err(|e| Error::Invalid(format!("The compiler's {n} threads did not start: {e}")))?;
        let opts = Options { threads: None, ..opts.clone() };
        log.msg(format!("* {n} worker threads of its own (-threads={n})"));
        return pool.install(|| compile(input, &opts, lighter, ctl, log));
    }
    let t = Instant::now();
    log.phase(READING);
    log.msg(input.level.join("build.prj").display().to_string());
    let prj = BuildProject::read(&read(&input.level.join("build.prj"))?)?;
    log.msg(format!("version: {}", prj.version));
    let shaders = input.sources.shaders()?;
    ctl.check().map_err(adapt)?;
    log.status("Processing the textures".into());
    let textures = input.sources.textures(&prj.textures, TextureUse::Level { radiosity: false })?;
    ctl.check().map_err(adapt)?;
    let out = match compile_project(&prj, shaders, textures, opts, lighter, ctl, log) {
        Ok(out) => out,
        Err(e) if e.cancelled() => {
            clean_stage(&input.level);
            return Err(e);
        }
        Err(Error::InvalidFaces(n, err)) => {
            // written before the refusal: the author finds the faces in the viewport from it
            let path = input.level.join("build.err");
            std::fs::write(&path, &err)
                .map_err(|e| Error::Io(format!("{} could not be written: {e}", path.display())))?;
            return Err(Error::InvalidFaces(n, Vec::new()));
        }
        Err(e) => return Err(e),
    };
    let mut report = Report {
        invalid_faces: out.invalid_faces,
        structure: out.structure,
        lighter: lighter.name(),
        samples: lighter.samples(),
        ..Report::default()
    };
    // the last moment a Stop costs nothing: the level folder is not touched yet
    ctl.check().map_err(adapt)?;
    write_all(&input.level, &out, true, &mut report)?;
    report.seconds = t.elapsed().as_secs_f64();
    report.lines = std::mem::take(&mut log.lines);
    report.phases = log.phases.clone();
    report.warnings = log.warnings.clone();
    Ok(report)
}

/// The detail slots of a level folder lit: `build.details` into `level.details` from `build.cform`,
/// `build.lights` and `build.prj` of the level folder.
pub fn compile_details(input: &Input, lighter: &dyn Lighter, ctl: &Ctl<'_>, log: &mut Log) -> Result<Report> {
    let t = Instant::now();
    log.phase(DETAIL_PHASES[0]);
    let dir = &input.level;
    let prj = BuildProject::read(&read(&dir.join("build.prj"))?)?;
    let shaders = input.sources.shaders()?;
    let textures = input.sources.textures(&prj.textures, TextureUse::Details)?;
    let details = read(&dir.join("build.details"))?;
    let cform = read(&dir.join("build.cform"))?;
    let lights = read(&dir.join("build.lights"))?;
    ctl.check().map_err(adapt)?;
    log.phase(DETAIL_PHASES[1]);
    let bytes =
        compile_details_in_memory(&prj, shaders, textures, &details, &cform, &lights, lighter, ctl, log)
            .inspect_err(|_| clean_stage(dir))?;
    ctl.check().map_err(adapt)?;
    let mut out = Output::default();
    out.put("level.details", bytes);
    let mut report = Report { lighter: lighter.name(), samples: lighter.samples(), ..Report::default() };
    // the detail compiler writes level.details alone: the level's pages are not its own
    write_all(dir, &out, false, &mut report)?;
    log.end_phase();
    report.seconds = t.elapsed().as_secs_f64();
    report.lines = std::mem::take(&mut log.lines);
    report.phases = log.phases.clone();
    report.warnings = log.warnings.clone();
    Ok(report)
}

/// FNV-1a over the words the structure is made of.
struct Fnv(u64);

impl Fnv {
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 = (self.0 ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn u(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    fn f(&mut self, v: f32) {
        self.u(u64::from(v.to_bits()));
    }

    fn v3(&mut self, v: glam::Vec3) {
        [v.x, v.y, v.z].into_iter().for_each(|x| self.f(x));
    }

    fn aabb(&mut self, b: &gz_fmt_level::Aabb) {
        self.v3(b.min);
        self.v3(b.max);
    }
}

/// [`Output::structure`]: everything section 2 of the level build's contract holds still across
/// lighters - which faces make which visual, their order, bounds, vertices' positions, normals,
/// tangent basis, base texture coordinates, indices and windows, the static models' visuals and
/// references, the sectors and the collision - and nothing of the light: no lightmap coordinate,
/// page, colour or fit.
fn structure_digest(tree: &tree::Tree, sectors: &[tree::Sector], out: &Output) -> u64 {
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    let ogf = |h: &mut Fnv, o: &ogf::Ogf| {
        h.u(u64::from(o.material));
        h.u(u64::from(o.sector));
        h.u(o.data.vertices.len() as u64);
        for v in &o.data.vertices {
            h.v3(v.p);
            h.v3(v.n);
            h.bytes(&[v.t.x, v.t.y, v.t.z, v.b.x, v.b.y, v.b.z]);
            if let Some(uv) = v.uv.first() {
                h.f(uv.x);
                h.f(uv.y);
            }
        }
        for f in o.data.faces.iter().chain(&o.fast.faces) {
            f.iter().for_each(|&i| h.u(u64::from(i)));
        }
        for s in o.data.swi.iter().chain(&o.fast.swi) {
            h.u(u64::from(s.offset));
            h.u(u64::from(s.num_tris));
            h.u(u64::from(s.num_verts));
        }
        o.fast.vertices.iter().for_each(|&p| h.v3(p));
    };
    h.u(tree.nodes.len() as u64);
    for n in &tree.nodes {
        h.u(n.base.level as u64);
        h.u(u64::from(n.base.sector));
        h.aabb(&n.base.bbox);
        h.v3(n.base.c);
        h.f(n.base.r);
        match &n.item {
            tree::Item::Ogf(o) => {
                h.u(1);
                ogf(&mut h, o);
            }
            tree::Item::Ref(r) => {
                h.u(2);
                h.u(u64::from(r.material));
                h.u(r.model.0 as u64);
                h.u(r.model.1 as u64);
                r.xform.m.iter().flatten().for_each(|&x| h.f(x));
            }
            tree::Item::Node(children) => {
                h.u(3);
                children.iter().for_each(|&c| h.u(u64::from(c)));
            }
            tree::Item::Lod { children, material, faces } => {
                h.u(4);
                children.iter().for_each(|&c| h.u(u64::from(c)));
                h.u(u64::from(*material));
                for v in faces.iter().flatten() {
                    h.v3(v.v);
                    h.f(v.t.x);
                    h.f(v.t.y);
                }
            }
        }
    }
    for m in tree.mu.iter().flatten() {
        ogf(&mut h, m);
    }
    for s in sectors {
        h.u(s.root as u64);
        s.portals.iter().for_each(|&p| h.u(u64::from(p)));
    }
    if let Some((_, cform)) = out.files.iter().find(|(n, _)| n == "level.cform") {
        h.bytes(cform);
    }
    h.0
}
