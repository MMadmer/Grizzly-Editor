use std::sync::Arc;

use arc_swap::ArcSwapOption;

type WakeFn = Box<dyn Fn() + Send + Sync>;

/// Asks the window for a frame. The editor draws on demand, so whatever changes what a frame
/// shows - a store, a query, a job - pings this. Headless it does nothing.
#[derive(Clone, Default)]
pub struct Wake(Arc<ArcSwapOption<WakeFn>>);

impl Wake {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs what a ping calls. Set once by the window when it exists.
    pub fn set(&self, f: impl Fn() + Send + Sync + 'static) {
        self.0.store(Some(Arc::new(Box::new(f))));
    }

    pub fn ping(&self) {
        if let Some(f) = self.0.load().as_deref() {
            f();
        }
    }
}
