//! Widgets the kit lacks, local to the graph tab (docs/UI_DESIGN.md, section 4: a widget the kit
//! lacks starts in its package and moves into the kit when a second surface wants it).

use egui::{Align2, Color32, CornerRadius, Id, Key, Rect, Response, ScrollArea, Sense, Ui, UiBuilder, Vec2, pos2, vec2};
use gz_nq::Catalog;
use gz_ui::menu::{MenuItem, menu_item};
use gz_ui::theme::metrics;
use gz_ui::{Clip, Font, Icon, SearchField, icon_button_sized, icons, palette, text};

use crate::look;

/// Rows a search popup shows before it scrolls.
const PICKER_ROWS: f32 = 12.0;

/// A folding header outside a property grid (`XUI::Section` with a `HeaderAction`): a chevron,
/// the title, a count badge, and one icon action at its right end. Returns (open, action).
pub fn local_fold_header(ui: &mut Ui, id_salt: &str, title: &str, badge: Option<&str>, default_open: bool, action: Option<(Icon, &str)>) -> (bool, bool) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, metrics::SECTION_HEADER), Sense::hover());
    let id = fold_id(id_salt);
    let mut open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(default_open);
    let action_w = if action.is_some() { metrics::SECTION_HEADER } else { 0.0 };
    let head = Rect::from_min_max(rect.min, pos2(rect.max.x - action_w, rect.max.y));
    let resp = ui.interact(head, id.with("head"), Sense::click());
    if resp.clicked() {
        open = !open;
        ctx.data_mut(|d| d.insert_temp(id, open));
    }
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(metrics::RADIUS), if resp.hovered() { p.hover } else { p.raised });
        let cy = rect.center().y;
        icons::draw(painter, if open { Icon::ChevronDown } else { Icon::ChevronRight }, Rect::from_min_size(pos2(rect.min.x + 6.0, cy - 6.0), Vec2::splat(metrics::GLYPH_STATUS)), p.text_secondary);
        let mut right = head.max.x - 8.0;
        if let Some(b) = badge.filter(|b| !b.is_empty()) {
            let font = Font::Small.id();
            let bw = text::width(&ctx, &font, b) + 12.0;
            let pill = Rect::from_min_max(pos2(right - bw, cy - 9.0), pos2(right, cy + 9.0));
            painter.rect_filled(pill, CornerRadius::same(9), p.base);
            painter.text(pos2(pill.min.x + 6.0, cy), Align2::LEFT_CENTER, b, font, p.text_secondary);
            right = pill.min.x - 6.0;
        }
        let x = rect.min.x + 24.0;
        text::paint(painter, &Font::Strong.id(), pos2(x, cy), Align2::LEFT_CENTER, p.text, title, Some((right - x).max(0.0)), Clip::End);
    }
    gz_ui::audit::control(&ctx, title, true, None, None, rect);
    let mut clicked = false;
    if let Some((icon, tip)) = action {
        let r = Rect::from_min_size(pos2(rect.max.x - action_w, rect.min.y), Vec2::splat(action_w));
        let mut child = ui.new_child(UiBuilder::new().id_salt(id.with("action")).max_rect(r));
        clicked = icon_button_sized(&mut child, icon, tip, false, true, action_w, metrics::GLYPH_ROW).clicked();
    }
    (open, clicked)
}

/// The state of a fold header: keyed by the caller's key alone, so it can be read and set from
/// outside the scroll area that draws it. The caller scopes the key (the tab's file).
fn fold_id(key: &str) -> Id {
    Id::unique(("gz-ui-graph.fold", key))
}

/// Opens or folds a [`local_fold_header`] from outside (a scroll asked for over MCP).
pub fn local_fold_set(ui: &Ui, key: &str, open: bool) {
    ui.ctx().data_mut(|d| d.insert_temp(fold_id(key), open));
}

/// Whether a [`local_fold_header`] is open, read before it is drawn.
#[must_use]
pub fn local_fold_open(ui: &Ui, key: &str, default_open: bool) -> bool {
    ui.ctx().data(|d| d.get_temp::<bool>(fold_id(key))).unwrap_or(default_open)
}

