//! [`Catalog`]: Submit to Mod Browser while the editor runs - one job at a time, whether the
//! window or MCP started it, and what the jobs learned about the module.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gz_editor::{Editor, JobError, JobId};
use gz_github::{AccountSource, CatalogIssue, Github, Scan, Secret, scan};
use gz_mod::ManifestService;
use gz_task::Cancelled;
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::jobs::Run;
use crate::model::{Check, Checked, Input, Listing, Standing};

/// The jobs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Job {
    #[default]
    None,
    Check,
    Vt,
    Submit,
    Submission,
    Withdraw,
}

impl Job {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Job::None => "",
            Job::Check => "check",
            Job::Vt => "vt",
            Job::Submit => "submit",
            Job::Submission => "submission",
            Job::Withdraw => "withdraw",
        }
    }

    /// The Task Center's title and Stop label.
    fn card(self) -> (&'static str, &'static str) {
        match self {
            Job::Vt => ("Scanning on VirusTotal", "Stop Scanning"),
            Job::Submit => ("Submitting to the Mod Browser", "Stop Submitting"),
            Job::Submission => ("Reading the Submission", "Stop Refreshing"),
            Job::Withdraw => ("Withdrawing from the Mod Browser", "Stop Withdrawing"),
            Job::Check | Job::None => ("Checking the Mod Browser Requirements", "Stop Checking"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JobState {
    #[default]
    Idle,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            JobState::Idle => "idle",
            JobState::Running => "running",
            JobState::Done => "done",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }
}

/// What the window and `status` show.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub job: Job,
    pub state: JobState,
    pub stage: String,
    pub error: String,
    /// What the results below are about.
    pub root: PathBuf,
    pub module_id: String,
    pub login: String,
    pub check: Check,
    /// What VirusTotal was asked last, by any job.
    pub virustotal: Scan,
    /// By a scan, not by the check's lookups.
    pub vt_scanned: bool,
    pub submission: CatalogIssue,
    /// Looked up: "no submission" is known, not assumed.
    pub submission_read: bool,
    /// Why the last look-up failed.
    pub submission_error: String,
    pub withdrawal: CatalogIssue,
    pub listing: Listing,
    /// Of a submission: check, scan, send.
    pub step: i32,
}

impl View {
    #[must_use]
    pub fn listed(&self) -> bool {
        self.listing.state == "listed" && self.listing.this_key
    }
}

/// Submit to Mod Browser of the editor. Lives in `editor.services`.
#[derive(Default)]
pub struct Catalog {
    view: Mutex<View>,
    job_id: Mutex<Option<JobId>>,
    from_ui: AtomicBool,
    /// The window is open.
    window: AtomicBool,
    /// `show confirm=withdraw`: the withdrawal's question over the window, to be looked at.
    want_withdraw: AtomicBool,
    /// Where the account comes from, read at most every few seconds: the GitHub CLI answers by
    /// starting a process.
    account: Mutex<Option<(AccountSource, Instant)>>,
}

/// A job's card in the Task Center (`gz_app::TaskCardTable`, keyed by a std type).
type CardTable = Mutex<BTreeMap<JobId, Value>>;

impl Catalog {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    #[must_use]
    pub fn view(&self) -> View {
        self.view.lock().clone()
    }

    #[must_use]
    pub fn running(&self) -> bool {
        self.view.lock().state == JobState::Running
    }

    #[must_use]
    pub fn window_open(&self) -> bool {
        self.window.load(Ordering::Relaxed)
    }

    pub fn set_window(&self, open: bool) {
        self.window.store(open, Ordering::Relaxed);
        if !open {
            self.want_withdraw.store(false, Ordering::Relaxed);
        }
    }

    /// The withdrawal question an MCP caller asked to see.
    #[must_use]
    pub fn wants_withdraw_question(&self) -> bool {
        self.want_withdraw.load(Ordering::Relaxed)
    }

    pub fn set_withdraw_question(&self, shown: bool) {
        self.want_withdraw.store(shown, Ordering::Relaxed);
    }

    /// Stops the running job at its next step.
    pub fn cancel(&self, editor: &Editor) {
        if self.running()
            && let Some(id) = *self.job_id.lock()
        {
            editor.jobs.cancel(id);
        }
    }

    pub(crate) fn set_stage(&self, text: &str) {
        self.view.lock().stage = text.to_owned();
    }

    pub(crate) fn set_login(&self, login: &str) {
        self.view.lock().login = login.to_owned();
    }

    pub(crate) fn set_step(&self, step: i32) {
        self.view.lock().step = step;
    }

    /// Opens the window and checks the requirements at once (`Request`).
    pub async fn request(self: &Arc<Self>, editor: &Arc<Editor>) {
        self.drop_stale(editor).await;
        self.account_source(editor, true).await;
        self.set_window(true);
        if !self.running() {
            let _ = self.start(editor, Job::Check, true).await;
        }
    }

    /// The module the results are about, now.
    async fn current(editor: &Arc<Editor>) -> (PathBuf, String) {
        let Ok(session) = editor.project() else { return (PathBuf::new(), String::new()) };
        let id = match ManifestService::of_session(editor, &session).await {
            Ok(m) => m.snapshot().manifest().map(|m| m.id.clone()).unwrap_or_default(),
            Err(_) => String::new(),
        };
        (session.root.clone(), id)
    }

    /// Results belong to one project and one module: another one starts from nothing.
    pub(crate) async fn drop_stale(&self, editor: &Arc<Editor>) {
        if self.running() {
            return;
        }
        let (root, id) = Self::current(editor).await;
        let mut v = self.view.lock();
        if v.root == root && v.module_id == id {
            return;
        }
        *v = View { root, module_id: id, ..View::default() };
    }

    /// Where the GitHub account comes from; read again when asked or after ten seconds.
    pub async fn account_source(&self, editor: &Editor, refresh: bool) -> AccountSource {
        let cached = *self.account.lock();
        if let Some((source, at)) = cached
            && !refresh
            && at.elapsed() <= Duration::from_secs(10)
        {
            return source;
        }
        let source = Github::of(editor).resolve_account().await.source;
        *self.account.lock() = Some((source, Instant::now()));
        source
    }

    /// Starts a job: the checks, the input taken now, then the work. The one path the window and
    /// MCP share.
    pub async fn start(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        job: Job,
        from_ui: bool,
    ) -> Result<JobId, String> {
        if self.running() {
            return Err("a Mod Browser job is running already".to_owned());
        }
        let session = editor.project().map_err(|_| "no active project".to_owned())?;
        self.drop_stale(editor).await;
        let manifest = ManifestService::of_session(editor, &session).await.map_err(|e| e.message)?;
        manifest.sync().await.map_err(|e| e.message)?;
        let m = manifest
            .snapshot()
            .manifest()
            .ok_or_else(|| "mod.ltx not found in the project".to_owned())?
            .clone();
        if !gz_mod::validate_id(&m.id) {
            return Err("the module id is missing or invalid (Mod > Mod Settings)".to_owned());
        }
        let key = gz_release::key_state(editor).await;
        let gh = Github::of(editor);
        let account = gh.resolve_account().await;
        *self.account.lock() = Some((account.source, Instant::now()));
        let signed_in = account.signed_in();
        if matches!(job, Job::Submit | Job::Submission | Job::Withdraw) && !signed_in {
            return Err("not signed in to GitHub: sign in from Mod > Publish Release (MCP: mod_publish action=sign_in or save_token)".to_owned());
        }
        // a withdrawal needs neither key: only the account that owns the listing counts
        let vt_key =
            if matches!(job, Job::Submission | Job::Withdraw) { None } else { gh.read_vt_key().await };
        if matches!(job, Job::Vt | Job::Submit) && vt_key.is_none() {
            return Err("no VirusTotal key: the catalog lists a package only when VirusTotal knows it, and the packages are scanned with your own key - a free account at virustotal.com has one (MCP: mod_catalog action=vt_key)".to_owned());
        }
        let input = Input {
            root: session.root.clone(),
            manifest: m,
            key,
            signed_in,
            token: account.token,
            vt_key: vt_key.unwrap_or_else(Secret::default),
        };
        // the last job has said it ended (the view) a moment before its worker leaves the job
        // table: a start right after it waits for that, instead of being refused as busy
        let ending: Vec<JobId> = editor
            .jobs
            .snapshot()
            .into_iter()
            .filter(|j| j.running() && j.group.as_deref() == Some("catalog"))
            .map(|j| j.id)
            .collect();
        for id in ending {
            editor.jobs.wait(id).await;
        }
        {
            let mut v = self.view.lock();
            v.job = job;
            v.state = JobState::Running;
            v.stage = "Starting".to_owned();
            v.step = 0;
            v.error.clear();
        }
        self.from_ui.store(from_ui, Ordering::Relaxed);

        let (title, stop) = job.card();
        let (me, ed) = (self.clone(), editor.clone());
        let module_id = input.manifest.id.clone();
        let started = editor.jobs.start("catalog", Some("catalog"), title, move |ctx| async move {
            let steps: Vec<&str> = if job == Job::Submit { vec!["Check", "Scan", "Send"] } else { Vec::new() };
            ed.services.get_or_init::<CardTable>(Default::default).lock().insert(
                ctx.id,
                json!({
                    "key": "catalog",
                    "subtitle": module_id,
                    "steps": steps,
                    "from_mcp": !me.from_ui.load(Ordering::Relaxed),
                    "stop_label": stop,
                    "stop_tip": "Stops the job at its next step: nothing more is sent. What already reached GitHub or VirusTotal stays there.",
                }),
            );
            let run = Run { svc: me, gh: Github::of(&ed), exec: ed.exec.clone(), ctx };
            match run.work(job, input).await {
                Ok(()) => Ok(Value::Null),
                Err(e) if e == gz_github::CANCELLED => Err(JobError::from(Cancelled)),
                Err(e) => Err(JobError::new(e)),
            }
        });
        match started {
            Ok(id) => {
                *self.job_id.lock() = Some(id);
                Ok(id)
            }
            Err(_) => {
                let mut v = self.view.lock();
                v.state = JobState::Idle;
                v.stage.clear();
                Err("a Mod Browser job is running already".to_owned())
            }
        }
    }

    /// What a job leaves (the worker's end).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finish(
        &self,
        job: Job,
        input: &Input,
        cancelled: bool,
        outcome: &Result<(), String>,
        check: Check,
        seen: Checked,
        standing: Standing,
    ) {
        let ok = outcome.is_ok();
        let mut v = self.view.lock();
        v.state = if cancelled {
            JobState::Cancelled
        } else if ok {
            JobState::Done
        } else {
            JobState::Failed
        };
        v.error = if cancelled {
            gz_github::CANCELLED.to_owned()
        } else {
            outcome.as_ref().err().cloned().unwrap_or_default()
        };
        v.stage.clear();
        let check_done = check.done;
        if check_done {
            v.check = check;
        }
        if seen.vt.state != "none" && seen.vt.state != "cancelled" {
            v.virustotal = seen.vt;
            v.vt_scanned = job != Job::Check;
        }
        // what the catalog and GitHub said about the module, once a job asked them
        let asked = match job {
            Job::Submission | Job::Withdraw => ok,
            Job::Check | Job::Submit => check_done,
            _ => false,
        };
        if asked && !cancelled {
            // a submission GitHub did not name is unknown, never "not submitted"
            v.submission_read = input.signed_in && standing.submission_read;
            v.submission = if v.submission_read { standing.submission } else { CatalogIssue::default() };
            v.submission_error = standing.submission_error;
            v.withdrawal = if input.signed_in { standing.withdrawal } else { CatalogIssue::default() };
            v.listing = standing.listing;
        }
    }

    /// `mod_catalog status`.
    pub async fn status_json(&self, editor: &Arc<Editor>) -> Value {
        self.drop_stale(editor).await;
        let v = self.view();
        let key = gz_release::key_state(editor).await;
        let gh = Github::of(editor);
        let vt_key = gh.read_vt_key().await.is_some();
        let source = self.account_source(editor, false).await;
        let issue = |i: &CatalogIssue| {
            json!({
                "number": i.number,
                "state": i.state,
                "verdict": i.verdict,
                "verdict_text": i.verdict_text,
                "url": i.url,
                "note": i.note,
            })
        };
        let mut submission = issue(&v.submission);
        if let Value::Object(map) = &mut submission {
            map.insert("known".to_owned(), Value::Bool(v.submission_read));
            map.insert("listed".to_owned(), Value::Bool(v.listed()));
        }
        let findings: Vec<Value> = v
            .check
            .findings
            .iter()
            .map(|f| json!({ "level": f.level.name(), "code": f.code, "text": f.text }))
            .collect();
        json!({
            "catalog": gh.endpoints().catalog_repo,
            "key": {
                "exists": key.exists,
                "locked": key.locked,
                "path": key.path,
                "key_id": key.key_id,
                "public": key.public_key,
                "error": key.error,
            },
            "vt_key": vt_key,
            "account": { "signed_in": source != AccountSource::None, "source": source.text(), "login": v.login },
            "job": {
                "kind": v.job.name(),
                "state": v.state.name(),
                "pending": v.state == JobState::Running,
                "stage": v.stage,
                "error": v.error,
            },
            "check": {
                "done": v.check.done,
                "blocking": v.check.blocking,
                "release": { "version": v.check.version, "url": v.check.url, "signed": v.check.signed, "key_id": v.check.key_id },
                "findings": findings,
            },
            "virustotal": scan::json(&v.virustotal),
            "submission": submission,
            "withdrawal": issue(&v.withdrawal),
            "listing": { "state": v.listing.state, "github": v.listing.github, "this_key": v.listing.this_key },
            "window": self.window_open(),
        })
    }
}
