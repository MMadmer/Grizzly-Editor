//! Widgets the kit lacks, local to this crate until a second surface wants them
//! (docs/UI_DESIGN.md, section 4: "A widget the kit lacks is written as a local one").

use std::f32::consts::TAU;

use egui::{Align2, Context, CornerRadius, Id, LayerId, Margin, Painter, Pos2, Rect, Response, ScrollArea, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2, pos2};
use gz_ui::menu::menu_popup;
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{Font, Icon, MenuItem, SearchField, Tone, icons, menu_item, metrics, palette};

/// Three quarters of a ring turning once a second, painted into `rect`: a field whose list is
/// still being read. It asks for the next frame.
pub fn local_spinner_at(ctx: &Context, painter: &Painter, rect: Rect) {
    let p = palette(ctx);
    let r = rect.width().min(rect.height()) * 0.5 - 1.0;
    let start = (ctx.input(|i| i.time) as f32 % 1.0) * TAU;
    let points: Vec<Pos2> = (0..=18).map(|k| rect.center() + Vec2::angled(start + TAU * 0.75 * k as f32 / 18.0) * r).collect();
    painter.add(Shape::line(points, Stroke::new(1.5, p.accent_bright)));
    ctx.request_repaint();
}

/// The id a [`local_section`] keeps its state under.
#[must_use]
pub fn section_id(key: &str) -> Id {
    Id::unique(("gz-ui-world.section", key))
}

/// Opens or folds a [`local_section`] from outside (MCP: what it asks for is under More).
pub fn set_section_open(ctx: &Context, key: &str, open: bool) {
    ctx.data_mut(|d| d.insert_temp(section_id(key), open));
}

/// A folding header outside a property grid (the C++ `XUI::Section`): a chevron, the title, and
/// at the right what it holds in a few words; the whole row a click target. `open` is its state
/// until it is clicked. Returns whether it is open.
pub fn local_section(ui: &mut Ui, key: &str, title: &str, open: bool, right: Option<&str>) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let id = section_id(key);
    let mut is_open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(open);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), metrics::SECTION_HEADER), Sense::hover());
    let resp = ui.interact(rect, id.with("header"), Sense::click());
    if resp.clicked() {
        is_open = !is_open;
    }
    ctx.data_mut(|d| d.insert_temp(id, is_open));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(metrics::RADIUS), if resp.hovered() { p.hover } else { p.raised });
        let cy = rect.center().y;
        let chevron = Rect::from_min_size(pos2(rect.min.x + 6.0, cy - metrics::GLYPH_STATUS * 0.5), Vec2::splat(metrics::GLYPH_STATUS));
        icons::draw(painter, if is_open { Icon::ChevronDown } else { Icon::ChevronRight }, chevron, p.text_secondary);
        let mut end = rect.max.x - 8.0;
        if let Some(right) = right.filter(|r| !r.is_empty()) {
            let font = Font::Small.id();
            let w = (text::width(&ctx, &font, right) + 12.0).min(rect.width() * 0.5);
            let pill = Rect::from_min_max(pos2(end - w, cy - 9.0), pos2(end, cy + 9.0));
            painter.rect_filled(pill, CornerRadius::same(9), p.base);
            text::paint(painter, &font, pos2(pill.min.x + 6.0, cy), Align2::LEFT_CENTER, p.text_secondary, right, Some(w - 12.0), Clip::End);
            end = pill.min.x - 6.0;
        }
        let x = chevron.max.x + 6.0;
        text::paint(painter, &Font::Strong.id(), pos2(x, cy), Align2::LEFT_CENTER, p.text, title, Some((end - x).max(0.0)), Clip::End);
    }
    gz_ui::audit::control(&ctx, title, true, None, None, rect);
    if is_open {
        ui.add_space(4.0);
    }
    is_open
}

