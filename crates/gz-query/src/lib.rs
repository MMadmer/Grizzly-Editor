//! Derived data.
//!
//! A [`Query`] holds what is computed FROM state: a validation report, a picker's list, the
//! references to an asset, a tile's icon. Each value is a pure function of inputs that carry a
//! revision; the value is kept with the revision it was computed for and is recomputed when the
//! revision moves. Nobody invalidates anything by hand.
//!
//! The UI asks with [`Query::get`], which never waits: it answers with the fresh value, with the
//! previous one while a new one is computed on the CPU pool, or with nothing yet - and asks the
//! window for a frame when the value arrives. A job or an MCP command asks with
//! [`Query::fresh`] and awaits the real thing.

use std::hash::Hash;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;

use gz_base::Rev;
use gz_task::Executors;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;
use tokio::sync::Notify;

/// What a non-blocking read gets.
#[derive(Debug)]
pub enum Answer<V> {
    /// Computed for the revision asked about.
    Fresh(Arc<V>),
    /// Computed for an earlier revision; the new value is on its way.
    Stale(Arc<V>),
    /// Nothing yet; the value is on its way.
    Pending,
}

impl<V> Clone for Answer<V> {
    fn clone(&self) -> Self {
        match self {
            Answer::Fresh(v) => Answer::Fresh(v.clone()),
            Answer::Stale(v) => Answer::Stale(v.clone()),
            Answer::Pending => Answer::Pending,
        }
    }
}

impl<V> Answer<V> {
    /// The value to draw, fresh or not.
    #[must_use]
    pub fn value(&self) -> Option<&Arc<V>> {
        match self {
            Answer::Fresh(v) | Answer::Stale(v) => Some(v),
            Answer::Pending => None,
        }
    }

    #[must_use]
    pub fn is_fresh(&self) -> bool {
        matches!(self, Answer::Fresh(_))
    }

    /// Whether a computation is running for it: what a spinner asks.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        !self.is_fresh()
    }
}

struct Slot<V> {
    /// The revision `value` was computed for.
    rev: Rev,
    value: Option<Arc<V>>,
    /// A computation is running (for whatever revision was current when it started).
    running: bool,
    /// Told when a computation ends: the readers that wait for one instead of starting their own.
    done: Arc<Notify>,
}

impl<V> Slot<V> {
    fn new() -> Self {
        Self { rev: Rev::ZERO, value: None, running: false, done: Arc::new(Notify::new()) }
    }
}

struct Inner<K, V> {
    exec: Executors,
    slots: Mutex<FxHashMap<K, Slot<V>>>,
}

/// A memo table of one derived value per key.
pub struct Query<K, V> {
    inner: Arc<Inner<K, V>>,
}

impl<K, V> Clone for Query<K, V> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

/// What a waiting read does next.
enum Step<V> {
    Done(Arc<V>),
    Wait(Arc<Notify>),
    Compute,
}

/// Ends a computation: its value for `rev` (none after a panic), and the waiters told.
fn finish<K: Eq + Hash, V>(inner: &Inner<K, V>, key: &K, rev: Rev, value: Option<Arc<V>>) {
    let mut slots = inner.slots.lock();
    if let Some(slot) = slots.get_mut(key) {
        slot.running = false;
        if let Some(value) = value {
            slot.value = Some(value);
            slot.rev = rev;
        }
        slot.done.notify_waiters();
    }
}

