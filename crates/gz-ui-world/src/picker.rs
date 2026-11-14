//! The pickers: what comes from a list is PICKED, not typed (CLAUDE.md, "Characters"). Each is
//! the kit's [`Combo`] field with the kit's [`PickList`] under it; what this file adds is what
//! the lists of the linked game need.
//!
//! - [`list_picker`] - a long list (items, dialogs, models, communities): the field shows
//!   "caption (id)". A value the list does not hold is kept, shown in the warning tone and said
//!   so in the tip - never replaced: the text path may name what only the module brings.
//! - [`item_picker`] - the one item picker of both tabs: a kit row, a stock row, the mode's start
//!   kit.
//! - [`row_combo`] - a short list with a preview of its own (Base, Rank, the transitions'
//!   level).
//! - [`local_multi_combo`] - ticks over names, the picked joined in the preview; a picked name
//!   the list lacks stays ticked and says so. The kit has no list of ticks: this one is the world
//!   tabs' own, on the kit's combo field.

use std::sync::Arc;

use egui::{Context, Id, Popup, Response, ScrollArea, Ui};
use gz_ui::menu::menu_popup;
use gz_ui::pick::{PickItem, pick_popup};
use gz_ui::units::em;
use gz_ui::{Combo, PickList, Tone, text};

use crate::lists::{Indexed, ListsView};

/// The height of a picker's popup list.
const POPUP_H: f32 = 320.0;
/// What a field says while the linked game's lists are being read.
pub(crate) const READING: &str = "The linked game's lists are being read";

/// What a field's list and its popup live under.
fn pick_id(salt: &str) -> (&'static str, &str) {
    ("gz-ui-world.pick", salt)
}

/// The rows of a long list as the kit's pick items, for one read of the list.
#[derive(Clone)]
struct Rows {
    serial: u64,
    len: usize,
    none: Option<String>,
    items: Arc<Vec<PickItem>>,
}

/// The pick items of `rows`, with the row that clears the value first; made once per read of
/// the list and kept for the field while its popup is used.
fn rows_of(ctx: &Context, key: Id, rows: &Indexed, none: Option<&str>) -> Arc<Vec<PickItem>> {
    if let Some(r) = ctx.data(|d| d.get_temp::<Rows>(key))
        && r.serial == rows.serial
        && r.len == rows.entries.len()
        && r.none.as_deref() == none
    {
        return r.items;
    }
    let mut items = Vec::with_capacity(rows.entries.len() + 1);
    if let Some(none) = none {
        items.push(PickItem::new("", none));
    }
    items.extend(rows.entries.iter().map(|e| PickItem::new(e.id.as_str(), Indexed::label(e))));
    let items = Arc::new(items);
    ctx.data_mut(|d| {
        d.insert_temp(
            key,
            Rows {
                serial: rows.serial,
                len: rows.entries.len(),
                none: none.map(str::to_owned),
                items: items.clone(),
            },
        );
    });
    items
}

/// The field's press and a request from outside (`characters.view show=`, `world.view`): the
/// popup opens on an empty search, or closes. Returns whether it is open now.
fn opened(ctx: &Context, salt: &str, popup: Id, resp: &Response, busy: bool) -> bool {
    let open = Popup::is_id_open(ctx, popup);
    let asked = !busy && gz_ui::menu::take_popup_request(ctx, salt);
    if (resp.clicked() && !open) || (asked && !open) {
        PickList::reset(ctx, pick_id(salt));
        Popup::open_id(ctx, popup);
    } else if resp.clicked() {
        Popup::close_id(ctx, popup);
    }
    Popup::is_id_open(ctx, popup)
}

/// A long list picked from a popup. `lookup` names the values (their captions, whether a value
/// is known), `rows` is what the popup offers (a subset of `lookup`, or the same list). `none`
/// is the row that clears the value - and what the field shows while it is empty. `busy`: the
/// list is still being read. `salt` tells this field from every other one of the surface.
/// Returns whether the value changed.
pub fn list_picker(
    ui: &mut Ui,
    salt: &str,
    value: &mut String,
    lookup: &Indexed,
    rows: &Indexed,
    none: Option<&str>,
    busy: bool,
) -> bool {
    let ctx = ui.ctx().clone();
    let cur = lookup.get(value);
    let preview = match cur {
        _ if value.is_empty() => none.unwrap_or("").to_owned(),
        Some(e) if !e.name.is_empty() => format!("{}  ({})", e.name, value),
        _ => value.clone(),
    };
    let unknown = !value.is_empty() && cur.is_none() && !lookup.is_empty();
    let tip = if unknown {
        format!("'{value}' is not in the linked game or the project: kept as written")
    } else {
        String::new()
    };
    let combo = Combo::new(pick_id(salt), &preview)
        .warn(unknown)
        .placeholder(value.is_empty())
        .busy(busy, READING)
        .tip(&tip);
    let popup = combo.popup_id();
    let resp = combo.field(ui, Popup::is_id_open(&ctx, popup));
    if !opened(&ctx, salt, popup, &resp, busy) {
        return false;
    }
    // the rows are made for a list that is open: a closed field costs its text
    let items = rows_of(&ctx, popup.with("rows"), rows, none);
    let list = PickList::new(pick_id(salt), &items)
        .current(value)
        .hint("Type to filter")
        .empty("Nothing matches.")
        .width(resp.rect.width().max(em(18.0)))
        .max_height(POPUP_H);
    match pick_popup(&ctx, popup, resp.rect, ui.layer_id(), list).and_then(|n| items.get(n)) {
        Some(item) if item.id != *value => {
            value.clone_from(&item.id);
            true
        }
        _ => false,
    }
}

