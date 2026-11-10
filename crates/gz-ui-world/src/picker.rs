//! The pickers: what comes from a list is PICKED, not typed (CLAUDE.md, "Characters").
//!
//! - [`local_list_picker`] - a long list (items, dialogs, models, communities) in a popup with a
//!   search on top and only the rows in view laid out (`ListPicker`); the field shows "caption
//!   (id)". A value the list does not hold is kept, shown in the warning colour and said so in
//!   the tip - never replaced: the text path may name what only the module brings.
//! - [`item_picker`] - the one item picker of both tabs: a kit row, a stock row, the mode's start
//!   kit (`EditorCharacters::ItemPicker`).
//! - [`local_combo`] - a short list with a preview of its own (Base, Rank, the transitions'
//!   level).
//! - [`local_multi_combo`] - ticks over names, the picked joined in the preview; a picked name
//!   the list lacks stays ticked and says so (`XUI_LocalMultiCombo`).

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, Id, Popup, Rect, Response, ScrollArea, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};
use gz_ui::menu::menu_popup;
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{Font, Icon, MenuItem, SearchField, Tone, icons, menu_item, metrics, palette};

use crate::lists::{Indexed, ListsView};
use crate::local::local_check;

/// The height of a picker's popup list.
const POPUP_H: f32 = 320.0;
/// A row of a menu (the kit's `menu_item`).
const MENU_ROW: f32 = 24.0;

/// What a popup's search left: the query and the rows it matched, for one generation of the list.
#[derive(Clone, Default)]
struct Filtered {
    query: String,
    serial: u64,
    len: usize,
    rows: Arc<Vec<u32>>,
}

fn popup_id(salt: &str) -> Id {
    Id::unique(("gz-ui-world.pick", salt))
}

/// The field a picker opens from: the input fill, the value cut to fit, a chevron or a spinner.
/// Returns the response and whether the text was cut.
pub(crate) fn paint_field(ui: &mut Ui, text_: &str, color: Color32, busy: bool) -> (Response, bool) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let w = ui.available_width().max(em(3.0));
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, metrics::FRAME), if busy { Sense::hover() } else { Sense::click() });
    let mut cut = false;
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(metrics::RADIUS);
        ui.painter().rect_filled(rect, radius, if resp.hovered() && !busy { p.input_hover } else { p.input });
        if resp.has_focus() {
            ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, p.accent_bright), StrokeKind::Inside);
        }
        let glyph = Rect::from_min_size(pos2(rect.max.x - 8.0 - metrics::GLYPH_STATUS, rect.center().y - metrics::GLYPH_STATUS * 0.5), Vec2::splat(metrics::GLYPH_STATUS));
        if busy {
            crate::local::local_spinner_at(&ctx, ui.painter(), glyph);
        } else {
            icons::draw(ui.painter(), Icon::ChevronDown, glyph, p.text_secondary);
        }
        let x = rect.min.x + 8.0;
        let room = (glyph.min.x - 6.0 - x).max(0.0);
        cut = text::paint(ui.painter(), &Font::Body.id(), pos2(x, rect.center().y), Align2::LEFT_CENTER, color, text_, Some(room), Clip::End);
        if cut {
            let (shown, _) = text::ellipsize(&ctx, &Font::Body.id(), text_, room, Clip::End);
            gz_ui::audit::truncated(&ctx, text_, &shown, rect);
        }
    }
    (resp, cut)
}

/// The search on top of a popup, focused when it opens; returns the query.
fn popup_search(ui: &mut Ui, id: Id, width: f32) -> String {
    let ctx = ui.ctx().clone();
    let mut query = ctx.data(|d| d.get_temp::<String>(id.with("search"))).unwrap_or_default();
    let field = SearchField::new(&mut query).hint("Type to filter").width(width).id_salt("gz-ui-world.pick_search").show(ui);
    if ctx.data_mut(|d| d.remove_temp::<bool>(id.with("focus"))).is_some() {
        field.request_focus();
    }
    ctx.data_mut(|d| d.insert_temp(id.with("search"), query.clone()));
    ui.add_space(4.0);
    query
}

/// Opens a picker's popup: an empty search that takes the keyboard.
fn open_popup(ctx: &egui::Context, id: Id) {
    ctx.data_mut(|d| {
        d.insert_temp(id.with("search"), String::new());
        d.insert_temp(id.with("focus"), true);
    });
    Popup::open_id(ctx, id);
}

