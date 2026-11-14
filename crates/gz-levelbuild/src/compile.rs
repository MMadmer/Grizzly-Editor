//! The level compilers in the editor's process: `gz_lc::compile` (the geometry and the light) and
//! `gz_lc::compile_details` (the detail objects' light) on a level folder, inside the job that
//! runs them - a step of the level build, or `level.compile` - on the editor's CPU pool (every
//! core), stopped by the job's Stop.
//!
//! The compile reads the editor's own data (`shaders_xrlc.xr`, the textures and their `.thm`, as
//! the editor's `fsgame.ltx` names them) and the level folder's build files, and writes the level
//! beside them. Its phases are the job's stage, its log lines go to the Output Log, its report -
//! seconds per phase, the files, the warnings, the invalid faces - is the job's result.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use gz_editor::{Editor, JobCtx, JobError, json};
use gz_lc::{Input, Log, Options, Sources};
use gz_lc_light::{Ctl, Report as Progress};

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
    let (opts, unknown) =
        Options::parse(req.flags.split_whitespace()).map_err(|e| JobError::new(e.to_string()))?;
    for u in &unknown {
        editor.log.warn(format!("The compiler does not know the option '{u}'; it is left out."));
    }
    let threads = editor.exec.cpu_pool().current_num_threads();
    let sources = Sources::of_root(&editor.paths.root);
    editor.log.info(format!(
        "Compiling {} on {threads} threads; data from {}.",
        req.level,
        sources.game_data.display()
    ));
    let mut parts = Vec::new();
    let mut total = 0.0;
    for part in &req.parts {
        let part = *part;
        ctx.check()?;
        ctx.stage(match part {
            Part::Level => "Compiling the level",
            Part::Details => "Lighting the detail objects",
        });
        let (input, opts2, ctx2, log) = (
            Input { level: req.dir.clone(), sources: sources.clone() },
            opts.clone(),
            ctx.clone(),
            editor.log.clone(),
        );
        let done = editor
            .exec
            .cpu(move || {
                let out = Throttle::new(ctx2.clone(), log);
                let echo_out = out.clone();
                let echo = move |line: &str| echo_out.line(line);
                let report = |r: Progress<'_>| match r {
                    Progress::Stage(s) => out.stage(s),
                    Progress::Progress { done, total } => out.progress(done, total),
                };
                let ctl = Ctl::new(&ctx2.cancel, &report);
                let mut lc_log = Log::with_echo(Box::new(echo));
                let r = match part {
                    Part::Level => gz_lc::compile(&input, &opts2, &ctl, &mut lc_log),
                    Part::Details => gz_lc::compile_details(&input, &ctl, &mut lc_log),
                };
                out.flush();
                r
            })
            .await;
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
        for w in &report.warnings {
            editor.log.warn(w);
        }
        editor.log.info(format!("{} ended in {:.1} s.", capital(part.title()), report.seconds));
        total += report.seconds;
        parts.push(json!({
            "part": part.name(),
            "seconds": report.seconds,
            "phases": report.phases.iter().map(|(n, s)| json!({ "name": n, "seconds": s })).collect::<Vec<_>>(),
            "files": report.files.iter().map(|(n, s)| json!({ "name": n, "size": s })).collect::<Vec<_>>(),
            "warnings": report.warnings,
            "invalid_faces": report.invalid_faces,
        }));
    }
    Ok(json!({
        "level": req.level,
        "dir": req.dir.display().to_string().replace('\\', "/"),
        "threads": threads,
        "seconds": total,
        "parts": parts,
    }))
}

/// What the compile tells the job and the Output Log, passed on four times a second: the lights
/// report per deflector from every worker and the compiler writes thousands of lines, and every
/// report wakes the window - which, on the Level tab, draws the scene again on the CPU pool the
/// compile needs (measured: l12u_control_monolith 93 s with every report passed on, 70 s with no
/// viewport drawn, 64 s headless).
#[derive(Clone)]
struct Throttle(Arc<Mutex<Pending>>, JobCtx, gz_editor::Log);

struct Pending {
    last: Instant,
    lines: Vec<String>,
    stage: Option<String>,
    progress: Option<(usize, usize)>,
}

const THROTTLE: Duration = Duration::from_millis(250);

impl Throttle {
    fn new(ctx: JobCtx, log: gz_editor::Log) -> Self {
        let p = Pending { last: Instant::now(), lines: Vec::new(), stage: None, progress: None };
        Self(Arc::new(Mutex::new(p)), ctx, log)
    }

    fn line(&self, line: &str) {
        let mut p = self.0.lock();
        if let Some(name) = line.strip_prefix("* New phase started: ") {
            p.stage = Some(name.trim_end_matches("...").to_owned());
            p.progress = None;
        }
        p.lines.push(line.to_owned());
        self.maybe(p);
    }

    fn stage(&self, text: &str) {
        let mut p = self.0.lock();
        p.stage = Some(text.to_owned());
        p.progress = None;
        self.maybe(p);
    }

    fn progress(&self, done: usize, total: usize) {
        let mut p = self.0.lock();
        p.progress = Some((done, total));
        self.maybe(p);
    }

    fn maybe(&self, p: parking_lot::MutexGuard<'_, Pending>) {
        if p.last.elapsed() >= THROTTLE {
            self.pass(p);
        }
    }

    fn flush(&self) {
        self.pass(self.0.lock());
    }

    fn pass(&self, mut p: parking_lot::MutexGuard<'_, Pending>) {
        p.last = Instant::now();
        let lines = std::mem::take(&mut p.lines);
        let (stage, progress) = (p.stage.take(), p.progress.take());
        drop(p);
        for l in &lines {
            self.2.plain(l);
        }
        if let Some(s) = stage {
            self.1.stage(s);
        }
        if let Some((done, total)) = progress {
            self.1.progress(done, total);
        }
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}