/// A one-line text field with a hint while it is empty, as wide as it gets (the kit's
/// `text_field` has no hint). Its id is its own: it keeps the keyboard while the form scrolls.
pub fn local_hint_field(ui: &mut Ui, salt: &str, value: &mut String, hint: &str) -> Response {
    let id = Id::unique(("gz-ui-world.text", salt));
    let w = ui.available_width().max(em(3.0));
    ui.add(
        TextEdit::singleline(value)
            .id(id)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .hint_text(hint)
            .desired_width(w)
            .min_size(Vec2::new(w, metrics::FRAME)),
    )
}

/// A whole number that may be empty: digits only, the hint shown while it is (the base's
/// amount, the new-game screen's). Returns the new value when it was typed: `Some(None)` for an
/// emptied field.
pub fn local_number_text(ui: &mut Ui, salt: &str, value: Option<i64>, hint: &str, width: f32) -> Option<Option<i64>> {
    let ctx = ui.ctx().clone();
    let id = Id::unique(("gz-ui-world.number", salt));
    let buf_id = id.with("buf");
    let shown = value.map(|v| v.to_string()).unwrap_or_default();
    let focused = ctx.memory(|m| m.has_focus(id));
    let mut text_ = if focused { ctx.data(|d| d.get_temp::<String>(buf_id)).unwrap_or_else(|| shown.clone()) } else { shown };
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

/// A box with its label after it; a click on either flips it. `width` is the room it takes. A
/// disabled `Ui` draws it dimmed and takes no click.
pub fn local_check(ui: &mut Ui, salt: &str, label: &str, value: &mut bool, width: f32) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let enabled = ui.is_enabled();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, metrics::FRAME), Sense::hover());
    let id = Id::unique(("gz-ui-world.check", salt));
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    let mut changed = false;
    if enabled && resp.clicked() {
        *value = !*value;
        changed = true;
    }
    if ui.is_rect_visible(rect) {
        let b = Rect::from_center_size(pos2(rect.min.x + metrics::FRAME * 0.5, rect.center().y), Vec2::splat(16.0));
        let radius = CornerRadius::same(3);
        if *value {
            let fill = if !enabled {
                p.text_disabled
            } else if resp.hovered() {
                p.primary_hover
            } else {
                p.accent
            };
            ui.painter().rect_filled(b, radius, fill);
            icons::draw(ui.painter(), Icon::Check, b.shrink(2.0), p.on_fill);
        } else {
            ui.painter().rect(b, radius, if resp.hovered() && enabled { p.input_hover } else { p.input }, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);
        }
        if !label.is_empty() {
            let x = rect.min.x + metrics::FRAME + 4.0;
            let color = if enabled { p.text } else { p.text_disabled };
            text::paint(ui.painter(), &Font::Body.id(), pos2(x, rect.center().y), Align2::LEFT_CENTER, color, label, Some((rect.max.x - x).max(0.0)), Clip::End);
        }
    }
    gz_ui::audit::control(&ctx, label, enabled, None, None, rect);
    changed
}

/// A box in the middle of a table cell `width` wide; `tip` on hover. Returns whether it flipped.
pub fn local_cell_check(ui: &mut Ui, salt: &str, value: &mut bool, width: f32, enabled: bool, tip: &str) -> bool {
    let (cell, _) = ui.allocate_exact_size(Vec2::new(width, metrics::FRAME), Sense::hover());
    let r = Rect::from_center_size(cell.center(), Vec2::splat(metrics::FRAME));
    let mut child = ui.new_child(UiBuilder::new().id_salt(("gz-ui-world.cell_check", salt)).max_rect(r));
    if !enabled {
        child.disable();
    }
    let changed = local_check(&mut child, salt, "", value, metrics::FRAME);
    if !tip.is_empty() {
        ui.interact(r, Id::unique(("gz-ui-world.cell_tip", salt)), Sense::hover()).on_hover_text(tip);
    }
    changed
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

/// The problems drawer at the bottom of a tab (`XUI_LocalProblemsDrawer`): a header that says how
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
        format!("Problems: {errors} error{}, {warnings} warning{}", if errors == 1 { "" } else { "s" }, if warnings == 1 { "" } else { "s" })
    };
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(metrics::RADIUS), if resp.hovered() { p.hover } else { p.raised });
        let cy = rect.center().y;
        icons::draw(painter, if *open { Icon::ChevronDown } else { Icon::ChevronUp }, Rect::from_min_size(pos2(rect.min.x + 6.0, cy - 6.0), Vec2::splat(12.0)), p.text_secondary);
        let tone = if errors > 0 {
            p.error
        } else if warnings > 0 {
            p.warning
        } else {
            p.text_secondary
        };
        icons::draw(painter, Icon::Problems, Rect::from_min_size(pos2(rect.min.x + 24.0, cy - 7.0), Vec2::splat(metrics::GLYPH_ROW)), tone);
        text::paint(painter, &Font::Strong.id(), pos2(rect.min.x + 44.0, cy), Align2::LEFT_CENTER, p.text, &summary, Some((width - 52.0).max(0.0)), Clip::End);
    }
    gz_ui::audit::control(&ctx, &summary, true, None, None, rect);
    if !*open {
        return;
    }
    let body = (height - header).max(24.0);
    ScrollArea::vertical().id_salt("gz-ui-world.problems").max_height(body).auto_shrink([false, false]).show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            for (tone, line) in lines {
                let _ = text::status(ui, *tone, line);
            }
            if lines.is_empty() {
                let _ = text::status(ui, Tone::Success, ok);
            }
        });
    });
}

