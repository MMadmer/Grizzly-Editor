//! Plays Instead: the picker of a copyrighted sound's replacement (docs/UI_DESIGN.md, "Sound
//! Preview"). The kit's combo field opens a list with the kit's search and ranking; every row
//! plays its sound with the button at its start and picks it with a click on its name. The rows
//! are the module's sound assets without the mark and the linked game's sounds its own list does
//! not name (`gz_media::sound::free_sounds`): a copyrighted sound is never offered.
//!
//! The order for a query is worked out when the query changes, never per frame, and only the
//! rows in view are laid out: a list of twenty thousand game sounds holds no frame.

use egui::{
    Align2, Context, CornerRadius, Id, Popup, PopupAnchor, PopupCloseBehavior, PopupKind, Rect, ScrollArea,
    Sense, Ui, UiBuilder, Vec2, pos2,
};
use gz_media::sound::FreeSound;
use gz_ui::text::{self, Clip};
use gz_ui::{Combo, Font, Icon, SearchField, Tx, icon_button_sized, icons, metrics, palette};

/// pt.
const ROW_H: f32 = 26.0;
const LIST_W: f32 = 440.0;
const LIST_H: f32 = 320.0;

/// What the list says before a replacement is picked, and its first row.
pub const NOTHING: &str = "Nothing: it is left out";

/// What the list keeps between frames.
#[derive(Clone, Default)]
struct State {
    query: String,
    order: Vec<u32>,
    /// The query and the number of rows the order was made for.
    made_for: Option<(String, usize)>,
}

/// What the author asked of the picker this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asked {
    /// The row picked: its index in the rows.
    pub picked: Option<usize>,
    /// A row whose sound is to be played.
    pub play: Option<usize>,
    /// Nothing was picked: the sound is left out in the streamer mode.
    pub clear: bool,
}

/// Where a row comes from, in words the list shows on its right.
#[must_use]
pub fn from_label(f: &FreeSound) -> &'static str {
    if f.asset.is_some() { "Sound Asset" } else { "Game Sound" }
}

fn combo<'a>(id_salt: &str, shown: Tx<'a>) -> Combo<'a> {
    Combo::new(("gz-ui-previews.streamer", id_salt.to_owned()), shown)
}

/// Takes the list of the picker `id_salt` down.
pub fn close(ctx: &Context, id_salt: &str) {
    Popup::close_id(ctx, combo(id_salt, Tx::EMPTY).popup_id());
}

/// Whether the list of the picker `id_salt` is up.
#[must_use]
pub fn is_up(ctx: &Context, id_salt: &str) -> bool {
    Popup::is_id_open(ctx, combo(id_salt, Tx::EMPTY).popup_id())
}

/// The field and, while it is open, the list. `open` puts the list up from outside
/// (`preview.sound show=streamer`).
pub fn picker(ui: &mut Ui, id_salt: &str, current: &str, rows: &[FreeSound], open: bool) -> Asked {
    let ctx = ui.ctx().clone();
    let shown: Tx<'_> = if current.is_empty() { Tx::from(NOTHING) } else { Tx::raw(current) };
    let field = combo(id_salt, shown)
        .placeholder(current.is_empty())
        .tip("A sound that is not copyrighted itself: it plays while the streamer mode is on.");
    let popup_id = field.popup_id();
    let state_id = Id::unique(("gz-ui-previews.streamer_list", id_salt));
    let is_open = Popup::is_id_open(&ctx, popup_id);
    let resp = field.field(ui, is_open);
    if (resp.clicked() && !is_open) || (open && !is_open) {
        ctx.data_mut(|d| d.remove::<State>(state_id));
        Popup::open_id(&ctx, popup_id);
    } else if resp.clicked() {
        Popup::close_id(&ctx, popup_id);
    }
    let mut asked = Asked::default();
    let shown = Popup::new(popup_id, ctx.clone(), PopupAnchor::ParentRect(resp.rect), ui.layer_id())
        .kind(PopupKind::Popup)
        .open_memory(None)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .gap(2.0)
        .frame(gz_ui::menu::popup_frame(&ctx))
        .show(|ui| list(ui, &ctx, state_id, current, rows, &mut asked));
    if shown.is_some() && (asked.picked.is_some() || asked.clear) {
        Popup::close_id(&ctx, popup_id);
    }
    asked
}

