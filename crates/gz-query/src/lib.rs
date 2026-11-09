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
use std::sync::Arc;

use gz_base::Rev;
use gz_task::Executors;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

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
        let slot = slots.entry(key.clone()).or_insert_with(|| Slot { rev: Rev::ZERO, value: None, running: false });
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
                let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(compute)).ok();
                {
                    let mut slots = inner.slots.lock();
                    if let Some(slot) = slots.get_mut(&key) {
                        slot.running = false;
                        if let Some(value) = value {
                            slot.value = Some(Arc::new(value));
                            slot.rev = rev;
                        }
                    }
                }
                inner.exec.wake().ping();
            });
        }
        answer
    }

    /// Awaits the value for `rev`, computing it on the CPU pool when it is missing.
    pub async fn fresh(&self, key: &K, rev: Rev, compute: impl FnOnce() -> V + Send + 'static) -> Arc<V> {
        if let Some(v) = self.peek_at(key, rev) {
            return v;
        }
        let value = Arc::new(self.inner.exec.cpu(compute).await);
        let mut slots = self.inner.slots.lock();
        let slot = slots.entry(key.clone()).or_insert_with(|| Slot { rev, value: None, running: false });
        slot.value = Some(value.clone());
        slot.rev = rev;
        value
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
