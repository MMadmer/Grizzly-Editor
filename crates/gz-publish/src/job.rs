//! The publish job: package the module (through `gz-release`, into `<project>\releases\<version>`),
//! scan a listed module's packages first, then put everything into a GitHub draft, check every
//! stored asset against the package and publish it as the latest release.
//!
//! Two jobs of one card (`key` `publish`): the packaging runs in the group `build` - one build-like
//! job at a time - and ends by starting the scan and the upload in the group `publish`, so a build
//! may run while the files go up. `mod_publish` answers when the packaging has ended and the
//! upload is on its way; `status` follows the rest.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use gz_editor::{CmdError, Editor, JobCtx, JobError, JobId, ProjectSession};
use gz_github::{Asset, Github, Package, PublishRequest, PublishSink, Scan, Secret, scan};
use gz_mod::ManifestService;
use gz_release::{PackageRequest, PackageResult, Releases, run_package};
use gz_task::Cancelled;
use parking_lot::Mutex;
use serde_json::{Map, Value, json};

use crate::manage::ReleaseList;
use crate::scan::{ScanPlan, scan_first};
use crate::sign::SignView;
use crate::tags::{default_title, deleted_tags_file, fresh_tag, now_seconds, valid_tag};

/// GitHub stops a release body at 125 000 characters.
pub const MAX_NOTES: usize = 120_000;

const TITLE: &str = "Publishing Release";
const STOP: &str = "Stop Publishing";
const STEPS: [&str; 4] = ["Check", "Copy", "Pack", "Upload"];

/// Where the job is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PublishState {
    #[default]
    Idle,
    Packaging,
    Scanning,
    Uploading,
    Done,
    Failed,
    Cancelled,
}

impl PublishState {
    #[must_use]
    pub fn running(self) -> bool {
        matches!(self, Self::Packaging | Self::Scanning | Self::Uploading)
    }

    #[must_use]
    pub fn text(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Packaging => "packaging",
            Self::Scanning => "scanning",
            Self::Uploading => "uploading",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// What the job shows of itself.
#[derive(Clone, Debug, Default)]
pub struct JobView {
    pub state: PublishState,
    pub id: String,
    pub version: String,
    pub repo: String,
    pub tag: String,
    pub stage: String,
    /// Bytes of the whole release.
    pub sent: u64,
    pub total: u64,
    pub url: String,
    pub error: String,
    pub warnings: String,
    pub served: bool,
    /// `none`: the Mod Browser does not list the module.
    pub virustotal: Scan,
    /// The scan kept the release back.
    pub refused: bool,
    /// Something was sent to GitHub.
    pub github: bool,
}

/// What a start is asked.
#[derive(Clone, Debug)]
pub struct PublishOptions {
    /// Empty = the fresh tag of the version.
    pub tag: String,
    /// Empty = `<name> <version>`.
    pub title: String,
    pub notes: String,
    /// `None` = the linked game's version, `Some("")` = no requirement.
    pub requires_game: Option<String>,
    pub prerelease: bool,
    pub latest: bool,
}

impl Default for PublishOptions {
    fn default() -> Self {
        Self {
            tag: String::new(),
            title: String::new(),
            notes: String::new(),
            requires_game: None,
            prerelease: false,
            latest: true,
        }
    }
}

/// What the upload needs once the packaging has ended.
struct Pending {
    repo: String,
    module_id: String,
    version: String,
    tag: String,
    title: String,
    options: PublishOptions,
    token: Secret,
}

/// Publish Release of the editor. Lives in `editor.services`.
#[derive(Default)]
pub struct Publisher {
    job: Mutex<JobView>,
    /// What the packaging of the last start came to.
    package: Mutex<Option<Arc<PackageResult>>>,
    packaging_job: Mutex<Option<JobId>>,
    upload_job: Mutex<Option<JobId>>,
    from_ui: AtomicBool,
    /// A start is between its first check and its job: a second start is refused, never let
    /// through to overwrite the first one's view.
    starting: AtomicBool,
    pub(crate) sign: Mutex<SignView>,
    pub(crate) sign_cancel: Mutex<Option<gz_editor::Cancel>>,
}

/// The one start under way; let go when the start returns, whatever it answered.
pub(crate) struct StartClaim<'a>(&'a AtomicBool);

impl Drop for StartClaim<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// A job's card in the Task Center (`gz_app::TaskCardTable`, keyed by a std type).
type CardTable = Mutex<BTreeMap<JobId, Value>>;

fn publish_card(editor: &Editor, job: JobId, card: Value) {
    editor.services.get_or_init::<CardTable>(Default::default).lock().insert(job, card);
    editor.exec.wake().ping();
}

fn cancelled_text(err: &str) -> bool {
    err == gz_github::CANCELLED
}

impl Publisher {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    #[must_use]
    pub fn view(&self) -> JobView {
        self.job.lock().clone()
    }

