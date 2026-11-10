//! A value being made off the UI thread: a frame looks whether it is there, a command awaits it.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use gz_task::Executors;
use tokio::sync::watch;

/// What a load job hands over once: the frame reads it with [`Pending::get`] (never waits), an MCP
/// command awaits [`Pending::wait`].
pub struct Pending<T> {
    rx: watch::Receiver<Option<Arc<T>>>,
}

impl<T> Clone for Pending<T> {
    fn clone(&self) -> Self {
        Self { rx: self.rx.clone() }
    }
}

impl<T: Send + Sync + 'static> Pending<T> {
    /// Runs `work` on the runtime; its value arrives when it is done and the window is pinged.
    pub fn spawn<F>(exec: &Executors, work: F) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        let (tx, rx) = watch::channel(None);
        let wake = exec.wake().clone();
        exec.spawn(async move {
            let value = work.await;
            tx.send_replace(Some(Arc::new(value)));
            wake.ping();
        });
        Self { rx }
    }

    /// A value that is there already.
    #[must_use]
    pub fn ready(value: T) -> Self {
        let (_tx, rx) = watch::channel(Some(Arc::new(value)));
        Self { rx }
    }

    /// The value, when it has arrived.
    #[must_use]
    pub fn get(&self) -> Option<Arc<T>> {
        self.rx.borrow().clone()
    }

    /// Waits for the value, at most `timeout`; `None` when the job went away or took too long.
    pub async fn wait(&self, timeout: Duration) -> Option<Arc<T>> {
        let mut rx = self.rx.clone();
        match tokio::time::timeout(timeout, rx.wait_for(Option::is_some)).await {
            Ok(Ok(v)) => v.clone(),
            _ => self.get(),
        }
    }
}