impl<K, V> Query<K, V>
where
    K: Eq + Hash + Clone + Send + 'static,
    V: Send + Sync + 'static,
{
    #[must_use]
    pub fn new(exec: Executors) -> Self {
        Self { inner: Arc::new(Inner { exec, slots: Mutex::new(FxHashMap::default()) }) }
    }

    /// Reads without waiting. `rev` is the fingerprint of everything `compute` reads (fold several
    /// with [`Rev::mix`]); `compute` runs on the CPU pool when the value for `rev` is missing and
    /// no computation for this key is running. One computation per key at a time: a revision that
    /// moves while one runs is picked up by the next read.
    pub fn get(&self, key: &K, rev: Rev, compute: impl FnOnce() -> V + Send + 'static) -> Answer<V> {
        let mut slots = self.inner.slots.lock();
        let slot = slots.entry(key.clone()).or_insert_with(Slot::new);
        if slot.rev == rev
            && let Some(v) = &slot.value
        {
            return Answer::Fresh(v.clone());
        }
        let answer = slot.value.clone().map_or(Answer::Pending, Answer::Stale);
        if !slot.running {
            slot.running = true;
            let inner = self.inner.clone();
            let key = key.clone();
            self.inner.exec.spawn_cpu(move || {
                // a panic in `compute` must not leave the slot marked running for ever
                let value = catch_unwind(AssertUnwindSafe(compute)).ok().map(Arc::new);
                finish(&inner, &key, rev, value);
                inner.exec.wake().ping();
            });
        }
        answer
    }

    /// Awaits the value for `rev`, computing it on the CPU pool when it is missing. Callers that
    /// ask while a computation of the key runs wait for it, so one revision is computed once
    /// however many ask at the same time. A panic in `compute` continues in the caller that
    /// started it; the others compute again.
    pub async fn fresh(&self, key: &K, rev: Rev, compute: impl FnOnce() -> V + Send + 'static) -> Arc<V> {
        let mut compute = Some(compute);
        loop {
            // decided under the lock, acted on after it: no guard lives across an await
            let step = {
                let mut slots = self.inner.slots.lock();
                let slot = slots.entry(key.clone()).or_insert_with(Slot::new);
                if slot.rev == rev
                    && let Some(v) = &slot.value
                {
                    Step::Done(v.clone())
                } else if slot.running {
                    Step::Wait(slot.done.clone())
                } else {
                    slot.running = true;
                    Step::Compute
                }
            };
            match step {
                Step::Done(v) => return v,
                Step::Wait(done) => {
                    let wait = done.notified();
                    tokio::pin!(wait);
                    // registered before the lock is taken again: an end between here and the
                    // check is not missed
                    wait.as_mut().enable();
                    let running = self.inner.slots.lock().get(key).is_some_and(|s| s.running);
                    if running {
                        wait.await;
                    }
                }
                Step::Compute => {
                    let Some(compute) = compute.take() else { continue };
                    let inner = self.inner.clone();
                    let key = key.clone();
                    // the slot is settled on the pool, so a caller that stops waiting leaves no
                    // computation marked running
                    let out = self
                        .inner
                        .exec
                        .cpu(move || match catch_unwind(AssertUnwindSafe(compute)) {
                            Ok(v) => {
                                let v = Arc::new(v);
                                finish(&inner, &key, rev, Some(v.clone()));
                                Ok(v)
                            }
                            Err(payload) => {
                                finish(&inner, &key, rev, None);
                                Err(payload)
                            }
                        })
                        .await;
                    return out.unwrap_or_else(|payload| resume_unwind(payload));
                }
            }
        }
    }

    /// The value for exactly `rev`, when it is there.
    #[must_use]
    pub fn peek_at(&self, key: &K, rev: Rev) -> Option<Arc<V>> {
        let slots = self.inner.slots.lock();
        slots.get(key).filter(|s| s.rev == rev).and_then(|s| s.value.clone())
    }

    /// Whatever value the key holds, for any revision.
    #[must_use]
    pub fn peek(&self, key: &K) -> Option<Arc<V>> {
        self.inner.slots.lock().get(key).and_then(|s| s.value.clone())
    }

    /// Drops the keys `keep` refuses: files that are gone, documents that closed.
    pub fn retain(&self, mut keep: impl FnMut(&K) -> bool) {
        self.inner.slots.lock().retain(|k, s| s.running || keep(k));
    }

    pub fn clear(&self) {
        self.inner.slots.lock().retain(|_, s| s.running);
    }
}

/// A single derived value: a [`Query`] with one key.
pub struct Derived<V>(Query<(), V>);

impl<V> Clone for Derived<V> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<V: Send + Sync + 'static> Derived<V> {
    #[must_use]
    pub fn new(exec: Executors) -> Self {
        Self(Query::new(exec))
    }

    pub fn get(&self, rev: Rev, compute: impl FnOnce() -> V + Send + 'static) -> Answer<V> {
        self.0.get(&(), rev, compute)
    }

    pub async fn fresh(&self, rev: Rev, compute: impl FnOnce() -> V + Send + 'static) -> Arc<V> {
        self.0.fresh(&(), rev, compute).await
    }

    #[must_use]
    pub fn peek(&self) -> Option<Arc<V>> {
        self.0.peek(&())
    }
}

#[cfg(test)]
mod tests;
