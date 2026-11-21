//! The feedback service of a running editor: the log it writes, when it started, what other layers
//! say into a package, the crash a start offers and the last report sent.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use gz_editor::{Editor, Value};
use parking_lot::Mutex;

use crate::crashes::{self, Crash};

type Provider = Arc<dyn Fn(&Arc<Editor>) -> Value + Send + Sync>;

/// A crash a start offers to send.
#[derive(Clone, Debug)]
pub struct Offer {
    pub crash: Crash,
}

#[derive(Default)]
struct Inner {
    offer: Option<Offer>,
    /// What the last report sent came to (`feedback.send`'s answer).
    last: Option<Value>,
}

pub struct FeedbackService {
    /// The log this run writes.
    pub(crate) log_file: PathBuf,
    pub(crate) started: Instant,
    providers: Mutex<Vec<(&'static str, Provider)>>,
    inner: Mutex<Inner>,
    rev: AtomicU64,
    /// The SHA-256 of the running program: the build a report is about.
    pub(crate) build_id: OnceLock<String>,
    /// The commit this build was made from (`grizzly`'s build script), empty outside a repository.
    pub(crate) commit: &'static str,
    /// The reports made so far: what keeps two ids of one moment apart.
    reports: AtomicU64,
}

impl FeedbackService {
    #[must_use]
    pub fn of(editor: &Editor) -> Option<Arc<Self>> {
        editor.services.get::<Self>()
    }

    /// Moves whenever what the window shows of feedback changed.
    #[must_use]
    pub fn rev(&self) -> u64 {
        self.rev.load(Ordering::Relaxed)
    }

    fn touch(&self, editor: &Editor) {
        self.rev.fetch_add(1, Ordering::Relaxed);
        editor.exec.wake().ping();
    }

    pub(crate) fn next_report(&self) -> u64 {
        self.reports.fetch_add(1, Ordering::Relaxed)
    }

    /// The crash a start offers to send, until it is sent or declined.
    #[must_use]
    pub fn offer(&self) -> Option<Offer> {
        self.inner.lock().offer.clone()
    }

    pub(crate) fn settle_offer(&self, editor: &Editor, name: &str) {
        let mut i = self.inner.lock();
        if i.offer.as_ref().is_some_and(|o| o.crash.name == name) {
            i.offer = None;
        }
        drop(i);
        self.touch(editor);
    }

    pub(crate) fn set_last(&self, editor: &Editor, answer: Value) {
        self.inner.lock().last = Some(answer);
        self.touch(editor);
    }

    #[must_use]
    pub fn last(&self) -> Option<Value> {
        self.inner.lock().last.clone()
    }

    /// What every provider says now, by its name.
    pub(crate) fn provided(&self, editor: &Arc<Editor>) -> serde_json::Map<String, Value> {
        let providers = self.providers.lock().clone();
        providers.into_iter().map(|(name, f)| (name.to_owned(), f(editor))).collect()
    }
}

/// Puts the service on the editor: the crash a start offers is looked for on the IO pool, with the
/// tail of the crashed run's log bound to it and the folder cut to its ten newest crashes.
/// `commit` is the commit the program was built from, as its own build script found it.
pub fn install(editor: &Arc<Editor>, log_file: PathBuf, commit: &'static str) -> Arc<FeedbackService> {
    let svc = editor.services.insert(FeedbackService {
        log_file,
        started: Instant::now(),
        providers: Mutex::new(Vec::new()),
        inner: Mutex::new(Inner::default()),
        rev: AtomicU64::new(1),
        build_id: OnceLock::new(),
        commit,
        reports: AtomicU64::new(0),
    });
    let (ed, s) = (editor.clone(), svc.clone());
    editor.exec.spawn_io(move || {
        let dir = &ed.paths.logs;
        crashes::bind_previous_log(dir);
        crashes::tidy(dir);
        if let Some(crash) = crashes::offer(dir) {
            ed.log.info(format!("[feedback] the last run stopped: {} ({})", crash.summary(), crash.name));
            s.inner.lock().offer = Some(Offer { crash });
            s.touch(&ed);
        }
    });
    svc
}

/// Lets a layer that knows more say it into every package as `editor.json`'s `<name>`: the
/// graphics card, the window's language and scale. Called on a pool thread while a package is made.
pub fn provide(
    editor: &Editor,
    name: &'static str,
    f: impl Fn(&Arc<Editor>) -> Value + Send + Sync + 'static,
) {
    if let Some(svc) = FeedbackService::of(editor) {
        let mut p = svc.providers.lock();
        p.retain(|(n, _)| *n != name);
        p.push((name, Arc::new(f)));
    }
}
