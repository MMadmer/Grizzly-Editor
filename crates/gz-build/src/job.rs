//! A build runs beside the editor, as a job: the checks, the walk and the file work off the UI
//! thread, one build at a time (`build` group, shared with the packaging), Stop between files.

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::text::fits_cp1251;
use gz_editor::{CmdError, Editor, JobCtx, JobError, JobId, ProjectSession};
use gz_mod::text::{count_text, outside_cp1251};
use gz_mod::{ManifestService, command_check, validate};
use gz_project::Project;
use gz_task::Cancelled;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::json;

use crate::check::{BuildChecks, Verdict};
use crate::install::{Feed, export_flat, flat_report, install_build};
use crate::target::plan_target;
use crate::walk::{BuildWalk, WalkScope};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildKind {
    /// Build Into Game: the linked game is the target.
    IntoGame,
    /// Build to Folder: `<target>\modules\<id>`.
    ToFolder,
    /// Export Flat Gamedata: `<target>\gamedata_<id>`.
    Flat,
}

#[derive(Clone, Debug)]
pub struct BuildRequest {
    pub kind: BuildKind,
    /// The folder the build goes to: the game root (or any folder) that gets `modules\<id>`.
    pub target: String,
    /// The author agreed to the files there going (an MCP build always has).
    pub clean: bool,
    /// On success the target is remembered as `project.ltx [xms] export_target` (the window's
    /// builds do; MCP builds do not).
    pub remember_target: bool,
}

/// What a build answers.
#[derive(Clone, Debug, Default, Serialize)]
pub struct BuildResult {
    pub ok: bool,
    pub cancelled: bool,
    pub files: usize,
    /// The folder written, or the one that refused.
    pub path: String,
    pub err: String,
    /// The problems that did not stop the build, a line each.
    pub warnings: String,
    /// What the scene's bake did.
    pub bake_note: String,
    /// What the walk passed by: tooling, `[exclude]`, links - project-relative.
    pub left_out: Vec<String>,
    /// The clean build's refusal: the first ten of the files it would delete, and how many.
    pub stale_names: Vec<String>,
    pub stale_count: usize,
}

/// The clean build's refusal as the question the window asks before it builds again with
/// `clean`. The dialog is drawn elsewhere; over MCP a build is always clean.
#[derive(Clone, Debug, Serialize)]
pub struct CleanQuestion {
    pub id: &'static str,
    pub title: &'static str,
    pub headline: String,
    pub items: Vec<String>,
    pub total: usize,
    pub consequence: &'static str,
    pub action: &'static str,
}

impl CleanQuestion {
    #[must_use]
    pub fn of(r: &BuildResult) -> Option<Self> {
        if r.ok || !r.err.starts_with("target already holds a module") {
            return None;
        }
        Some(Self {
            id: "clean_target",
            title: "Replace Module Files",
            headline: format!(
                "{} holds {} this build does not write.",
                r.path,
                count_text(r.stale_count, "file", "files")
            ),
            items: r.stale_names.clone(),
            total: r.stale_count,
            consequence: "Deleted for good - this cannot be undone. The folder then holds exactly this build.",
            action: "Delete and Build",
        })
    }
}

/// What the Task Center's row of a running build shows.
#[derive(Clone, Debug, Default, Serialize)]
pub struct BuildStatus {
    pub title: String,
    pub subtitle: String,
    /// `Check`, `Copy` and `Install` (a flat export has no Install).
    pub steps: Vec<&'static str>,
    pub step: usize,
    pub stage: String,
    /// The file being written.
    pub file: String,
    pub done: u64,
    pub total: u64,
    pub files: usize,
    pub files_total: usize,
    pub stop_label: &'static str,
    pub stop_tip: &'static str,
}

pub(crate) fn bytes_text(bytes: u64) -> String {
    let b = bytes as f64;
    if bytes >= 1 << 30 {
        format!("{:.1} GB", b / f64::from(1u32 << 30))
    } else if bytes >= 1 << 20 {
        format!("{:.0} MB", b / f64::from(1u32 << 20))
    } else {
        format!("{:.0} KB", b / 1024.0)
    }
}

