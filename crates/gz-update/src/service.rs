//! The updater of the running editor: where its installation is, what the last check found, the
//! jobs that check, download and apply, and what the window and `update.status` read of it.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gz_editor::{CmdError, Editor, JobCtx, JobError, JobId, Origin, Value, json};
use parking_lot::Mutex;

use crate::apply;
use crate::delta;
use crate::hashes::HashCache;
use crate::manifest::{self, MANIFEST, Manifest, Row, WORK, path_key};
use crate::net::{self, Asset, Net, NetError, Ranged, Release};
use crate::prepare::{self, PrepError, Prepared};
use crate::record::{self, RecordError, Sink};
use crate::schedule::{Next, Retry};
use crate::stage::{self, Mode, Stage, StageState};
use crate::sys;
use crate::version::Version;

/// The repository every installation updates from.
pub const REPOSITORY: &str = "MMadmer/Grizzly-Editor";
const API: &str = "https://api.github.com";
const WEB: &str = "https://github.com";
/// The job group: one update job at a time.
pub const GROUP: &str = "update";
/// The jobs a restart would cut short (a build, a packaging, a compile, Play, a publish).
const BLOCKING_GROUPS: [&str; 3] = ["build", "game", "publish"];
/// How long a move of the transaction is tried, and how long the restarted editor has to report.
const MOVE_WAIT: Duration = Duration::from_secs(5);
const RESTART_WAIT: Duration = Duration::from_secs(30);

/// What `update.status` calls the updater's state (docs/UPDATES.md, section 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    Idle,
    Checking,
    Available,
    Downloading,
    Waiting,
    Ready,
    Applying,
    Failed,
    /// The release can only be installed by hand: its manifest is newer than this updater, or
    /// the editor cannot write into its folder.
    Manual,
}

impl State {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Idle => "idle",
            Self::Checking => "checking",
            Self::Available => "available",
            Self::Downloading => "downloading",
            Self::Waiting => "waiting",
            Self::Ready => "ready",
            Self::Applying => "applying",
            Self::Failed => "failed",
            Self::Manual => "manual",
        }
    }
}

/// Where the installation is and where its releases come from.
#[derive(Clone, Debug)]
pub struct Install {
    pub root: PathBuf,
    pub repo: String,
    pub api: String,
    pub web: String,
    pub qa: bool,
    /// What a restart starts.
    pub program: PathBuf,
    /// The installed manifest: the managed set.
    pub manifest: Manifest,
    /// The pace of retries (a second; shorter in a QA run).
    pub pace: Duration,
    pub move_wait: Duration,
}

/// How the editor was started, as the updater needs to know it.
#[derive(Clone, Debug, Default)]
pub struct Start {
    /// `-updated <version>`: this run is the new version an update restarted into.
    pub updated_from: Option<String>,
    /// The run has a window.
    pub window: bool,
    /// A commandlet: it runs its commands and ends, nothing starts by itself.
    pub commandlet: bool,
}

/// A note for the author, for the window to show once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub error: bool,
    pub text: String,
    /// "What's New" opens the notes.
    pub whats_new: bool,
}

/// What happens when the editor exits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The update was applied by `update.apply`: start the new version.
    Restart,
    /// Apply the staged update (Update on Exit), then start the new version when `restart`.
    Apply { restart: bool },
}

/// The plan of one release: what is read out of its archive, and how.
#[derive(Clone, Debug)]
pub struct Plan {
    pub release: Release,
    pub asset: Asset,
    pub mode: Mode,
    /// `None` until the whole archive is down, when the server does not take ranges.
    pub prepared: Option<Prepared>,
    /// The rows to stage.
    pub needed: Vec<Row>,
    /// The bytes the download costs.
    pub bytes: u64,
}

impl Plan {
    fn version(&self) -> String {
        self.release.tag.clone()
    }
}

struct Inner {
    state: State,
    latest: Option<Release>,
    plan: Option<Arc<Plan>>,
    error: Option<String>,
    /// The sentence of an idle state after a check.
    note: Option<String>,
    done: u64,
    total: u64,
    staged: usize,
    job: Option<JobId>,
    on_exit: bool,
    exit: Option<Exit>,
    updated_from: Option<String>,
    updated_to: Option<String>,
    /// The version this run replaced with `update.apply`: what the restart says it came from.
    applied_from: Option<String>,
    updated_notes: String,
    toasts: Vec<Toast>,
    /// The check at startup: an update it finds is a toast.
    startup: bool,
    /// The editor's exit was asked for by the dialog's Restart and Update.
    restart_asked: bool,
}

/// The updater. One per editor, an `Editor` service.
pub struct UpdateService {
    pub install: Option<Install>,
    off_reason: String,
    start: Start,
    inner: Mutex<Inner>,
    rev: AtomicU64,
    cache: HashCache,
    net: Option<Net>,
    settings_file: PathBuf,
    check_at_startup: Mutex<bool>,
    /// `.update\lock` while a job writes the stage: two editors never stage into one folder.
    lock: Mutex<Option<File>>,
    /// The restarted editor's report, kept open for its life.
    event: Mutex<Option<sys::Event>>,
    /// Whether this start was reported, and whether the work folder waits for that to go.
    report: Mutex<(bool, bool)>,
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_default().trim().to_owned()
}

/// Where the installation is: the executable's folder when it holds a manifest; under
/// `-qa_github` the folder `GRIZZLY_UPDATE_ROOT` names, read from the mock only.
fn resolve(editor: &Editor) -> Result<Install, String> {
    let qa = editor.flags.qa_github;
    let qa_root = if qa { env("GRIZZLY_UPDATE_ROOT") } else { String::new() };
    // a QA run talks to the mock or to nothing: without an installation named for it, it has no updater
    if qa && qa_root.is_empty() {
        return Err("A QA run without GRIZZLY_UPDATE_ROOT does not update itself.".to_owned());
    }
    let (root, repo, api, web) = if qa_root.is_empty() {
        (editor.paths.exe_dir.clone(), REPOSITORY.to_owned(), API.to_owned(), WEB.to_owned())
    } else {
        let api = env("GRIZZLY_GITHUB_API");
        if !net::is_loopback(&api) {
            return Err(
                "A QA run updates from the mock alone: GRIZZLY_GITHUB_API names no address on this computer."
                    .to_owned(),
            );
        }
        let repo = env("GRIZZLY_UPDATE_REPO");
        let web = env("GRIZZLY_GITHUB_WEB");
        (
            PathBuf::from(qa_root),
            if repo.is_empty() { REPOSITORY.to_owned() } else { repo },
            api,
            if web.is_empty() { env("GRIZZLY_GITHUB_API") } else { web },
        )
    };
    let bytes = fs::read(root.join(MANIFEST))
        .map_err(|_| "This is a development build: it does not update itself.".to_owned())?;
    let manifest = manifest::parse(&bytes).map_err(|e| {
        format!("The installation's {MANIFEST} cannot be read ({e}): the editor does not update itself.")
    })?;
    let qa_dir = !editor.paths.exe_dir.join(MANIFEST).is_file() && qa;
    let program = if qa_dir {
        std::env::current_exe().unwrap_or_else(|_| root.join("Grizzly.exe"))
    } else {
        root.join("Grizzly.exe")
    };
    let ms = |name: &str, default: u64| -> Duration {
        Duration::from_millis(if qa { env(name).parse().unwrap_or(default) } else { default })
    };
    Ok(Install {
        root,
        repo,
        api: api.trim_end_matches('/').to_owned(),
        web: web.trim_end_matches('/').to_owned(),
        qa,
        program,
        manifest,
        pace: ms("GRIZZLY_UPDATE_PACE_MS", 1000),
        move_wait: ms("GRIZZLY_UPDATE_MOVE_WAIT_MS", MOVE_WAIT.as_millis() as u64),
    })
}