/// The rows of `list` the query matches, worked out once per query and per read of the list.
fn filtered(ctx: &egui::Context, id: Id, list: &Indexed, query: &str) -> Arc<Vec<u32>> {
    let key = id.with("filtered");
    if let Some(f) = ctx.data(|d| d.get_temp::<Filtered>(key))
        && f.query == query
        && f.serial == list.serial
        && f.len == list.entries.len()
    {
        return f.rows;
    }
    let rows: Vec<u32> = list
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| query.trim().is_empty() || gz_ui::search::matches(query, &format!("{} {}", e.id, e.name)))
        .filter_map(|(i, _)| u32::try_from(i).ok())
        .collect();
    let rows = Arc::new(rows);
    ctx.data_mut(|d| d.insert_temp(key, Filtered { query: query.to_owned(), serial: list.serial, len: list.entries.len(), rows: rows.clone() }));
    rows
}

/// A long list picked from a popup. `lookup` names the values (their captions, whether a value
/// is known), `rows` is what the popup offers (a subset of `lookup`, or the same list). `none`
/// is the row that clears the value - and what the field shows while it is empty. `busy`: the
/// list is still being read. `salt` tells this field from every other one of the surface.
/// Returns whether the value changed.
#[allow(clippy::too_many_arguments)]
pub fn local_list_picker(ui: &mut Ui, salt: &str, value: &mut String, lookup: &Indexed, rows: &Indexed, none: Option<&str>, busy: bool) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let id = popup_id(salt);
    let cur = lookup.get(value);
    let preview = match cur {
        _ if value.is_empty() => none.unwrap_or("").to_owned(),
        Some(e) if !e.name.is_empty() => format!("{}  ({})", e.name, value),
        _ => value.clone(),
    };
    let unknown = !value.is_empty() && cur.is_none() && !lookup.is_empty();
    let color = if unknown {
        p.warning
    } else if value.is_empty() {
        p.text_secondary
    } else {
        p.text
    };
    let (resp, cut) = paint_field(ui, &preview, color, busy);
    let why = busy.then_some("The linked game's lists are being read");
    gz_ui::audit::control(&ctx, &preview, !busy, why, None, resp.rect);
    let resp = if busy {
        resp.on_hover_text("Reading the linked game's lists...")
    } else if unknown {
        resp.on_hover_text(format!("'{value}' is not in the linked game or the project: kept as written"))
    } else if cut {
        resp.on_hover_text(preview.as_str())
    } else {
        resp
    };
    if resp.clicked() {
        if Popup::is_id_open(&ctx, id) {
            Popup::close_id(&ctx, id);
        } else {
            open_popup(&ctx, id);
        }
    }
    // put up from outside to be looked at (`characters_view show=`)
    if !busy && gz_ui::menu::take_popup_request(&ctx, salt) {
        open_popup(&ctx, id);
    }
    if !Popup::is_id_open(&ctx, id) {
        return false;
    }
    let width = resp.rect.width().max(em(18.0));
    let mut picked: Option<String> = None;
    let _ = menu_popup(&ctx, id, resp.rect, ui.layer_id(), |ui| {
        ui.set_width(width);
        let query = popup_search(ui, id, width);
        if let Some(none) = none
            && menu_item(ui, none, MenuItem { selected: value.is_empty(), ..MenuItem::default() })
        {
            picked = Some(String::new());
        }
        let shown = filtered(&ctx, id, rows, &query);
        if shown.is_empty() {
            let _ = text::status(ui, Tone::Muted, "Nothing matches.");
            return;
        }
        ScrollArea::vertical().id_salt(id.with("rows")).max_height(POPUP_H).auto_shrink([false, true]).show_rows(ui, MENU_ROW, shown.len(), |ui, range| {
            for i in range {
                let Some(e) = shown.get(i).and_then(|r| rows.entries.get(*r as usize)) else { continue };
                let label = Indexed::label(e);
                let out = ui.push_id(i, |ui| menu_item(ui, &label, MenuItem { selected: e.id == *value, ..MenuItem::default() }));
                if out.inner {
                    picked = Some(e.id.clone());
                }
            }
        });
    });
    match picked {
        Some(v) if v != *value => {
            *value = v;
            Popup::close_id(&ctx, id);
            true
        }
        Some(_) => {
            Popup::close_id(&ctx, id);
            false
        }
        None => false,
    }
}