impl BuildStatus {
    /// `38 MB of 1.1 GB · 12 of 24 files`; empty while the stage counts nothing.
    #[must_use]
    pub fn numbers(&self) -> String {
        if self.total == 0 {
            return String::new();
        }
        let mut out = format!("{} of {}", bytes_text(self.done), bytes_text(self.total));
        if self.files_total > 0 {
            out.push_str(&format!(
                " \u{b7} {} of {} files",
                self.files.min(self.files_total),
                self.files_total
            ));
        }
        out
    }
}

/// The builds of the editor: the job, its row, the answers. Lives in `editor.services`.
#[derive(Default)]
pub struct Builds {
    results: Mutex<Vec<(JobId, Arc<BuildResult>)>>,
    last: Mutex<Option<Arc<BuildResult>>>,
    status: Mutex<Option<BuildStatus>>,
}

/// Answers kept for callers that wait on their job.
const KEEP_RESULTS: usize = 8;

impl Builds {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    /// Starts a build of the open project as a job. Refused while another build-like job runs.
    pub fn start(editor: &Arc<Editor>, req: BuildRequest) -> Result<JobId, CmdError> {
        let session = editor.project()?;
        let builds = Self::of(editor);
        let title = if req.kind == BuildKind::Flat { "Exporting Gamedata" } else { "Building Module" };
        let ed = editor.clone();
        editor
            .jobs
            .start("build", Some("build"), title, move |ctx| async move {
                let r = Arc::new(run_build(&ed, &session, &req, &ctx).await);
                builds.finish(ctx.id, r.clone());
                if r.ok {
                    Ok(json!({ "files": r.files, "path": r.path.replace('\\', "/") }))
                } else if r.cancelled {
                    Err(JobError::from(Cancelled))
                } else {
                    Err(JobError::new(r.err.clone()))
                }
            })
            .map_err(|_| CmdError::busy("a build is running: wait for it to finish, or stop it"))
    }

    fn finish(&self, id: JobId, r: Arc<BuildResult>) {
        let mut results = self.results.lock();
        results.push((id, r.clone()));
        let excess = results.len().saturating_sub(KEEP_RESULTS);
        results.drain(..excess);
        *self.last.lock() = Some(r);
    }

    /// The answer of a build job that has ended.
    #[must_use]
    pub fn result(&self, id: JobId) -> Option<Arc<BuildResult>> {
        self.results.lock().iter().find(|(j, _)| *j == id).map(|(_, r)| r.clone())
    }

    /// The last build's answer, for a caller held until it ended.
    #[must_use]
    pub fn last(&self) -> Option<Arc<BuildResult>> {
        self.last.lock().clone()
    }

    /// The running build's row, `None` when none runs.
    #[must_use]
    pub fn status(&self) -> Option<BuildStatus> {
        self.status.lock().clone()
    }

    fn begin(&self, req: &BuildRequest) {
        let flat = req.kind == BuildKind::Flat;
        let mut steps = vec!["Check", "Copy"];
        if !flat {
            steps.push("Install");
        }
        *self.status.lock() = Some(BuildStatus {
            title: if flat { "Exporting Gamedata" } else { "Building Module" }.to_owned(),
            subtitle: req.target.clone(),
            steps,
            stop_label: if flat { "Stop Export" } else { "Stop Build" },
            stop_tip: if flat {
                "Stops the export. The folder keeps the files written so far."
            } else {
                "Stops the build. The module in the target stays as it was."
            },
            ..BuildStatus::default()
        });
    }

    fn subtitle(&self, text: String) {
        if let Some(s) = self.status.lock().as_mut() {
            s.subtitle = text;
        }
    }

    fn end(&self) {
        *self.status.lock() = None;
    }

    fn feed(&self, f: Feed<'_>, ctx: &JobCtx) {
        let mut status = self.status.lock();
        match f {
            Feed::Stage { step, text, bytes, files } => {
                if let Some(s) = status.as_mut() {
                    s.step = step;
                    s.stage = text.to_owned();
                    s.file.clear();
                    s.done = 0;
                    s.total = bytes;
                    s.files = 0;
                    s.files_total = files;
                }
                ctx.stage(text);
            }
            Feed::File { rel, bytes } => {
                if let Some(s) = status.as_mut() {
                    s.file = rel.to_backslash();
                    s.files += 1;
                    s.done += bytes;
                    if s.total > 0 {
                        ctx.fraction((s.done as f64 / s.total as f64) as f32);
                    }
                }
            }
        }
    }
}

