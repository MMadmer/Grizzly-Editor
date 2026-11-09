//! The layout store: sizes the author dragged - splitters, panel widths, label columns - by
//! key, in points. One store per context; the application saves and restores it.

use std::sync::Arc;

use egui::{Context, Id};
use gz_base::SmolStr;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

#[derive(Default)]
struct Store {
    values: FxHashMap<SmolStr, f32>,
    /// Changed since the application last saved it.
    dirty: bool,
}

#[derive(Clone, Default)]
struct Handle(Arc<Mutex<Store>>);

fn handle(ctx: &Context) -> Handle {
    let id = Id::unique("gz-ui.layout");
    ctx.data(|d| d.get_temp::<Handle>(id)).unwrap_or_else(|| {
        let made = Handle::default();
        ctx.data_mut(|d| d.insert_temp(id, made.clone()));
        made
    })
}

/// The stored value of a key, or `default`.
#[must_use]
pub fn get(ctx: &Context, key: &str, default: f32) -> f32 {
    handle(ctx).0.lock().values.get(key).copied().unwrap_or(default)
}

#[must_use]
pub fn has(ctx: &Context, key: &str) -> bool {
    handle(ctx).0.lock().values.contains_key(key)
}

pub fn set(ctx: &Context, key: &str, value: f32) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    if s.values.get(key) != Some(&value) {
        s.values.insert(SmolStr::new(key), value);
        s.dirty = true;
    }
}

/// Forgets a key: its default applies again.
pub fn reset(ctx: &Context, key: &str) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    if s.values.remove(key).is_some() {
        s.dirty = true;
    }
}

pub fn reset_all(ctx: &Context) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    if !s.values.is_empty() {
        s.values.clear();
        s.dirty = true;
    }
}

/// Every key with its value, sorted by key.
#[must_use]
pub fn all(ctx: &Context) -> Vec<(SmolStr, f32)> {
    let mut out: Vec<(SmolStr, f32)> = handle(ctx).0.lock().values.iter().map(|(k, v)| (k.clone(), *v)).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Replaces the store with saved values: at start.
pub fn load(ctx: &Context, values: impl IntoIterator<Item = (SmolStr, f32)>) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    s.values = values.into_iter().collect();
    s.dirty = false;
}

/// Whether the store changed since the last call, and resets the flag: the application saves
/// when this says so.
#[must_use]
pub fn take_dirty(ctx: &Context) -> bool {
    std::mem::take(&mut handle(ctx).0.lock().dirty)
}
