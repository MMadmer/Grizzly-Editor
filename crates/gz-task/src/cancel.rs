use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

/// The request to stop. Cheap to clone and to check; work checks it between steps.
#[derive(Clone, Default)]
pub struct Cancel(Arc<Inner>);

#[derive(Default)]
struct Inner {
    flag: AtomicBool,
    notify: Notify,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("stopped")]
pub struct Cancelled;

impl Cancel {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.flag.store(true, Ordering::Release);
        self.0.notify.notify_waiters();
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.flag.load(Ordering::Acquire)
    }

    /// `Err` once stopped: `cancel.check()?` between the steps of a long function.
    pub fn check(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() { Err(Cancelled) } else { Ok(()) }
    }

    /// Resolves when stopped, for a `select!` beside a wait.
    pub async fn cancelled(&self) {
        loop {
            let waiting = self.0.notify.notified();
            if self.is_cancelled() {
                return;
            }
            waiting.await;
        }
    }
}
