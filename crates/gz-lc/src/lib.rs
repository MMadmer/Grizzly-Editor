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
#[cfg(test)]
mod tests_pieces;

use std::path::{Path, PathBuf};
use std::time::Instant;

use gz_fmt_level::BuildProject;
use gz_lc_light::{Ctl, LcError, LcGlobalData, LightChoice, Lighter, Shaders, Smoothing, TextureUse};
use rayon::prelude::*;

pub use details::compile_details_in_memory;
pub use load::{AssetTextures, Extra, Sources};

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
    /// `-piece`: the build project is a piece of a level of the game - its authored objects, with
    /// the rest of the level as occluders: the structure, the light and the atlas as for a level,
    /// without the collision, the sectors, the portals and the lights; the output is one piece.
    pub piece: bool,
    /// `-cells=<metres>`: the level streams by cells of this size (0: it does not). The level keeps
    /// its sectors, portals, lights and every visual larger than a cell; the rest is one piece per
    /// cell.
    pub cells: u32,
}

/// What a compile makes of its build project.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Layout {
    /// The level, as one file.
    Whole,
    /// The level beside a piece per streaming cell of this many metres.
    Cells(f32),
    /// One piece of a level the game has.
    Piece,
}

impl Layout {
    fn of(o: &Options) -> Self {
        match (o.piece, o.cells) {
            (true, _) => Self::Piece,
            (false, 0) => Self::Whole,
            (false, c) => Self::Cells(c as f32),
        }
    }

    fn cells(self) -> f32 {
        if let Self::Cells(c) = self { c } else { 0.0 }
    }
}

/// The name of a module's piece of a level of the game, the folder under the level's `pieces`.
pub const PIECE_NAME: &str = "main";
/// What a streaming cell's piece is named: `cell_<x>_<z>`.
pub const CELL_PREFIX: &str = "cell_";

/// The name of a streaming cell's piece.
#[must_use]
pub fn cell_name(c: [i32; 2]) -> String {
    format!("{CELL_PREFIX}{}_{}", c[0], c[1])
}

