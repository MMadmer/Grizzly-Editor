//! The level compilers in the editor's process: `gz_lc::compile` (the geometry and the light) and
//! `gz_lc::compile_details` (the detail objects' light) on a level folder, inside the job that
//! runs them - a step of the level build, or `level.compile` - on the editor's CPU pool (every
//! core), stopped by the job's Stop.
//!
//! The compile reads the editor's own data (`shaders_xrlc.xr`, the textures and their `.thm`, as
//! the editor's `fsgame.ltx` names them) - a texture the open project's texture asset publishes
//! read from that asset first ([`asset_textures`]) - and the level folder's build files, and
//! writes the level beside them. Its phases are the job's stage, its log lines go to the Output Log, its report -
//! seconds per phase, the files, the warnings, the invalid faces - is the job's result.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use gz_base::{Finding, Severity};
use gz_editor::{Editor, JobCtx, JobError, json};
use gz_lc::{AssetTextures, GameData, Input, Log, Options, Sources};
use gz_lc_light::{Ctl, Lighter, Report as Progress};
use gz_media::Index;
use gz_project::asset_kind::MediaKind;

/// The textures the project's texture assets publish, by the name a surface gives: the
/// description and the picture of the asset (`docs/ASSETS.md` section 6), what the compiler
/// asks before the textures folder. Reads the asset when it is asked for. An excluded asset
/// publishes nothing.
#[must_use]
pub fn asset_textures(root: PathBuf, index: Arc<Index>) -> AssetTextures {
    AssetTextures(Arc::new(move |name: &str| {
        let e = index.by_name(MediaKind::Texture, name)?;
        let t = gz_media::resolve::texture(&root, e.path.as_str(), &e.name).ok()?;
        Some((t.params, t.image))
    }))
}

/// What a compile runs, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The geometry, the collision and the light.
    Level,
    /// The detail objects' light.
    Details,
}

impl Part {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Level => "level",
            Self::Details => "details",
        }
    }

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Level => "the level compiler",
            Self::Details => "the detail light compiler",
        }
    }
}

/// A compile of one level folder.
#[derive(Clone, Debug)]
pub struct Request {
    pub level: String,
    /// The project's `gamedata\levels\<level>`.
    pub dir: PathBuf,
    pub parts: Vec<Part>,
    /// The compiler's flags (`-nosun`, `-noise`, ...); unknown ones are named in the log.
    pub flags: String,
    /// The level's `pieces` folder in the module (`<project>\levels\<level>\pieces`), where its
    /// streaming cells or the module's piece go; `None`: the compile writes no piece and leaves the
    /// folder as it is.
    pub pieces: Option<PathBuf>,
    /// `dir` holds a piece's build project: the compile makes the module's piece of a level of the
    /// game (`-piece`).
    pub piece: bool,
    /// The level's streaming cells in metres (`-cells=`), 0: none.
    pub cells: u32,
}

/// The parts a level folder can be compiled with: the level always, the details when the build
/// wrote `build.details`. Blocking.
#[must_use]
pub fn parts_of(dir: &std::path::Path, details: Option<bool>) -> Vec<Part> {
    let with_details = details.unwrap_or_else(|| dir.join("build.details").is_file());
    if with_details { vec![Part::Level, Part::Details] } else { vec![Part::Level] }
}

