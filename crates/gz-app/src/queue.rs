//! How other threads reach the shell: a closure queued for the UI thread.
//!
//! The UI thread owns the shell. A command that needs it posts a closure; the shell drains the
//! queue at the START of the next frame, so an opener activates its tab in the same frame, and the
//! answer comes back through a oneshot. Nothing of the UI is shared mutably and shell state needs
//! no lock.

use std::sync::Arc;

use gz_editor::{CmdError, Editor};
use gz_task::Wake;
use parking_lot::Mutex;

use crate::shell::ShellState;

type Job = Box<dyn FnOnce(&mut ShellState, &egui::Context) + Send>;

/// The queue to the UI thread: an `Editor` service while the window runs.
pub struct UiQueue {
    jobs: Mutex<Vec<Job>>,
    /// Off once the window is gone: nothing more is taken.
    open: Mutex<bool>,
    wake: Wake,
}

/// What a command answers when the editor has no window.
#[must_use]
pub fn no_window() -> CmdError {
    CmdError::new("no_window", "the editor runs without a window")
}

impl UiQueue {
    pub(crate) fn new(wake: Wake) -> Self {
        Self { jobs: Mutex::new(Vec::new()), open: Mutex::new(true), wake }
    }

    /// The queue of this editor; `Err` when it runs without a window.
    pub fn of(editor: &Editor) -> Result<Arc<UiQueue>, CmdError> {
        editor.services.get::<UiQueue>().filter(|q| *q.open.lock()).ok_or_else(no_window)
    }

    /// Runs `f` on the UI thread at the start of the next frame and answers what it returned.
    pub async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut ShellState, &egui::Context) -> Result<T, CmdError> + Send + 'static,
    ) -> Result<T, CmdError> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            if !*self.open.lock() {
                return Err(no_window());
            }
            // the closure works for the request that queued it: a job it starts is that request's
            let scope = gz_task::current_scope();
            self.jobs.lock().push(Box::new(move |shell, ctx| {
                let answer = match scope {
                    Some(scope) => gz_task::in_scope_sync(scope, || f(shell, ctx)),
                    None => f(shell, ctx),
                };
                let _ = tx.send(answer);
            }));
        }
        self.wake.ping();
        rx.await.map_err(|_| CmdError::internal("the window is gone"))?
    }

    /// Queues `f` for the start of the next frame and does not wait for it: an answer of a kit
    /// question, a file read that came back.
    pub fn post(&self, f: impl FnOnce(&mut ShellState, &egui::Context) + Send + 'static) {
        if !*self.open.lock() {
            return;
        }
        match gz_task::current_scope() {
            Some(scope) => self
                .jobs
                .lock()
                .push(Box::new(move |shell, ctx| gz_task::in_scope_sync(scope, || f(shell, ctx)))),
            None => self.jobs.lock().push(Box::new(f)),
        }
        self.wake.ping();
    }

    /// Takes what queued up. The UI thread calls it first thing in a frame.
    pub(crate) fn drain(&self) -> Vec<Job> {
        std::mem::take(&mut *self.jobs.lock())
    }

    /// The window closes: what is queued is dropped (its callers get an error) and nothing more
    /// is taken.
    pub(crate) fn close(&self) {
        *self.open.lock() = false;
        self.jobs.lock().clear();
    }
}

/// Runs `f` on the UI thread: [`UiQueue::run`] on this editor's queue, or the "no window" error.
pub async fn on_ui<T: Send + 'static>(
    editor: &Editor,
    f: impl FnOnce(&mut ShellState, &egui::Context) -> Result<T, CmdError> + Send + 'static,
) -> Result<T, CmdError> {
    UiQueue::of(editor)?.run(f).await
}
