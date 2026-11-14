//! A gate many pass and one closes.
//!
//! Work that may run beside anything else passes through; the one thing that has to run alone
//! closes the gate for as long as it takes. The editor has one use for it: threads that submit
//! to the GPU pass, and the window closes it while it configures a surface - the graphics
//! runtime refuses a configuration that another thread's submission crosses.

use std::sync::Arc;

use parking_lot::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Cloned freely: every clone is the same gate.
#[derive(Clone, Default)]
pub struct Gate(Arc<RwLock<()>>);

/// Inside the gate; it cannot close until this is dropped.
pub struct Passing<'a>(#[allow(dead_code)] RwLockReadGuard<'a, ()>);

/// The gate is closed until this is dropped.
pub struct Closed<'a>(#[allow(dead_code)] RwLockWriteGuard<'a, ()>);

impl Gate {
    /// Passes through. Waits only while the gate is closed; keep what happens inside short.
    #[must_use]
    pub fn pass(&self) -> Passing<'_> {
        Passing(self.0.read())
    }

    /// Closes the gate: waits until everyone inside has left, then nobody passes.
    #[must_use]
    pub fn close(&self) -> Closed<'_> {
        Closed(self.0.write())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::*;

    #[test]
    fn nobody_passes_a_closed_gate() {
        let gate = Gate::default();
        let through = Arc::new(AtomicBool::new(false));
        // many pass at once
        let (a, b) = (gate.pass(), gate.pass());
        drop((a, b));
        let closed = gate.close();
        let (g, flag) = (gate.clone(), through.clone());
        let worker = std::thread::spawn(move || {
            let _inside = g.pass();
            flag.store(true, Ordering::Release);
        });
        std::thread::sleep(Duration::from_millis(30));
        assert!(!through.load(Ordering::Acquire));
        drop(closed);
        worker.join().unwrap();
        assert!(through.load(Ordering::Acquire));
    }
}
