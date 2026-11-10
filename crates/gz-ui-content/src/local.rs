//! Widgets the kit lacks, local to this crate until a second surface wants them
//! (docs/UI_DESIGN.md, section 4: "A widget the kit lacks is written as a local one").

use egui::text::{CCursor, CCursorRange};
use egui::{Align2, Color32, CornerRadius, Id, Key, Margin, Pos2, Rect, Response, Sense, Stroke, TextEdit, Ui, UiBuilder, Vec2, pos2};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, icons, metrics, palette};

/// A row of the folder tree: the chevron when it has sub-folders, a folder glyph, the name cut
/// to the pane. A click on the chevron toggles it; anywhere else picks it.
pub struct TreeRow {
    pub clicked: bool,
    pub toggled: bool,
    pub response: Response,
}

#[allow(clippy::too_many_arguments)]
pub fn local_tree_row(ui: &mut Ui, id: Id, depth: usize, label: &str, open: bool, has_children: bool, selected: bool, veiled: bool) -> TreeRow {
    let p = palette(ui.ctx());
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, metrics::ROW), Sense::hover());
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let indent = 4.0 + depth as f32 * metrics::INDENT;
    let chevron = Rect::from_min_size(pos2(rect.min.x + indent, rect.center().y - metrics::GLYPH_STATUS * 0.5), Vec2::splat(metrics::GLYPH_STATUS));
    let toggle_hit = Rect::from_min_max(pos2(chevron.min.x - 3.0, rect.min.y), pos2(chevron.max.x + 3.0, rect.max.y));
    let pointer_on_chevron = response.interact_pointer_pos().is_some_and(|at| toggle_hit.contains(at));
    let toggled = has_children && response.clicked() && pointer_on_chevron;
    let clicked = response.clicked() && !toggled;
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let radius = CornerRadius::same(3);
        if selected {
            painter.rect_filled(rect, radius, p.selection);
        } else if response.hovered() {
            painter.rect_filled(rect, radius, p.hover);
        }
        if has_children {
            icons::draw(painter, if open { Icon::ChevronDown } else { Icon::ChevronRight }, chevron, p.text_secondary);
        }
        let glyph = Rect::from_min_size(pos2(chevron.max.x + 4.0, rect.center().y - metrics::GLYPH_ROW * 0.5), Vec2::splat(metrics::GLYPH_ROW));
        icons::draw(painter, if open && has_children { Icon::FolderOpen } else { Icon::Folder }, glyph, if veiled { p.text_disabled } else { p.text_secondary });
        let x = glyph.max.x + 6.0;
        text::paint(painter, &Font::Body.id(), pos2(x, rect.center().y), Align2::LEFT_CENTER, if veiled { p.text_muted } else { p.text }, label, Some((rect.max.x - 4.0 - x).max(0.0)), Clip::End);
    }
    gz_ui::audit::control(ui.ctx(), label, true, None, None, rect);
    TreeRow { clicked, toggled, response }
}

/// A folding header outside a property grid (the C++ `XUI::Section`): a chevron, the title and
/// a count on the right. Its state is kept by `id_salt`; `open` is the state until it is
/// clicked. Returns whether it is open.
pub fn local_section(ui: &mut Ui, id_salt: &str, title: &str, open: bool, right: Option<&str>) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let id = ui.make_persistent_id(("gz-ui-content.section", id_salt));
    let mut is_open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(open);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), metrics::SECTION_HEADER), Sense::hover());
    let resp = ui.interact(rect, id.with("header"), Sense::click());
    if resp.clicked() {
        is_open = !is_open;
    }
    ctx.data_mut(|d| d.insert_temp(id, is_open));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if resp.hovered() {
            painter.rect_filled(rect, CornerRadius::same(3), p.hover);
        }
        let chevron = Rect::from_min_size(pos2(rect.min.x + 4.0, rect.center().y - metrics::GLYPH_STATUS * 0.5), Vec2::splat(metrics::GLYPH_STATUS));
        icons::draw(painter, if is_open { Icon::ChevronDown } else { Icon::ChevronRight }, chevron, p.text_secondary);
        let mut right_x = rect.max.x - 6.0;
        if let Some(r) = right {
            let font = Font::Small.id();
            right_x -= text::width(&ctx, &font, r) + 8.0;
            painter.text(pos2(rect.max.x - 6.0, rect.center().y), Align2::RIGHT_CENTER, r, font, p.text_muted);
        }
        let x = chevron.max.x + 6.0;
        text::paint(painter, &Font::Strong.id(), pos2(x, rect.center().y), Align2::LEFT_CENTER, p.text, title, Some((right_x - x).max(0.0)), Clip::End);
    }
    gz_ui::audit::control(&ctx, title, true, None, None, rect);
    is_open
}