/// The line at the top of a pane: the icon and the name of what it shows, a muted id after it
/// (`PaneHeader`). `right` keeps room at the end for the caller's buttons.
pub fn local_pane_header(ui: &mut Ui, icon: Icon, icon_color: Color32, title: &str, sub: &str, right: f32) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let w = ui.available_width();
    let h = 28.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, h + 2.0), Sense::hover());
    let painter = ui.painter();
    let cy = rect.min.y + h * 0.5;
    icons::draw(painter, icon, Rect::from_min_size(pos2(rect.min.x, cy - 8.0), Vec2::splat(16.0)), icon_color);
    let x = rect.min.x + 24.0;
    let mut room = (rect.max.x - right - x).max(0.0);
    let strong = Font::Strong.id();
    let tw = text::width(&ctx, &strong, title).min(room * if sub.is_empty() { 1.0 } else { 0.65 });
    text::paint(painter, &strong, pos2(x, cy), Align2::LEFT_CENTER, p.text, title, Some(tw + 0.5), Clip::End);
    room -= tw + 8.0;
    if !sub.is_empty() && room > 24.0 {
        text::paint(painter, &Font::Small.id(), pos2(x + tw + 8.0, cy), Align2::LEFT_CENTER, p.text_muted, sub, Some(room), Clip::End);
    }
}

/// A switch with its label beside it: two flags that share one row (Group, Optional; "not").
pub fn local_check(ui: &mut Ui, label: &str, value: &mut bool, tip: &str) -> bool {
    let resp = gz_ui::grid::switch(ui, value);
    let changed = resp.changed();
    let l = text::label(ui, label, Font::Body, palette(ui.ctx()).text_secondary, Clip::End);
    if !tip.is_empty() {
        resp.on_hover_text(tip);
        l.on_hover_text(tip);
    }
    changed
}

/// The kinds of a use mask as a combo, grouped by their family, under the names the document
/// speaks (`DrawKindCombo`). Returns whether the kind changed.
pub fn local_kind_combo(ui: &mut Ui, id_salt: &str, cat: &Catalog, mask: u8, behaviour: bool, kind: &mut String) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let shown = look::kind_title(cat, kind, behaviour);
    let mut changed = false;
    let w = ui.available_width();
    let resp = egui::ComboBox::from_id_salt(("gz-ui-graph.kind", id_salt)).selected_text(shown.clone()).width(w).height(420.0).show_ui(ui, |ui| {
        let mut group = String::new();
        for k in cat.kinds_for(mask, behaviour) {
            if k.group != group {
                group.clone_from(&k.group);
                text::label(ui, &group, Font::Small, p.text_muted, Clip::End);
            }
            let title = look::kind_title(cat, &k.id, behaviour);
            let picked = menu_item(ui, &title, MenuItem { selected: k.id == *kind, ..MenuItem::default() });
            if picked && k.id != *kind {
                kind.clone_from(&k.id);
                changed = true;
            }
        }
    });
    gz_ui::audit::control(&ctx, &shown, true, None, None, resp.response.rect);
    resp.response.on_hover_text(look::kind_desc(cat, kind, behaviour));
    changed
}

/// The body of a kind search popup (`PickKind`): a full-width filter over a scrolling list of the
/// kinds that match, grouped, driven from the keyboard alone (Up, Down, Enter). Returns the kind
/// picked.
#[allow(clippy::too_many_arguments)]
pub fn pick_kind(ui: &mut Ui, cat: &Catalog, mask: u8, behaviour: bool, hint: &str, filter: &mut String, sel: &mut usize, focus: &mut bool) -> Option<String> {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let mut kinds: Vec<(&gz_nq::Kind, String)> = Vec::new();
    let query = filter.trim().to_owned();
    for k in cat.kinds_for(mask, behaviour) {
        let title = look::kind_title(cat, &k.id, behaviour);
        if !query.is_empty() && !gz_ui::search::matches(&query, &k.id) && !gz_ui::search::matches(&query, &title) {
            continue;
        }
        kinds.push((k, title));
    }
    // as wide as the widest title with its id, within 16..40 em
    let font = Font::Body.id();
    let widest = kinds.iter().map(|(k, t)| text::width(&ctx, &font, t) + text::width(&ctx, &Font::Small.id(), &k.id) + 32.0).fold(0.0f32, f32::max);
    let w = widest.clamp(gz_ui::units::em(16.0), gz_ui::units::em(40.0));
    ui.set_min_width(w);
    ui.set_max_width(w);
    let resp = SearchField::new(filter).hint(hint).min_em(12.0).width(w).id_salt("gz-ui-graph.pick_kind").show(ui);
    if *focus {
        resp.request_focus();
        *focus = false;
    }
    if resp.changed() {
        *sel = 0;
    }
    ui.separator();
    if kinds.is_empty() {
        text::label(ui, "No kind matches", Font::Body, p.text_disabled, Clip::End);
        return None;
    }
    let n = kinds.len();
    let (down, up, enter) = ctx.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp), i.key_pressed(Key::Enter)));
    let before = *sel;
    if down {
        *sel = (*sel + 1) % n;
    }
    if up {
        *sel = if *sel == 0 { n - 1 } else { *sel - 1 };
    }
    *sel = (*sel).min(n - 1);
    let follow = resp.changed() || *sel != before;
    let mut picked = if enter { kinds.get(*sel).map(|(k, _)| k.id.clone()) } else { None };
    let row_h = metrics::ROW;
    let groups = {
        let mut g = 0usize;
        let mut last = "";
        for (k, _) in &kinds {
            if k.group != last {
                g += 1;
                last = &k.group;
            }
        }
        g
    };
    let rows = PICKER_ROWS.min((n + groups) as f32);
    ScrollArea::vertical().id_salt("gz-ui-graph.pick_kind.list").max_height(rows * row_h).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut group = "";
        for (i, (k, title)) in kinds.iter().enumerate() {
            if k.group != group {
                group = &k.group;
                text::label(ui, group, Font::Small, p.text_muted, Clip::End);
            }
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::hover());
            let r = ui.interact(rect, Id::unique(("gz-ui-graph.pick_kind.row", k.id.as_str())), Sense::click());
            if i == *sel || r.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(3), if i == *sel { p.selection_focused } else { p.hover });
            }
            let idw = text::width(&ctx, &Font::Small.id(), &k.id);
            let title_w = text::width(&ctx, &font, title);
            let room = rect.width() - 16.0;
            text::paint(ui.painter(), &font, pos2(rect.min.x + 6.0, rect.center().y), Align2::LEFT_CENTER, p.text, title, Some((room - if title_w + idw + 16.0 < room { idw + 8.0 } else { 0.0 }).max(0.0)), Clip::End);
            // the technical id keeps its place at the right edge while the title leaves room
            if title_w + idw + 16.0 < room {
                ui.painter().text(pos2(rect.max.x - 6.0, rect.center().y), Align2::RIGHT_CENTER, k.id.as_str(), Font::Small.id(), p.text_muted);
            }
            let desc = look::kind_desc(cat, &k.id, behaviour);
            let r = if desc.is_empty() { r } else { r.on_hover_text(desc) };
            if r.clicked() {
                picked = Some(k.id.clone());
            }
            if follow && i == *sel {
                ui.scroll_to_rect(rect, Some(egui::Align::Center));
            }
        }
    });
    picked
}

