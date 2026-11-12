//! Widgets the kit lacks, local to this crate until a second surface wants them
//! (docs/UI_DESIGN.md, section 4: "A widget the kit lacks is written as a local one").

use egui::{
    Align2, ComboBox, CornerRadius, Frame, Key, Margin, Modifiers, PopupCloseBehavior, Rect, Sense, Shape,
    Stroke, StrokeKind, TextEdit, Ui, Vec2, pos2,
};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Tone, icon_button, icons, metrics, palette};

/// A list's header: its title in the strong face and a + at its right end. Returns whether the
/// + was pressed.
pub fn local_list_header(ui: &mut Ui, title: &str, tip: &str) -> bool {
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

/// A folding header outside a property grid (`XUI::Section`): a chevron and the title, the
/// whole row a click target; `open` is its state until it is clicked.
pub fn local_section(ui: &mut Ui, id_salt: &str, title: &str, open: bool) -> bool {
    let id = ui.make_persistent_id(("gz-ui-pda.section", id_salt));
    gz_ui::form::section(ui, id, title, open)
}

/// A strip of tabs, 26 pt (a panel's tab strip): the selected one in the text colour on the
/// base fill under an accent overline. Returns whether the choice changed.
pub fn local_tab_strip(ui: &mut Ui, id_salt: &str, labels: &[String], current: &mut usize) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let font = Font::Body.id();
    let w = ui.available_width();
    let (strip, _) = ui.allocate_exact_size(Vec2::new(w, metrics::PANEL_TABS), Sense::hover());
    ui.painter().line_segment(
        [pos2(strip.min.x, strip.max.y - 0.5), pos2(strip.max.x, strip.max.y - 0.5)],
        Stroke::new(1.0, p.border_subtle),
    );
    // the tabs share the strip when it is too narrow for their labels
    let wants: Vec<f32> = labels.iter().map(|l| text::width(&ctx, &font, l) + 24.0).collect();
    let total: f32 = wants.iter().sum();
    let shrink = if total > w && total > 0.0 { w / total } else { 1.0 };
    let mut x = strip.min.x;
    let mut changed = false;
    for (i, label) in labels.iter().enumerate() {
        let tw = wants.get(i).copied().unwrap_or(0.0) * shrink;
        let r = Rect::from_min_max(pos2(x, strip.min.y), pos2(x + tw, strip.max.y));
        x += tw;
        let resp = ui.interact(r, ui.make_persistent_id(("gz-ui-pda.tab", id_salt, i)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, *current == i, label));
        if resp.clicked() && *current != i {
            *current = i;
            changed = true;
        }
        let on = *current == i;
        let painter = ui.painter();
        if on {
            painter.rect_filled(r, CornerRadius::ZERO, p.base);
            painter.rect_filled(
                Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + 2.0)),
                CornerRadius::ZERO,
                p.accent_bright,
            );
        } else if resp.hovered() {
            painter.rect_filled(r, CornerRadius::ZERO, p.hover);
        }
        let shown_cut = text::paint(
            painter,
            &font,
            r.center(),
            Align2::CENTER_CENTER,
            if on { p.text } else { p.text_secondary },
            label,
            Some((tw - 12.0).max(0.0)),
            Clip::End,
        );
        if shown_cut {
            gz_ui::audit::truncated(&ctx, label, label, r);
        }
        gz_ui::audit::control(&ctx, label, true, None, None, r);
    }
    changed
}

/// A switch with its label after it, as a row of switches reads (`ImGui::Checkbox`).
pub fn local_check(ui: &mut Ui, label: &str, value: &mut bool) -> bool {
    let p = palette(ui.ctx());
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        changed = gz_ui::grid::switch(ui, value).changed();
        let r = text::label(ui, label, Font::Body, p.text, Clip::End);
        if r.interact(Sense::click()).clicked() {
            *value = !*value;
            changed = true;
        }
    });
    changed
}

/// A combo whose list stays open while its rows are toggled (a set picked from a list): `shown`
/// is what the field says, `add` draws the rows.
pub fn local_multi_combo(ui: &mut Ui, id_salt: &str, shown: &str, add: impl FnOnce(&mut Ui)) {
    let w = ui.available_width();
    let resp = ComboBox::from_id_salt(("gz-ui-pda.multi", id_salt))
        .selected_text(shown)
        .width(w)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, add);
    gz_ui::audit::control(ui.ctx(), shown, true, None, None, resp.response.rect);
}

/// What a [`local_code_view`] was asked to do this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodeOut {
    pub changed: bool,
    /// Ctrl+Enter.
    pub apply: bool,
    /// Ctrl+S while the text has the keys.
    pub save: bool,
}