    #[must_use]
    pub fn running(&self) -> bool {
        self.job.lock().state.running()
    }

    /// What the toolbar's tip says of the job; the button's label stays "Publish Release".
    #[must_use]
    pub fn button_text(&self) -> String {
        let job = self.view();
        match job.state {
            PublishState::Packaging => "Packaging Release".to_owned(),
            PublishState::Scanning => "Scanning Release".to_owned(),
            PublishState::Uploading => {
                format!("Publishing {}%", (job.sent * 100).checked_div(job.total).unwrap_or(0))
            }
            _ => "Publish Release".to_owned(),
        }
    }

    /// The last packaging's answer.
    #[must_use]
    pub fn package_result(&self) -> Option<Arc<PackageResult>> {
        self.package.lock().clone()
    }

    /// Takes the right to start: `None` while another start is under way or a job runs.
    pub(crate) fn claim(&self) -> Option<StartClaim<'_>> {
        if self.starting.swap(true, Ordering::AcqRel) {
            return None;
        }
        let claim = StartClaim(&self.starting);
        if self.running() {
            return None;
        }
        Some(claim)
    }

    /// Asks the running job to stop.
    pub fn cancel(&self, editor: &Editor) {
        if !self.running() {
            return;
        }
        for id in [*self.packaging_job.lock(), *self.upload_job.lock()].into_iter().flatten() {
            editor.jobs.cancel(id);
        }
    }

    fn subtitle(&self) -> String {
        let job = self.job.lock();
        format!("{} {} to {}", job.id, job.version, job.repo)
    }

    fn card(&self, editor: &Editor, job: JobId, step: usize, numbers: String, stop_tip: &str) {
        self.card_at(editor, job, step, numbers, "", stop_tip);
    }

    fn card_at(&self, editor: &Editor, job: JobId, step: usize, numbers: String, file: &str, stop_tip: &str) {
        publish_card(
            editor,
            job,
            json!({
                "key": "publish",
                "subtitle": self.subtitle(),
                "steps": STEPS,
                "step": step,
                "numbers": numbers,
                "file": file,
                "from_mcp": !self.from_ui.load(Ordering::Relaxed),
                "stop_label": STOP,
                "stop_tip": stop_tip,
            }),
        );
    }

