//! The audit: what a frame cut short, let escape, drew disabled - and why - or still waits for.
//!
//! Kit widgets report themselves here while they draw, but only in a frame somebody asked to
//! audit (`ui.audit` is held for that frame), so an ordinary frame pays one flag check per
//! widget and no allocation.

use std::sync::Arc;

use egui::{Context, Id, Rect};
use gz_base::SmolStr;
use parking_lot::Mutex;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuditEntry {
    /// A text that did not fit and was cut with an ellipsis.
    Truncated { surface: SmolStr, full: String, shown: String, rect: [f32; 4] },
    /// Content that lies outside its surface on an axis that cannot scroll.
    Escaped { surface: SmolStr, what: String, rect: [f32; 4] },
    /// A control: its label, whether it can be pressed, and the reason when it cannot.
    Control {
        surface: SmolStr,
        label: String,
        enabled: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        why: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        help: Option<String>,
        rect: [f32; 4],
    },
    /// A surface that shows a busy line instead of its content: what it is waiting for.
    Loading { surface: SmolStr, what: String, rect: [f32; 4] },
    /// A surface that shows an error state in place of its content: its title and its sentence.
    Error { surface: SmolStr, title: String, text: String, rect: [f32; 4] },
}

#[derive(Default)]
struct Shared {
    /// Collect during the frames being drawn.
    wanted: bool,
    entries: Vec<AuditEntry>,
    /// The surface widgets are being drawn into.
    surface: Vec<SmolStr>,
}

#[derive(Clone, Default)]
struct Handle(Arc<Mutex<Shared>>);

fn handle(ctx: &Context) -> Handle {
    let id = Id::unique("gz-ui.audit");
    ctx.data(|d| d.get_temp::<Handle>(id)).unwrap_or_else(|| {
        let made = Handle::default();
        ctx.data_mut(|d| d.insert_temp(id, made.clone()));
        made
    })
}

fn px(ctx: &Context, rect: Rect) -> [f32; 4] {
    let s = ctx.pixels_per_point();
    [rect.min.x * s, rect.min.y * s, rect.max.x * s, rect.max.y * s]
}

/// Starts collecting: the next frames report themselves. Clears what was collected before.
pub fn begin(ctx: &Context) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    s.wanted = true;
    s.entries.clear();
}

/// Stops collecting and returns what the frames reported.
#[must_use]
pub fn take(ctx: &Context) -> Vec<AuditEntry> {
    let h = handle(ctx);
    let mut s = h.0.lock();
    s.wanted = false;
    std::mem::take(&mut s.entries)
}

/// Forgets what the last frame reported, keeping the audit on: called at the start of a frame,
/// so that an audit holds one frame's entries.
pub fn new_frame(ctx: &Context) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    s.entries.clear();
    s.surface.clear();
}

/// Whether this frame is being audited.
#[must_use]
pub fn wanted(ctx: &Context) -> bool {
    handle(ctx).0.lock().wanted
}

/// Names the surface the widgets drawn inside `body` belong to.
pub fn surface<R>(ctx: &Context, name: &str, body: impl FnOnce() -> R) -> R {
    let h = handle(ctx);
    h.0.lock().surface.push(SmolStr::new(name));
    let out = body();
    h.0.lock().surface.pop();
    out
}

fn current(s: &Shared) -> SmolStr {
    s.surface.last().cloned().unwrap_or_default()
}

/// A text was cut: `full` is what it should read, `shown` what is on screen.
pub fn truncated(ctx: &Context, full: &str, shown: &str, rect: Rect) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    if s.wanted {
        let surface = current(&s);
        s.entries.push(AuditEntry::Truncated {
            surface,
            full: full.to_owned(),
            shown: shown.to_owned(),
            rect: px(ctx, rect),
        });
    }
}

/// Content left its surface.
pub fn escaped(ctx: &Context, what: &str, rect: Rect) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    if s.wanted {
        let surface = current(&s);
        s.entries.push(AuditEntry::Escaped { surface, what: what.to_owned(), rect: px(ctx, rect) });
    }
}

/// A busy line was drawn: the surface around it has not got its content yet.
pub fn loading(ctx: &Context, what: &str, rect: Rect) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    if s.wanted {
        let surface = current(&s);
        s.entries.push(AuditEntry::Loading { surface, what: what.to_owned(), rect: px(ctx, rect) });
    }
}

/// An error state was drawn in place of a surface's content.
pub fn error(ctx: &Context, title: &str, text: &str, rect: Rect) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    if s.wanted {
        let surface = current(&s);
        s.entries.push(AuditEntry::Error {
            surface,
            title: title.to_owned(),
            text: text.to_owned(),
            rect: px(ctx, rect),
        });
    }
}

/// A control was drawn.
pub fn control(ctx: &Context, label: &str, enabled: bool, why: Option<&str>, help: Option<&str>, rect: Rect) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    if s.wanted {
        let surface = current(&s);
        s.entries.push(AuditEntry::Control {
            surface,
            label: label.to_owned(),
            enabled,
            why: why.map(str::to_owned),
            help: help.map(str::to_owned),
            rect: px(ctx, rect),
        });
    }
}