/// A level of the linked game picked from a searchable list, the ones listed already left out
/// (`XUI_LocalLevelPicker`). `levels` are `(name, caption)`. Open it with `Popup::open_id`.
/// Returns the level picked.
pub fn local_level_picker(ctx: &Context, popup: Id, anchor: Rect, layer: LayerId, levels: &[(String, String)], taken: &[String]) -> Option<String> {
    if !egui::Popup::is_id_open(ctx, popup) {
        return None;
    }
    let width = em(24.0);
    let search_id = popup.with("search");
    let mut picked = None;
    let _ = menu_popup(ctx, popup, anchor, layer, |ui| {
        ui.set_width(width);
        let mut query = ctx.data(|d| d.get_temp::<String>(search_id)).unwrap_or_default();
        let field = SearchField::new(&mut query).hint("Search levels").width(width).id_salt("gz-ui-world.level_search").show(ui);
        if ctx.data_mut(|d| d.remove_temp::<bool>(popup.with("focus"))).is_some() {
            field.request_focus();
        }
        ctx.data_mut(|d| d.insert_temp(search_id, query.clone()));
        ui.add_space(4.0);
        if levels.is_empty() {
            let _ = text::status(ui, Tone::Muted, "Link a game to pick its levels.");
            return;
        }
        ScrollArea::vertical().id_salt(popup.with("rows")).max_height(280.0).auto_shrink([false, true]).show(ui, |ui| {
            let mut shown = 0;
            for (name, caption) in levels {
                if taken.contains(name) {
                    continue;
                }
                let label = if caption.is_empty() { name.clone() } else { format!("{name}  -  {caption}") };
                if !gz_ui::search::matches(&query, &label) {
                    continue;
                }
                shown += 1;
                if ui.push_id(name.as_str(), |ui| menu_item(ui, &label, MenuItem::default())).inner {
                    picked = Some(name.clone());
                }
            }
            if shown == 0 {
                let _ = text::status(ui, Tone::Muted, "No level matches.");
            }
        });
    });
    if picked.is_some() {
        egui::Popup::close_id(ctx, popup);
    }
    picked
}

/// Opens a [`local_level_picker`]: an empty search that takes the keyboard.
pub fn open_level_picker(ctx: &Context, popup: Id) {
    ctx.data_mut(|d| {
        d.insert_temp(popup.with("search"), String::new());
        d.insert_temp(popup.with("focus"), true);
    });
    egui::Popup::open_id(ctx, popup);
}

/// A cell of a table row exactly `size` big, its content laid out by `layout` (egui allocates
/// only what the content used, and the next column would start early).
pub fn local_cell<R>(ui: &mut Ui, size: Vec2, layout: egui::Layout, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    ui.allocate_ui_with_layout(size, layout, |ui| {
        ui.set_min_size(size);
        add(ui)
    })
}