/// Runs the request inside a job: each part on the CPU pool, the job's stage following the
/// compiler's phases. Answers the report of every part.
pub async fn run(editor: &Arc<Editor>, req: Request, ctx: &JobCtx) -> Result<serde_json::Value, JobError> {
    let (mut opts, unknown) =
        Options::parse(req.flags.split_whitespace()).map_err(|e| JobError::new(e.to_string()))?;
    opts.piece |= req.piece;
    if req.cells > 0 {
        opts.cells = req.cells;
    }
    for u in &unknown {
        editor.log.warn(format!("The compiler does not know the option '{u}'; it is left out."));
    }
    // the lighter of the job, made once: a graphics card's device is made here and nowhere else
    let choice = opts.light;
    let chosen = editor
        .exec
        .cpu(move || gz_lc_gpu::lighter(choice))
        .await
        .map_err(|why| JobError::new(format!("light={}: {why}", choice.name())))?;
    if let Some(note) = &chosen.note {
        editor.log.info(note);
    }
    let lighter: Arc<dyn Lighter> = Arc::from(chosen.lighter);
    let on = lighter.name();
    let threads = editor.exec.cpu_pool().current_num_threads();
    let mut sources = Sources::of_root(&editor.paths.root);
    if editor.project_opt().is_some()
        && let Ok(media) = gz_media::Media::of(editor).await
    {
        let index = media.fresh_index().await;
        sources = sources.with_assets(asset_textures(media.root().to_path_buf(), index));
    }
    // a level of the game wears textures only the game has: read from it, never written
    if let Ok((_, project)) = gz_project::Project::of(editor) {
        let p = project.clone();
        if let Ok(files) = editor.exec.io(move || p.game_files()).await {
            sources = sources.with_game(GameData(Arc::new(move |path: &str| files.read(path).ok())));
        }
    }
    editor.log.info(format!(
        "Compiling {} on {threads} threads, the light baked on {on}; data from {}.",
        req.level,
        sources.game_data.display()
    ));
    let mut parts = Vec::new();
    let mut total = 0.0;
    for part in &req.parts {
        let part = *part;
        ctx.check()?;
        ctx.stage(match part {
            Part::Level => format!("Compiling the level, the light on {on}"),
            Part::Details => format!("Lighting the detail objects on {on}"),
        });
        let input = Input { level: req.dir.clone(), sources: sources.clone(), pieces: req.pieces.clone() };
        let (opts2, ctx2) = (opts.clone(), ctx.clone());
        let out = Throttle::new(ctx.clone(), editor.log.clone(), on.clone());
        let lighter = lighter.clone();
        let worker = out.clone();
        let work = editor.exec.cpu(move || {
            let echo_out = worker.clone();
            let echo = move |line: &str| echo_out.line(line);
            let report = |r: Progress<'_>| match r {
                Progress::Stage(s) => worker.stage(s),
                Progress::Progress { done, total } => worker.progress(done, total),
            };
            let ctl = Ctl::new(&ctx2.cancel, &report);
            let mut lc_log = Log::with_echo(Box::new(echo));
            match part {
                Part::Level => gz_lc::compile(&input, &opts2, lighter.as_ref(), &ctl, &mut lc_log),
                Part::Details => gz_lc::compile_details(&input, lighter.as_ref(), &ctl, &mut lc_log),
            }
        });
        // a silent phase still reaches the job and the log: what is pending is passed on at the
        // throttle's pace by a timer, never faster
        tokio::pin!(work);
        let mut tick = tokio::time::interval(THROTTLE);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let done = loop {
            tokio::select! {
                r = &mut work => break r,
                _ = tick.tick() => out.tick(),
            }
        };
        out.flush();
        let report = match done {
            Ok(r) => r,
            Err(e) if e.cancelled() => {
                editor.log.warn(format!("The compile of {} was stopped.", req.level));
                return Err(JobError { message: "Stopped.".to_owned(), cancelled: true });
            }
            Err(e) => {
                let text = format!("{} failed on {}: {e}", capital(part.title()), req.level);
                editor.log.error(&text);
                return Err(JobError::new(text));
            }
        };
        // the compiler's marks are read here, once: nothing downstream shows one
        let warnings: Vec<&str> = report.warnings.iter().map(|w| read_mark(w.trim()).1).collect();
        for w in &warnings {
            editor.log.warn(w);
        }
        editor.log.info(format!("{} ended in {:.1} s.", capital(part.title()), report.seconds));
        total += report.seconds;
        parts.push(json!({
            "part": part.name(),
            "seconds": report.seconds,
            "phases": report.phases.iter().map(|(n, s)| json!({ "name": n, "seconds": s })).collect::<Vec<_>>(),
            "files": report.files.iter().map(|(n, s)| json!({ "name": n, "size": s })).collect::<Vec<_>>(),
            "warnings": warnings,
            "invalid_faces": report.invalid_faces,
            "lighter": report.lighter,
            "samples": report.samples,
            "structure": format!("{:016x}", report.structure),
            "pieces": report.pieces.iter().map(|(name, files)| json!({
                "name": name,
                "files": files.iter().map(|(n, s)| json!({ "name": n, "size": s })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }));
    }
    Ok(json!({
        "level": req.level,
        "dir": req.dir.display().to_string().replace('\\', "/"),
        "threads": threads,
        "seconds": total,
        "lighter": lighter.name(),
        "samples": lighter.samples(),
        "parts": parts,
    }))
}

/// The level the compiler's own marks give a line (`! ` an error, `~ ` a warning, `* ` or `*** `
/// a report), read once where the line enters the editor, and the line without them.
fn read_mark(line: &str) -> (Option<Severity>, &str) {
    for (mark, severity) in [
        ("*** ", Severity::Warning),
        ("! ", Severity::Error),
        ("~ ", Severity::Warning),
        ("* ", Severity::Note),
    ] {
        if let Some(text) = line.strip_prefix(mark) {
            return (Some(severity), text);
        }
    }
    (None, line)
}

/// What the compile tells the job and the Output Log, passed on four times a second: the lights
/// report per deflector from every worker and the compiler writes thousands of lines, and every
/// report wakes the window - which, on the Level tab, draws the scene again on the CPU pool the
/// compile needs (measured: l12u_control_monolith 93 s with every report passed on, 70 s with no
/// viewport drawn, 64 s headless). A timer ([`Throttle::tick`]) passes on what waits while the
/// compiler says nothing, so a long silent phase shows the moment it begins.
///
/// The job's stage is the phase's name with what the phase says of itself under it: "Lighting the
/// static models: the references, 120 of 300" - a phase's own stage never hides the phase.
#[derive(Clone)]
struct Throttle(Arc<Mutex<Pending>>, JobCtx, gz_editor::Log);

#[derive(Default)]
struct Pending {
    last: Option<Instant>,
    lines: Vec<String>,
    stage: Stage,
    /// The stage changed since it was passed on.
    moved: bool,
}

/// Where the compile is: its phase, the phase's own stage, the items done of how many.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Stage {
    phase: Option<String>,
    sub: Option<String>,
    progress: Option<(usize, usize)>,
    /// Where the light is baked: a phase that lights says so.
    on: Option<String>,
}

impl Stage {
    /// The job's stage: "Lighting the lightmaps: 1200 of 5400".
    fn text(&self) -> Option<String> {
        let phase = self.phase.as_deref()?;
        let mut out = phase.to_owned();
        if let Some(on) = self.on.as_deref().filter(|_| phase.starts_with("Lighting")) {
            out.push_str(" on ");
            out.push_str(on);
        }
        if let Some(sub) = &self.sub {
            out.push_str(": ");
            out.push_str(sub);
        }
        if let Some((done, total)) = self.progress {
            out.push_str(if self.sub.is_some() { ", " } else { ": " });
            out.push_str(&format!("{} of {total}", done.min(total)));
        }
        Some(out)
    }
}

const THROTTLE: Duration = Duration::from_millis(250);

impl Throttle {
    fn new(ctx: JobCtx, log: gz_editor::Log, on: String) -> Self {
        let p = Pending { stage: Stage { on: Some(on), ..Stage::default() }, ..Pending::default() };
        Self(Arc::new(Mutex::new(p)), ctx, log)
    }

    fn line(&self, line: &str) {
        let mut p = self.0.lock();
        if let Some(name) = line.strip_prefix(gz_lc::PHASE_STARTED) {
            let on = p.stage.on.take();
            p.stage = Stage { phase: Some(name.trim().to_owned()), sub: None, progress: None, on };
            p.moved = true;
        }
        p.lines.push(line.to_owned());
        self.maybe(p);
    }

    fn stage(&self, text: &str) {
        let mut p = self.0.lock();
        p.stage.sub = Some(text.trim().to_owned());
        p.stage.progress = None;
        p.moved = true;
        self.maybe(p);
    }

    fn progress(&self, done: usize, total: usize) {
        let mut p = self.0.lock();
        p.stage.progress = Some((done, total));
        p.moved = true;
        self.maybe(p);
    }

    /// Passes on what waits when the last pass is a throttle period old.
    fn maybe(&self, p: parking_lot::MutexGuard<'_, Pending>) {
        if p.last.is_none_or(|t| t.elapsed() >= THROTTLE) {
            self.pass(p);
        }
    }

    /// The timer's turn: what waits goes once the period is over, nothing when nothing waits.
    fn tick(&self) {
        let p = self.0.lock();
        if p.moved || !p.lines.is_empty() {
            self.maybe(p);
        }
    }

    fn flush(&self) {
        self.pass(self.0.lock());
    }

    fn pass(&self, mut p: parking_lot::MutexGuard<'_, Pending>) {
        p.last = Some(Instant::now());
        let lines = std::mem::take(&mut p.lines);
        let stage = std::mem::take(&mut p.moved).then(|| p.stage.clone());
        drop(p);
        for l in &lines {
            match read_mark(l) {
                (Some(severity), text) => self.2.finding(&Finding::new(severity, text)),
                (None, text) => self.2.plain(text),
            }
        }
        if let Some(s) = stage {
            if let Some(text) = s.text() {
                self.1.stage(text);
            }
            if let Some((done, total)) = s.progress {
                self.1.progress(done, total);
            }
        }
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An uncompressed 32-bit TGA, top-left first.
    pub(crate) fn tga(w: u16, h: u16, bgra: [u8; 4]) -> Vec<u8> {
        let mut out = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.extend_from_slice(&[32, 0x28]);
        for _ in 0..u32::from(w) * u32::from(h) {
            out.extend_from_slice(&bgra);
        }
        out
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn a_surface_that_names_a_texture_asset_is_built_from_the_asset() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let put = |rel: &str, bytes: &[u8]| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, bytes).unwrap();
        };
        put("art\\wall.tga", &tga(4, 4, [30, 60, 120, 255]));
        put("art\\wall.texasset", b"return { tex = 1, format = \"dxt5\" }\n");
        put("wip\\old.tga", &tga(2, 2, [0, 0, 0, 255]));
        put("wip\\old.texasset", b"return { tex = 1, name = \"od\\\\old\" }\n");
        put("mod.ltx", b"[module]\nid = od\n[exclude]\nwip\n");
        let assets = asset_textures(root.to_path_buf(), Arc::new(gz_media::scan(root)));
        let (params, image) = (assets.0)("OD\\Wall").unwrap();
        assert_eq!((image.width, image.height), (4, 4));
        assert_eq!(params.fmt, gz_fmt_tex::TextureFormat::DXT5);
        // excluded: no texture of the build
        assert!((assets.0)("od\\old").is_none());
        assert!((assets.0)("od\\none").is_none());

        // the compiler's textures: the asset first, the folder (empty here) for the rest
        let empty = root.join("no_textures");
        let sources = Sources { game_data: empty.clone(), textures: empty, assets: None, game: None }
            .with_assets(assets);
        let raw = gz_fmt_level::BuildTexture {
            name: "od\\wall".into(),
            width: 64,
            height: 64,
            has_alpha: 0,
            reserve: 0,
        };
        let built = sources
            .textures(std::slice::from_ref(&raw), gz_lc_light::TextureUse::Level { radiosity: false })
            .unwrap();
        assert!(built[0].has_surface && built[0].has_alpha);
        assert_eq!((built[0].width, built[0].height), (4, 4));
        let other = gz_fmt_level::BuildTexture { name: "od\\none".into(), ..raw };
        assert!(sources.textures(&[other], gz_lc_light::TextureUse::Level { radiosity: false }).is_err());
    }

    #[test]
    fn a_mark_is_read_once_into_a_level_and_dropped() {
        assert_eq!(read_mark("! ERROR: split failed"), (Some(Severity::Error), "ERROR: split failed"));
        assert_eq!(read_mark("~ slow"), (Some(Severity::Warning), "slow"));
        assert_eq!(
            read_mark("* Phase: Lighting the lightmaps"),
            (Some(Severity::Note), "Phase: Lighting the lightmaps")
        );
        assert_eq!(read_mark("*** 3 junctions found."), (Some(Severity::Warning), "3 junctions found."));
        assert_eq!(read_mark("    |    | detail"), (None, "    |    | detail"));
        assert_eq!(read_mark("*not a mark"), (None, "*not a mark"));
    }

    #[test]
    fn the_stage_keeps_the_phase_and_says_what_it_counts() {
        let phase = |sub: Option<&str>, progress| Stage {
            phase: Some("Lighting the static models".into()),
            sub: sub.map(str::to_owned),
            progress,
            on: None,
        };
        assert_eq!(phase(None, None).text().unwrap(), "Lighting the static models");
        assert_eq!(
            phase(Some("the references"), Some((120, 300))).text().unwrap(),
            "Lighting the static models: the references, 120 of 300"
        );
        assert_eq!(phase(None, Some((7, 5))).text().unwrap(), "Lighting the static models: 5 of 5");
        assert!(Stage::default().text().is_none(), "no phase yet: the job keeps its own stage");
        // a phase that lights says where
        let lit =
            |name: &str| Stage { phase: Some(name.into()), on: Some("the CPU".into()), ..Stage::default() };
        assert_eq!(lit("Lighting the lightmaps").text().unwrap(), "Lighting the lightmaps on the CPU");
        assert_eq!(lit("Packing the lightmaps").text().unwrap(), "Packing the lightmaps");
        // every phase and every sub-stage of the compiler is a whole name: no "...", no shouting
        for name in gz_lc::PHASES.iter().chain(&gz_lc::DETAIL_PHASES).chain([&gz_lc::READING]) {
            assert!(
                !name.ends_with('.') && !name.contains(':') && name.chars().next().unwrap().is_uppercase()
            );
            assert!(name.split(' ').all(|w| w.chars().any(char::is_lowercase)), "{name}");
        }
    }
}
