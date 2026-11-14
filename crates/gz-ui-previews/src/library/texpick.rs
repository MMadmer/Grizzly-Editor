//! The texture pick of a description: its detail texture, its bump map and its normal map are
//! each a texture of the library or none. The kit's [`Combo`] field with the kit's [`PickList`]
//! under it - search, ranking, every texture on virtual rows - and nothing of its own: the rows
//! are made once per read of the library's list, and only while the popup is open.

use std::sync::Arc;

use egui::{Context, Id, Popup, Ui};
use gz_ui::pick::{PickItem, pick_popup};
use gz_ui::units::em;
use gz_ui::{Combo, PickList};

use super::textures::TexItem;

/// The height of the popup's list, pt.
const POPUP_H: f32 = 320.0;

/// The textures a pick offers: the library's list as it was read, or none (New Textures offers
/// nothing: its pictures are not in the library yet).
#[derive(Clone, Copy)]
pub struct Names<'a>(Option<&'a Arc<Vec<TexItem>>>);

impl<'a> Names<'a> {
    #[must_use]
    pub fn of(items: &'a Arc<Vec<TexItem>>) -> Self {
        Self(Some(items))
    }

    #[must_use]
    pub fn none() -> Self {
        Self(None)
    }
}

/// The rows made for one read of the list.
#[derive(Clone)]
struct Rows {
    list: usize,
    len: usize,
    items: Arc<Vec<PickItem>>,
}

/// The pick items of the list, None first; kept for as long as the same list is read.
fn rows_of(ctx: &Context, key: Id, names: Names<'_>) -> Arc<Vec<PickItem>> {
    let (list, len) = names.0.map_or((0, 0), |a| (Arc::as_ptr(a) as usize, a.len()));
    if let Some(r) = ctx.data(|d| d.get_temp::<Rows>(key))
        && r.list == list
        && r.len == len
    {
        return r.items;
    }
    let mut items = Vec::with_capacity(len + 1);
    items.push(PickItem::new("", "None"));
    if let Some(all) = names.0 {
        items.extend(all.iter().map(|t| PickItem::new(t.name.as_str(), gz_ui::Tx::raw(t.name.as_str()))));
    }
    let items = Arc::new(items);
    ctx.data_mut(|d| d.insert_temp(key, Rows { list, len, items: items.clone() }));
    items
}

/// The name a pick's popup is asked for by (`library.view show=`).
fn request(key: &str) -> String {
    format!("library.{key}")
}

/// Asks for the pick `key` (`detail`, `bump`, `normal`) to be put up on the next frame that draws it.
pub fn ask(ctx: &Context, key: &str) {
    gz_ui::menu::show_popup(ctx, &request(key));
}

/// Whether the pick `key` is up; a request nobody took (the description has no such row) is
/// taken back.
pub fn is_up(ctx: &Context, key: &str) -> bool {
    let _ = gz_ui::menu::take_popup_request(ctx, &request(key));
    Popup::is_id_open(ctx, Combo::new(("gz-ui-previews.pick", key), "").popup_id())
}

/// A texture of the library picked from its list, or none. `key` tells the description's picks
/// apart (`detail`, `bump`, `normal`: two of them are labelled Texture). Returns the name picked
/// (empty for none) when the author picked.
pub fn texture_pick(ui: &mut Ui, key: &str, value: &str, names: Names<'_>) -> Option<String> {
    let ctx = ui.ctx().clone();
    let pick_id = ("gz-ui-previews.pick", key);
    let shown = if value.is_empty() {
        gz_ui::Tx::from("None")
    } else {
        gz_ui::Tx::raw(gz_ui::grid::leaf_name(value))
    };
    let combo = Combo::new(pick_id, shown).placeholder(value.is_empty()).tip(gz_ui::Tx::raw(value));
    let popup = combo.popup_id();
    let open = Popup::is_id_open(&ctx, popup);
    let resp = combo.field(ui, open);
    // put up from outside to be looked at
    let asked = gz_ui::menu::take_popup_request(&ctx, &request(key));
    if resp.clicked() || (asked && !open) {
        if open {
            Popup::close_id(&ctx, popup);
        } else {
            // a list that opens starts from an empty search, on what is chosen
            PickList::reset(&ctx, pick_id);
            Popup::open_id(&ctx, popup);
        }
    }
    if !Popup::is_id_open(&ctx, popup) {
        return None;
    }
    let items = rows_of(&ctx, popup.with("rows"), names);
    let list = PickList::new(pick_id, &items)
        .current(value)
        .hint("Search")
        .empty("No textures match.")
        .width(resp.rect.width().max(em(18.0)))
        .max_height(POPUP_H);
    pick_popup(&ctx, popup, resp.rect, ui.layer_id(), list).and_then(|n| items.get(n)).map(|i| i.id.clone())
}
