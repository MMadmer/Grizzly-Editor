//! Widgets the kit lacks, local to these windows (UI_DESIGN section 4: a widget the kit lacks
//! starts as `local_<name>` and moves into the kit when a second surface wants it).

use egui::{Align, Align2, Layout, Margin, Rect, Sense, Stroke, TextEdit, Ui, UiBuilder, Vec2, pos2};
use gz_ui::list::Column;
use gz_ui::text::{self, Clip};
use gz_ui::theme::{Tone, metrics, palette};
use gz_ui::units::{button_width, em};
use gz_ui::{Button, Font, Kind, PropertyGrid, audit};

/// A one-line field whose text is never drawn (a password, a token, an API key): dots, a hint
/// while it is empty. `width` is the field's. Returns whether Enter was pressed in it.
pub(crate) fn local_secret_field(ui: &mut Ui, id_salt: &str, value: &mut String, hint: &str, width: f32) -> bool {
    let p = palette(ui.ctx());
    let response = ui.add(
        TextEdit::singleline(value)
            .id_salt(("gz-ui-release.secret", id_salt))
            .password(true)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .hint_text(egui::RichText::new(hint).color(p.text_muted))
            .desired_width(width)
            .min_size(Vec2::new(width, metrics::FRAME)),
    );
    audit::control(ui.ctx(), hint, true, None, None, response.rect);
    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

/// A secret field and its button on one line, the field taking what the button leaves. Returns
/// whether the button was pressed, or Enter in the field while the button is on.
pub(crate) fn local_secret_row(ui: &mut Ui, id_salt: &str, value: &mut String, hint: &str, button: &str, enabled: bool, why: &str) -> bool {
    let ctx = ui.ctx().clone();
    let bw = button_width(&ctx, button, Kind::Secondary, false, 0.0);
    let gap = ui.spacing().item_spacing.x;
    let field_w = (ui.available_width() - bw - gap).max(em(6.0));
    let mut pressed = false;
    ui.horizontal(|ui| {
        let entered = local_secret_field(ui, id_salt, value, hint, field_w);
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
        let bw = button.map_or(0.0, |(label, _, _)| button_width(&ctx, label, Kind::Secondary, false, 0.0) + 8.0);
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

/// A block that folds under a header, outside a property grid: the grid's own section around
/// one block. `open` is its state until it is clicked.
pub(crate) fn local_fold(ui: &mut Ui, role: &str, title: &str, open: bool, add: impl FnOnce(&mut Ui)) {
    PropertyGrid::new(role).show(ui, |g| {
        g.section(title, open, None, |g| {
            g.block(add);
        });
    });
}

/// A sortable table's header: the kit's headers, with column lines in the header only (under the
/// last row is empty table, not columns). Returns the column clicked.
pub(crate) fn local_table_head(ui: &mut Ui, columns: &[Column<'_>]) -> Option<usize> {
    let p = palette(ui.ctx());
    let origin = ui.cursor().min;
    let clicked = gz_ui::list::table_headers(ui, columns);
    let mut x = origin.x;
    for c in columns.iter().take(columns.len().saturating_sub(1)) {
        x += c.width;
        let at = x.round() - 0.5;
        ui.painter().line_segment([pos2(at, origin.y + 5.0), pos2(at, origin.y + 19.0)], Stroke::new(1.0, p.border_subtle));
    }
    clicked
}

/// A child laid out left to right in `rect`, for the cells of a row painted by hand.
pub(crate) fn cell(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, rect: Rect) -> Ui {
    ui.new_child(UiBuilder::new().id_salt(salt).max_rect(rect).layout(Layout::left_to_right(Align::Center)))
}
