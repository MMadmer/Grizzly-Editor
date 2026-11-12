use std::ops::Deref;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use arc_swap::ArcSwap;
use gz_base::{Origin, Rev, SmolStr};
use gz_task::{Executors, Wake, panic_message};
use tokio::sync::{mpsc, oneshot, watch};

use crate::error::CmdError;
use crate::history::{History, HistoryInfo, Step};

/// A value a [`Store`] owns.
///
/// `Clone` must be cheap: keep collections in [`imbl`] structures and bulk data behind `Arc`.
pub trait State: Clone + Send + Sync + 'static {
    type Cmd: Send + 'static;
    type Reply: Send + 'static;

    /// Applies a command to `self`, which is a private copy: on `Err` (or a panic) the copy is
    /// dropped and the published state stays as it was. Must not block: no file, no network,
    /// no waiting - work that takes time runs in a job and sends its result as a command.
    fn apply(&mut self, cmd: Self::Cmd, cx: &mut Apply) -> Result<Self::Reply, CmdError>;

    /// What Undo or Redo publishes: `self` is the recorded snapshot, `current` the state being
    /// left. Carry over here what is not content - a camera, a selection of things that still
    /// exist.
    #[must_use]
    fn restored(&self, _current: &Self) -> Self {
        self.clone()
    }
}

/// What a command tells the store about itself while it is applied.
pub struct Apply {
    origin: Origin,
    changed: bool,
    content: bool,
    undo: Option<(String, Option<SmolStr>)>,
    saved: bool,
    reset: bool,
}

impl Apply {
    fn new(origin: Origin) -> Self {
        Self { origin, changed: true, content: false, undo: None, saved: false, reset: false }
    }

    #[must_use]
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// The command only read: nothing is published and no revision moves.
    pub fn unchanged(&mut self) {
        self.changed = false;
    }

    /// The command edited the document: it becomes an undo step with this label.
    pub fn undoable(&mut self, label: impl Into<String>) {
        self.content = true;
        self.undo = Some((label.into(), None));
    }

    /// An edit that is part of a gesture: edits with one key in a row are one undo step.
    pub fn undoable_merged(&mut self, label: impl Into<String>, key: impl Into<SmolStr>) {
        self.content = true;
        self.undo = Some((label.into(), Some(key.into())));
    }

    /// The command changed the document in a way Undo does not take back.
    pub fn content_changed(&mut self) {
        self.content = true;
    }

    /// What the command leaves is what is on disk: the document is clean.
    pub fn mark_saved(&mut self) {
        self.saved = true;
    }

    /// The command replaced the document (it was loaded or created): history starts over, clean.
    pub fn new_document(&mut self) {
        self.content = true;
        self.reset = true;
    }
}

/// A published state with its revision.
pub struct Snap<S> {
    /// Moves with every published change.
    pub rev: Rev,
    /// The serial of the document's content. Undo brings an earlier serial back, so "dirty" is
    /// "this serial is not the saved one".
    pub edit: u64,
    pub state: S,
}

impl<S> Deref for Snap<S> {
    type Target = S;
    fn deref(&self) -> &S {
        &self.state
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StoreOptions {
    /// Undo steps kept.
    pub history_limit: usize,
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self { history_limit: 256 }
    }
}

enum Msg<S: State> {
    Cmd { cmd: S::Cmd, origin: Origin, done: Option<oneshot::Sender<Result<S::Reply, CmdError>>> },
    Undo(oneshot::Sender<Option<String>>),
    Redo(oneshot::Sender<Option<String>>),
    Seal,
    Flush(oneshot::Sender<()>),
}

struct Shared<S> {
    name: &'static str,
    snap: ArcSwap<Snap<S>>,
    hist: ArcSwap<HistoryInfo>,
    changed: watch::Sender<Rev>,
    wake: Wake,
}

impl<S> Shared<S> {
    fn publish(&self, snap: Snap<S>, hist: HistoryInfo) {
        let rev = snap.rev;
        self.snap.store(Arc::new(snap));
        self.hist.store(Arc::new(hist));
        self.changed.send_replace(rev);
        self.wake.ping();
    }
}

/// The one writer of a state, and the handle everybody else reads and commands it through.
pub struct Store<S: State> {
    shared: Arc<Shared<S>>,
    tx: mpsc::UnboundedSender<Msg<S>>,
}

impl<S: State> Clone for Store<S> {
    fn clone(&self) -> Self {
        Self { shared: self.shared.clone(), tx: self.tx.clone() }
    }
}

impl<S: State> Store<S> {
    /// Starts the store's actor on the async runtime. The store lives until its last handle is
    /// dropped.
    pub fn spawn(name: &'static str, initial: S, exec: &Executors, opts: StoreOptions) -> Self {
        let (changed, _) = watch::channel(Rev::ZERO);
        let shared = Arc::new(Shared {
            name,
            snap: ArcSwap::from_pointee(Snap { rev: Rev::ZERO, edit: 0, state: initial }),
            hist: ArcSwap::from_pointee(HistoryInfo::default()),
            changed,
            wake: exec.wake().clone(),
        });
        let (tx, rx) = mpsc::unbounded_channel();
        exec.spawn(run(shared.clone(), rx, opts));
        Self { shared, tx }
    }

    /// The current state. Lock-free; call it from any thread, as often as wanted.
    #[must_use]
    pub fn snapshot(&self) -> Arc<Snap<S>> {
        self.shared.snap.load_full()
    }