    /// Starts Publish Release of the open project: the checks, then the packaging job. The one
    /// path the dialog and MCP share. Answers the packaging job.
    pub async fn start(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        options: PublishOptions,
        from_ui: bool,
    ) -> Result<JobId, String> {
        let session = editor.project().map_err(|e| e.message)?;
        // one start at a time, taken before the first await: a second start racing this one is
        // refused instead of resetting the view of the job this one starts
        let Some(_claim) = self.claim() else {
            return Err("a release is being published already".to_owned());
        };
        if ReleaseList::of(editor).running() {
            return Err("the release list is busy - try again in a moment".to_owned());
        }
        let manifest = ManifestService::of_session(editor, &session).await.map_err(|e| e.message)?;
        manifest.sync().await.map_err(|e| e.message)?;
        let m = manifest
            .snapshot()
            .manifest()
            .ok_or_else(|| "mod.ltx not found in the project".to_owned())?
            .clone();
        if m.update_github.is_empty() {
            return Err("no GitHub repository in the manifest: set it in Mod Settings (MCP: mod_set_manifest github=owner/repo)".to_owned());
        }
        if !gz_mod::validate_github_repo(&m.update_github) {
            return Err("invalid [update] github in mod.ltx: owner/repo expected".to_owned());
        }

        let tag = if options.tag.is_empty() {
            let file = deleted_tags_file(&session.root);
            let deleted = editor.exec.io(move || std::fs::read_to_string(file).unwrap_or_default()).await;
            fresh_tag(&m.version, &deleted, now_seconds())
        } else {
            options.tag.clone()
        };
        let title = if options.title.is_empty() {
            default_title(&m.id, &m.name, &m.version)
        } else {
            options.title.clone()
        };
        if !valid_tag(&tag) {
            return Err(
                "invalid tag: 1-100 characters of A-Z a-z 0-9 . _ -, not starting with '-' or '.'".to_owned()
            );
        }
        if title.len() > 200 {
            return Err("the title is longer than 200 characters".to_owned());
        }
        if options.notes.len() > MAX_NOTES {
            return Err("the notes are longer than 120 000 characters".to_owned());
        }

        // A release that goes out unsigned while the author has a key would burn its version for
        // the Mod Browser: a published tag is never overwritten. The key is opened first.
        let key = gz_release::key_state(editor).await;
        if key.exists && key.error.is_empty() && key.locked {
            return Err("the author key is locked: give its password first (Mod > Publish Release, or MCP password=) - an unsigned release is one the Mod Browser does not take".to_owned());
        }

        // before the packaging: no point in zipping a module nobody can upload
        let account = Github::of(editor).resolve_account().await;
        if !account.signed_in() {
            return Err("not signed in to GitHub: sign in from Mod > Publish Release, sign in to the GitHub CLI (gh auth login) or set GH_TOKEN".to_owned());
        }

        *self.job.lock() = JobView {
            state: PublishState::Packaging,
            id: m.id.clone(),
            version: m.version.clone(),
            repo: m.update_github.clone(),
            tag: tag.clone(),
            stage: "Packaging".to_owned(),
            ..JobView::default()
        };
        *self.package.lock() = None;
        *self.upload_job.lock() = None;
        self.from_ui.store(from_ui, Ordering::Relaxed);

        let pending = Pending {
            repo: m.update_github.clone(),
            module_id: m.id.clone(),
            version: m.version.clone(),
            tag,
            title,
            options: options.clone(),
            token: account.token,
        };
        let request = PackageRequest { out_dir: String::new(), requires_game: options.requires_game };
        let (me, ed) = (self.clone(), editor.clone());
        let started = editor.jobs.start("publish", Some("build"), TITLE, move |ctx| async move {
            me.card(&ed, ctx.id, 0, String::new(), "Stops the packaging. Nothing has reached GitHub yet.");
            let r = Arc::new(me.package_with_card(&ed, &session, &request, &ctx).await);
            me.packaging_done(&ed, r.clone(), pending);
            if r.ok {
                Ok(json!({ "descriptor": r.descriptor.replace('\\', "/"), "files": r.files }))
            } else if r.cancelled {
                Err(JobError::from(Cancelled))
            } else {
                Err(JobError::new(r.err.clone()))
            }
        });
        match started {
            Ok(id) => {
                *self.packaging_job.lock() = Some(id);
                Ok(id)
            }
            Err(_) => {
                *self.job.lock() = JobView::default();
                Err("a build is running: wait for it to finish, or stop it".to_owned())
            }
        }
    }