/// A code text: Cascadia Mono, line numbers in a gutter, an error line marked; the field is as
/// tall as the text, so the pane scrolls - the gutter with it - and follows the caret. `goto`
/// scrolls to a line (1-based) once.
pub fn local_code_view(
    ui: &mut Ui,
    id_salt: &str,
    value: &mut String,
    error_line: u32,
    goto: &mut u32,
) -> CodeOut {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let font = Font::Mono.id();
    let line_h = Font::Mono.line();
    let edit_id = ui.make_persistent_id(("gz-ui-pda.code", id_salt));
    let mut out = CodeOut::default();
    // the keys the field would take: Ctrl+Enter applies, Ctrl+S saves
    if ctx.memory(|m| m.has_focus(edit_id)) {
        out.apply = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Enter));
        out.save = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::S));
    }
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.inset);
    let gutter = text::width(&ctx, &font, "00000") + 12.0;
    let mut scroll =
        egui::ScrollArea::both().id_salt(("gz-ui-pda.code_scroll", id_salt)).auto_shrink([false, false]);
    if *goto > 0 {
        scroll = scroll
            .vertical_scroll_offset((4.0 + (*goto as f32 - 1.0) * line_h - rect.height() / 3.0).max(0.0));
        *goto = 0;
    }
    scroll.show(ui, |ui| {
        let under = ui.painter().add(Shape::Noop);
        let lines = 1 + value.bytes().filter(|&b| b == b'\n').count();
        let min_h = (lines as f32 * line_h + 8.0 + 2.0 * line_h).max(rect.height());
        let left = ui.cursor().min.x;
        ui.horizontal_top(|ui| {
            // the gutter is a column of its own: the field starts after it
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(gutter);
            let edit = TextEdit::multiline(value)
                .id(edit_id)
                .font(font.clone())
                .code_editor()
                .frame(Frame::NONE)
                .margin(Margin { left: 4, right: 6, top: 4, bottom: 4 })
                .desired_width(f32::INFINITY)
                .min_size(Vec2::new(ui.available_width(), min_h))
                .lock_focus(true);
            let shown = edit.show(ui);
            out.changed = shown.response.response.changed();
            let caret = shown.cursor_range.map(|c| c.primary.index.0);
            let origin = shown.galley_pos;
            let area = shown.response.response.rect;
            // the line of every row: a new line starts after a row that ends one
            let mut line = 1u32;
            let mut starts = true;
            let mut chars = 0usize;
            let mut caret_line = 0u32;
            let painter = ui.painter();
            for row in &shown.galley.rows {
                let r = row.rect().translate(origin.to_vec2());
                let n = row.row.char_count_excluding_newline().0;
                if caret.is_some_and(|c| c >= chars && c <= chars + n) && caret_line == 0 {
                    caret_line = line;
                }
                if starts {
                    if line == error_line {
                        let band = Rect::from_min_max(
                            pos2(left, r.min.y),
                            pos2(area.max.x, r.min.y + line_h.max(r.height())),
                        );
                        painter.set(under, Shape::rect_filled(band, CornerRadius::ZERO, p.tint(Tone::Error)));
                    }
                    let color = if line == error_line {
                        p.error
                    } else if caret.is_some() && caret_line == line {
                        p.text_secondary
                    } else {
                        p.text_disabled
                    };
                    painter.text(
                        pos2(left + gutter - 8.0, r.min.y),
                        Align2::RIGHT_TOP,
                        line.to_string(),
                        font.clone(),
                        color,
                    );
                }
                chars += n + usize::from(row.ends_with_newline);
                starts = row.ends_with_newline;
                if row.ends_with_newline {
                    line += 1;
                }
            }
            let x = left + gutter - 3.0;
            painter.line_segment(
                [pos2(x, area.min.y), pos2(x, area.max.y.max(rect.max.y))],
                Stroke::new(1.0, p.border_subtle),
            );
        });
    });
    out
}

/// A row of a list that toggles a member of a set and stays open (`Selectable` with
/// `DontClosePopups`): a check mark while it is on.
pub fn local_toggle_row(ui: &mut Ui, label: &str, on: bool) -> bool {
    let p = palette(ui.ctx());
    let font = Font::Body.id();
    let w = (16.0 + metrics::GLYPH + 8.0 + text::width(ui.ctx(), &font, label)).max(ui.available_width());
    let resp = ui.allocate_response(Vec2::new(w, 24.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, label));
    let rect = resp.rect;
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if resp.hovered() {
            painter.rect_filled(
                rect,
                CornerRadius::same(metrics::RADIUS),
                if resp.is_pointer_button_down_on() { p.pressed } else { p.hover },
            );
        }
        let glyph = Rect::from_min_size(
            pos2(rect.min.x + 8.0, rect.center().y - metrics::GLYPH * 0.5),
            Vec2::splat(metrics::GLYPH),
        );
        if on {
            icons::draw(painter, Icon::Check, glyph, p.accent_bright);
        }
        let x = glyph.max.x + 8.0;
        text::paint(
            painter,
            &font,
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            p.text,
            label,
            Some((rect.max.x - 8.0 - x).max(0.0)),
            Clip::End,
        );
    }
    gz_ui::audit::control(ui.ctx(), label, true, None, None, rect);
    resp.clicked()
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