/// An item of the linked game (or of the project), by its in-game name: the one item picker of
/// the Characters and World Rules tabs. `none` is the row that clears it (and what an empty
/// field shows). Returns whether the value changed.
pub fn item_picker(ui: &mut Ui, salt: &str, value: &mut String, lists: &ListsView, none: Option<&str>) -> bool {
    local_list_picker(ui, salt, value, &lists.items, &lists.items, none, !lists.loaded)
}

/// A short list with a preview of its own: returns the row picked. `warn` paints the preview in
/// the warning colour (a value the list does not hold).
pub fn local_combo(ui: &mut Ui, salt: &str, preview: &str, warn: bool, rows: &[String], selected: Option<usize>) -> Option<usize> {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let id = popup_id(salt);
    let (resp, cut) = paint_field(ui, preview, if warn { p.warning } else { p.text }, false);
    gz_ui::audit::control(&ctx, preview, true, None, None, resp.rect);
    let resp = if cut { resp.on_hover_text(preview) } else { resp };
    if resp.clicked() {
        Popup::toggle_id(&ctx, id);
    }
    if gz_ui::menu::take_popup_request(&ctx, salt) {
        Popup::open_id(&ctx, id);
    }
    if !Popup::is_id_open(&ctx, id) {
        return None;
    }
    let width = resp.rect.width().max(em(12.0));
    let mut picked = None;
    let _ = menu_popup(&ctx, id, resp.rect, ui.layer_id(), |ui| {
        ui.set_min_width(width);
        ScrollArea::vertical().id_salt(id.with("rows")).max_height(POPUP_H).auto_shrink([false, true]).show(ui, |ui| {
            for (i, row) in rows.iter().enumerate() {
                if ui.push_id(i, |ui| menu_item(ui, row, MenuItem { selected: selected == Some(i), ..MenuItem::default() })).inner {
                    picked = Some(i);
                }
            }
        });
    });
    if picked.is_some() {
        Popup::close_id(&ctx, id);
    }
    picked
}

/// Ticks over the names a list offers: the preview joins what is picked (`none` while nothing
/// is), the popup ticks each one. A picked name the list does not hold stays, says so and can be
/// unticked. `busy`: the list is still being read. Returns whether the picks changed.
pub fn local_multi_combo(ui: &mut Ui, salt: &str, picked: &mut Vec<String>, names: &[String], none: &str, busy: bool) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let id = popup_id(salt);
    let preview = if picked.is_empty() { none.to_owned() } else { picked.join(", ") };
    let (resp, cut) = paint_field(ui, &preview, if picked.is_empty() { p.text_secondary } else { p.text }, busy);
    gz_ui::audit::control(&ctx, &preview, !busy, busy.then_some("The linked game's lists are being read"), None, resp.rect);
    let resp = if cut { resp.on_hover_text(preview.as_str()) } else { resp };
    if resp.clicked() {
        Popup::toggle_id(&ctx, id);
    }
    if !busy && gz_ui::menu::take_popup_request(&ctx, salt) {
        Popup::open_id(&ctx, id);
    }
    if !Popup::is_id_open(&ctx, id) {
        return false;
    }
    let width = resp.rect.width().max(em(14.0));
    let mut changed = false;
    let _ = menu_popup(&ctx, id, resp.rect, ui.layer_id(), |ui| {
        ui.set_min_width(width);
        let unlisted: Vec<String> = picked.iter().filter(|x| !names.contains(x)).cloned().collect();
        let mut toggle = |ui: &mut Ui, name: &str, caption: &str| {
            let mut on = picked.iter().any(|x| x == name);
            if local_check(ui, &format!("{salt}.{name}"), caption, &mut on, width) {
                if on {
                    picked.push(name.to_owned());
                } else if let Some(at) = picked.iter().position(|x| x == name) {
                    picked.remove(at);
                }
                changed = true;
            }
        };
        ScrollArea::vertical().id_salt(id.with("rows")).max_height(POPUP_H).auto_shrink([false, true]).show(ui, |ui| {
            for name in names {
                toggle(ui, name, name);
            }
            for name in &unlisted {
                toggle(ui, name, &format!("{name}  (not in the linked game)"));
            }
            if names.is_empty() {
                let _ = text::status(ui, Tone::Muted, "The linked game lists no trade categories.");
            }
        });
    });
    changed
}