/// The Task Center's half of a build, cheap to hand to the IO pool.
#[derive(Clone)]
struct Feeder {
    builds: Arc<Builds>,
    ctx: JobCtx,
    profile: Arc<StageProfile>,
}

impl Feeder {
    fn feed(&self, f: Feed<'_>) {
        if let Feed::Stage { text, .. } = &f {
            self.profile.begin(text);
        }
        self.builds.feed(f, &self.ctx);
    }

    fn stage(&self, step: usize, text: &str) {
        self.feed(Feed::Stage { step, text, bytes: 0, files: 0 });
    }
}

/// Where a build's time went: each stage's wall time and the process's CPU time over it (the
/// share of the cores it kept busy). Written to the log when the build ends.
#[derive(Default)]
pub(crate) struct StageProfile {
    stages: Mutex<Vec<(String, std::time::Instant, f64)>>,
}

fn process_cpu() -> f64 {
    gz_win::process_counters().map_or(0.0, |c| c.cpu_seconds)
}

impl StageProfile {
    fn begin(&self, name: &str) {
        let mut s = self.stages.lock();
        if s.last().is_some_and(|l| l.0 == name) {
            return;
        }
        s.push((name.to_owned(), std::time::Instant::now(), process_cpu()));
    }

    /// `stage wall s, cpu s (cores)` per stage, and the whole.
    pub(crate) fn text(&self) -> String {
        let s = self.stages.lock();
        let (now, cpu_now) = (std::time::Instant::now(), process_cpu());
        let cores = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get) as f64;
        let mut parts = Vec::new();
        for (i, (name, at, cpu)) in s.iter().enumerate() {
            let (end, cpu_end) = s.get(i + 1).map_or((now, cpu_now), |n| (n.1, n.2));
            let wall = end.duration_since(*at).as_secs_f64();
            let used = (cpu_end - cpu).max(0.0);
            let load = if wall > 0.0 { used / wall / cores } else { 0.0 };
            parts.push(format!("{name} {wall:.3} s wall, {used:.3} s cpu ({load:.2} of the cores)"));
        }
        if let Some(first) = s.first() {
            let wall = now.duration_since(first.1).as_secs_f64();
            let used = (cpu_now - first.2).max(0.0);
            let load = if wall > 0.0 { used / wall / cores } else { 0.0 };
            parts.push(format!("whole {wall:.3} s wall, {used:.3} s cpu ({load:.2} of the cores)"));
        }
        parts.join("; ")
    }
}

fn cancelled() -> String {
    "cancelled".to_owned()
}

/// A job of a project that closed meanwhile has nothing left to build.
fn still_open(editor: &Editor, session: &Arc<ProjectSession>, ctx: &JobCtx) -> Result<(), String> {
    let open = editor.project_opt().is_some_and(|p| Arc::ptr_eq(&p, session));
    if !open || ctx.check().is_err() { Err(cancelled()) } else { Ok(()) }
}

/// `head` and up to twelve items of `list`, `and N more` after them.
fn listed(list: &[String], sep: &str) -> String {
    let mut out = list.iter().take(12).cloned().collect::<Vec<_>>().join(sep);
    if list.len() > 12 {
        out.push_str(&format!("{}and {} more", if sep == ", " { " " } else { sep }, list.len() - 12));
    }
    out
}

/// Runs a build to its end inside a job and answers what it did. The release packaging stages
/// its module through this very function, so a release ships what a build ships and refuses
/// what a build refuses.
pub async fn run_build(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    req: &BuildRequest,
    ctx: &JobCtx,
) -> BuildResult {
    let builds = Builds::of(editor);
    builds.begin(req);
    let feeder = Feeder { builds: builds.clone(), ctx: ctx.clone(), profile: Arc::default() };
    let mut out = BuildResult::default();
    let mut warnings: Vec<String> = Vec::new();
    match build(editor, session, req, &feeder, &mut out, &mut warnings).await {
        Ok(()) => out.ok = true,
        Err(err) => out.err = err,
    }
    editor.log.info(format!("* [XMS] build stages: {}", feeder.profile.text()));
    out.warnings = warnings.join("\n");
    out.cancelled = !out.ok && out.err == "cancelled";
    builds.end();
    out
}