/// The folder path as crumbs: the root, then each folder, chevrons between; the leading crumbs
/// fold into "…" until the rest fits, the folder shown always stays and gives up its middle
/// last. Returns the path picked (`""` for the root), when one was.
pub fn local_breadcrumbs(ui: &mut Ui, root_label: &str, folder: &str) -> Option<String> {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let font = Font::Body.id();
    let (pad, sep, fold_w) = (12.0, 16.0, 28.0);
    let width = ui.available_width();
    let h = metrics::FRAME;
    let mut crumbs: Vec<(String, String, f32)> = vec![(root_label.to_owned(), String::new(), text::width(&ctx, &font, root_label) + pad)];
    let mut path = String::new();
    for part in folder.split('\\').filter(|s| !s.is_empty()) {
        if !path.is_empty() {
            path.push('\\');
        }
        path.push_str(part);
        crumbs.push((part.to_owned(), path.clone(), text::width(&ctx, &font, part) + pad));
    }
    let n = crumbs.len();
    let mut total: f32 = crumbs.iter().enumerate().map(|(i, c)| c.2 + if i > 0 { sep } else { 0.0 }).sum();
    let mut first = 0usize;
    while total > width && first + 1 < n {
        total -= crumbs.get(first).map_or(0.0, |c| c.2) + sep;
        if first == 0 {
            total += fold_w + sep;
        }
        first += 1;
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, h), Sense::hover());
    let painter = ui.painter().clone();
    let mut x = rect.min.x;
    let right = rect.max.x;
    let mut go: Option<String> = None;
    let chevron = |painter: &egui::Painter, x: &mut f32| {
        icons::draw(painter, Icon::ChevronRight, Rect::from_min_size(pos2(*x + 2.0, rect.center().y - 6.0), Vec2::splat(12.0)), p.text_muted);
        *x += sep;
    };
    let fold_id = ui.make_persistent_id("gz-ui-content.crumbs.fold");
    if first > 0 {
        let r = Rect::from_min_size(pos2(x, rect.min.y), Vec2::new(fold_w, h));
        let resp = ui.interact(r, fold_id.with("button"), Sense::click()).on_hover_text("The folders above");
        if resp.hovered() {
            painter.rect_filled(r, CornerRadius::same(metrics::RADIUS), if resp.is_pointer_button_down_on() { p.pressed } else { p.hover });
        }
        painter.text(r.center(), Align2::CENTER_CENTER, crate::caption::ELLIPSIS, font.clone(), if resp.hovered() { p.text } else { p.text_secondary });
        if resp.clicked() {
            egui::Popup::toggle_id(&ctx, fold_id);
        }
        gz_ui::menu::menu_popup(&ctx, fold_id, r, ui.layer_id(), |ui| {
            for (label, path, _) in crumbs.iter().take(first) {
                if gz_ui::menu_item(ui, label, gz_ui::MenuItem::default()) {
                    go = Some(path.clone());
                }
            }
        });
        x += fold_w;
        chevron(&painter, &mut x);
    }
    for (i, (label, path, w)) in crumbs.iter().enumerate().skip(first) {
        if i > first {
            chevron(&painter, &mut x);
        }
        let last = i + 1 == n;
        let w = if last { (right - x).min(*w).max(24.0) } else { *w };
        let r = Rect::from_min_size(pos2(x, rect.min.y), Vec2::new(w, h));
        let resp = ui.interact(r, fold_id.with(("crumb", i)), if last { Sense::hover() } else { Sense::click() });
        if resp.hovered() && !last {
            painter.rect_filled(r, CornerRadius::same(metrics::RADIUS), if resp.is_pointer_button_down_on() { p.pressed } else { p.hover });
        }
        let (shown, cut) = text::ellipsize(&ctx, &font, label, w - pad, Clip::End);
        let color = if last || resp.hovered() { p.text } else { p.text_secondary };
        painter.text(r.center(), Align2::CENTER_CENTER, &shown, font.clone(), color);
        if cut {
            gz_ui::audit::truncated(&ctx, label, &shown, r);
            resp.clone().on_hover_text(label.as_str());
        }
        if resp.clicked() && !last {
            go = Some(path.clone());
        }
        x += w;
    }
    go
}

