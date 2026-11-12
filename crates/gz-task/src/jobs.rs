//! Jobs: long work the author can watch and stop, and a caller can wait for.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use futures::FutureExt;
use gz_base::{Rev, RevCounter, SmolStr};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::cancel::{Cancel, Cancelled};
use crate::exec::Executors;
use crate::panic_message;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Done,
    Failed,
    Cancelled,
}

/// What a job shows of itself at a moment.
#[derive(Clone, Debug, Serialize)]
pub struct JobInfo {
    pub id: JobId,
    /// The machine name of what runs: `build`, `package`, `publish`, `scene_load`, ...
    pub kind: SmolStr,
    /// Jobs of one group run one at a time.
    pub group: Option<SmolStr>,
    pub title: String,
    pub stage: String,
    /// 0..1 when the job can tell.
    pub fraction: Option<f32>,
    pub state: JobState,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub result: Option<Arc<serde_json::Value>>,
    pub error: Option<String>,
}

impl JobInfo {
    #[must_use]
    pub fn running(&self) -> bool {
        self.state == JobState::Running
    }
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct JobError {
    pub message: String,
    pub cancelled: bool,
}

impl JobError {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), cancelled: false }
    }
}

impl From<Cancelled> for JobError {
    fn from(_: Cancelled) -> Self {
        Self { message: "Stopped.".to_owned(), cancelled: true }
    }
}

/// A job of this group is already running.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{title} is still running")]
pub struct JobBusy {
    pub id: JobId,
    pub title: String,
}

/// What a running job holds: its stop request, its progress, the pools.
#[derive(Clone)]
pub struct JobCtx {
    pub id: JobId,
    pub cancel: Cancel,
    pub exec: Executors,
    entry: Arc<Entry>,
    jobs: Arc<Inner>,
}

impl JobCtx {
    /// Names the stage the job is in. Shown on its card and in the status bar.
    pub fn stage(&self, text: impl Into<String>) {
        *self.entry.stage.lock() = text.into();
        self.jobs.touch();
    }

    /// Progress 0..1. Cheap enough to call per file.
    pub fn fraction(&self, value: f32) {
        self.entry.fraction.store(value.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        self.jobs.exec.wake().ping();
    }

    /// Progress as done / total.
    pub fn progress(&self, done: usize, total: usize) {
        if total > 0 {
            self.fraction(done as f32 / total as f32);
        }
    }

    pub fn check(&self) -> Result<(), Cancelled> {
        self.cancel.check()
    }
}

const NO_FRACTION: u32 = u32::MAX;

struct Entry {
    id: JobId,
    kind: SmolStr,
    group: Option<SmolStr>,
    title: String,
    cancel: Cancel,
    stage: Mutex<String>,
    fraction: AtomicU32,
    started_ms: u64,
    end: Mutex<Option<End>>,
}

struct End {
    state: JobState,
    at_ms: u64,
    result: Option<Arc<serde_json::Value>>,
    error: Option<String>,
}

impl Entry {
    fn info(&self) -> JobInfo {
        let bits = self.fraction.load(Ordering::Relaxed);
        let end = self.end.lock();
        JobInfo {
            id: self.id,
            kind: self.kind.clone(),
            group: self.group.clone(),
            title: self.title.clone(),
            stage: self.stage.lock().clone(),
            fraction: (bits != NO_FRACTION).then(|| f32::from_bits(bits)),
            state: end.as_ref().map_or(JobState::Running, |e| e.state),
            started_ms: self.started_ms,
            ended_ms: end.as_ref().map(|e| e.at_ms),
            result: end.as_ref().and_then(|e| e.result.clone()),
            error: end.as_ref().and_then(|e| e.error.clone()),
        }
    }

