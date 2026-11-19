//! What the PDA and Screens tabs draw that no other surface does.

use egui::{Align2, CornerRadius, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, icon_button, metrics, palette};

/// A list's header: its title in the strong face and a + at its right end. Returns whether the
/// + was pressed.
pub fn local_list_header(ui: &mut Ui, title: &'static str, tip: &'static str) -> bool {
    let p = palette(ui.ctx());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, metrics::ICON_BUTTON), Sense::hover());
    text::paint(
        ui.painter(),
        &Font::Strong.id(),
        pos2(rect.min.x, rect.center().y),
        Align2::LEFT_CENTER,
        p.text,
        title,
        Some((w - metrics::ICON_BUTTON - 4.0).max(0.0)),
        Clip::End,
    );
    let b = Rect::from_min_size(
        pos2(rect.max.x - metrics::ICON_BUTTON, rect.min.y),
        Vec2::splat(metrics::ICON_BUTTON),
    );
    let mut child =
        ui.new_child(egui::UiBuilder::new().id_salt(("gz-ui-pda.list_header", title)).max_rect(b));
    icon_button(&mut child, Icon::Plus, tip, false, true).clicked()
}

/// A line of the preview's action log or its toast: the preview's own notes ("The PDA closes",
/// "The game calls f()") are the editor's words, an action as the layout writes it is data.
#[must_use]
pub fn local_log_line(line: &str) -> gz_ui::Tx<'static> {
    const OWN: [&str; 4] = ["The PDA closes", "Quests receive signal ", "The game calls ", "mod.ltx: "];
    if OWN.iter().any(|p| line.starts_with(p)) {
        gz_ui::Tx::ui(line.to_owned())
    } else {
        gz_ui::Tx::raw(line.to_owned())
    }
}

/// The frame round the row a dragged row would land on.
pub fn local_drop_mark(ui: &Ui, rect: Rect) {
    let p = palette(ui.ctx());
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(metrics::RADIUS),
        Stroke::new(2.0, p.accent_bright),
        StrokeKind::Inside,
    );
}