/// The files of one piece of a level, by their names in its folder.
#[derive(Clone, Debug, Default)]
pub struct PieceFiles {
    /// The folder under the level's `pieces`: [`PIECE_NAME`] or a cell's ([`cell_name`]).
    pub name: String,
    pub files: Vec<(String, Vec<u8>)>,
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
                "silent" | "o" | "keep_temp_files" => {}
                t if t.starts_with("light=") => match LightChoice::parse(&t["light=".len()..]) {
                    Some(c) => o.light = c,
                    None => {
                        return Err(Error::Refused(format!(
                            "{a}: the light is baked on auto, gpu, cpu or off."
                        )));
                    }
                },
                "piece" => o.piece = true,
                t if t.starts_with("cells=") => match t["cells=".len()..].parse::<u32>() {
                    Ok(n) => o.cells = n,
                    _ => {
                        return Err(Error::Refused(format!(
                            "{a}: the streaming cells' size is a whole number of metres (0: none)."
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
    /// The pieces the compile made (`-piece`: one; `-cells=`: one per cell), each written into a
    /// folder of its own under the level's `pieces`.
    pub pieces: Vec<PieceFiles>,
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
    /// The pieces written, each with its files.
    pub pieces: Vec<(String, Vec<(String, usize)>)>,
}

/// Where a compile reads and writes.
#[derive(Clone, Debug)]
pub struct Input {
    /// The level folder: `build.prj` and the other build files in, the level out (for a piece: the
    /// build's scratch).
    pub level: PathBuf,
    pub sources: Sources,
    /// The level's `pieces` folder, where the pieces go - and where the streaming cells of an
    /// earlier compile this one does not write are removed from. `None`: no pieces are written.
    pub pieces: Option<PathBuf>,
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
    let layout = Layout::of(opts);
    if layout != Layout::Piece && !prj.occluders.is_empty() {
        return Err(Error::Invalid(
            "build.prj holds occluders, which only a piece's build has: compile it with -piece.".into(),
        ));
    }
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
    let cform = cform::build_cform(&mut g, log, ctl)?;
    // a piece's collision is the module's overlay, made from the scene
    if layout != Layout::Piece {
        out.put("level.cform", cform);
        tree::save_portals(&extra, &mut level);
    }
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
    split::merge_geometry(&g, &mut splits, layout.cells(), log, ctl)?;
    let charts = split::visual_charts(&g, &splits);
    let mut places = split::places(&g, &splits, layout.cells());
    ctl.check().map_err(adapt)?;
    // the files of each piece, by its folder's name
    let mut piece_files: std::collections::BTreeMap<String, Vec<(String, Vec<u8>)>> =
        std::collections::BTreeMap::new();

    // LIGHT: every light value through the lighter
    g.shrink = !opts.no_lm_shrink;
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
        // the textures a piece's visuals name lie in its folder; a level's cells read the level's
        if layout == Layout::Piece {
            let files = piece_files.entry(PIECE_NAME.to_owned()).or_default();
            files.push((format!("{name}.dds"), base));
            files.push((format!("{name}_lm.dds"), lm));
        } else {
            out.put(format!("{name}.dds"), base);
            out.put(format!("{name}_lm.dds"), lm);
        }
    }
    ctl.check().map_err(adapt)?;

    // ATLAS: what was lit packed, the charts of one visual on one page
    log.phase(PHASES[15]);
    // with cells, every cell's visuals on pages of its own (group 0 the level's)
    let (pages, packed): (Vec<(Option<String>, gz_lc_light::LightmapPage)>, _) = match layout {
        Layout::Whole => {
            let (pages, packed) = g.pack_lightmaps(&charts, ctl)?;
            (pages.into_iter().map(|p| (None, p)).collect(), packed)
        }
        Layout::Piece => {
            // a piece's pages are its own: the smallest that holds it
            let groups = vec![0u32; charts.len()];
            let (pages, packed) = g.pack_lightmaps_grouped(&charts, &groups, 0, ctl)?;
            (pages.into_iter().map(|(_, p)| (Some(PIECE_NAME.to_owned()), p)).collect(), packed)
        }
        Layout::Cells(_) => {
            let mut cells: Vec<[i32; 2]> = places
                .iter()
                .filter_map(|p| match p {
                    tree::Place::Cell(c) => Some(*c),
                    _ => None,
                })
                .collect();
            cells.sort_unstable();
            cells.dedup();
            let groups: Vec<u32> = places
                .iter()
                .map(|p| match p {
                    tree::Place::Cell(c) => cells.binary_search(c).map_or(0, |k| k as u32 + 1),
                    _ => 0,
                })
                .collect();
            // the base keeps whole pages, a cell the smallest that holds it
            let (pages, packed) = g.pack_lightmaps_grouped(&charts, &groups, 1, ctl)?;
            let folder =
                |grp: u32| grp.checked_sub(1).and_then(|k| cells.get(k as usize)).map(|c| cell_name(*c));
            (pages.into_iter().map(|(grp, p)| (folder(grp), p)).collect(), packed)
        }
    };
    log.msg(format!(
        "{} lightmap pages, {:.1} % of their texels taken by the charts.",
        packed.pages,
        packed.fill * 100.0
    ));
    let (cut, bare) = split::split_by_page(&g, &mut splits, &mut places, log);
    if cut > 0 {
        log.warn(format!("{cut} visuals were cut by lightmap page: their charts did not fit one page."));
    }
    if bare > 0 {
        log.msg(format!(
            "{bare} visuals hold faces without area, which take no lightmap: those faces are visuals of their own."
        ));
    }
    let encoded: Vec<_> =
        pages.par_iter().map(|(at, p)| p.dds().map(|d| (at.clone(), p.name.clone(), d))).collect();
    drop(pages);
    for r in encoded {
        let (at, name, (a, b)) = r?;
        match at {
            None => {
                out.put(format!("{name}_1.dds"), a);
                out.put(format!("{name}_2.dds"), b);
            }
            Some(piece) => {
                let files = piece_files.entry(piece).or_default();
                files.push((format!("{name}_1.dds"), a));
                files.push((format!("{name}_2.dds"), b));
            }
        }
    }
    ctl.check().map_err(adapt)?;

    // OUTPUT
    log.phase(PHASES[16]);
    tree::flex2ogf(&g, splits, &places, &mut tree, opts.noise, log, ctl)?;
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[17]);
    log.status("The models".into());
    tree::mu_calc_ogf(&g, &mut tree, log, ctl)?;
    tree::mu_export_geometry(&g, &mut tree);
    log.status("The references".into());
    tree::mu_export_refs(&g, &extra, &mut tree, layout != Layout::Piece)?;
    if let Layout::Cells(size) = layout {
        tree::place_models(&mut tree, size);
    }
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[18]);
    let mut rng = sphere::MsvcRand::min_sphere();
    let parts = match layout {
        Layout::Whole => {
            let sectors = tree::build_sectors(&mut extra, &mut tree, log)?;
            tree::take_bounds(&mut tree, &mut rng);
            Parts { sectors, cells: Vec::new(), piece: Vec::new() }
        }
        Layout::Cells(_) => {
            let (sectors, cells) = tree::build_sectors_cells(&mut extra, &mut tree, log)?;
            tree::take_bounds(&mut tree, &mut rng);
            tree::bound_empty_roots(&mut tree, &sectors, &cells);
            log.msg(format!("{} streaming cells.", cells.len()));
            Parts { sectors, cells, piece: Vec::new() }
        }
        Layout::Piece => {
            let piece = tree::piece_roots(&mut extra, &mut tree);
            tree::take_bounds(&mut tree, &mut rng);
            log.msg(format!("{} roots of the piece.", piece.len()));
            Parts { sectors: Vec::new(), cells: Vec::new(), piece }
        }
    };
    out.structure = structure_digest(&tree, &parts, layout != Layout::Whole, &out);
    ctl.check().map_err(adapt)?;

    log.phase(PHASES[19]);
    ctl.check().map_err(adapt)?;
    let quality = g.params.quality;
    match layout {
        Layout::Whole => {
            tree::save_lights(&extra, &mut level);
            tree::save_glows(&g, &extra, &mut tree, &mut level);
            let (geom_main, geom_fast) = tree::save_tree(&g, &extra, &mut tree, &mut level, log);
            tree::save_sectors(&parts.sectors, &mut level);
            out.put("level", level.into_bytes());
            out.put("level.geom", geom_main);
            out.put("level.geomx", geom_fast);
        }
        Layout::Cells(_) => {
            let ids: Vec<usize> =
                (0..tree.nodes.len()).filter(|&i| tree.nodes[i].place == tree::Place::Base).collect();
            let (mut base, map) = tree::sub_tree(&g, &tree, &ids)?;
            tree::save_lights(&extra, &mut level);
            tree::save_glows(&g, &extra, &mut base, &mut level);
            let (geom_main, geom_fast) = tree::save_tree(&g, &extra, &mut base, &mut level, log);
            let sectors: Vec<tree::Sector> = parts
                .sectors
                .iter()
                .map(|s| tree::Sector {
                    root: map.get(&s.root).map_or(0, |&r| r as usize),
                    portals: s.portals.clone(),
                })
                .collect();
            tree::save_sectors(&sectors, &mut level);
            out.put("level", level.into_bytes());
            out.put("level.geom", geom_main);
            out.put("level.geomx", geom_fast);
            for c in &parts.cells {
                ctl.check().map_err(adapt)?;
                let place = tree::Place::Cell(c.cell);
                let ids: Vec<usize> =
                    (0..tree.nodes.len()).filter(|&i| tree.nodes[i].place == place).collect();
                let files = save_piece(&g, &extra, &tree, &ids, &c.roots, quality, log)?;
                piece_files.entry(cell_name(c.cell)).or_default().extend(files);
            }
        }
        Layout::Piece => {
            let ids: Vec<usize> =
                (0..tree.nodes.len()).filter(|&i| tree.nodes[i].place != tree::Place::Dropped).collect();
            let roots: Vec<(usize, u16)> =
                parts.piece.iter().map(|&r| (r, gz_fmt_level::PIECE_SECTOR_AUTO)).collect();
            let files = save_piece(&g, &extra, &tree, &ids, &roots, quality, log)?;
            piece_files.entry(PIECE_NAME.to_owned()).or_default().extend(files);
        }
    }
    out.put("build.err", g.mesh.errors.write_err());
    out.invalid_faces = g.mesh.errors.invalid_faces;
    out.pieces = piece_files.into_iter().map(|(name, files)| PieceFiles { name, files }).collect();
    log.end_phase();
    Ok(out)
}

/// What the sectors' phase made: the level's sectors, the cells' pieces with their roots, a
/// module piece's roots.
struct Parts {
    sectors: Vec<tree::Sector>,
    cells: Vec<tree::CellPiece>,
    piece: Vec<usize>,
}

/// One piece's files from the nodes `ids` of the tree: `piece` (the header, the shader table,
/// the visuals, the roots - each with its sector), `piece.geom`, `piece.geomx` when a visual has
/// fast geometry, and `piece.ltx` with the roots' box.
fn save_piece(
    g: &gz_lc_light::LcGlobalData,
    extra: &Extra,
    tree: &tree::Tree,
    ids: &[usize],
    roots: &[(usize, u16)],
    quality: u16,
    log: &mut Log,
) -> Result<Vec<(String, Vec<u8>)>> {
    let (mut sub, map) = tree::sub_tree(g, tree, ids)?;
    let mut piece = gz_chunk::Writer::new();
    piece.chunk(1, |w| {
        w.w_u16(gz_fmt_level::LEVEL_COMPILER_VERSION);
        w.w_u16(quality);
    });
    let (geom_main, geom_fast) = tree::save_tree(g, extra, &mut sub, &mut piece, log);
    let mut written = Vec::with_capacity(roots.len());
    let mut bb = gz_fmt_level::Aabb::INVALID;
    for &(r, sector) in roots {
        let at = map.get(&r).copied().ok_or_else(|| {
            Error::Invalid(format!("The root {r} of a piece is not one of the piece's visuals."))
        })?;
        if let Some(n) = tree.nodes.get(r) {
            bb.merge(&n.base.bbox);
        }
        written.push((at, sector));
    }
    tree::save_roots(&written, &mut piece);
    let mut files = vec![("piece".to_owned(), piece.into_bytes()), ("piece.geom".to_owned(), geom_main)];
    if tree::has_fast(&sub) {
        files.push(("piece.geomx".to_owned(), geom_fast));
    }
    files.push(("piece.ltx".to_owned(), piece_ltx(&bb).into_bytes()));
    Ok(files)
}

/// A piece's manifest: version 1, its box, its sectors by each root, streamed.
#[must_use]
pub fn piece_ltx(b: &gz_fmt_level::Aabb) -> String {
    let n = [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z].map(|v| format!("{v:.3}")).join(", ");
    format!("[piece]\r\nversion = 1\r\nbox     = {n}\r\nsector  = auto\r\nstream  = true\r\n")
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

/// The mark of a swap that went through, in [`STAGE_OLD`]: what lies beside it only waits to be
/// removed.
const SWAPPED: &str = ".swapped";

/// What a compile that was stopped, failed or crashed left of its stage beside the level.
fn clean_stage(dir: &Path) {
    let _ = recover_stage(dir, false);
}

/// What an earlier compile left of its stage in `dir`. A swap that did not end left what it had
/// moved aside in [`STAGE_OLD`]: it goes back where it was, over whatever the swap had put there
/// (`folders`: a piece's folder whole; else file by file), so the last level stands whole again.
/// Then the stage goes. What cannot go back stays where it is, and the stage with it: nothing a
/// compile moved aside is removed before its place holds it again.
fn recover_stage(dir: &Path, folders: bool) -> std::io::Result<()> {
    let (new, old) = (dir.join(STAGE_NEW), dir.join(STAGE_OLD));
    if old.is_dir() {
        if !old.join(SWAPPED).is_file() {
            if folders {
                for e in std::fs::read_dir(&old)? {
                    let e = e?;
                    let at = dir.join(e.file_name());
                    if at.is_dir() {
                        std::fs::remove_dir_all(&at)?;
                    }
                    rename(&e.path(), &at)?;
                }
            } else {
                put_back(&old, dir)?;
            }
        }
        std::fs::remove_dir_all(&old)?;
    }
    if new.is_dir() {
        std::fs::remove_dir_all(&new)?;
    }
    Ok(())
}

/// Every file under `from` moved to the same place under `to`, over what is there.
fn put_back(from: &Path, to: &Path) -> std::io::Result<()> {
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type()?.is_dir() {
            std::fs::create_dir_all(&dst)?;
            put_back(&src, &dst)?;
        } else {
            rename(&src, &dst)?;
        }
    }
    Ok(())
}

/// The swap went through: the stage says so, then goes. A stage that cannot be removed now - a
/// scanner reads a file of it - goes with the next compile, which the mark tells to remove it
/// and put nothing back.
fn end_swap(new: &Path, old: &Path) {
    if old.is_dir() {
        let _ = std::fs::write(old.join(SWAPPED), b"");
    }
    let _ = std::fs::remove_dir_all(new);
    let _ = std::fs::remove_dir_all(old);
}

/// A swap that failed: what it moved aside goes back; when that fails too, the error says where
/// the earlier files are.
fn undo_swap(dir: &Path, folders: bool, e: Error) -> Error {
    match recover_stage(dir, folders) {
        Ok(()) => e,
        Err(back) => Error::Io(format!(
            "{e} The files of the earlier compile could not be put back ({back}): they are in {}.",
            dir.join(STAGE_OLD).display()
        )),
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
    rename(from, to).map_err(|e| io_err("moved into place", to, &e))
}

/// A move inside the level's folder. Whoever reads what the compile has just written - an
/// on-access scanner, an indexer - holds it for a moment and the system refuses the move
/// meanwhile: a few tries some milliseconds apart, under a second in all.
fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0u64;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if tries < 9 && e.kind() == std::io::ErrorKind::PermissionDenied => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(20 * tries));
            }
            r => return r,
        }
    }
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
    // what a compile that crashed while it moved left behind: the last level whole again first
    recover_stage(dir, false).map_err(|e| io_err("cleared of an earlier compile's stage", dir, &e))?;
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
        // a new file that replaced nothing goes; what was moved aside goes back over the rest
        for name in placed.iter().filter(|n| !aside.contains(n)) {
            let _ = std::fs::remove_file(rel_path(dir, name));
        }
        return Err(undo_swap(dir, false, e));
    }
    // the level is in place
    end_swap(&new, &old);
    for (name, bytes) in &out.files {
        report.files.push((name.clone(), bytes.len()));
    }
    Ok(())
}

