//! What the World Rules and Characters tabs draw that no other surface does.

use egui::{
    Align2, Context, CornerRadius, Id, Margin, Painter, Pos2, Rect, ScrollArea, Sense, TextEdit, Ui, Vec2,
    pos2,
};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Tone, icons, metrics, palette};

/// A whole number that may be empty: digits only, the hint shown while it is (the base's
/// amount, the new-game screen's). Returns the new value when it was typed: `Some(None)` for an
/// emptied field.
pub fn local_number_text(
    ui: &mut Ui,
    salt: &str,
    value: Option<i64>,
    hint: &str,
    width: f32,
) -> Option<Option<i64>> {
    let ctx = ui.ctx().clone();
    let id = Id::unique(("gz-ui-world.number", salt));
    let buf_id = id.with("buf");
    let shown = value.map(|v| v.to_string()).unwrap_or_default();
    let focused = ctx.memory(|m| m.has_focus(id));
    let mut text_ = if focused {
        ctx.data(|d| d.get_temp::<String>(buf_id)).unwrap_or_else(|| shown.clone())
    } else {
        shown
    };
    let resp = ui.add(
        TextEdit::singleline(&mut text_)
            .id(id)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .hint_text(hint)
            .desired_width(width)
            .min_size(Vec2::new(width, metrics::FRAME)),
    );
    let clean: String = text_.chars().filter(char::is_ascii_digit).take(10).collect();
    if resp.has_focus() {
        ctx.data_mut(|d| d.insert_temp(buf_id, clean.clone()));
    } else {
        ctx.data_mut(|d| d.remove::<String>(buf_id));
    }
    if !resp.changed() {
        return None;
    }
    Some(if clean.is_empty() { None } else { clean.parse::<i64>().ok().map(|v| v.min(i64::from(i32::MAX))) })
}

/// The dot of unsaved edits at the end of a row.
pub fn local_dirty_dot(painter: &Painter, centre: Pos2, ctx: &Context) {
    painter.circle_filled(centre, 3.5, palette(ctx).text);
}

/// The height the problems drawer takes: its header, and the list under it while open.
#[must_use]
pub fn drawer_height(open: bool, avail: f32) -> f32 {
    let header = metrics::SECTION_HEADER;
    if open { header + 180.0_f32.min(avail * 0.4) } else { header }
}

/// The problems drawer at the bottom of a tab: a header that says how
/// many, the list under it while open. `lines` are the sentences with their tones; `ok` is said
/// when there are none.
pub fn local_problems_drawer(ui: &mut Ui, open: &mut bool, height: f32, lines: &[(Tone, String)], ok: &str) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let errors = lines.iter().filter(|(t, _)| *t == Tone::Error).count();
    let warnings = lines.len() - errors;
    let width = ui.available_width().max(1.0);
    let header = metrics::SECTION_HEADER;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, header), Sense::hover());
    let resp = ui.interact(rect, Id::unique("gz-ui-world.problems_header"), Sense::click());
    if resp.clicked() {
        *open = !*open;
    }
    let resp = resp.on_hover_text(if *open { "Hide the problems" } else { "Show the problems" });
    let summary = if errors + warnings == 0 {
        "Problems: none".to_owned()
    } else {
        format!(
            "Problems: {errors} error{}, {warnings} warning{}",
            if errors == 1 { "" } else { "s" },
            if warnings == 1 { "" } else { "s" }
        )
    };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            CornerRadius::same(metrics::RADIUS),
            if resp.hovered() { p.hover } else { p.raised },
        );
        let cy = rect.center().y;
        icons::draw(
            painter,
            if *open { Icon::ChevronDown } else { Icon::ChevronUp },
            Rect::from_min_size(pos2(rect.min.x + 6.0, cy - 6.0), Vec2::splat(12.0)),
            p.text_secondary,
        );
        let tone = if errors > 0 {
            p.error
        } else if warnings > 0 {
            p.warning
        } else {
            p.text_secondary
        };
        icons::draw(
            painter,
            Icon::Problems,
            Rect::from_min_size(pos2(rect.min.x + 24.0, cy - 7.0), Vec2::splat(metrics::GLYPH_ROW)),
            tone,
        );
        text::paint(
            painter,
            &Font::Strong.id(),
            pos2(rect.min.x + 44.0, cy),
            Align2::LEFT_CENTER,
            p.text,
            &summary,
            Some((width - 52.0).max(0.0)),
            Clip::End,
        );
    }
    gz_ui::audit::control(&ctx, &summary, true, None, None, rect);
    if !*open {
        return;
    }
    let body = (height - header).max(24.0);
    ScrollArea::vertical().id_salt("gz-ui-world.problems").max_height(body).auto_shrink([false, false]).show(
        ui,
        |ui| {
            egui::Frame::new().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                for (tone, line) in lines {
                    let _ = text::status(ui, *tone, line);
                }
                if lines.is_empty() {
                    let _ = text::status(ui, Tone::Success, ok);
                }
            });
        },
    );
}

/// A cell of a table row exactly `size` big, its content laid out by `layout` (egui allocates
/// only what the content used, and the next column would start early).
pub fn local_cell<R>(
    ui: &mut Ui,
    size: Vec2,
    layout: egui::Layout,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.allocate_ui_with_layout(size, layout, |ui| {
        ui.set_min_size(size);
        add(ui)
    })
}