    /// The packaging, with the card following the packaging's own steps.
    async fn package_with_card(
        &self,
        editor: &Arc<Editor>,
        session: &Arc<ProjectSession>,
        request: &PackageRequest,
        ctx: &JobCtx,
    ) -> PackageResult {
        let work = run_package(editor, session, request, ctx);
        let mut work = std::pin::pin!(work);
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(300));
        let mut shown: Option<(usize, String, String)> = None;
        loop {
            tokio::select! {
                r = &mut work => return r,
                _ = tick.tick() => {
                    // the packaging's own row: its step, how far, the file at hand
                    let row = Releases::of(editor).status(editor).map_or((0, String::new(), String::new()), |s| (s.step, s.numbers(), s.file));
                    if shown.as_ref() != Some(&row) {
                        self.card_at(editor, ctx.id, row.0, row.1.clone(), &row.2, "Stops the packaging. Nothing has reached GitHub yet.");
                        shown = Some(row);
                    }
                }
            }
        }
    }

    /// The packaging has ended: the upload starts here.
    fn packaging_done(self: &Arc<Self>, editor: &Arc<Editor>, r: Arc<PackageResult>, pending: Pending) {
        *self.package.lock() = Some(r.clone());
        if !r.ok {
            let mut job = self.job.lock();
            job.state = if r.cancelled { PublishState::Cancelled } else { PublishState::Failed };
            job.error = r.err.clone();
            job.refused = !r.cancelled;
            drop(job);
            self.report(editor);
            return;
        }

        let out_dir = std::path::PathBuf::from(&r.out_dir);
        let packages: Vec<Package> = r
            .packages
            .iter()
            .map(|p| Package {
                name: p.name.clone(),
                path: out_dir.join(&p.name),
                size: p.size,
                sha256: p.sha256.clone(),
            })
            .collect();
        // the descriptor goes last, as it does on disk: it vouches for the rest
        let assets: Vec<Asset> = r
            .packages
            .iter()
            .chain([&r.index, &r.descriptor_file])
            .map(|f| Asset {
                path: out_dir.join(&f.name),
                name: f.name.clone(),
                size: f.size,
                sha256: f.sha256.clone(),
            })
            .collect();
        let total = assets.iter().map(|a| a.size).sum();
        let request = PublishRequest {
            repo: pending.repo.clone(),
            module_id: pending.module_id.clone(),
            version: pending.version.clone(),
            tag: pending.tag.clone(),
            title: pending.title.clone(),
            notes: pending.options.notes.clone(),
            prerelease: pending.options.prerelease,
            latest: pending.options.latest && !pending.options.prerelease,
            token: pending.token,
            assets,
            descriptor: std::path::PathBuf::from(&r.descriptor),
        };
        {
            let mut job = self.job.lock();
            job.state = PublishState::Uploading;
            job.stage = "Starting".to_owned();
            job.total = total;
        }
        editor.log.info(format!(
            "[XMS] publishing {} {} to {} as {}",
            pending.module_id, pending.version, pending.repo, pending.tag
        ));

        let (me, ed) = (self.clone(), editor.clone());
        let module_id = pending.module_id;
        let started = editor.jobs.start("publish", Some("publish"), TITLE, move |ctx| async move {
            let gh = Github::of(&ed);
            let plan = ScanPlan { module_id, packages, vt_key: gh.read_vt_key().await };
            me.upload(&ed, &gh, request, plan, &ctx).await
        });
        match started {
            Ok(id) => *self.upload_job.lock() = Some(id),
            Err(_) => {
                let mut job = self.job.lock();
                job.state = PublishState::Failed;
                job.error = "a release is being published already".to_owned();
            }
        }
    }

    /// The scan of a listed module, then GitHub.
    async fn upload(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        gh: &Github,
        request: PublishRequest,
        plan: ScanPlan,
        ctx: &JobCtx,
    ) -> Result<Value, JobError> {
        let started = Instant::now();
        let sink = Sink {
            me: self.clone(),
            editor: editor.clone(),
            ctx: ctx.clone(),
            names: request.assets.iter().map(|a| a.name.clone()).collect(),
            sent: request.assets.iter().map(|_| AtomicU64::new(0)).collect(),
            started,
        };
        self.card(
            editor,
            ctx.id,
            3,
            String::new(),
            "Stops the upload. Nothing is published; the draft stays on GitHub for the next attempt.",
        );
        let stage = |text: &str| sink.stage(text);
        let set_scan = |state: PublishState, scan: &Scan| {
            let mut job = self.job.lock();
            job.state = state;
            job.virustotal = scan.clone();
            drop(job);
            if state == PublishState::Scanning {
                self.card(
                    editor,
                    ctx.id,
                    3,
                    String::new(),
                    "Stops the VirusTotal scan. Nothing is published and nothing reaches GitHub.",
                );
            }
        };
        let scanned = scan_first(gh, &plan, &ctx.cancel, &stage, &set_scan).await;
        drop(plan);
        let (scan_warnings, outcome) = match scanned {
            Ok(w) => {
                self.job.lock().github = true;
                (w, gh.publish(&request, &ctx.cancel, &sink).await)
            }
            Err(e) => (String::new(), Err(e)),
        };
        let refused = matches!(&outcome, Err(e) if !cancelled_text(e)) && !self.job.lock().github;
        drop(request);

        let mut job = self.job.lock();
        let (ok, err) = match &outcome {
            Ok(_) => (true, String::new()),
            Err(e) => (false, e.clone()),
        };
        job.state = if ok {
            PublishState::Done
        } else if cancelled_text(&err) {
            PublishState::Cancelled
        } else {
            PublishState::Failed
        };
        let result = outcome.as_ref().ok().cloned().unwrap_or_default();
        job.url = result.url.clone();
        job.warnings = match (scan_warnings.is_empty(), result.warnings.is_empty()) {
            (true, _) => result.warnings.clone(),
            (false, true) => scan_warnings,
            (false, false) => format!("{scan_warnings}\n{}", result.warnings),
        };
        job.served = result.served;
        job.error = err.clone();
        job.refused = refused;
        if ok {
            job.sent = job.total;
        }
        let state = job.state;
        drop(job);
        self.report(editor);
        match state {
            PublishState::Done => Ok(json!({ "url": result.url, "served": result.served })),
            PublishState::Cancelled => Err(JobError::from(Cancelled)),
            _ => Err(JobError::new(err)),
        }
    }

    /// The end of a job, in the log.
    fn report(&self, editor: &Editor) {
        let job = self.view();
        match job.state {
            PublishState::Done => {
                editor.log.info(format!("[XMS] release {} {} published: {}", job.id, job.version, job.url))
            }
            PublishState::Cancelled => {
                editor.log.info(format!("[XMS] publishing {} {} cancelled", job.id, job.version))
            }
            _ if job.refused => editor
                .log
                .warn(format!("[XMS] release {} {} not published: {}", job.id, job.version, job.error)),
            _ => {
                editor.log.warn(format!("[XMS] publishing {} {} failed: {}", job.id, job.version, job.error))
            }
        }
        if !job.warnings.is_empty() {
            editor.log.warn(format!("[XMS] {}", job.warnings));
        }
    }

    /// `mod_publish status`: `ok` is false for a job that failed or was stopped.
    #[must_use]
    pub fn status_json(&self) -> (bool, Value) {
        let job = self.view();
        let failed = matches!(job.state, PublishState::Failed | PublishState::Cancelled);
        let error = if job.state == PublishState::Cancelled {
            gz_github::CANCELLED.to_owned()
        } else {
            job.error.clone()
        };
        (
            !failed,
            json!({
                "state": job.state.text(),
                "pending": job.state.running(),
                "id": job.id,
                "version": job.version,
                "repo": job.repo,
                "tag": job.tag,
                "stage": job.stage,
                "sent": job.sent,
                "total": job.total,
                "url": job.url,
                "served": job.served,
                "warnings": job.warnings,
                "error": error,
                "virustotal": scan::json(&job.virustotal),
            }),
        )
    }

    /// The status with the packaging half of the answer, as `mod_package` gives it: what
    /// `mod_publish` answers once the packaging has ended.
    #[must_use]
    pub fn started_json(&self) -> (bool, Value) {
        let (ok, mut status) = self.status_json();
        let state = self.view().state;
        if matches!(state, PublishState::Failed | PublishState::Cancelled) {
            return (ok, status);
        }
        let Some(r) = self.package_result() else { return (ok, status) };
        let assets: Vec<Value> = r
            .packages
            .iter()
            .chain([&r.index, &r.descriptor_file])
            .map(|f| json!({ "name": f.name, "size": f.size, "sha256": f.sha256 }))
            .collect();
        if let Value::Object(map) = &mut status {
            let extra: Map<String, Value> = [
                ("assets".to_owned(), Value::Array(assets)),
                ("files".to_owned(), json!(r.files)),
                ("requires_game".to_owned(), json!(r.requires_game)),
                ("baked".to_owned(), json!(r.baked)),
                ("package_warnings".to_owned(), json!(r.warnings)),
            ]
            .into_iter()
            .collect();
            map.extend(extra);
        }
        (ok, status)
    }
}