/// The pieces into their folders under `dir`, each whole or not at all: written beside into
/// [`STAGE_NEW`], the folder an earlier compile left moved aside into [`STAGE_OLD`], the new one
/// moved in, the old one removed. With `stale_cells`, the cells' folders (`cell_*`) this compile
/// does not write are removed too. The caller's write guard has judged `dir`.
fn write_pieces(dir: &Path, pieces: &[PieceFiles], stale_cells: bool, report: &mut Report) -> Result<()> {
    let (new, old) = (dir.join(STAGE_NEW), dir.join(STAGE_OLD));
    if dir.is_dir() {
        recover_stage(dir, true).map_err(|e| io_err("cleared of an earlier compile's stage", dir, &e))?;
    }
    // the pieces this call has put into place
    let mut placed: Vec<&str> = Vec::new();
    for p in pieces {
        let staged = new.join(&p.name);
        let made = (|| {
            std::fs::create_dir_all(&staged).map_err(|e| io_err("made", &staged, &e))?;
            for (name, bytes) in &p.files {
                let path = rel_path(&staged, name);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| io_err("made", parent, &e))?;
                }
                std::fs::write(&path, bytes).map_err(|e| io_err("written", &path, &e))?;
            }
            let at = dir.join(&p.name);
            if at.exists() {
                std::fs::create_dir_all(&old).map_err(|e| io_err("made", &old, &e))?;
                rename(&at, &old.join(&p.name)).map_err(|e| io_err("moved aside", &at, &e))?;
            }
            rename(&staged, &at).map_err(|e| io_err("moved into place", &at, &e))
        })();
        if let Err(e) = made {
            // a new piece that replaced none goes; the pieces moved aside go back, this compile's
            // earlier ones too: the pieces are the last compile's, all of them
            for name in placed.iter().filter(|n| !old.join(n).exists()) {
                let _ = std::fs::remove_dir_all(dir.join(name));
            }
            return Err(undo_swap(dir, true, e));
        }
        placed.push(p.name.as_str());
        report.pieces.push((p.name.clone(), p.files.iter().map(|(n, b)| (n.clone(), b.len())).collect()));
    }
    end_swap(&new, &old);
    if stale_cells && let Ok(listing) = std::fs::read_dir(dir) {
        for e in listing.flatten() {
            let Ok(name) = e.file_name().into_string() else { continue };
            let cell = name.to_ascii_lowercase().starts_with(CELL_PREFIX);
            if cell
                && e.file_type().is_ok_and(|t| t.is_dir())
                && !pieces.iter().any(|p| p.name.eq_ignore_ascii_case(&name))
            {
                std::fs::remove_dir_all(e.path()).map_err(|err| io_err("removed", &e.path(), &err))?;
            }
        }
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
    let layout = Layout::of(opts);
    if layout != Layout::Whole && input.pieces.is_none() {
        return Err(Error::Refused(
            "A level's pieces need the folder they go into: build the level with level.build.".into(),
        ));
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
    write_all(&input.level, &out, layout != Layout::Piece, &mut report)?;
    if let Some(dir) = &input.pieces {
        // a level's own build owns its cells: those it does not write are an earlier build's
        let stale_cells = layout != Layout::Piece;
        write_pieces(dir, &out.pieces, stale_cells, &mut report)?;
    }
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
/// page, colour or fit. A level that streams and a piece also hold where every visual went and the
/// roots of the pieces (`parted`: left out for a level of one file, whose number stays as it was).
fn structure_digest(tree: &tree::Tree, parts: &Parts, parted: bool, out: &Output) -> u64 {
    let sectors = &parts.sectors;
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
        // every list with its length: no two structures run into the same stream
        for faces in [&o.data.faces, &o.fast.faces] {
            h.u(faces.len() as u64);
            for f in faces {
                f.iter().for_each(|&i| h.u(u64::from(i)));
            }
        }
        for swi in [&o.data.swi, &o.fast.swi] {
            h.u(swi.len() as u64);
            for s in swi {
                h.u(u64::from(s.offset));
                h.u(u64::from(s.num_tris));
                h.u(u64::from(s.num_verts));
            }
        }
        h.u(o.fast.vertices.len() as u64);
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
                h.u(children.len() as u64);
                children.iter().for_each(|&c| h.u(u64::from(c)));
            }
            tree::Item::Lod { children, material, faces } => {
                h.u(4);
                h.u(children.len() as u64);
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
    for m in &tree.mu {
        h.u(m.len() as u64);
        for o in m {
            ogf(&mut h, o);
        }
    }
    h.u(sectors.len() as u64);
    for s in sectors {
        h.u(s.root as u64);
        h.u(s.portals.len() as u64);
        s.portals.iter().for_each(|&p| h.u(u64::from(p)));
    }
    if let Some((_, cform)) = out.files.iter().find(|(n, _)| n == "level.cform") {
        h.bytes(cform);
    }
    if parted {
        for n in &tree.nodes {
            match n.place {
                tree::Place::Base => h.u(0),
                tree::Place::Cell([x, z]) => {
                    h.u(1);
                    h.u(x as u32 as u64);
                    h.u(z as u32 as u64);
                }
                tree::Place::Dropped => h.u(2),
            }
        }
        h.u(parts.cells.len() as u64);
        for c in &parts.cells {
            h.u(c.cell[0] as u32 as u64);
            h.u(c.cell[1] as u32 as u64);
            h.u(c.roots.len() as u64);
            for &(r, s) in &c.roots {
                h.u(r as u64);
                h.u(u64::from(s));
            }
        }
        parts.piece.iter().for_each(|&r| h.u(r as u64));
    }
    h.0
}
