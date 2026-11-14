//! What Mod Settings and Package draw that no other surface does.

use egui::{
    Align, CornerRadius, Layout, Rect, Response, Sense, Stroke, StrokeKind, TextureId, Ui, UiBuilder, Vec2,
    pos2,
};
use gz_ui::units::em;
use gz_ui::{Icon, icons, metrics, palette};

/// A block inside one property row: what `add` draws flows top-down from the value's left edge,
/// and the row grows to hold it (the row's own line is laid out left to right and centred).
pub fn local_value_column<R>(ui: &mut Ui, id_salt: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let origin = ui.max_rect().min;
    let w = ui.available_width().max(em(3.0));
    let rect = Rect::from_min_size(origin, Vec2::new(w, f32::INFINITY));
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(("gz-ui-mod.column", id_salt))
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    let r = add(&mut child);
    let used = child.min_rect();
    ui.allocate_rect(
        Rect::from_min_max(origin, pos2(origin.x + w, used.max.y.max(origin.y + metrics::FRAME))),
        Sense::hover(),
    );
    r
}

/// A picture in a frame of `side` points: the texture, or the frame and a picture glyph while
/// there is none, so the row does not jump.
pub fn local_picture(ui: &mut Ui, texture: Option<TextureId>, side: f32) -> Response {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(metrics::RADIUS);
        ui.painter().rect_filled(rect, radius, p.inset);
        match texture {
            Some(id) => {
                ui.painter().image(
                    id,
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            None => {
                ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
                let glyph = 32.0_f32.min(side * 0.5);
                icons::draw(
                    ui.painter(),
                    Icon::Picture,
                    Rect::from_center_size(rect.center(), Vec2::splat(glyph)),
                    p.text_muted,
                );
            }
        }
    }
    resp
}
