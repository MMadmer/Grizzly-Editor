//! The layout store: sizes the author dragged - splitters, panel widths, label columns - by
//! key, in points, and the short lists a widget keeps for the author (a pick list's recent and
//! favourite entries, a section folded open). One store per context; the application saves and
//! restores it.

use std::sync::Arc;

use egui::{Context, Id};
use gz_base::SmolStr;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

#[derive(Default)]
struct Store {
    values: FxHashMap<SmolStr, f32>,
    lists: FxHashMap<SmolStr, Vec<String>>,
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

/// Forgets every size (the lists stay: they are the author's picks, not a layout).
pub fn reset_all(ctx: &Context) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    if !s.values.is_empty() {
        s.values.clear();
        s.dirty = true;
    }
}

/// A list kept under `key`: empty when none was kept.
#[must_use]
pub fn list(ctx: &Context, key: &str) -> Vec<String> {
    handle(ctx).0.lock().lists.get(key).cloned().unwrap_or_default()
}

pub fn set_list(ctx: &Context, key: &str, items: Vec<String>) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    if s.lists.get(key) != Some(&items) {
        if items.is_empty() {
            s.lists.remove(key);
        } else {
            s.lists.insert(SmolStr::new(key), items);
        }
        s.dirty = true;
    }
}

/// Puts `item` first in the list under `key`, which keeps at most `keep`: what was picked last.
pub fn push_recent(ctx: &Context, key: &str, item: &str, keep: usize) {
    let mut l = list(ctx, key);
    l.retain(|x| x != item);
    l.insert(0, item.to_owned());
    l.truncate(keep);
    set_list(ctx, key, l);
}

/// Every list, sorted by key.
#[must_use]
pub fn all_lists(ctx: &Context) -> Vec<(SmolStr, Vec<String>)> {
    let mut out: Vec<(SmolStr, Vec<String>)> =
        handle(ctx).0.lock().lists.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Replaces the lists with saved ones: at start.
pub fn load_lists(ctx: &Context, lists: impl IntoIterator<Item = (SmolStr, Vec<String>)>) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    s.lists = lists.into_iter().collect();
}

/// Every key with its value, sorted by key.
#[must_use]
pub fn all(ctx: &Context) -> Vec<(SmolStr, f32)> {
    let mut out: Vec<(SmolStr, f32)> =
        handle(ctx).0.lock().values.iter().map(|(k, v)| (k.clone(), *v)).collect();
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
