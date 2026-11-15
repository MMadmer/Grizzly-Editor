//! What the World Rules and Characters tabs draw that no other surface does.

use egui::{
    Align2, Context, CornerRadius, Id, Margin, Painter, Pos2, Rect, ScrollArea, Sense, TextEdit, Ui, Vec2,
    pos2,
};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, TipExt, Tone, Tx, icons, metrics, palette, tr};

/// A whole number that may be empty: digits only, the hint shown while it is (the base's
/// amount, the new-game screen's). Returns the new value when it was typed: `Some(None)` for an
/// emptied field.
pub fn local_number_text(
    ui: &mut Ui,
    salt: &str,
    value: Option<i64>,
    hint: Tx<'_>,
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
            .hint_text(hint.text(&ctx)) // ui-text: translated by Tx::text
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

/// A value that may be the item's own or inherited: the own text while it has one, else empty
/// with `hint` (the base's or the game's value) shown muted. Returns the text when it was typed
/// (empty: the author took the own value away) and whether the field has the keys.
pub fn local_value_text(
    ui: &mut Ui,
    salt: &str,
    own: Option<&str>,
    hint: Tx<'_>,
    width: f32,
) -> (Option<String>, bool) {
    let ctx = ui.ctx().clone();
    let id = Id::unique(("gz-ui-world.value", salt));
    let buf_id = id.with("buf");
    let shown = own.unwrap_or("").to_owned();
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
            .hint_text(hint.text(&ctx)) // ui-text: translated by Tx::text
            .desired_width(width)
            .min_size(Vec2::new(width, metrics::FRAME)),
    );
    if resp.has_focus() {
        ctx.data_mut(|d| d.insert_temp(buf_id, text_.clone()));
    } else {
        ctx.data_mut(|d| d.remove::<String>(buf_id));
    }
    gz_ui::audit::control(&ctx, salt, true, None, None, resp.rect);
    let focus = resp.has_focus() || resp.clicked();
    (resp.changed().then_some(text_), focus)
}

/// A field that adds what is typed into it when Enter is pressed: a member of a list. Returns
/// the text then.
pub fn local_add_field(ui: &mut Ui, salt: &str, hint: Tx<'_>, width: f32) -> Option<String> {
    let ctx = ui.ctx().clone();
    let id = Id::unique(("gz-ui-world.add", salt));
    let mut text_ = ctx.data(|d| d.get_temp::<String>(id)).unwrap_or_default();
    let resp = ui.add(
        TextEdit::singleline(&mut text_)
            .id(id.with("edit"))
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .hint_text(hint.text(&ctx)) // ui-text: translated by Tx::text
            .desired_width(width)
            .min_size(Vec2::new(width, metrics::FRAME)),
    );
    let enter = resp.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter));
    if enter && !text_.trim().is_empty() {
        ctx.data_mut(|d| d.remove::<String>(id));
        return Some(text_.trim().to_owned());
    }
    ctx.data_mut(|d| d.insert_temp(id, text_));
    None
}

/// One of a few words picked from a list (a token of the catalog), the value shown muted while
/// it is inherited and in the warning tone when the list lacks it. `items.view show=` puts its
/// list up through `salt`. Returns the word picked.
pub fn local_value_combo(
    ui: &mut Ui,
    salt: &str,
    shown: &str,
    own: bool,
    words: &[String],
) -> Option<String> {
    let ctx = ui.ctx().clone();
    let unknown = !shown.is_empty() && !words.iter().any(|w| w.eq_ignore_ascii_case(shown));
    let combo =
        gz_ui::Combo::new(("gz-ui-world.value_combo", salt), Tx::raw(shown)).placeholder(!own).warn(unknown);
    let popup = combo.popup_id();
    let resp = combo.field(ui, egui::Popup::is_id_open(&ctx, popup));
    let asked = gz_ui::menu::take_popup_request(&ctx, salt);
    if resp.clicked() || asked {
        if egui::Popup::is_id_open(&ctx, popup) && !asked {
            egui::Popup::close_id(&ctx, popup);
        } else {
            gz_ui::PickList::reset(&ctx, ("gz-ui-world.value_pick", salt));
            egui::Popup::open_id(&ctx, popup);
        }
    }
    if !egui::Popup::is_id_open(&ctx, popup) {
        return None;
    }
    let items: Vec<gz_ui::pick::PickItem> =
        words.iter().map(|w| gz_ui::pick::PickItem::new(w.as_str(), Tx::raw(w.as_str()))).collect();
    let list = gz_ui::PickList::new(("gz-ui-world.value_pick", salt), &items)
        .without_search()
        .current(shown)
        .width(resp.rect.width().max(gz_ui::units::em(10.0)))
        .max_height(320.0);
    gz_ui::pick::pick_popup(&ctx, popup, resp.rect, ui.layer_id(), list).and_then(|i| words.get(i).cloned())
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
pub fn local_problems_drawer(
    ui: &mut Ui,
    open: &mut bool,
    height: f32,
    lines: &[(Tone, String)],
    ok: &'static str,
) {
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
    let resp = resp.tip(if *open { "Hide the problems" } else { "Show the problems" });
    let summary = if errors + warnings == 0 {
        Tx::from("Problems: none")
    } else {
        tr!(
            "Problems: {errors} {errors:error|errors}, {warnings} {warnings:warning|warnings}",
            errors = errors,
            warnings = warnings
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
    gz_ui::audit::control(&ctx, &summary.text(&ctx), true, None, None, rect);
    if !*open {
        return;
    }
    let body = (height - header).max(24.0);
    ScrollArea::vertical().id_salt("gz-ui-world.problems").max_height(body).auto_shrink([false, false]).show(
        ui,
        |ui| {
            egui::Frame::new().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
                // the problems are the domain's sentences, shown translated where the catalog knows them
                for (tone, line) in lines {
                    let _ = text::status(ui, *tone, Tx::ui(line.as_str()));
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