    fn running(&self) -> bool {
        self.end.lock().is_none()
    }
}

struct Inner {
    exec: Executors,
    list: Mutex<Vec<Arc<Entry>>>,
    next: AtomicU64,
    rev: RevCounter,
    changed: watch::Sender<Rev>,
    keep_finished: usize,
}

impl Inner {
    fn touch(&self) {
        let rev = self.rev.bump();
        self.changed.send_replace(rev);
        self.exec.wake().ping();
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// The jobs one request started: a command run inside [`in_scope`] records here every job it
/// starts, from its own task, from the tasks it spawns through [`Executors::spawn`] and from a
/// closure it queues for another thread and runs there inside [`in_scope_sync`] (the UI thread's
/// queue does). The MCP endpoint asks it when a command outlasts the request's timeout.
#[derive(Clone, Debug, Default)]
pub struct JobScope(Arc<Mutex<Vec<JobId>>>);

impl JobScope {
    /// The jobs started inside the scope so far, oldest first.
    #[must_use]
    pub fn ids(&self) -> Vec<JobId> {
        self.0.lock().clone()
    }
}

tokio::task_local! {
    // the request a task works for: a value only inside the future `in_scope` runs, nothing
    // outside it (the key holds no state of its own)
    static SCOPE: JobScope;
}

/// Runs `fut` for a request: every job started inside it is recorded in `scope`.
pub async fn in_scope<F: Future>(scope: JobScope, fut: F) -> F::Output {
    SCOPE.scope(scope, fut).await
}

/// Runs `f` for a request on a thread that is not the request's task - a closure queued for
/// the UI thread: every job started inside it is recorded in `scope`.
pub fn in_scope_sync<R>(scope: JobScope, f: impl FnOnce() -> R) -> R {
    SCOPE.sync_scope(scope, f)
}

/// The scope of the task that asks, when it runs for a request.
#[must_use]
pub fn current_scope() -> Option<JobScope> {
    SCOPE.try_with(Clone::clone).ok()
}

/// The job center. One per application.
#[derive(Clone)]
pub struct Jobs(Arc<Inner>);

impl Jobs {
    #[must_use]
    pub fn new(exec: Executors) -> Self {
        let (changed, _) = watch::channel(Rev::ZERO);
        Self(Arc::new(Inner {
            exec,
            list: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
            rev: RevCounter::new(),
            changed,
            keep_finished: 32,
        }))
    }

    /// Starts a job. With a `group`, refuses while another job of that group runs.
    pub fn start<F, Fut>(
        &self,
        kind: &str,
        group: Option<&str>,
        title: impl Into<String>,
        work: F,
    ) -> Result<JobId, JobBusy>
    where
        F: FnOnce(JobCtx) -> Fut + Send + 'static,
        Fut: Future<Output = Result<serde_json::Value, JobError>> + Send + 'static,
    {
        let inner = &self.0;
        let entry = {
            let mut list = inner.list.lock();
            if let Some(group) = group
                && let Some(busy) = list.iter().find(|e| e.group.as_deref() == Some(group) && e.running())
            {
                return Err(JobBusy { id: busy.id, title: busy.title.clone() });
            }
            let entry = Arc::new(Entry {
                id: JobId(inner.next.fetch_add(1, Ordering::Relaxed)),
                kind: SmolStr::new(kind),
                group: group.map(SmolStr::new),
                title: title.into(),
                cancel: Cancel::new(),
                stage: Mutex::new(String::new()),
                fraction: AtomicU32::new(NO_FRACTION),
                started_ms: now_ms(),
                end: Mutex::new(None),
            });
            list.push(entry.clone());
            // finished cards beyond the limit go, oldest first
            let finished = list.iter().filter(|e| !e.running()).count();
            if finished > inner.keep_finished {
                let mut drop_n = finished - inner.keep_finished;
                list.retain(|e| {
                    if drop_n > 0 && !e.running() {
                        drop_n -= 1;
                        false
                    } else {
                        true
                    }
                });
            }
            entry
        };
        inner.touch();
        // the request this job was started for, when there is one, learns its id
        let _ = SCOPE.try_with(|s| s.0.lock().push(entry.id));

        let ctx = JobCtx {
            id: entry.id,
            cancel: entry.cancel.clone(),
            exec: inner.exec.clone(),
            entry: entry.clone(),
            jobs: inner.clone(),
        };
        let jobs = inner.clone();
        let id = entry.id;
        inner.exec.spawn(async move {
            let outcome = AssertUnwindSafe(async move { work(ctx).await }).catch_unwind().await;
            let end = match outcome {
                Ok(Ok(value)) => {
                    End { state: JobState::Done, at_ms: now_ms(), result: Some(Arc::new(value)), error: None }
                }
                Ok(Err(e)) => End {
                    state: if e.cancelled { JobState::Cancelled } else { JobState::Failed },
                    at_ms: now_ms(),
                    result: None,
                    error: Some(e.message),
                },
                Err(payload) => End {
                    state: JobState::Failed,
                    at_ms: now_ms(),
                    result: None,
                    error: Some(format!(
                        "The job stopped on an internal error: {}",
                        panic_message(&*payload)
                    )),
                },
            };
            *entry.end.lock() = Some(end);
            jobs.touch();
        });
        Ok(id)
    }

    /// Asks a job to stop. It ends as `Cancelled` when it notices.
    pub fn cancel(&self, id: JobId) -> bool {
        match self.find(id) {
            Some(e) if e.running() => {
                e.cancel.cancel();
                true
            }
            _ => false,
        }
    }

    fn find(&self, id: JobId) -> Option<Arc<Entry>> {
        self.0.list.lock().iter().find(|e| e.id == id).cloned()
    }

    #[must_use]
    pub fn get(&self, id: JobId) -> Option<JobInfo> {
        self.find(id).map(|e| e.info())
    }

    /// Every job the center still shows, oldest first.
    #[must_use]
    pub fn snapshot(&self) -> Vec<JobInfo> {
        let entries: Vec<Arc<Entry>> = self.0.list.lock().clone();
        entries.iter().map(|e| e.info()).collect()
    }

    /// The running job of a group, when there is one.
    #[must_use]
    pub fn running_in(&self, group: &str) -> Option<JobInfo> {
        let entry =
            self.0.list.lock().iter().find(|e| e.group.as_deref() == Some(group) && e.running()).cloned();
        entry.map(|e| e.info())
    }

    #[must_use]
    pub fn any_running(&self) -> bool {
        self.0.list.lock().iter().any(|e| e.running())
    }

    /// Bumps when a job starts, ends or changes its stage.
    #[must_use]
    pub fn rev(&self) -> Rev {
        self.0.rev.get()
    }

    /// Waits for a job to end and returns what it left. `None` for an id the center forgot.
    pub async fn wait(&self, id: JobId) -> Option<JobInfo> {
        let mut changed = self.0.changed.subscribe();
        loop {
            let info = self.get(id)?;
            if !info.running() {
                return Some(info);
            }
            if changed.changed().await.is_err() {
                return self.get(id);
            }
        }
    }

    /// Forgets a finished job (its card was dismissed).
    pub fn dismiss(&self, id: JobId) {
        self.0.list.lock().retain(|e| e.id != id || e.running());
        self.0.touch();
    }
}