/// What the rename box did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenameOut {
    /// Enter, or the box let go of the keys: the name is taken.
    pub done: bool,
    /// Esc: nothing is renamed.
    pub cancelled: bool,
}

/// The inline rename field at `rect`: on its first frame it takes the keys and selects the first
/// `select` bytes of the text - the name, never the extension.
pub fn local_rename_box(ui: &mut Ui, id: Id, rect: Rect, text_: &mut String, select: usize, focus: bool) -> RenameOut {
    let mut child = ui.new_child(UiBuilder::new().id_salt(id).max_rect(rect));
    let edit_id = id.with("edit");
    if focus {
        let mut state = egui::text_edit::TextEditState::load(child.ctx(), edit_id).unwrap_or_default();
        let chars = text_.get(..select.min(text_.len())).map_or(0, |s| s.chars().count());
        state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(chars))));
        state.store(child.ctx(), edit_id);
    }
    let resp = child.add(
        TextEdit::singleline(text_)
            .id(edit_id)
            .font(Font::Body.id())
            .margin(Margin::symmetric(6, 3))
            .desired_width(rect.width())
            .min_size(Vec2::new(rect.width(), metrics::FRAME)),
    );
    if focus {
        resp.request_focus();
    }
    let esc = resp.has_focus() && child.input(|i| i.key_pressed(Key::Escape)) || (resp.lost_focus() && child.input(|i| i.key_pressed(Key::Escape)));
    if esc {
        return RenameOut { done: false, cancelled: true };
    }
    RenameOut { done: !focus && resp.lost_focus(), cancelled: false }
}

/// The checkerboard every image editor puts under transparency: cells of 8 pt, down to 4 in a
/// small picture.
pub fn local_checker(painter: &egui::Painter, rect: Rect) {
    let p = palette(painter.ctx());
    let (w, h) = (rect.width(), rect.height());
    if w < 1.0 || h < 1.0 {
        return;
    }
    let cell = (w.max(h) / 8.0).clamp(4.0, 8.0).round().max(2.0);
    painter.rect_filled(rect, CornerRadius::ZERO, p.border);
    let mut y = rect.min.y;
    let mut row = 0;
    while y < rect.max.y {
        let mut x = rect.min.x + if row % 2 == 1 { cell } else { 0.0 };
        while x < rect.max.x {
            painter.rect_filled(Rect::from_min_max(pos2(x, y), pos2((x + cell).min(rect.max.x), (y + cell).min(rect.max.y))), CornerRadius::ZERO, p.border_strong);
            x += cell * 2.0;
        }
        y += cell;
        row += 1;
    }
}

/// A heading inside a menu: Small muted text (ImGui's `SeparatorText`).
pub fn local_menu_heading(ui: &mut Ui, title: &str) {
    let p = palette(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
    ui.painter().text(pos2(rect.min.x + 8.0, rect.center().y + 2.0), Align2::LEFT_CENTER, title, Font::Small.id(), p.text_muted);
}

/// A rubber band: the focused selection's fill and the bright accent's outline.
pub fn local_marquee(painter: &egui::Painter, a: Pos2, b: Pos2) {
    let p = palette(painter.ctx());
    let r = Rect::from_two_pos(a, b);
    painter.rect_filled(r, CornerRadius::ZERO, p.selection_focused.gamma_multiply(0.35));
    painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, p.accent_bright), egui::StrokeKind::Inside);
}

/// A colour at a fraction of its alpha.
#[must_use]
pub fn faded(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a)
}
