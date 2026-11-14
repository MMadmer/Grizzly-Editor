//! What the release windows draw that no other surface does.

use egui::{Align, Align2, Layout, Rect, Sense, Ui, UiBuilder, Vec2, pos2};
use gz_ui::text::{self, Clip};
use gz_ui::theme::{Tone, metrics, palette};
use gz_ui::units::{button_width, em};
use gz_ui::{Button, Font, Kind};

/// The kit's secret field and its button on one line, the field taking what the button leaves. Returns
/// whether the button was pressed, or Enter in the field while the button is on.
pub(crate) fn local_secret_row(
    ui: &mut Ui,
    id_salt: &str,
    value: &mut String,
    hint: &str,
    button: &str,
    enabled: bool,
    why: &str,
) -> bool {
    let ctx = ui.ctx().clone();
    let bw = button_width(&ctx, button, Kind::Secondary, false, 0.0);
    let gap = ui.spacing().item_spacing.x;
    let field_w = (ui.available_width() - bw - gap).max(em(6.0));
    let mut pressed = false;
    ui.horizontal(|ui| {
        let id = ui.make_persistent_id(("gz-ui-release.secret", id_salt));
        let field = gz_ui::form::secret_field(ui, id, value, hint, field_w);
        let entered = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        pressed = Button::new(button).enabled(enabled, why).show(ui).clicked() || (entered && enabled);
    });
    pressed
}

/// The device code the author types on GitHub: Mono, as tall as a Title line.
pub(crate) fn local_code(ui: &mut Ui, code: &str) {
    let p = palette(ui.ctx());
    // Cascadia's line is 1.2 em: the em of an 18 pt line
    let font = Font::Mono.at(Font::Title.line() / 1.2);
    let w = text::width(ui.ctx(), &font, code);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, Font::Title.line()), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().text(pos2(rect.min.x, rect.center().y), Align2::LEFT_CENTER, code, font, p.text);
    }
}

/// A card's title line: the title (in the tone's colour when `toned`), a badge after it, and a
/// button flush right. Returns whether the button was pressed.
#[allow(clippy::too_many_arguments)]
pub(crate) fn local_card_title(
    ui: &mut Ui,
    title: &str,
    tone: Tone,
    toned: bool,
    badge: Option<(&str, Tone)>,
    button: Option<(&str, bool, &str)>,
) -> bool {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let mut pressed = false;
    ui.horizontal(|ui| {
        ui.set_min_height(metrics::FRAME);
        let bw =
            button.map_or(0.0, |(label, _, _)| button_width(&ctx, label, Kind::Secondary, false, 0.0) + 8.0);
        let room = (ui.available_width() - bw).max(em(4.0));
        ui.scope(|ui| {
            ui.set_max_width(room);
            let color = if toned { p.tone(tone) } else { p.text };
            text::label(ui, title, Font::Strong, color, Clip::End);
            if let Some((text_, t)) = badge {
                gz_ui::state::badge(ui, text_, t);
            }
        });
        if let Some((label, enabled, why)) = button {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                pressed = Button::new(label).enabled(enabled, why).show(ui).clicked();
            });
        }
    });
    pressed
}

/// A child laid out left to right in `rect`, for the cells of a row painted by hand.
pub(crate) fn cell(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, rect: Rect) -> Ui {
    ui.new_child(UiBuilder::new().id_salt(salt).max_rect(rect).layout(Layout::left_to_right(Align::Center)))
}