async fn build(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    req: &BuildRequest,
    feeder: &Feeder,
    out: &mut BuildResult,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let ctx = &feeder.ctx;
    let flat = req.kind == BuildKind::Flat;
    feeder.stage(0, "Reading the project");
    let project = session.services.get::<Project>().ok_or_else(|| "no active project".to_owned())?;
    project.tree.refresh().await;
    still_open(editor, session, ctx)?;

    // The bake of the open scene and the module's checks run at once: the checks read what the
    // project holds and the scene's own data, never the layer the bake writes, so neither waits
    // for the other (each is parallel work of its own on the pools). The files are listed again
    // once the bake has written its layer.
    let checks = BuildChecks::of(editor);
    let baking = {
        let (bake, ed, s) = (checks.bake(), editor.clone(), session.clone());
        async move {
            match bake {
                Some(b) => Some(b(ed, s).await),
                None => None,
            }
        }
    };
    if checks.bake().is_some() {
        feeder.stage(0, "Baking the scene and checking the module");
    }
    let (baked, checked) =
        futures::join!(baking, check_module(editor, session, req, feeder, &project, warnings));
    if let Some(note) = baked {
        out.bake_note = note;
    }
    let (m, dst) = checked?;
    out.path.clone_from(&dst);
    still_open(editor, session, ctx)?;

    feeder.stage(0, "Listing the files");
    project.tree.refresh().await;
    let tree = project.tree.snapshot();
    let scope = if flat { WalkScope::Gamedata } else { WalkScope::Module };
    let (rules, walk_tree, exec) = (m.exclude.clone(), tree.clone(), editor.exec.clone());
    let walk = Arc::new(editor.exec.io(move || BuildWalk::scan(&walk_tree, &rules, scope, &exec)).await);
    still_open(editor, session, ctx)?;

    let report = if flat { flat_report(&tree, &m) } else { String::new() };
    let (walk_io, clean, exec, io_feeder) = (walk.clone(), req.clean, editor.exec.clone(), feeder.clone());
    let dst_path = PathBuf::from(&dst);
    let written = editor
        .exec
        .io(move || {
            let feed = |f: Feed<'_>| io_feeder.feed(f);
            if flat {
                export_flat(&walk_io, &dst_path, &report, &exec, &io_feeder.ctx, &feed)
            } else {
                install_build(&walk_io, &dst_path, clean, &exec, &io_feeder.ctx, &feed)
            }
        })
        .await;
    match written {
        Ok(files) => out.files = files,
        Err(refusal) => {
            if refusal.err.starts_with("cannot copy ") {
                editor.log.warn(format!("! [XMS] build: {}", refusal.err));
            }
            out.stale_names = refusal.stale_names;
            out.stale_count = refusal.stale_count;
            return Err(refusal.err);
        }
    }

    out.left_out = walk.left_out_texts();
    if !out.left_out.is_empty() {
        editor.log.info(format!("* [XMS] left out of the build: {}", listed(&out.left_out, ", ")));
    }
    // The editor takes any name; the game reads a module through the ANSI APIs of a cp1251
    // system, where a character cp1251 lacks comes back as '?' and the file never opens. It
    // still ships - it is the author's.
    let unreadable: Vec<String> = walk
        .files
        .iter()
        .filter(|f| !fits_cp1251(f.rel.as_str()))
        .map(|f| {
            let rel = f.rel.to_backslash();
            let bad = outside_cp1251(&rel).unwrap_or_default();
            format!("{rel} ({bad})")
        })
        .collect();
    for name in &unreadable {
        editor.log.warn(format!("! [XMS] build: the game cannot open {name} - a character outside cp1251"));
    }
    if !unreadable.is_empty() {
        warnings.push(format!(
            "file names the game cannot open (characters outside cp1251): {}\n  {}",
            unreadable.len(),
            listed(&unreadable, "\n  ")
        ));
    }
    editor.log.info(format!(
        "* [XMS] export {}: {} file(s) -> {dst}",
        if flat { "flat" } else { "module" },
        out.files
    ));
    // what is installed in the linked game changed: its modes and modules follow
    // (C++ EditorGameModes::Invalidate)
    let game = project.game_root();
    if !flat && !game.is_empty() && gz_project::paths::same_path(&req.target, &game) {
        let p = project.clone();
        if let Err(e) = editor.exec.io(move || p.refresh_game()).await {
            editor.log.warn(format!("the linked game could not be read again: {e}"));
        }
    }
    if req.remember_target
        && let Err(e) = project.set_value("xms", "export_target", &req.target).await
    {
        editor.log.warn(e.message);
    }
    Ok(())
}