/// One row of a [`local_pick_list`].
#[derive(Clone, Debug, Default)]
pub struct PickRow {
    pub label: String,
    pub value: String,
    pub extra: String,
}

/// The body of a picker popup (`PickerPopup`): a search field over a list of ids with what the
/// index knows of each; `note` says why the list is empty. Returns the value picked.
pub fn local_pick_list(ui: &mut Ui, rows: &[PickRow], note: &str, search: &mut String, focus: &mut bool) -> Option<String> {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let font = Font::Body.id();
    let small = Font::Small.id();
    let mut w = text::width(&ctx, &font, "Nothing declared in this quest");
    for r in rows.iter().take(400) {
        w = w.max(text::width(&ctx, &font, &r.label) + if r.extra.is_empty() { 0.0 } else { 12.0 + text::width(&ctx, &small, &r.extra) });
    }
    let w = (w + 40.0).clamp(gz_ui::units::em(16.0), gz_ui::units::em(34.0));
    ui.set_min_width(w);
    ui.set_max_width(w);
    let resp = SearchField::new(search).hint("Search").min_em(8.0).width(w).id_salt("gz-ui-graph.pick_list").show(ui);
    if *focus {
        resp.request_focus();
        *focus = false;
    }
    ui.separator();
    let mut picked = None;
    let rows_shown = (rows.len().max(1) as f32).min(14.0);
    ScrollArea::vertical().id_salt("gz-ui-graph.pick_list.rows").max_height(rows_shown * metrics::ROW).auto_shrink([false, true]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, r) in rows.iter().enumerate() {
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), metrics::ROW), Sense::hover());
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let resp: Response = ui.interact(rect, Id::unique(("gz-ui-graph.pick_list.row", i)), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, CornerRadius::same(3), p.hover);
            }
            let ew = if r.extra.is_empty() { 0.0 } else { text::width(&ctx, &small, &r.extra) + 12.0 };
            let room = (rect.width() - 12.0 - ew).max(rect.width() * 0.5);
            text::paint(ui.painter(), &font, pos2(rect.min.x + 6.0, rect.center().y), Align2::LEFT_CENTER, p.text, &r.label, Some(room), Clip::End);
            if !r.extra.is_empty() {
                text::paint(ui.painter(), &small, pos2(rect.max.x - 6.0, rect.center().y), Align2::RIGHT_CENTER, p.text_muted, &r.extra, Some((rect.width() - room - 12.0).max(0.0)), Clip::End);
            }
            if resp.clicked() {
                picked = Some(r.value.clone());
            }
        }
        if !note.is_empty() {
            text::label(ui, note, Font::Body, p.text_disabled, Clip::End);
        }
    });
    picked
}