fn list(ui: &mut Ui, ctx: &Context, state_id: Id, current: &str, rows: &[FreeSound], asked: &mut Asked) {
    let p = palette(ctx);
    let width = LIST_W.max(ui.available_width());
    ui.set_width(width);
    let mut st = ctx.data(|d| d.get_temp::<State>(state_id)).unwrap_or_default();
    SearchField::new(&mut st.query).id_salt("gz-ui-previews.streamer_search").width(width).show(ui);
    if st.made_for.as_ref() != Some(&(st.query.clone(), rows.len())) {
        let order = gz_ui::rank::rank(rows.len(), &st.query, |i| {
            let r = rows.get(i).map_or("", |r| r.name.as_str());
            (r.rsplit('\\').next().unwrap_or(r), r, "")
        });
        st.order = order.into_iter().filter_map(|i| u32::try_from(i).ok()).collect();
        st.made_for = Some((st.query.clone(), rows.len()));
    }
    ui.add_space(4.0);
    // the replacement taken away: the first row, while nothing is searched for
    if st.query.trim().is_empty() {
        let (at, resp) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
        if current.is_empty() {
            ui.painter().rect_filled(at, CornerRadius::same(3), p.selection);
        } else if resp.hovered() {
            ui.painter().rect_filled(at, CornerRadius::same(3), p.hover);
        }
        icons::draw(
            ui.painter(),
            Icon::Clear,
            Rect::from_min_size(pos2(at.min.x + 7.0, at.center().y - 7.0), Vec2::splat(14.0)),
            p.text_secondary,
        );
        text::paint(
            ui.painter(),
            &Font::Body.id(),
            pos2(at.min.x + ROW_H + 26.0, at.center().y),
            Align2::LEFT_CENTER,
            p.text_secondary,
            NOTHING,
            None,
            Clip::End,
        );
        if resp.clicked() {
            asked.clear = true;
        }
    }
    if st.order.is_empty() {
        let text = if rows.is_empty() {
            Tx::from("No sound without the mark: add a sound asset that is not copyrighted.")
        } else {
            Tx::from("Nothing matches")
        };
        text::status(ui, gz_ui::Tone::Muted, text);
    }
    let order = st.order.clone();
    ScrollArea::vertical()
        .id_salt(state_id.with("rows"))
        .max_height(LIST_H)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_H, order.len(), |ui, range| {
            for k in range {
                let Some(i) = order.get(k).and_then(|&i| usize::try_from(i).ok()) else { continue };
                let Some(r) = rows.get(i) else { continue };
                row(ui, i, r, r.name.eq_ignore_ascii_case(current), &p, asked);
            }
        });
    ctx.data_mut(|d| d.insert_temp(state_id, st));
}

fn row(ui: &mut Ui, i: usize, r: &FreeSound, chosen: bool, p: &gz_ui::Palette, asked: &mut Asked) {
    let w = ui.available_width();
    let (at, resp) = ui.allocate_exact_size(Vec2::new(w, ROW_H), Sense::click());
    if chosen {
        ui.painter().rect_filled(at, CornerRadius::same(3), p.selection);
    } else if resp.hovered() {
        ui.painter().rect_filled(at, CornerRadius::same(3), p.hover);
    }
    let b = Rect::from_min_size(pos2(at.min.x + 2.0, at.min.y + 2.0), Vec2::splat(ROW_H - 4.0));
    let mut child = ui.new_child(UiBuilder::new().id_salt(("gz-ui-previews.streamer_play", i)).max_rect(b));
    if icon_button_sized(&mut child, Icon::Play, "Play it", false, true, ROW_H - 4.0, metrics::GLYPH_ROW)
        .clicked()
    {
        asked.play = Some(i);
    }
    let icon = if r.asset.is_some() { Icon::SoundAsset } else { Icon::SoundSource };
    let cy = at.center().y;
    icons::draw(
        ui.painter(),
        icon,
        Rect::from_min_size(pos2(b.max.x + 6.0, cy - 7.0), Vec2::splat(14.0)),
        p.text_secondary,
    );
    let font = Font::Small.id();
    let from = from_label(r);
    let from_w = text::width(ui.ctx(), &font, Tx::from(from)) + 8.0;
    let name_x = b.max.x + 26.0;
    text::paint(
        ui.painter(),
        &Font::Body.id(),
        pos2(name_x, cy),
        Align2::LEFT_CENTER,
        p.text,
        Tx::raw(r.name.as_str()),
        Some((at.max.x - from_w - name_x - 6.0).max(0.0)),
        Clip::Start,
    );
    text::paint(
        ui.painter(),
        &font,
        pos2(at.max.x - 6.0, cy),
        Align2::RIGHT_CENTER,
        p.text_muted,
        from,
        None,
        Clip::End,
    );
    if resp.clicked() {
        asked.picked = Some(i);
    }
    resp.on_hover_text(r.name.as_str()); // ui-text: a sound's game name is data
}
