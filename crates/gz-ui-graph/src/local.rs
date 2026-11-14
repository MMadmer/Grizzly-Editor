//! What the graph tab draws that no other surface does.

use egui::{Align2, Color32, Rect, Sense, Ui, Vec2, pos2, vec2};
use gz_ui::{Clip, Font, Icon, Tx, icons, palette, text};

/// The line at the top of a pane: the icon and the name of what it shows, a muted id after it
/// (data). `right` keeps room at the end for the caller's buttons.
pub fn local_pane_header<'a>(
    ui: &mut Ui,
    icon: Icon,
    icon_color: Color32,
    title: impl Into<Tx<'a>>,
    sub: &str,
    right: f32,
) {
    let ctx = ui.ctx().clone();
    let title = title.into();
    let p = palette(&ctx);
    let w = ui.available_width();
    let h = 28.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h + 2.0), Sense::hover());
    let painter = ui.painter();
    let cy = rect.min.y + h * 0.5;
    icons::draw(
        painter,
        icon,
        Rect::from_min_size(pos2(rect.min.x, cy - 8.0), Vec2::splat(16.0)),
        icon_color,
    );
    let x = rect.min.x + 24.0;
    let mut room = (rect.max.x - right - x).max(0.0);
    let strong = Font::Strong.id();
    let tw = text::width(&ctx, &strong, &title).min(room * if sub.is_empty() { 1.0 } else { 0.65 });
    text::paint(
        painter,
        &strong,
        pos2(x, cy),
        Align2::LEFT_CENTER,
        p.text,
        &title,
        Some(tw + 0.5),
        Clip::End,
    );
    room -= tw + 8.0;
    if !sub.is_empty() && room > 24.0 {
        text::paint(
            painter,
            &Font::Small.id(),
            pos2(x + tw + 8.0, cy),
            Align2::LEFT_CENTER,
            p.text_muted,
            Tx::raw(sub),
            Some(room),
            Clip::End,
        );
    }
}