/// What the publication tells the job and its card.
struct Sink {
    me: Arc<Publisher>,
    editor: Arc<Editor>,
    ctx: JobCtx,
    names: Vec<String>,
    /// Bytes of each asset sent: uploads run side by side.
    sent: Vec<AtomicU64>,
    started: Instant,
}

impl PublishSink for Sink {
    fn stage(&self, text: &str) {
        self.me.job.lock().stage = text.to_owned();
        self.ctx.stage(text);
    }

    fn upload(&self, asset: &str, sent: u64, _total: u64) {
        if let Some(slot) = self.names.iter().position(|n| n == asset).and_then(|i| self.sent.get(i)) {
            slot.store(sent, Ordering::Relaxed);
        }
        let done: u64 = self.sent.iter().map(|s| s.load(Ordering::Relaxed)).sum();
        let total = {
            let mut job = self.me.job.lock();
            job.sent = done.min(job.total);
            job.total
        };
        if total > 0 {
            self.ctx.fraction((done as f64 / total as f64) as f32);
            let mb = |b: u64| b as f64 / 1_048_576.0;
            let mut numbers = format!("{:.1} of {:.1} MB", mb(done), mb(total));
            let spent = self.started.elapsed().as_millis() as u64;
            if spent > 2000 && done > 0 {
                let left = (spent as f64 * (total.saturating_sub(done)) as f64 / done as f64) as u64;
                numbers.push_str(&format!(" · {} s left", left / 1000));
            }
            self.me.card(
                &self.editor,
                self.ctx.id,
                3,
                numbers,
                "Stops the upload. Nothing is published; the draft stays on GitHub for the next attempt.",
            );
        }
    }
}