/// "38 MB", "640 KB".
#[must_use]
pub fn size_text(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= 10.0 * MB {
        format!("{:.0} MB", b / MB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if bytes >= 1024 {
        format!("{} KB", bytes.div_ceil(1024))
    } else {
        format!("{bytes} bytes")
    }
}

impl UpdateService {
    fn new(editor: &Editor, start: Start, found: Result<Install, String>) -> Self {
        let home = editor.paths.home.join("update");
        let settings_file = home.join("settings.ltx");
        let check_at_startup = fs::read_to_string(&settings_file)
            .ok()
            .and_then(|t| {
                t.lines().find_map(|l| {
                    l.split_once('=')
                        .filter(|(k, _)| k.trim() == "check_at_startup")
                        .map(|(_, v)| v.trim() != "false")
                })
            })
            .unwrap_or(true);
        let (install, off_reason) = match found {
            Ok(i) => (Some(i), String::new()),
            Err(why) => (None, why),
        };
        let net = install
            .as_ref()
            .and_then(|i| Net::new(&i.api, i.qa).map_err(|e| editor.log.warn(format!("[update] {e}"))).ok());
        Self {
            install,
            off_reason,
            start,
            inner: Mutex::new(Inner {
                state: State::Idle,
                latest: None,
                plan: None,
                error: None,
                note: None,
                done: 0,
                total: 0,
                staged: 0,
                job: None,
                on_exit: false,
                exit: None,
                updated_from: None,
                updated_to: None,
                applied_from: None,
                updated_notes: String::new(),
                toasts: Vec::new(),
                startup: false,
                restart_asked: false,
            }),
            rev: AtomicU64::new(1),
            cache: HashCache::load(home.join("hashes.ltx")),
            net,
            settings_file,
            check_at_startup: Mutex::new(check_at_startup),
            lock: Mutex::new(None),
            event: Mutex::new(None),
            report: Mutex::new((false, false)),
        }
    }

    /// The service of a running editor.
    #[must_use]
    pub fn of(editor: &Editor) -> Option<Arc<Self>> {
        editor.services.get::<Self>()
    }

    /// Moves whenever something the window shows changed.
    #[must_use]
    pub fn rev(&self) -> u64 {
        self.rev.load(Ordering::Relaxed)
    }

    fn touch(&self, editor: &Editor) {
        self.rev.fetch_add(1, Ordering::Relaxed);
        editor.exec.wake().ping();
    }

    fn with<R>(&self, editor: &Editor, f: impl FnOnce(&mut Inner) -> R) -> R {
        let r = f(&mut self.inner.lock());
        self.touch(editor);
        r
    }

    #[must_use]
    pub fn state(&self) -> State {
        if self.install.is_none() { State::Off } else { self.inner.lock().state }
    }

    /// The installed version: the manifest's (the version these programs were built as, for a
    /// real installation).
    #[must_use]
    pub fn installed(&self) -> Version {
        self.install.as_ref().map_or_else(Version::compiled, |i| i.manifest.version)
    }

    #[must_use]
    pub fn check_at_startup(&self) -> bool {
        *self.check_at_startup.lock()
    }

    /// Keeps the Preferences setting in the editor's home (`update\settings.ltx`).
    pub fn set_check_at_startup(&self, editor: &Editor, on: bool) -> Result<(), CmdError> {
        *self.check_at_startup.lock() = on;
        if let Some(dir) = self.settings_file.parent() {
            fs::create_dir_all(dir)
                .map_err(|e| CmdError::new("io", format!("the update settings could not be written: {e}")))?;
        }
        fs::write(&self.settings_file, format!("[update]\ncheck_at_startup = {on}\n"))
            .map_err(|e| CmdError::new("io", format!("the update settings could not be written: {e}")))?;
        self.touch(editor);
        Ok(())
    }

    /// The notes for the window to show once; taken.
    pub fn take_toasts(&self) -> Vec<Toast> {
        std::mem::take(&mut self.inner.lock().toasts)
    }

    fn toast(&self, editor: &Editor, t: Toast) {
        if t.error {
            editor.log.warn(format!("[update] {}", t.text));
        } else {
            editor.log.info(format!("[update] {}", t.text));
        }
        self.with(editor, |i| i.toasts.push(t));
    }

    /// What the editor does when it exits, with a staged update.
    pub fn set_on_exit(&self, editor: &Editor, on: bool) {
        self.with(editor, |i| i.on_exit = on);
    }

    #[must_use]
    pub fn on_exit(&self) -> bool {
        self.inner.lock().on_exit
    }

    /// The dialog's Restart and Update asked the editor to exit; a cancelled exit leaves Update on
    /// Exit.
    pub fn set_restart_asked(&self, editor: &Editor, on: bool) {
        self.with(editor, |i| {
            i.restart_asked = on;
            if on {
                i.on_exit = true;
            }
        });
    }

    #[must_use]
    pub fn restart_asked(&self) -> bool {
        self.inner.lock().restart_asked
    }

    /// The release notes and the versions of the update this start came from, when it is one.
    #[must_use]
    pub fn updated(&self) -> Option<(String, String, String)> {
        let i = self.inner.lock();
        Some((i.updated_from.clone()?, i.updated_to.clone().unwrap_or_default(), i.updated_notes.clone()))
    }

    /// The release the last check found.
    #[must_use]
    pub fn latest(&self) -> Option<Release> {
        self.inner.lock().latest.clone()
    }

    /// The release page of a version, or of the repository.
    #[must_use]
    pub fn release_page(&self) -> String {
        let latest = self.inner.lock().latest.clone();
        let Some(inst) = &self.install else { return format!("{WEB}/{REPOSITORY}/releases") };
        match latest {
            Some(r) if r.page.starts_with(&inst.web) => r.page,
            Some(r) => format!("{}/{}/releases/tag/{}", inst.web, inst.repo, r.tag),
            None => format!("{}/{}/releases", inst.web, inst.repo),
        }
    }

    /// The sentence that says where the updater stands.
    #[must_use]
    pub fn text(&self) -> String {
        if self.install.is_none() {
            return self.off_reason.clone();
        }
        let i = self.inner.lock();
        let have = self.installed();
        let new = i.latest.as_ref().map(|r| r.tag.clone()).unwrap_or_default();
        match i.state {
            State::Off => self.off_reason.clone(),
            State::Idle => i.note.clone().unwrap_or_else(|| format!("Grizzly Editor {have} is installed.")),
            State::Checking => "Looking for a newer version of Grizzly Editor.".to_owned(),
            State::Available => format!("Grizzly Editor {new} is available. You have {have}."),
            State::Downloading => format!("Downloading Grizzly Editor {new}."),
            State::Waiting => format!("Waiting for the connection to continue the download of {new}."),
            State::Ready => format!("Grizzly Editor {new} is ready to install."),
            State::Applying => format!("Updating to {new}."),
            State::Failed | State::Manual => i.error.clone().unwrap_or_default(),
        }
    }

    /// Everything the dialog shows (docs/UPDATES.md, section 6).
    #[must_use]
    pub fn status(&self, editor: &Editor) -> Value {
        let state = self.state();
        let text = self.text();
        let i = self.inner.lock();
        let plan = i.plan.clone();
        let job = i.job.filter(|j| editor.jobs.get(*j).is_some_and(|x| x.running()));
        json!({
            "version": self.installed().to_string(),
            "latest": i.latest.as_ref().map(|r| r.tag.clone()),
            "state": state.word(),
            "text": text,
            "error": i.error,
            "installation": self.install.as_ref().map(|x| x.root.display().to_string()),
            "repository": self.install.as_ref().map_or(REPOSITORY.to_owned(), |x| x.repo.clone()),
            "download": plan.as_ref().map(|p| json!({
                "bytes": p.bytes,
                "archive": p.asset.size,
                "done": i.done,
                "total": i.total,
                "files": p.needed.len(),
                "staged": i.staged,
                "mode": p.mode.word(),
            })),
            "notes": i.latest.as_ref().map(|r| r.notes.clone()),
            "release_page": self.release_page_locked(&i),
            "check_at_startup": *self.check_at_startup.lock(),
            "on_exit": i.on_exit,
            "updated_from": i.updated_from,
            "updated_to": i.updated_to,
            "job": job,
        })
    }

    fn release_page_locked(&self, i: &Inner) -> String {
        let Some(inst) = &self.install else { return format!("{WEB}/{REPOSITORY}/releases") };
        match &i.latest {
            Some(r) if r.page.starts_with(&inst.web) => r.page.clone(),
            Some(r) => format!("{}/{}/releases/tag/{}", inst.web, inst.repo, r.tag),
            None => format!("{}/{}/releases", inst.web, inst.repo),
        }
    }

    fn installed_or_refuse(&self) -> Result<&Install, CmdError> {
        self.install.as_ref().ok_or_else(|| CmdError::new("refused", self.off_reason.clone()))
    }

    fn net(&self) -> Result<&Net, JobError> {
        self.net.as_ref().ok_or_else(|| JobError::new("The network client could not start."))
    }

    // ---- jobs --------------------------------------------------------------------------------

    /// Starts a check. `then_download`: an update it finds is downloaded in the same job (a
    /// continued download, `update.download`, `update.apply`).
    pub fn start_job(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        then_download: bool,
        origin: Origin,
    ) -> Result<JobId, CmdError> {
        let inst = self.installed_or_refuse()?.clone();
        let title = if then_download { "Updating Grizzly Editor" } else { "Checking for Updates" };
        let (svc, ed) = (self.clone(), editor.clone());
        let kind = if then_download { "update_download" } else { "update_check" };
        let started = editor.jobs.start_for_editor(kind, Some(GROUP), title, move |ctx| async move {
            let result = run_job(&ed, &svc, &inst, &ctx, then_download).await;
            svc.finish_job(&ed, &result);
            result
        });
        match started {
            Ok(id) => {
                self.with(editor, |i| i.job = Some(id));
                self.card(editor, id, origin == Origin::Mcp);
                Ok(id)
            }
            Err(busy) => Err(CmdError::new("busy", format!("{} is still running.", busy.title))
                .with_data(json!({ "job": busy.id }))),
        }
    }

    fn card(&self, editor: &Editor, job: JobId, from_mcp: bool) {
        let (bytes, done, tag) = {
            let i = self.inner.lock();
            (i.plan.as_ref().map_or(0, |p| p.bytes), i.done, i.latest.as_ref().map(|r| r.tag.clone()))
        };
        let card = json!({
            "key": "update",
            "subtitle": tag.map_or_else(|| "Grizzly Editor".to_owned(), |t| format!("Grizzly Editor {t}")),
            "numbers": if bytes > 0 { format!("{} of {}", size_text(done), size_text(bytes)) } else { String::new() },
            "from_mcp": from_mcp,
            "stop_label": "Stop Updating",
            "stop_tip": "Stops the update. What is downloaded stays and the next download continues from it.",
        });
        editor
            .services
            .get_or_init::<Mutex<BTreeMap<JobId, Value>>>(Default::default)
            .lock()
            .insert(job, card);
    }

    fn finish_job(&self, editor: &Editor, result: &Result<Value, JobError>) {
        self.release_lock();
        self.cache.save();
        self.with(editor, |i| {
            i.job = None;
            match result {
                Ok(_) => {}
                Err(e) if e.cancelled => {
                    if matches!(i.state, State::Checking) {
                        i.state = State::Idle;
                    } else if matches!(i.state, State::Downloading | State::Waiting) {
                        i.state = State::Available;
                    }
                }
                Err(e) => {
                    if matches!(i.state, State::Checking) {
                        // a check that could not look: nothing to offer, the reason kept
                        i.state = State::Idle;
                        i.note = Some(e.message.clone());
                        i.error = Some(e.message.clone());
                    } else if !matches!(i.state, State::Manual) {
                        i.state = State::Failed;
                        i.error = Some(e.message.clone());
                    }
                }
            }
        });
    }

    fn take_lock(&self, inst: &Install) -> Result<(), JobError> {
        let dir = inst.root.join(WORK);
        fs::create_dir_all(&dir).map_err(|e| {
            JobError::new(format!("The update cannot write into {}: {e}", inst.root.display()))
        })?;
        let mut opts = fs::OpenOptions::new();
        opts.create(true).truncate(false).write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            opts.share_mode(0);
        }
        let file = opts
            .open(dir.join("lock"))
            .map_err(|_| JobError::new("Another Grizzly Editor is updating this installation."))?;
        *self.lock.lock() = Some(file);
        Ok(())
    }

    fn release_lock(&self) {
        if let Some(inst) = &self.install
            && self.lock.lock().take().is_some()
        {
            let _ = fs::remove_file(inst.root.join(WORK).join("lock"));
        }
    }

    /// Stops the update job; what is staged stays.
    pub fn stop(&self, editor: &Editor) -> bool {
        editor.jobs.running_in(GROUP).is_some_and(|j| editor.jobs.cancel(j.id))
    }

    // ---- apply -------------------------------------------------------------------------------

    /// The job a restart would cut short, when one runs.
    #[must_use]
    pub fn blocking_job(editor: &Editor) -> Option<String> {
        BLOCKING_GROUPS.iter().find_map(|g| editor.jobs.running_in(g)).map(|j| j.title)
    }

    /// Verifies the stage and runs the transaction. Blocking: the IO pool or the exit.
    pub fn apply_blocking(&self, editor: &Editor) -> Result<String, String> {
        let inst = self.install.as_ref().ok_or_else(|| self.off_reason.clone())?;
        let plan = self.inner.lock().plan.clone().ok_or_else(|| "No update is staged.".to_owned())?;
        let stage = Stage::new(&inst.root, &plan.version());
        let state = stage.read_state().ok_or_else(|| "No update is staged.".to_owned())?;
        let bytes = fs::read(stage.manifest())
            .map_err(|_| "The update's manifest is missing from its stage.".to_owned())?;
        let new =
            manifest::parse(&bytes).map_err(|e| format!("The update's manifest cannot be read: {e}"))?;
        let installed = fs::read(inst.root.join(MANIFEST)).ok().and_then(|b| manifest::parse(&b).ok());
        let from = installed.as_ref().map_or_else(|| self.installed().to_string(), |m| m.version.to_string());
        let staged = apply::verify(&inst.root, &stage, &state, &new, &self.cache)?;
        apply::apply(&inst.root, &stage, &staged, installed.as_ref(), &new, &from, inst.move_wait)?;
        let _ = fs::write(stage.notes(), plan.release.notes.as_bytes());
        editor.log.info(format!("[update] the installation is now {} (it was {from})", new.version));
        Ok(from)
    }

    /// `update.apply` without `on_exit`: the transaction now, then the exit (and the restart
    /// when asked). The caller settled the unsaved work.
    pub async fn apply_now(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        restart: bool,
    ) -> Result<String, CmdError> {
        self.with(editor, |i| i.state = State::Applying);
        let (svc, ed) = (self.clone(), editor.clone());
        let done = editor.exec.io(move || svc.apply_blocking(&ed)).await;
        match done {
            Ok(from) => {
                self.with(editor, |i| {
                    i.exit = restart.then_some(Exit::Restart);
                    i.on_exit = false;
                    i.applied_from = Some(from.clone());
                });
                Ok(from)
            }
            Err(e) => {
                self.with(editor, |i| {
                    i.state = State::Ready;
                    i.error = Some(e.clone());
                });
                Err(CmdError::new("failed", e))
            }
        }
    }

    /// Asks for the exit an apply ends with; the answer of the command goes out first.
    pub fn quit_soon(editor: &Arc<Editor>) {
        let ed = editor.clone();
        editor.exec.spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            ed.request_quit();
        });
    }

    /// The editor ends: a staged update is applied (Update on Exit) and a restart starts the new
    /// version. Runs on the main thread after the window and the project closed.
    pub fn at_exit(&self, editor: &Editor) {
        let Some(inst) = &self.install else { return };
        let (exit, on_exit, ready, restart_asked) = {
            let i = self.inner.lock();
            (i.exit, i.on_exit, i.state == State::Ready, i.restart_asked)
        };
        let exit = exit.or((on_exit && ready).then_some(Exit::Apply { restart: restart_asked }));
        let Some(exit) = exit else { return };
        let version = self.inner.lock().plan.as_ref().map(|p| p.version()).unwrap_or_default();
        let stage = Stage::new(&inst.root, &version);
        let from = match exit {
            Exit::Restart => self.inner.lock().applied_from.clone().unwrap_or_default(),
            Exit::Apply { .. } => match self.apply_blocking(editor) {
                Ok(from) => from,
                Err(e) => {
                    editor.log.error(format!("[update] the update to {version} at exit did not happen: {e}"));
                    let _ = fs::write(
                        stage.failed_note(),
                        format!("The update to {version} did not happen: {e}"),
                    );
                    return;
                }
            },
        };
        if matches!(exit, Exit::Restart | Exit::Apply { restart: true }) {
            self.restart(editor, inst, &stage, &version, &from);
        }
    }

    /// Starts the new version with `-updated <from>` and the original command line and waits for
    /// its report; without one the update is rolled back.
    fn restart(&self, editor: &Editor, inst: &Install, stage: &Stage, version: &str, from: &str) {
        let mut args: Vec<std::ffi::OsString> = Vec::new();
        let mut it = std::env::args_os().skip(1);
        while let Some(a) = it.next() {
            if a.to_string_lossy().trim_start_matches('-').eq_ignore_ascii_case("updated") {
                it.next();
                continue;
            }
            args.push(a);
        }
        let started = std::process::Command::new(&inst.program)
            .arg("-updated")
            .arg(from)
            .args(&args)
            .stdin(std::process::Stdio::null())
            .spawn();
        let failure = match started {
            Err(e) => Some(format!("{} could not be started: {e}", inst.program.display())),
            Ok(mut child) => {
                let event = sys::Event::open(&sys::started_event_name(child.id()));
                let deadline = std::time::Instant::now() + RESTART_WAIT;
                loop {
                    if event.as_ref().is_ok_and(sys::Event::is_set) {
                        break None;
                    }
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        break Some(format!("Grizzly Editor {version} ended as it started"));
                    }
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        break Some(format!("Grizzly Editor {version} did not report its start in time"));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        };
        match failure {
            None => editor.log.info(format!("[update] restarted into {version}")),
            Some(why) => {
                let undone = apply::roll_back(&inst.root, stage, inst.move_wait);
                let text = match undone {
                    Ok(()) => format!("{why}, so the update was undone: Grizzly Editor {from} is installed."),
                    Err(e) => format!("{why}, and the update could not be undone: {e}"),
                };
                editor.log.error(format!("[update] {text}"));
                let _ = fs::write(stage.failed_note(), text);
            }
        }
    }

    // ---- start -------------------------------------------------------------------------------

    /// The new version reports its start to the editor that restarted into it, then tidies up.
    pub fn report_started(&self, editor: &Editor) {
        if let Some(from) = &self.start.updated_from {
            match sys::Event::open(&sys::started_event_name(std::process::id()))
                .and_then(|e| e.set().map(|()| e))
            {
                Ok(e) => *self.event.lock() = Some(e),
                Err(e) => editor.log.warn(format!(
                    "[update] the start after the update from {from} could not be reported: {e}"
                )),
            }
        }
        let cleanup = {
            let mut r = self.report.lock();
            r.0 = true;
            std::mem::take(&mut r.1)
        };
        if cleanup && let Some(inst) = &self.install {
            let root = inst.root.clone();
            let log = editor.log.clone();
            editor.exec.spawn_io(move || remove_work(&root, &log));
        }
    }

    /// What a start does: a finished update is tidied up and said; one cut short is undone; a
    /// download of the still-latest release continues; the window checks once.
    fn startup(self: &Arc<Self>, editor: &Arc<Editor>) {
        let Some(inst) = self.install.clone() else { return };
        let (svc, ed) = (self.clone(), editor.clone());
        editor.exec.spawn(async move {
            let (s2, e2, i2) = (svc.clone(), ed.clone(), inst.clone());
            let found = ed.exec.io(move || housekeeping(&s2, &e2, &i2)).await;
            if found.continue_download && !svc.start.commandlet {
                let _ = svc.start_job(&ed, true, Origin::Ui);
            }
            if svc.start.window {
                // after the first frame is on the screen
                while !ed.frames.has_window() {
                    if ed.quit_token().is_cancelled() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                svc.report_started(&ed);
                if !found.continue_download && svc.check_at_startup() && ed.jobs.running_in(GROUP).is_none() {
                    svc.with(&ed, |i| i.startup = true);
                    let _ = svc.start_job(&ed, false, Origin::Ui);
                }
            }
        });
    }
}

/// What a start found in the work folder.
#[derive(Default)]
struct Found {
    continue_download: bool,
}

fn remove_work(root: &Path, log: &gz_editor::Log) {
    let dir = root.join(WORK);
    // the editor that restarted into this one may still be letting go of its program
    for _ in 0..60 {
        if stage::remove_tree(&dir).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    log.warn(format!("[update] {} could not be removed yet; the next start tries again", dir.display()));
}

fn housekeeping(svc: &UpdateService, editor: &Editor, inst: &Install) -> Found {
    let mut found = Found::default();
    let updated_arg = svc.start.updated_from.clone();
    let mut finished = false;
    for stage in stage::stages(&inst.root) {
        if let Ok(note) = fs::read_to_string(stage.failed_note()) {
            let _ = fs::remove_file(stage.failed_note());
            svc.toast(editor, Toast { error: true, text: note, whats_new: false });
        }
        match apply::journal_state(&stage) {
            Some(j) if j.complete => {
                finished = true;
                let notes = fs::read_to_string(stage.notes()).unwrap_or_default();
                let from = updated_arg.clone().unwrap_or(j.from.clone());
                svc.with(editor, |i| {
                    i.updated_from = Some(from);
                    i.updated_to = Some(j.to.clone());
                    i.updated_notes = notes;
                });
                svc.toast(
                    editor,
                    Toast { error: false, text: format!("Updated to {}", j.to), whats_new: true },
                );
            }
            Some(j) => {
                // an apply that a crash cut short: the installation goes back to what it was
                let text = match apply::roll_back(&inst.root, &stage, inst.move_wait) {
                    Ok(()) => format!("The update to {} was cut short and undone.", j.to),
                    Err(e) => format!("The update to {} was cut short and could not be undone: {e}", j.to),
                };
                svc.toast(editor, Toast { error: true, text, whats_new: false });
                found.continue_download |= stage.read_state().is_some();
            }
            None => found.continue_download |= stage.read_state().is_some(),
        }
    }
    if let Some(from) = updated_arg
        && !finished
    {
        svc.with(editor, |i| i.updated_from = Some(from));
    }
    if finished {
        found.continue_download = false;
        // the editor that restarted into this one rolls back until it hears from it: the work
        // folder goes once this start was reported
        let wait = svc.start.updated_from.is_some() && {
            let mut r = svc.report.lock();
            r.1 = !r.0;
            r.1
        };
        if !wait {
            remove_work(&inst.root, &editor.log);
        }
    }
    found
}

// ---- the job -------------------------------------------------------------------------------

fn fail(text: impl Into<String>) -> JobError {
    JobError::new(text)
}

fn from_net(e: NetError) -> JobError {
    match e {
        NetError::Cancelled => stopped(),
        NetError::Network(s) | NetError::Refused(s) => {
            fail(format!("Grizzly Editor could not look for updates: {s}."))
        }
    }
}

/// What a stopped job ends with.
fn stopped() -> JobError {
    JobError { message: "Stopped.".to_owned(), cancelled: true }
}

async fn run_job(
    editor: &Arc<Editor>,
    svc: &Arc<UpdateService>,
    inst: &Install,
    ctx: &JobCtx,
    then_download: bool,
) -> Result<Value, JobError> {
    let reuse = {
        let i = svc.inner.lock();
        i.plan.clone().filter(|_| then_download && matches!(i.state, State::Available | State::Waiting))
    };
    let plan = match reuse {
        Some(p) => p,
        None => match check(editor, svc, inst, ctx).await? {
            Some(p) => p,
            None => return Ok(svc.status(editor)),
        },
    };
    let startup = std::mem::take(&mut svc.inner.lock().startup);
    if !then_download {
        if startup && matches!(svc.state(), State::Available) {
            svc.toast(
                editor,
                Toast {
                    error: false,
                    text: format!("Grizzly Editor {} is available", plan.version()),
                    whats_new: false,
                },
            );
        }
        return Ok(svc.status(editor));
    }
    if matches!(svc.state(), State::Manual) {
        return Err(fail(svc.text()));
    }
    if svc.state() != State::Ready {
        download(editor, svc, inst, ctx, plan).await?;
    }
    Ok(svc.status(editor))
}

/// The check: the latest release, its archive's directory and manifest, the delta. `None` when
/// there is nothing to update to (the state says why).
async fn check(
    editor: &Arc<Editor>,
    svc: &Arc<UpdateService>,
    inst: &Install,
    ctx: &JobCtx,
) -> Result<Option<Arc<Plan>>, JobError> {
    svc.with(editor, |i| {
        i.state = State::Checking;
        i.error = None;
    });
    ctx.stage("Looking for a newer version");
    let net = svc.net()?;
    let release = net.latest(&inst.repo, &ctx.cancel).await.map_err(from_net)?;
    let have = svc.installed();
    let up_to_date = |svc: &UpdateService, latest: Option<Release>| {
        svc.with(editor, |i| {
            i.state = State::Idle;
            i.latest = latest;
            i.plan = None;
            i.note = Some("Grizzly Editor is up to date.".to_owned());
        });
    };
    let Some(release) = release else {
        up_to_date(svc, None);
        return Ok(None);
    };
    let newer = release.version.is_some_and(|v| v > have);
    let Some(asset) = release.asset.clone().filter(|_| newer) else {
        if newer {
            editor.log.warn(format!(
                "[update] the release {} carries no Grizzly-Editor-{}.zip",
                release.tag, release.tag
            ));
        }
        up_to_date(svc, (!newer).then_some(release));
        return Ok(None);
    };
    svc.with(editor, |i| i.latest = Some(release.clone()));
    // a folder the editor cannot write gets no download
    let root = inst.root.clone();
    if let Err(e) = editor.exec.io(move || probe_writable(&root)).await {
        svc.with(editor, |i| {
            i.state = State::Manual;
            i.error = Some(format!(
                "Grizzly Editor {} is available, but the editor cannot write into its folder {} ({e}): install it from the release page.",
                release.tag,
                inst.root.display()
            ));
        });
        return Ok(None);
    }
    ctx.stage("Reading the archive's directory");
    let src = net::Source::new(&asset.url);
    let http = prepare::Http { net, src: &src, cancel: &ctx.cancel };
    let prepared = match prepare::prepare(&http, asset.size).await {
        Ok(p) => Some(p),
        Err(PrepError::NoRanges) => None,
        Err(PrepError::Schema(_)) => {
            svc.with(editor, |i| {
                i.state = State::Manual;
                i.error = Some(format!(
                    "Grizzly Editor {} needs a newer updater than this one: install it from the release page.",
                    release.tag
                ));
            });
            return Ok(None);
        }
        Err(PrepError::Bad(s)) => {
            return Err(fail(format!("The archive of Grizzly Editor {} cannot be used: {s}.", release.tag)));
        }
        Err(PrepError::Net(e)) => return Err(from_net(e)),
    };
    if let Some(p) = &prepared
        && p.manifest.version.to_string() != release.tag
    {
        return Err(fail(format!(
            "The archive of Grizzly Editor {} says it is {}: it cannot be used.",
            release.tag, p.manifest.version
        )));
    }
    ctx.stage("Comparing the installed files");
    let plan = make_plan(editor, svc, inst, release, asset, prepared).await?;
    let plan = Arc::new(plan);
    let ready = stage_complete(inst, &plan);
    let windowed = svc.start.window;
    svc.with(editor, |i| {
        i.plan = Some(plan.clone());
        i.state = if ready { State::Ready } else { State::Available };
        i.note = None;
        // a staged update is installed when the editor exits, unless the author says otherwise
        if ready && windowed {
            i.on_exit = true;
        }
    });
    svc.progress(editor, inst, &plan);
    Ok(Some(plan))
}

fn probe_writable(root: &Path) -> std::io::Result<()> {
    let work = root.join(WORK);
    let made = !work.exists();
    fs::create_dir_all(&work)?;
    let probe = work.join(format!("probe-{}.tmp", std::process::id()));
    let result = fs::write(&probe, b"probe").and_then(|()| {
        let moved = work.join(format!("probe-{}.moved", std::process::id()));
        sys::rename(&probe, &moved)?;
        fs::remove_file(&moved)
    });
    let _ = fs::remove_file(&probe);
    if made {
        let _ = fs::remove_dir(&work);
    }
    result
}

/// The rows the installation lacks, the staged files kept or reused, the cost.
async fn make_plan(
    editor: &Arc<Editor>,
    svc: &Arc<UpdateService>,
    inst: &Install,
    release: Release,
    asset: Asset,
    prepared: Option<Prepared>,
) -> Result<Plan, JobError> {
    let version = release.tag.clone();
    let Some(prepared) = prepared else {
        // no ranges: what differs is known once the whole archive is here
        return Ok(Plan {
            release,
            asset: asset.clone(),
            mode: Mode::Whole,
            prepared: None,
            needed: Vec::new(),
            bytes: asset.size,
        });
    };
    let (ed, root, rows) = (editor.clone(), inst.root.clone(), prepared.manifest.rows.clone());
    let svc2 = svc.clone();
    let needed = editor.exec.io(move || needed_rows(&ed, &svc2.cache, &root, rows)).await;
    // a stage of another release, or of this one republished: what still matches is reused
    let (root, v, sha) = (inst.root.clone(), version.clone(), prepared.manifest_sha.clone());
    let (needed2, asset2) = (needed.clone(), asset.clone());
    let reused = editor.exec.io(move || reuse_stages(&root, &v, &asset2, &sha, &needed2)).await;
    if let Err(e) = reused {
        editor.log.warn(format!("[update] the staged files of an earlier download could not be reused: {e}"));
    }
    let stage = Stage::new(&inst.root, &version);
    let passed = stage.read_state().map(|s| s.passed).unwrap_or_default();
    let spans: Vec<(u64, u64)> = needed
        .iter()
        .filter(|r| !passed.contains_key(&path_key(&r.path)))
        .filter_map(|r| prepared.record(&r.path).map(|(_, s)| s))
        .collect();
    let bytes = delta::total(&delta::merge(spans, delta::GAP));
    let mode = if !prepared.ranged_ok || delta::whole_is_better(bytes, asset.size) {
        Mode::Whole
    } else {
        Mode::Ranges
    };
    let bytes = if mode == Mode::Whole { asset.size } else { bytes };
    Ok(Plan { release, asset, mode, prepared: Some(prepared), needed, bytes })
}

/// The manifest rows whose file is missing or differs (size and SHA-256), hashed on the IO pool.
fn needed_rows(editor: &Editor, cache: &HashCache, root: &Path, rows: Vec<Row>) -> Vec<Row> {
    let checked = editor.exec.par_io(rows, |row| {
        let same = matches!(cache.of(root, &row.path), Ok(Some((size, sha))) if size == row.size && sha == row.sha256);
        (row, same)
    });
    cache.save();
    checked.into_iter().filter(|(_, same)| !*same).map(|(r, _)| r).collect()
}

/// Keeps what the stage of this release holds; takes from any other stage the files that are
/// still right by hash and drops the rest of it.
fn reuse_stages(
    root: &Path,
    version: &str,
    asset: &Asset,
    manifest_sha: &str,
    needed: &[Row],
) -> std::io::Result<()> {
    let target = Stage::new(root, version);
    let own = target.read_state();
    let fresh =
        |s: &StageState| s.version == version && s.asset == asset.id && s.manifest_sha == manifest_sha;
    let mut keep = own.filter(fresh).unwrap_or_else(|| StageState {
        version: version.to_owned(),
        asset: asset.id,
        size: asset.size,
        manifest_sha: manifest_sha.to_owned(),
        mode: Mode::Ranges,
        passed: BTreeMap::new(),
    });
    let mut changed = false;
    for other in stage::stages(root) {
        let same_dir = other.dir == target.dir;
        if apply::journal_state(&other).is_some() {
            continue;
        }
        if same_dir && target.read_state().is_some_and(|s| fresh(&s)) {
            continue;
        }
        let Some(old) = other.read_state() else {
            if !same_dir {
                stage::remove_tree(&other.dir)?;
            }
            continue;
        };
        let moved_dir = if same_dir {
            // this release republished: its own stage is the source, moved aside first
            let aside = root.join(WORK).join(format!("{version}.old"));
            stage::remove_tree(&aside)?;
            fs::rename(&other.dir, &aside)?;
            Stage { dir: aside }
        } else {
            other.clone()
        };
        for row in needed {
            let Some((sha, size, path)) = old.passed.get(&path_key(&row.path)) else { continue };
            if *sha != row.sha256 || *size != row.size {
                continue;
            }
            let from = moved_dir.file(path);
            if crate::hashes::sha256_file(&from).is_ok_and(|(s, h)| s == row.size && h == row.sha256) {
                let to = target.file(&row.path);
                if let Some(d) = to.parent() {
                    fs::create_dir_all(d)?;
                }
                if fs::rename(&from, &to).is_ok() {
                    keep.passed.insert(path_key(&row.path), (row.sha256.clone(), row.size, row.path.clone()));
                    changed = true;
                }
            }
        }
        stage::remove_tree(&moved_dir.dir)?;
    }
    if changed {
        target.write_state(&keep)?;
    }
    Ok(())
}

/// Every needed file is staged and the stage holds the manifest.
fn stage_complete(inst: &Install, plan: &Plan) -> bool {
    let stage = Stage::new(&inst.root, &plan.version());
    let Some(state) = stage.read_state() else { return false };
    let Some(p) = &plan.prepared else { return false };
    state.manifest_sha == p.manifest_sha
        && stage.manifest().is_file()
        && plan
            .needed
            .iter()
            .all(|r| state.passed.contains_key(&path_key(&r.path)) && stage.file(&r.path).is_file())
}

impl UpdateService {
    /// The bytes received of the delta, for the chip and the card.
    fn progress(&self, editor: &Editor, inst: &Install, plan: &Plan) {
        let stage = Stage::new(&inst.root, &plan.version());
        let passed = stage.read_state().map(|s| s.passed).unwrap_or_default();
        let (done, total, staged) = match (&plan.prepared, plan.mode) {
            (Some(p), Mode::Ranges) => {
                let mut done = 0;
                let mut total = 0;
                let mut staged = 0;
                for r in &plan.needed {
                    let Some((_, (a, b))) = p.record(&r.path) else { continue };
                    total += b - a;
                    if passed.contains_key(&path_key(&r.path)) {
                        done += b - a;
                        staged += 1;
                    } else {
                        done += record::kept(&stage.files(), &r.path).min(b - a);
                    }
                }
                (done, total, staged)
            }
            _ => {
                let have = fs::metadata(stage.archive_part()).map_or(0, |m| m.len());
                (have.min(plan.asset.size), plan.asset.size, passed.len())
            }
        };
        self.with(editor, |i| {
            i.done = done;
            i.total = total;
            i.staged = staged;
        });
    }
}

/// What an attempt of the download came to.
enum Attempt {
    Done,
    /// The server answered a range with the whole archive.
    Whole,
    /// Worth another try; `true` when bytes arrived first.
    Network(String, bool),
}

fn record_error(e: RecordError) -> JobError {
    match e {
        RecordError::Mismatch(s) => fail(format!("{s}. Nothing was installed.")),
        RecordError::Io(e) => fail(format!("The update could not be written: {e}.")),
    }
}

async fn download(
    editor: &Arc<Editor>,
    svc: &Arc<UpdateService>,
    inst: &Install,
    ctx: &JobCtx,
    plan: Arc<Plan>,
) -> Result<(), JobError> {
    svc.take_lock(inst)?;
    let version = plan.version();
    let stage = Stage::new(&inst.root, &version);
    let mut plan = plan;
    // the stage names its release before anything goes into it
    {
        let mut state = stage.read_state().unwrap_or(StageState {
            version: version.clone(),
            asset: plan.asset.id,
            size: plan.asset.size,
            manifest_sha: String::new(),
            mode: plan.mode,
            passed: BTreeMap::new(),
        });
        if let Some(p) = &plan.prepared {
            state.manifest_sha.clone_from(&p.manifest_sha);
            fs::create_dir_all(&stage.dir)
                .map_err(|e| fail(format!("The update could not be written: {e}.")))?;
            fs::write(stage.manifest(), &p.manifest_bytes)
                .map_err(|e| fail(format!("The update could not be written: {e}.")))?;
        }
        state.mode = plan.mode;
        stage.write_state(&state).map_err(|e| fail(format!("The update could not be written: {e}.")))?;
        let _ = fs::write(stage.notes(), plan.release.notes.as_bytes());
    }
    svc.with(editor, |i| {
        i.state = State::Downloading;
        i.error = None;
    });
    let net = svc.net()?;
    let src = net::Source::new(&plan.asset.url);
    let mut retry = Retry::new(inst.pace);
    loop {
        ctx.stage(format!("Downloading {version}"));
        let attempt = match plan.mode {
            Mode::Ranges => ranges(editor, svc, inst, ctx, net, &src, &plan, &stage).await?,
            Mode::Whole => whole(editor, svc, inst, ctx, net, &src, &mut plan, &stage).await?,
        };
        match attempt {
            Attempt::Done => break,
            Attempt::Whole => {
                let mut p = (*plan).clone();
                p.mode = Mode::Whole;
                p.bytes = p.asset.size;
                plan = Arc::new(p);
                svc.with(editor, |i| i.plan = Some(plan.clone()));
            }
            Attempt::Network(why, progressed) => {
                editor.log.info(format!("[update] the download of {version} was interrupted: {why}"));
                let pause = match retry.failed(progressed) {
                    Next::Retry(d) => d,
                    Next::Park(d) => {
                        svc.with(editor, |i| i.state = State::Waiting);
                        ctx.stage("Waiting for the connection");
                        d
                    }
                };
                tokio::select! {
                    () = ctx.cancel.cancelled() => return Err(stopped()),
                    () = tokio::time::sleep(pause) => {}
                }
                svc.with(editor, |i| i.state = State::Downloading);
            }
        }
    }
    svc.progress(editor, inst, &plan);
    let windowed = svc.start.window;
    svc.with(editor, |i| {
        i.state = State::Ready;
        i.plan = Some(plan.clone());
        // closing the dialog means Update on Exit
        if windowed {
            i.on_exit = true;
        }
    });
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn ranges(
    editor: &Arc<Editor>,
    svc: &Arc<UpdateService>,
    inst: &Install,
    ctx: &JobCtx,
    net: &Net,
    src: &net::Source,
    plan: &Arc<Plan>,
    stage: &Stage,
) -> Result<Attempt, JobError> {
    let Some(prepared) = &plan.prepared else { return Ok(Attempt::Whole) };
    let mut state = stage.read_state().ok_or_else(|| fail("The update's stage is gone."))?;
    let files = stage.files();
    // the records still to come, from the byte an earlier attempt stopped at
    let mut pending: Vec<(Row, prepare_entry::Rec)> = plan
        .needed
        .iter()
        .filter(|r| !state.passed.contains_key(&path_key(&r.path)))
        .filter_map(|r| {
            let (e, span) = prepared.record(&r.path)?;
            Some((r.clone(), prepare_entry::Rec { entry: e.clone(), span }))
        })
        .collect();
    pending.sort_by_key(|(_, rec)| rec.span.0);
    if pending.is_empty() {
        return Ok(Attempt::Done);
    }
    let wanted: Vec<(u64, u64)> = pending
        .iter()
        .map(|(r, rec)| (rec.span.0 + record::kept(&files, &r.path).min(rec.span.1 - rec.span.0), rec.span.1))
        .collect();
    let requests = delta::merge(wanted, delta::GAP);
    let mut progressed = false;
    let mut last_card = std::time::Instant::now();
    // what is in the stage already, read once; then counted as the bytes arrive
    svc.progress(editor, inst, plan);
    let (mut done, total) = {
        let i = svc.inner.lock();
        (i.done, i.total)
    };
    for (a, b) in requests {
        let mut resp = match net.range(src, a, b, &ctx.cancel).await {
            Ok(Ranged::Partial(r)) => r,
            Ok(Ranged::Whole(_)) => return Ok(Attempt::Whole),
            Err(NetError::Cancelled) => return Err(stopped()),
            Err(NetError::Network(s)) => return Ok(Attempt::Network(s, progressed)),
            Err(NetError::Refused(s)) => {
                return Err(fail(format!("The download of {} was refused: {s}.", plan.version())));
            }
        };
        let mut pos = a;
        let mut sink: Option<(usize, Sink)> = None;
        while pos < b {
            let chunk = match net::chunk(&mut resp, &ctx.cancel).await {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(NetError::Cancelled) => return Err(stopped()),
                Err(NetError::Network(s) | NetError::Refused(s)) => {
                    return Ok(Attempt::Network(s, progressed));
                }
            };
            progressed = true;
            let mut bytes = chunk.as_ref();
            bytes =
                bytes.get(..bytes.len().min(usize::try_from(b - pos).unwrap_or(bytes.len()))).unwrap_or(&[]);
            while !bytes.is_empty() {
                // the record this byte belongs to, or the gap before the next one
                let Some(idx) = pending.iter().position(|(_, rec)| pos < rec.span.1 && rec.span.0 <= pos)
                else {
                    let next =
                        pending.iter().map(|(_, rec)| rec.span.0).filter(|&s| s > pos).min().unwrap_or(b);
                    let skip = usize::try_from(next - pos).unwrap_or(bytes.len()).min(bytes.len());
                    bytes = bytes.get(skip..).unwrap_or(&[]);
                    pos += skip as u64;
                    continue;
                };
                let (row, rec) = &pending[idx];
                if sink.as_ref().is_none_or(|(i, _)| *i != idx) {
                    let s =
                        Sink::open(&files, row, &rec.entry, rec.span.1 - rec.span.0).map_err(record_error)?;
                    if rec.span.0 + s.received() != pos {
                        // what the stage kept does not meet these bytes: skip to where it ends
                        let resume = rec.span.0 + s.received();
                        if resume > pos {
                            let skip = usize::try_from(resume - pos).unwrap_or(bytes.len()).min(bytes.len());
                            bytes = bytes.get(skip..).unwrap_or(&[]);
                            pos += skip as u64;
                            sink = Some((idx, s));
                            continue;
                        }
                        return Ok(Attempt::Network(
                            "the download did not continue where it stopped".to_owned(),
                            progressed,
                        ));
                    }
                    sink = Some((idx, s));
                }
                let Some((_, s)) = sink.as_mut() else { break };
                let n = usize::try_from(s.left()).unwrap_or(bytes.len()).min(bytes.len());
                s.feed(bytes.get(..n).unwrap_or(&[])).map_err(record_error)?;
                bytes = bytes.get(n..).unwrap_or(&[]);
                pos += n as u64;
                done += n as u64;
                if s.left() == 0
                    && let Some((_, done)) = sink.take()
                {
                    done.finish().map_err(record_error)?;
                    state
                        .passed
                        .insert(path_key(&row.path), (row.sha256.clone(), row.size, row.path.clone()));
                    stage
                        .write_state(&state)
                        .map_err(|e| fail(format!("The update could not be written: {e}.")))?;
                    pending.remove(idx);
                }
            }
            svc.with(editor, |i| i.done = done);
            if total > 0 {
                ctx.fraction(done as f32 / total as f32);
            }
            if last_card.elapsed() > Duration::from_millis(250) {
                last_card = std::time::Instant::now();
                svc.card(editor, ctx.id, false);
            }
        }
        if pos < b {
            return Ok(Attempt::Network("the connection was lost".to_owned(), progressed));
        }
    }
    if pending.is_empty() {
        Ok(Attempt::Done)
    } else {
        Ok(Attempt::Network("a part of the download is missing".to_owned(), progressed))
    }
}

/// The whole archive into the stage, resumed by range where the server allows, then the files
/// that differ out of it.
#[allow(clippy::too_many_arguments)]
async fn whole(
    editor: &Arc<Editor>,
    svc: &Arc<UpdateService>,
    inst: &Install,
    ctx: &JobCtx,
    net: &Net,
    src: &net::Source,
    plan: &mut Arc<Plan>,
    stage: &Stage,
) -> Result<Attempt, JobError> {
    use std::io::Write;
    let part = stage.archive_part();
    let size = plan.asset.size;
    let write_err = |e: std::io::Error| fail(format!("The update could not be written: {e}."));
    let mut have = fs::metadata(&part).map_or(0, |m| m.len());
    if have > size {
        fs::remove_file(&part).map_err(write_err)?;
        have = 0;
    }
    let mut progressed = false;
    if have < size {
        let (mut resp, append) = match net.range(src, have, size, &ctx.cancel).await {
            Ok(Ranged::Partial(r)) => (r, true),
            Ok(Ranged::Whole(r)) => (r, false),
            Err(NetError::Cancelled) => return Err(stopped()),
            Err(NetError::Network(s)) => return Ok(Attempt::Network(s, false)),
            Err(NetError::Refused(s)) => {
                return Err(fail(format!("The download of {} was refused: {s}.", plan.version())));
            }
        };
        if !append {
            have = 0;
        }
        let mut f = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&part)
            .map_err(write_err)?;
        loop {
            let chunk = match net::chunk(&mut resp, &ctx.cancel).await {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(NetError::Cancelled) => return Err(stopped()),
                Err(NetError::Network(s) | NetError::Refused(s)) => {
                    return Ok(Attempt::Network(s, progressed));
                }
            };
            progressed = true;
            let bytes = chunk.as_ref();
            let bytes = bytes
                .get(..bytes.len().min(usize::try_from(size - have).unwrap_or(bytes.len())))
                .unwrap_or(&[]);
            f.write_all(bytes).map_err(write_err)?;
            have += bytes.len() as u64;
            ctx.fraction(have as f32 / size.max(1) as f32);
            svc.with(editor, |i| {
                i.done = have;
                i.total = size;
            });
            if have >= size {
                break;
            }
        }
        f.sync_all().map_err(write_err)?;
        if have < size {
            return Ok(Attempt::Network("the connection was lost".to_owned(), progressed));
        }
    }
    ctx.stage("Unpacking");
    let local = prepare::Local { path: part.clone() };
    let prepared = match prepare::prepare(&local, size).await {
        Ok(p) => p,
        Err(PrepError::Schema(_)) => {
            let tag = plan.version();
            svc.with(editor, |i| {
                i.state = State::Manual;
                i.error = Some(format!("Grizzly Editor {tag} needs a newer updater than this one: install it from the release page."));
            });
            let _ = fs::remove_file(&part);
            return Err(fail(svc.text()));
        }
        Err(PrepError::Bad(s)) => {
            let _ = fs::remove_file(&part);
            return Err(fail(format!(
                "The archive of Grizzly Editor {} cannot be used: {s}.",
                plan.version()
            )));
        }
        Err(PrepError::NoRanges | PrepError::Net(_)) => {
            return Err(fail("The downloaded archive could not be read."));
        }
    };
    if prepared.manifest.version.to_string() != plan.version() {
        return Err(fail(format!(
            "The archive of Grizzly Editor {} says it is {}: it cannot be used.",
            plan.version(),
            prepared.manifest.version
        )));
    }
    if plan.prepared.as_ref().is_some_and(|p| p.manifest_sha != prepared.manifest_sha) {
        let _ = fs::remove_file(&part);
        return Err(fail(format!(
            "The archive of Grizzly Editor {} changed while it was downloaded: check again.",
            plan.version()
        )));
    }
    // what differs is known now when the check could not read the directory
    if plan.prepared.is_none() {
        let (ed, root, rows) = (editor.clone(), inst.root.clone(), prepared.manifest.rows.clone());
        let svc2 = svc.clone();
        let needed = editor.exec.io(move || needed_rows(&ed, &svc2.cache, &root, rows)).await;
        let mut p = (**plan).clone();
        p.needed = needed;
        p.prepared = Some(prepared.clone());
        *plan = Arc::new(p);
        svc.with(editor, |i| i.plan = Some(plan.clone()));
        fs::write(stage.manifest(), &prepared.manifest_bytes).map_err(write_err)?;
        if let Some(mut s) = stage.read_state() {
            s.manifest_sha.clone_from(&prepared.manifest_sha);
            stage.write_state(&s).map_err(write_err)?;
        }
    }
    let mut state = stage.read_state().ok_or_else(|| fail("The update's stage is gone."))?;
    let files = stage.files();
    for row in &plan.needed {
        ctx.check().map_err(JobError::from)?;
        if state.passed.contains_key(&path_key(&row.path)) {
            continue;
        }
        let (entry, (a, b)) = prepared.record(&row.path).ok_or_else(|| {
            fail(format!("The archive of Grizzly Editor {} does not hold {}.", plan.version(), row.path))
        })?;
        if !entry.readable() {
            return Err(fail(format!(
                "{} in the archive of Grizzly Editor {} cannot be unpacked.",
                row.path,
                plan.version()
            )));
        }
        let _ = fs::remove_file(record::Paths::of(&files, &row.path).raw);
        let mut s = Sink::open(&files, row, entry, b - a).map_err(record_error)?;
        let mut f = File::open(&part).map_err(write_err)?;
        std::io::Seek::seek(&mut f, std::io::SeekFrom::Start(a)).map_err(write_err)?;
        let mut left = b - a;
        let mut buf = vec![0u8; 1 << 20];
        while left > 0 {
            let n = usize::try_from(left).unwrap_or(buf.len()).min(buf.len());
            std::io::Read::read_exact(&mut f, buf.get_mut(..n).unwrap_or(&mut [])).map_err(write_err)?;
            s.feed(buf.get(..n).unwrap_or(&[])).map_err(record_error)?;
            left -= n as u64;
        }
        s.finish().map_err(record_error)?;
        state.passed.insert(path_key(&row.path), (row.sha256.clone(), row.size, row.path.clone()));
        stage.write_state(&state).map_err(write_err)?;
    }
    let _ = fs::remove_file(&part);
    Ok(Attempt::Done)
}

mod prepare_entry {
    use crate::zipdir::Entry;

    /// A record still to come.
    pub struct Rec {
        pub entry: Entry,
        pub span: (u64, u64),
    }
}

/// Puts the updater on the editor and does what a start does.
pub fn install(editor: &Arc<Editor>, start: Start) -> Arc<UpdateService> {
    install_found(editor, start, resolve(editor))
}

/// [`install`] on an installation found another way: the tests'.
pub(crate) fn install_found(
    editor: &Arc<Editor>,
    start: Start,
    found: Result<Install, String>,
) -> Arc<UpdateService> {
    let svc = editor.services.insert(UpdateService::new(editor, start, found));
    match &svc.install {
        Some(inst) => editor.log.info(format!(
            "[update] the installation {} is Grizzly Editor {}; updates from {}",
            inst.root.display(),
            inst.manifest.version,
            inst.repo
        )),
        None => editor.log.info(format!("[update] {}", svc.off_reason)),
    }
    svc.startup(editor);
    svc
}