/// The module's half of a build's checks: the manifest, the areas' checks over the files the
/// project holds, the console commands, the target folder. Answers the manifest and the folder
/// the build writes.
async fn check_module(
    editor: &Arc<Editor>,
    session: &Arc<ProjectSession>,
    req: &BuildRequest,
    feeder: &Feeder,
    project: &Arc<Project>,
    warnings: &mut Vec<String>,
) -> Result<(gz_mod::Manifest, String), String> {
    let ctx = &feeder.ctx;
    let flat = req.kind == BuildKind::Flat;
    let checks = BuildChecks::of(editor);
    let manifest = ManifestService::of_session(editor, session).await.map_err(|e| e.message)?;
    manifest.sync().await.map_err(|e| e.message)?;
    let snap = manifest.snapshot();
    let m = snap.manifest().ok_or_else(|| "mod.ltx not found in the project".to_owned())?.clone();
    feeder.builds.subtitle(format!("{} \u{b7} {}", m.title(), req.target));
    let own = Verdict::of(&validate(&m), None, true);
    warnings.extend(own.warnings);
    if !own.err.is_empty() {
        return Err(own.err);
    }

    // the files the project holds now: what the areas check
    let tree = project.tree.snapshot();
    let scope = if flat { WalkScope::Gamedata } else { WalkScope::Module };
    let (rules, walk_tree, exec) = (m.exclude.clone(), tree, editor.exec.clone());
    let walk = Arc::new(editor.exec.io(move || BuildWalk::scan(&walk_tree, &rules, scope, &exec)).await);
    still_open(editor, session, ctx)?;

    // the areas, in parallel over the same walk; the first in their order that refuses decides
    let areas = checks.checks();
    if !areas.is_empty() {
        if checks.bake().is_none() {
            feeder.stage(
                0,
                if areas.len() == 1 {
                    areas.first().map_or("Checking the module", |c| c.stage())
                } else {
                    "Checking the module"
                },
            );
        }
        let found = futures::future::join_all(
            areas.iter().map(|c| c.check(editor.clone(), session.clone(), walk.clone())),
        )
        .await;
        for (check, problems) in areas.iter().zip(found) {
            let v = Verdict::of(&problems, check.headline(), false);
            warnings.extend(v.warnings);
            if !v.err.is_empty() {
                return Err(v.err);
            }
        }
        still_open(editor, session, ctx)?;
    }

    // the game refuses a bad name, a taken one or a function it cannot call
    let (_, problems) = command_check(editor, session, &m).await;
    let v = Verdict::of(&problems, None, false);
    warnings.extend(v.warnings);
    if !v.err.is_empty() {
        return Err(v.err);
    }

    let (target, id, root, game) =
        (req.target.clone(), m.id.clone(), session.root.clone(), project.game_root());
    let p = project.clone();
    let dst = editor
        .exec
        .io(move || plan_target(&target, &id, flat, &root, &game, &p.write_guard(Some(&id))))
        .await?;
    Ok((m, dst))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts() {
        assert_eq!(bytes_text(600), "1 KB");
        assert_eq!(bytes_text(3 << 20), "3 MB");
        assert_eq!(bytes_text(3 << 29), "1.5 GB");
        let s = BuildStatus {
            done: 1 << 20,
            total: 2 << 20,
            files: 3,
            files_total: 24,
            ..BuildStatus::default()
        };
        assert_eq!(s.numbers(), "1 MB of 2 MB \u{b7} 3 of 24 files");
        let many: Vec<String> = (0..14).map(|i| format!("f{i}")).collect();
        assert!(listed(&many, ", ").ends_with("f11 and 2 more"));
        assert!(listed(&many, "\n  ").ends_with("f11\n  and 2 more"));
        let refused = BuildResult {
            path: "D:\\G\\modules\\od".into(),
            err: "target already holds a module: 2 file(s) there are not part of this build".into(),
            stale_count: 2,
            ..BuildResult::default()
        };
        let q = CleanQuestion::of(&refused).map(|q| q.headline);
        assert_eq!(q.as_deref(), Some("D:\\G\\modules\\od holds 2 files this build does not write."));
        assert!(CleanQuestion::of(&BuildResult { ok: true, ..refused }).is_none());
    }
}