/// An item of the linked game (or of the project), by its in-game name: the one item picker of
/// the Characters and World Rules tabs. `none` is the row that clears it (and what an empty
/// field shows). Returns whether the value changed.
pub fn item_picker(
    ui: &mut Ui,
    salt: &str,
    value: &mut String,
    lists: &ListsView,
    none: Option<&str>,
) -> bool {
    list_picker(ui, salt, value, &lists.items, &lists.items, none, !lists.loaded)
}

/// A short list with a preview of its own: returns the row picked. `warn` shows the preview in
/// the warning tone (a value the list does not hold).
pub fn row_combo(
    ui: &mut Ui,
    salt: &str,
    preview: &str,
    warn: bool,
    rows: &[String],
    selected: Option<usize>,
) -> Option<usize> {
    let ctx = ui.ctx().clone();
    let combo = Combo::new(pick_id(salt), preview).warn(warn);
    let popup = combo.popup_id();
    let resp = combo.field(ui, Popup::is_id_open(&ctx, popup));
    if !opened(&ctx, salt, popup, &resp, false) {
        return None;
    }
    let items: Vec<PickItem> =
        rows.iter().enumerate().map(|(i, row)| PickItem::new(i.to_string(), row.as_str())).collect();
    let current = selected.map(|i| i.to_string()).unwrap_or_default();
    let mut list = PickList::new(pick_id(salt), &items)
        .without_search()
        .width(resp.rect.width().max(em(12.0)))
        .max_height(POPUP_H);
    if selected.is_some() {
        list = list.current(&current);
    }
    pick_popup(&ctx, popup, resp.rect, ui.layer_id(), list)
}

/// Ticks over the names a list offers: the preview joins what is picked (`none` while nothing
/// is), the popup ticks each one. A picked name the list does not hold stays, says so and can be
/// unticked. `busy`: the list is still being read. Returns whether the picks changed.
pub fn local_multi_combo(
    ui: &mut Ui,
    salt: &str,
    picked: &mut Vec<String>,
    names: &[String],
    none: &str,
    busy: bool,
) -> bool {
    let ctx = ui.ctx().clone();
    let preview = if picked.is_empty() { none.to_owned() } else { picked.join(", ") };
    let combo = Combo::new(pick_id(salt), &preview).placeholder(picked.is_empty()).busy(busy, READING);
    let popup = combo.popup_id();
    let resp = combo.field(ui, Popup::is_id_open(&ctx, popup));
    if resp.clicked() {
        Popup::toggle_id(&ctx, popup);
    }
    if !busy && gz_ui::menu::take_popup_request(&ctx, salt) {
        Popup::open_id(&ctx, popup);
    }
    if !Popup::is_id_open(&ctx, popup) {
        return false;
    }
    let width = resp.rect.width().max(em(14.0));
    let mut changed = false;
    let _ = menu_popup(&ctx, popup, resp.rect, ui.layer_id(), |ui| {
        ui.set_min_width(width);
        let unlisted: Vec<String> = picked.iter().filter(|x| !names.contains(x)).cloned().collect();
        let mut toggle = |ui: &mut Ui, name: &str, caption: &str| {
            let mut on = picked.iter().any(|x| x == name);
            if gz_ui::form::check(
                ui,
                Id::unique(("gz-ui-world.check", format!("{salt}.{name}"))),
                caption,
                &mut on,
                width,
            ) {
                if on {
                    picked.push(name.to_owned());
                } else if let Some(at) = picked.iter().position(|x| x == name) {
                    picked.remove(at);
                }
                changed = true;
            }
        };
        ScrollArea::vertical()
            .id_salt(popup.with("rows"))
            .max_height(POPUP_H)
            .auto_shrink([false, true])
            .show(ui, |ui| {
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
