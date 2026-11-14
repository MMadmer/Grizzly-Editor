use std::panic::Location;
use std::sync::Arc;

use arc_swap::ArcSwapOption;
use parking_lot::Mutex;

type WakeFn = Box<dyn Fn() + Send + Sync>;

/// Asks the window for a frame. The editor draws on demand, so whatever changes what a frame
/// shows - a store, a query, a job - pings this. Headless it does nothing.
#[derive(Clone, Default)]
pub struct Wake(Arc<Inner>);

#[derive(Default)]
struct Inner {
    f: ArcSwapOption<WakeFn>,
    /// The code that pinged last: the window names it as the cause of the frame it draws for
    /// the ping (`perf.frames`' `ui_frame_causes`).
    last: Mutex<Option<&'static Location<'static>>>,
}

impl Wake {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs what a ping calls. Set once by the window when it exists.
    pub fn set(&self, f: impl Fn() + Send + Sync + 'static) {
        self.0.f.store(Some(Arc::new(Box::new(f))));
    }

    #[track_caller]
    pub fn ping(&self) {
        if let Some(f) = self.0.f.load().as_deref() {
            *self.0.last.lock() = Some(Location::caller());
            f();
        }
    }

    /// The code that pinged last since the previous call, and forgets it.
    #[must_use]
    pub fn take_last(&self) -> Option<&'static Location<'static>> {
        self.0.last.lock().take()
    }
}
