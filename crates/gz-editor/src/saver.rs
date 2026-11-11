//! Saving as it is edited.
//!
//! A document that has no Save button - `mod.ltx`, the project file, preferences - is written
//! by an [`AutoSave`]: a task that watches the document's store and puts every published state
//! on disk, in order, one write at a time, skipping states that were overtaken while it wrote.
//! A command that must answer only once its edit is on disk awaits [`AutoSave::flush`].

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::Rev;
use gz_store::{CmdError, State, Store};
use gz_task::Executors;
use tokio::sync::watch;

use crate::fsx;

#[derive(Clone, Debug, Default)]
struct Progress {
    /// The newest revision that is on disk (or needed no write).
    saved: Rev,
    /// Why the last write failed, until one succeeds.
    error: Option<String>,
}

/// Keeps a store's document on disk. It stops when the store or every handle is gone.
#[derive(Clone)]
pub struct AutoSave {
    progress: watch::Receiver<Progress>,
}

impl AutoSave {
    /// Starts saving. `render` turns a state into the file it belongs in and its bytes (`None`
    /// when this state has nothing to save); it runs on the IO pool, and a state whose bytes
    /// equal what is on disk is not written again.
    pub fn spawn<S, R>(
        exec: &Executors,
        store: &Store<S>,
        render: R,
        on_error: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self
    where
        S: State,
        R: Fn(&S) -> Option<(PathBuf, Vec<u8>)> + Send + Sync + 'static,
    {
        // subscribed before the revision is read: an edit published before the task runs is
        // then seen by it (subscribing inside the task lost it, and its flush waited for ever)
        let mut changed = store.subscribe();
        let (tx, progress) = watch::channel(Progress { saved: store.rev(), error: None });
        let store = store.clone();
        let io = exec.clone();
        let render = Arc::new(render);
        exec.spawn(async move {
            let mut on_disk: Option<(PathBuf, Vec<u8>)> = None;
            loop {
                if changed.changed().await.is_err() || tx.is_closed() {
                    break;
                }
                let snap = store.snapshot();
                let rev = snap.rev;
                let render = render.clone();
                let previous = on_disk.take();
                let (kept, outcome) = io
                    .io(move || match render(&snap.state) {
                        None => (previous, Ok(())),
                        Some(next) if previous.as_ref() == Some(&next) => (previous, Ok(())),
                        Some(next) => match fsx::write_atomic(&next.0, &next.1) {
                            Ok(()) => (Some(next), Ok(())),
                            Err(e) => {
                                let why = format!("{} could not be saved: {e}", next.0.display());
                                (previous, Err(why))
                            }
                        },
                    })
                    .await;
                on_disk = kept;
                if let Err(e) = &outcome {
                    on_error(e);
                }
                tx.send_replace(Progress { saved: rev, error: outcome.err() });
            }
        });
        Self { progress }
    }

    /// Waits until revision `rev` (or a later one) is on disk. `Err` when that write failed.
    pub async fn flush(&self, rev: Rev) -> Result<(), CmdError> {
        let mut progress = self.progress.clone();
        loop {
            {
                let now = progress.borrow_and_update();
                if now.saved >= rev {
                    return match &now.error {
                        Some(e) => Err(CmdError::io(e.clone())),
                        None => Ok(()),
                    };
                }
            }
            if progress.changed().await.is_err() {
                return Err(CmdError::internal("the document is no longer being saved"));
            }
        }
    }
}