    #[must_use]
    pub fn rev(&self) -> Rev {
        self.shared.snap.load().rev
    }

    #[must_use]
    pub fn history(&self) -> Arc<HistoryInfo> {
        self.shared.hist.load_full()
    }

    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.shared.hist.load().dirty
    }

    /// Resolves each time a change is published.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Rev> {
        self.shared.changed.subscribe()
    }

    /// Applies a command and returns its reply. The published snapshot already holds the result
    /// when this resolves.
    pub async fn send(&self, cmd: S::Cmd, origin: Origin) -> Result<S::Reply, CmdError> {
        let (done, rx) = oneshot::channel();
        self.tx.send(Msg::Cmd { cmd, origin, done: Some(done) }).map_err(|_| self.closed())?;
        rx.await.map_err(|_| self.closed())?
    }

    /// Queues a command without waiting for it: what the UI thread does.
    pub fn post(&self, cmd: S::Cmd, origin: Origin) {
        let _ = self.tx.send(Msg::Cmd { cmd, origin, done: None });
    }

    /// Queues a command and hands its outcome to `then` on the runtime.
    pub fn post_then(
        &self,
        exec: &Executors,
        cmd: S::Cmd,
        origin: Origin,
        then: impl FnOnce(Result<S::Reply, CmdError>) + Send + 'static,
    ) {
        let this = self.clone();
        exec.spawn(async move { then(this.send(cmd, origin).await) });
    }

    /// Takes the last undo step back. Returns its label, or `None` with nothing to undo.
    pub async fn undo(&self) -> Option<String> {
        let (done, rx) = oneshot::channel();
        self.tx.send(Msg::Undo(done)).ok()?;
        rx.await.ok().flatten()
    }

    pub async fn redo(&self) -> Option<String> {
        let (done, rx) = oneshot::channel();
        self.tx.send(Msg::Redo(done)).ok()?;
        rx.await.ok().flatten()
    }

    /// Ends a gesture: the next merged edit starts a new undo step.
    pub fn seal(&self) {
        let _ = self.tx.send(Msg::Seal);
    }

    /// Resolves once everything queued before it (`post` included) is applied and published.
    pub async fn flush(&self) {
        let (done, rx) = oneshot::channel();
        if self.tx.send(Msg::Flush(done)).is_ok() {
            let _ = rx.await;
        }
    }

    fn closed(&self) -> CmdError {
        CmdError::internal(format!("the {} store is closed", self.shared.name))
    }
}

async fn run<S: State>(shared: Arc<Shared<S>>, mut rx: mpsc::UnboundedReceiver<Msg<S>>, opts: StoreOptions) {
    let mut hist: History<S> = History::new(opts.history_limit, 0);
    let mut next_edit: u64 = 1;

    while let Some(msg) = rx.recv().await {
        match msg {
            Msg::Cmd { cmd, origin, done } => {
                let cur = shared.snap.load_full();
                let mut cx = Apply::new(origin);
                let mut next = cur.state.clone();
                let result = match catch_unwind(AssertUnwindSafe(|| next.apply(cmd, &mut cx))) {
                    Ok(r) => r,
                    Err(payload) => Err(CmdError::internal(format!(
                        "the {} store hit an internal error: {}",
                        shared.name,
                        panic_message(&*payload)
                    ))),
                };
                if result.is_ok() {
                    if cx.changed {
                        let edit = if cx.content {
                            next_edit += 1;
                            next_edit - 1
                        } else {
                            cur.edit
                        };
                        if cx.reset {
                            hist.clear();
                            hist.saved_edit = edit;
                        } else if let Some((label, merge)) = cx.undo.take() {
                            hist.push(cur.clone(), label, merge);
                        }
                        if cx.saved {
                            hist.saved_edit = edit;
                        }
                        let info = hist.info(edit);
                        shared.publish(Snap { rev: cur.rev.next(), edit, state: next }, info);
                    } else if cx.saved {
                        hist.saved_edit = cur.edit;
                        shared.hist.store(Arc::new(hist.info(cur.edit)));
                        shared.wake.ping();
                    }
                }
                if let Some(done) = done {
                    let _ = done.send(result);
                }
            }
            Msg::Undo(done) => {
                let label = hist.undo.pop_back().map(|step| {
                    let cur = shared.snap.load_full();
                    let label = step.label.clone();
                    let state = step.snap.state.restored(&cur.state);
                    let edit = step.snap.edit;
                    hist.redo.push(Step { snap: cur.clone(), label: step.label, merge: None });
                    let info = hist.info(edit);
                    shared.publish(Snap { rev: cur.rev.next(), edit, state }, info);
                    label
                });
                let _ = done.send(label);
            }
            Msg::Redo(done) => {
                let label = hist.redo.pop().map(|step| {
                    let cur = shared.snap.load_full();
                    let label = step.label.clone();
                    let state = step.snap.state.restored(&cur.state);
                    let edit = step.snap.edit;
                    hist.undo.push_back(Step { snap: cur.clone(), label: step.label, merge: None });
                    let info = hist.info(edit);
                    shared.publish(Snap { rev: cur.rev.next(), edit, state }, info);
                    label
                });
                let _ = done.send(label);
            }
            Msg::Seal => hist.seal(),
            Msg::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}
