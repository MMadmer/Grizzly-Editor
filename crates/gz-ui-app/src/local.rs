//! What the Start page and the shell's own surfaces draw that no other surface does.

use egui::{Stroke, Ui, pos2};
use gz_ui::palette;

/// A one-point rule from `x0` to `x1` at `y`, in the subtle border colour.
pub fn local_rule(ui: &Ui, x0: f32, x1: f32, y: f32) {
    if x1 <= x0 {
        return;
    }
    let p = palette(ui.ctx());
    let y = y.round() + 0.5;
    ui.painter().line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(1.0, p.border_subtle));
}

/// A size as the updater words it - `12 MB`, `3.4 MB`, `900 KB`, `512 bytes` - in the window's
/// language.
#[must_use]
pub fn size_tx(bytes: u64) -> gz_ui::Tx<'static> {
    const MB: f64 = 1024.0 * 1024.0;
    let b = bytes as f64;
    if b >= 10.0 * MB {
        gz_ui::tr!("{n} MB", n = format!("{:.0}", b / MB))
    } else if b >= MB {
        gz_ui::tr!("{n} MB", n = format!("{:.1}", b / MB))
    } else if bytes >= 1024 {
        gz_ui::tr!("{n} KB", n = bytes.div_ceil(1024))
    } else {
        gz_ui::tr!("{n} {n:byte|bytes}", n = bytes)
    }
}