/// `{"ok":false,...}` with the fields of an answer that failed.
pub(crate) fn reply(ok: bool, fields: Value) -> Result<Value, CmdError> {
    if ok {
        return Ok(fields);
    }
    let Value::Object(mut map) = fields else { return Err(CmdError::new("failed", "")) };
    let message = map.remove("error").and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
    Err(CmdError::new("failed", message).with_data(Value::Object(map)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_start_at_a_time() {
        let p = Publisher::default();
        let first = p.claim();
        assert!(first.is_some());
        // a second start while the first is between its checks and its job is refused
        assert!(p.claim().is_none());
        drop(first);
        assert!(p.claim().is_some());
        // a running job refuses a start too, and its view stays as it is
        p.job.lock().state = PublishState::Uploading;
        p.job.lock().tag = "v1.0.0".to_owned();
        assert!(p.claim().is_none());
        assert_eq!(p.view().tag, "v1.0.0");
        // the refusal gave the claim back
        p.job.lock().state = PublishState::Done;
        assert!(p.claim().is_some());
    }

    #[test]
    fn racing_starts_take_one_claim() {
        let p = Arc::new(Publisher::default());
        let won = AtomicU64::new(0);
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    barrier.wait();
                    if let Some(c) = p.claim() {
                        won.fetch_add(1, Ordering::AcqRel);
                        // held until every racer has tried
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        drop(c);
                    }
                });
            }
        });
        assert_eq!(won.load(Ordering::Acquire), 1);
    }
}
