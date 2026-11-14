//! [`PickList`] and [`Combo`] - choosing a value from a list (docs/UI_DESIGN.md, section 4).
//!
//! - **`PickList`** is the popup list every picker uses: a search ranked like the palette
//!   ([`crate::rank`]: word prefixes first, then letters in order), virtualized rows (only the rows
//!   on screen are laid out), an icon and a tint per row, a second muted line on the highlighted
//!   row, and - with a memory key - the sections Recent and Favourites above the rest, a star per
//!   row to add or take a favourite. Full keyboard use: type to search, Up / Down / Page Up /
//!   Page Down / Home / End move, Enter picks, Esc closes. The order is worked out when the query
//!   changes, never per frame.
//! - **`Combo`** is a value with the kit's chevron that opens a `PickList` under it. It is the
//!   one dropdown of the editor: nothing uses egui's own combo box.

use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};

use egui::{
    Align2, Color32, Context, CornerRadius, Id, Key, Margin, Modifiers, Popup, PopupAnchor,
    PopupCloseBehavior, PopupKind, Rect, Response, Role, ScrollArea, Sense, Stroke, StrokeKind, TextEdit, Ui,
    Vec2, WidgetInfo, pos2,
};
use rustc_hash::FxHasher;

use crate::audit;
use crate::font::Font;
use crate::icons::{self, Icon};
use crate::layout;
use crate::menu::popup_frame;
use crate::rank;
use crate::text::{self, Clip};
use crate::theme::{metrics, palette};
use crate::units::em;

/// One row of a pick list.
#[derive(Clone, Debug, PartialEq)]
pub struct PickItem {
    /// What the list remembers it by (recent, favourites) and the caller picks it by.
    pub id: String,
    /// What the row shows.
    pub label: String,
    /// The second line of the highlighted row, muted; searched like a path is.
    pub detail: String,
    pub icon: Icon,
    /// The icon's colour (a family, a tone); the text colour without one.
    pub tint: Option<Color32>,
    /// Off rows are shown and never picked.
    pub enabled: bool,
}

impl Default for PickItem {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            detail: String::new(),
            icon: Icon::None,
            tint: None,
            enabled: true,
        }
    }
}

impl PickItem {
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self { id: id.into(), label: label.into(), ..Self::default() }
    }

    #[must_use]
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }

    #[must_use]
    pub fn tint(mut self, tint: Color32) -> Self {
        self.tint = Some(tint);
        self
    }

    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// The height of a row, and the highlighted row's second line.
const ROW_H: f32 = 26.0;
const DETAIL_H: f32 = 16.0;
const HEAD_H: f32 = 24.0;
/// How many recent picks a list keeps.
const RECENT_KEEP: usize = 5;

/// A line of the list: a section's heading or an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Head(Heading),
    Item(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Heading {
    Recent,
    Favourites,
    All,
}

impl Heading {
    fn title(self) -> &'static str {
        match self {
            Heading::Recent => "Recent",
            Heading::Favourites => "Favourites",
            Heading::All => "All",
        }
    }
}

/// What a list keeps between frames.
#[derive(Clone, Debug, Default)]
struct State {
    query: String,
    /// The line under the keyboard.
    hi: usize,
    lines: Vec<Line>,
    /// What the lines were worked out for.
    made_for: u64,
    /// The search has been given the focus once.
    focused: bool,
    /// Scroll the highlighted line into view on the next frame.
    reveal: bool,
}

fn key_recent(memory: &str) -> String {
    format!("pick.{memory}.recent")
}

fn key_favourites(memory: &str) -> String {
    format!("pick.{memory}.favourites")
}

/// The favourites a list of this memory key keeps.
#[must_use]
pub fn favourites(ctx: &Context, memory: &str) -> Vec<String> {
    layout::list(ctx, &key_favourites(memory))
}

/// Adds or takes away a favourite, as a row's star does.
pub fn set_favourite(ctx: &Context, memory: &str, id: &str, on: bool) {
    let mut f = favourites(ctx, memory);
    f.retain(|x| x != id);
    if on {
        f.push(id.to_owned());
    }
    layout::set_list(ctx, &key_favourites(memory), f);
}

/// What a pick list answered this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picked {
    /// The index of the item picked (a click, or Enter).
    pub item: Option<usize>,
    /// Esc, or nothing to pick: the popup holding it should close.
    pub close: bool,
}

/// A searchable list. Draw it in a popup ([`pick_popup`]) or in a panel of its own.
#[must_use = "a pick list does nothing until it is shown"]
pub struct PickList<'a> {
    id: Id,
    items: &'a [PickItem],
    current: Option<&'a str>,
    memory: Option<&'a str>,
    hint: &'a str,
    width: f32,
    max_h: f32,
    search: bool,
    empty: Option<&'a str>,
}

fn fingerprint(items: &[PickItem], query: &str, recent: &[String], favs: &[String]) -> u64 {
    let mut h: FxHasher = BuildHasherDefault::<FxHasher>::default().build_hasher();
    items.len().hash(&mut h);
    if let (Some(a), Some(b)) = (items.first(), items.last()) {
        a.id.hash(&mut h);
        b.id.hash(&mut h);
        if let Some(m) = items.get(items.len() / 2) {
            m.id.hash(&mut h);
        }
    }
    query.hash(&mut h);
    recent.hash(&mut h);
    favs.hash(&mut h);
    h.finish()
}

/// The lines for a query: the ranked matches; without a query the Favourites and Recent
/// sections first (with a memory key), then every item.
fn lines_for(items: &[PickItem], query: &str, recent: &[String], favs: &[String]) -> Vec<Line> {
    let matched = rank::rank(items.len(), query, |i| {
        let it = &items[i];
        (it.label.as_str(), it.detail.as_str(), "")
    });
    if !query.trim().is_empty() || (recent.is_empty() && favs.is_empty()) {
        return matched.into_iter().map(Line::Item).collect();
    }
    let index_of = |id: &String| items.iter().position(|it| &it.id == id);
    let mut out = Vec::with_capacity(items.len() + 3);
    let f: Vec<usize> = favs.iter().filter_map(index_of).collect();
    if !f.is_empty() {
        out.push(Line::Head(Heading::Favourites));
        out.extend(f.into_iter().map(Line::Item));
    }
    let r: Vec<usize> = recent.iter().filter_map(index_of).collect();
    if !r.is_empty() {
        out.push(Line::Head(Heading::Recent));
        out.extend(r.into_iter().map(Line::Item));
    }
    out.push(Line::Head(Heading::All));
    out.extend(matched.into_iter().map(Line::Item));
    out
}

impl<'a> PickList<'a> {
    pub fn new(id_salt: impl Hash + std::fmt::Debug, items: &'a [PickItem]) -> Self {
        Self {
            id: Id::unique(("gz-ui.pick", id_salt)),
            items,
            current: None,
            memory: None,
            hint: "Search",
            width: em(18.0),
            max_h: 360.0,
            search: true,
            empty: None,
        }
    }

    /// The item now chosen: it is marked, and highlighted when the list opens.
    pub fn current(mut self, id: &'a str) -> Self {
        self.current = Some(id);
        self
    }

    /// Keeps the Recent and Favourites sections under this key (in the layout store).
    pub fn remember(mut self, key: &'a str) -> Self {
        self.memory = Some(key);
        self
    }

    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = hint;
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// The most the list's rows take; it is shorter with fewer rows.
    pub fn max_height(mut self, h: f32) -> Self {
        self.max_h = h;
        self
    }

    /// No search field: a few fixed choices.
    pub fn without_search(mut self) -> Self {
        self.search = false;
        self
    }

    /// What the list says when no row is left: a sentence, wrapped to the list's width.
    pub fn empty(mut self, text: &'a str) -> Self {
        self.empty = Some(text);
        self
    }

    /// What is typed in the search now, as of the last frame the list was shown: a caller that
    /// offers a row for what was typed reads it here.
    #[must_use]
    pub fn query(ctx: &Context, id_salt: impl Hash + std::fmt::Debug) -> String {
        let id = Id::unique(("gz-ui.pick", id_salt));
        ctx.data(|d| d.get_temp::<State>(id)).map(|s| s.query).unwrap_or_default()
    }

    /// Forgets the query and the highlight: the list opens fresh next time.
    pub fn reset(ctx: &Context, id_salt: impl Hash + std::fmt::Debug) {
        let id = Id::unique(("gz-ui.pick", id_salt));
        ctx.data_mut(|d| d.remove::<State>(id));
    }

    /// Draws the search and the rows. Returns what was picked.
    pub fn show(self, ui: &mut Ui) -> Picked {
        // the search over the rows, whatever layout the caller is in
        ui.vertical(|ui| self.show_rows(ui)).inner
    }

    fn show_rows(self, ui: &mut Ui) -> Picked {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let mut st = ctx.data(|d| d.get_temp::<State>(self.id)).unwrap_or_default();
        let (recent, favs) = match self.memory {
            Some(m) => (layout::list(&ctx, &key_recent(m)), favourites(&ctx, m)),
            None => (Vec::new(), Vec::new()),
        };
        let fresh = st.lines.is_empty() && st.made_for == 0;
        let mut out = Picked::default();

        // keys before the field takes them
        let (up, down, page_up, page_down, home, end, enter, esc) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::PageUp),
                i.consume_key(Modifiers::NONE, Key::PageDown),
                i.consume_key(Modifiers::CTRL, Key::Home),
                i.consume_key(Modifiers::CTRL, Key::End),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
            )
        });

        if self.search {
            let edit_id = self.id.with("search");
            if !st.focused {
                ctx.memory_mut(|m| m.request_focus(edit_id));
                st.focused = true;
            }
            let w = self.width;
            let (r, _) = ui.allocate_exact_size(Vec2::new(w, metrics::FRAME), Sense::hover());
            ui.painter().rect(
                r,
                CornerRadius::same(metrics::RADIUS),
                p.input,
                Stroke::new(1.0, p.border),
                StrokeKind::Inside,
            );
            icons::draw(
                ui.painter(),
                Icon::Search,
                Rect::from_min_size(pos2(r.min.x + 8.0, r.center().y - 7.0), Vec2::splat(14.0)),
                p.text_muted,
            );
            let field = Rect::from_min_max(pos2(r.min.x + 26.0, r.min.y), r.max);
            ui.put(
                field,
                TextEdit::singleline(&mut st.query)
                    .id(edit_id)
                    .frame(egui::Frame::NONE)
                    .margin(Margin::symmetric(4, 4))
                    .hint_text(self.hint)
                    .font(Font::Body.id())
                    .desired_width(field.width()),
            );
            ui.add_space(4.0);
        }

        let made_for = fingerprint(self.items, &st.query, &recent, &favs);
        if made_for != st.made_for {
            st.lines = lines_for(self.items, &st.query, &recent, &favs);
            st.made_for = made_for;
            // a list that opens highlights what is chosen; a new query, its best match
            let want = if fresh { self.current } else { None }.and_then(|c| {
                st.lines.iter().position(|l| matches!(l, Line::Item(i) if self.items[*i].id == c))
            });
            st.hi = want.unwrap_or_else(|| first_item(&st.lines, 0, 1).unwrap_or(0));
            st.reveal = true;
        }

        // the keyboard moves over the items, never over a heading
        let step = |st: &mut State, from: usize, dir: isize, n: usize| {
            let mut at = from;
            for _ in 0..n {
                match first_item(&st.lines, at as isize + dir, dir) {
                    Some(next) => at = next,
                    None => break,
                }
            }
            st.hi = at;
            st.reveal = true;
        };
        if down {
            let at = st.hi;
            step(&mut st, at, 1, 1);
        }
        if up {
            let at = st.hi;
            step(&mut st, at, -1, 1);
        }
        if page_down {
            let at = st.hi;
            step(&mut st, at, 1, 8);
        }
        if page_up {
            let at = st.hi;
            step(&mut st, at, -1, 8);
        }
        if home {
            st.hi = first_item(&st.lines, 0, 1).unwrap_or(0);
            st.reveal = true;
        }
        if end {
            st.hi = first_item(&st.lines, st.lines.len() as isize - 1, -1).unwrap_or(0);
            st.reveal = true;
        }
        if esc {
            out.close = true;
        }
        if enter
            && let Some(Line::Item(i)) = st.lines.get(st.hi)
            && self.items.get(*i).is_some_and(|it| it.enabled)
        {
            out.item = Some(*i);
        }

        // the rows: heights known up front, only those in view laid out
        let hi_detail =
            matches!(st.lines.get(st.hi), Some(Line::Item(i)) if !self.items[*i].detail.is_empty());
        let hi_now = st.hi;
        let height_of = move |n: usize, line: &Line| match line {
            Line::Head(_) => HEAD_H,
            Line::Item(_) if n == hi_now && hi_detail => ROW_H + DETAIL_H,
            Line::Item(_) => ROW_H,
        };
        let total: f32 = st.lines.iter().enumerate().map(|(n, l)| height_of(n, l)).sum();
        if st.lines.is_empty() {
            let said =
                self.empty.unwrap_or(if self.items.is_empty() { "Nothing to pick" } else { "No matches" });
            let galley =
                ui.painter().layout(said.to_owned(), Font::Body.id(), p.text_muted, self.width - 16.0);
            let (r, _) = ui.allocate_exact_size(
                Vec2::new(self.width, (galley.size().y + 16.0).max(ROW_H * 2.0)),
                Sense::hover(),
            );
            let at = r.center() - galley.size() * 0.5;
            ui.painter().galley(at, galley, p.text_muted);
        } else {
            let reveal = std::mem::take(&mut st.reveal);
            let hi = st.hi;
            let mut hi_top = 0.0;
            for (n, l) in st.lines.iter().enumerate() {
                if n == hi {
                    break;
                }
                hi_top += height_of(n, l);
            }
            let hi_h = st.lines.get(hi).map_or(ROW_H, |l| height_of(hi, l));
            let favs_now = favs.clone();
            let mut star = None;
            // the rows take their own height, up to the most the list may take, wherever the
            // caller lays it out (a row of a form gives a child no height of its own)
            let list_h = self.max_h.min(total);
            ui.allocate_ui(Vec2::new(self.width, list_h), |ui| {
                ScrollArea::vertical()
                    .id_salt(self.id.with("rows"))
                    .max_height(list_h)
                    .min_scrolled_height(list_h)
                    .auto_shrink([false, true])
                    .show_viewport(ui, |ui, view| {
                        ui.set_min_size(Vec2::new(self.width, total));
                        let origin = ui.min_rect().min;
                        if reveal {
                            let r = Rect::from_min_size(
                                pos2(origin.x, origin.y + hi_top),
                                Vec2::new(self.width, hi_h),
                            );
                            ui.scroll_to_rect(r, None);
                        }
                        let mut y = 0.0;
                        for (n, line) in st.lines.iter().enumerate() {
                            let h = height_of(n, line);
                            if y + h < view.min.y {
                                y += h;
                                continue;
                            }
                            if y > view.max.y {
                                break;
                            }
                            let rect =
                                Rect::from_min_size(pos2(origin.x, origin.y + y), Vec2::new(self.width, h));
                            y += h;
                            match line {
                                Line::Head(head) => {
                                    ui.painter().text(
                                        pos2(rect.min.x + 8.0, rect.center().y + 2.0),
                                        Align2::LEFT_CENTER,
                                        head.title(),
                                        Font::Small.id(),
                                        p.text_muted,
                                    );
                                }
                                Line::Item(i) => {
                                    let Some(it) = self.items.get(*i) else { continue };
                                    let fav = favs_now.iter().any(|f| f == &it.id);
                                    let row = draw_row(
                                        ui,
                                        rect,
                                        self.id.with(("row", n)),
                                        it,
                                        n == hi,
                                        self.current == Some(it.id.as_str()),
                                        self.memory.map(|_| fav),
                                    );
                                    if row.star {
                                        star = Some((it.id.clone(), !fav));
                                    } else if row.clicked && it.enabled {
                                        out.item = Some(*i);
                                    } else if row.hovered && !reveal {
                                        st.hi = n;
                                    }
                                }
                            }
                        }
                    });
            });
            if let (Some(m), Some((id, on))) = (self.memory, star) {
                set_favourite(&ctx, m, &id, on);
            }
        }
        if let (Some(i), Some(m)) = (out.item, self.memory)
            && let Some(it) = self.items.get(i)
        {
            layout::push_recent(&ctx, &key_recent(m), &it.id, RECENT_KEEP);
        }
        if out.item.is_some() || out.close {
            // opened again, the list starts fresh
            ctx.data_mut(|d| d.remove::<State>(self.id));
        } else {
            ctx.data_mut(|d| d.insert_temp(self.id, st));
        }
        out
    }
}

/// The first item line from `from` on in `dir`.
fn first_item(lines: &[Line], from: isize, dir: isize) -> Option<usize> {
    let mut at = from;
    while at >= 0 && (at as usize) < lines.len() {
        if matches!(lines.get(at as usize), Some(Line::Item(_))) {
            return Some(at as usize);
        }
        at += dir;
    }
    None
}

struct RowOut {
    clicked: bool,
    hovered: bool,
    star: bool,
}

fn draw_row(
    ui: &mut Ui,
    rect: Rect,
    id: Id,
    it: &PickItem,
    hi: bool,
    current: bool,
    fav: Option<bool>,
) -> RowOut {
    let p = palette(ui.ctx());
    let resp = ui.interact(rect, id, if it.enabled { Sense::click() } else { Sense::hover() });
    let star_rect =
        Rect::from_min_size(pos2(rect.max.x - 26.0, rect.min.y + (ROW_H - 22.0) * 0.5), Vec2::splat(22.0));
    let star_resp = fav.map(|_| ui.interact(star_rect, id.with("star"), Sense::click()));
    let painter = ui.painter();
    if hi {
        painter.rect_filled(
            rect.shrink2(Vec2::new(2.0, 1.0)),
            CornerRadius::same(metrics::RADIUS),
            p.selection_focused,
        );
    } else if resp.hovered() {
        painter.rect_filled(rect.shrink2(Vec2::new(2.0, 1.0)), CornerRadius::same(metrics::RADIUS), p.hover);
    }
    let cy = rect.min.y + ROW_H * 0.5;
    let mut x = rect.min.x + 8.0;
    if it.icon != Icon::None {
        let color = if !it.enabled { p.text_disabled } else { it.tint.unwrap_or(p.text_secondary) };
        icons::draw(painter, it.icon, Rect::from_min_size(pos2(x, cy - 8.0), Vec2::splat(16.0)), color);
        x += 24.0;
    }
    let right = rect.max.x - if fav.is_some() { 30.0 } else { 8.0 } - if current { 20.0 } else { 0.0 };
    text::paint(
        painter,
        &Font::Body.id(),
        pos2(x, cy),
        Align2::LEFT_CENTER,
        if it.enabled { p.text } else { p.text_disabled },
        &it.label,
        Some((right - x).max(0.0)),
        Clip::End,
    );
    if current {
        icons::draw(
            painter,
            Icon::Check,
            Rect::from_min_size(pos2(right + 4.0, cy - 7.0), Vec2::splat(14.0)),
            p.accent_bright,
        );
    }
    if hi && !it.detail.is_empty() {
        text::paint(
            painter,
            &Font::Small.id(),
            pos2(x, rect.min.y + ROW_H + DETAIL_H * 0.5 - 4.0),
            Align2::LEFT_CENTER,
            p.text_muted,
            &it.detail,
            Some((rect.max.x - 8.0 - x).max(0.0)),
            Clip::Start,
        );
    }
    let mut star = false;
    if let (Some(on), Some(sr)) = (fav, star_resp) {
        if on || resp.hovered() || sr.hovered() || hi {
            let color = if on {
                p.warning
            } else if sr.hovered() {
                p.text
            } else {
                p.text_muted
            };
            icons::draw(
                painter,
                if on { Icon::StarFilled } else { Icon::Star },
                Rect::from_center_size(star_rect.center(), Vec2::splat(14.0)),
                color,
            );
        }
        star = sr.clicked();
        let _ =
            sr.on_hover_text(if on { "Take it out of the favourites" } else { "Add it to the favourites" });
    }
    resp.widget_info(|| WidgetInfo::selected(Role::ListItem, it.enabled, current, &it.label));
    audit::control(ui.ctx(), &it.label, it.enabled, None, None, rect);
    RowOut { clicked: resp.clicked(), hovered: resp.hovered(), star }
}

/// A pick list in a popup under `anchor`, open while egui's memory says so (`Popup::open_id`).
/// Returns the index picked; the popup closes on a pick, Esc and a click outside.
pub fn pick_popup(
    ctx: &Context,
    popup_id: Id,
    anchor: Rect,
    layer: egui::LayerId,
    list: PickList<'_>,
) -> Option<usize> {
    let mut picked = None;
    let mut close = false;
    let shown = Popup::new(popup_id, ctx.clone(), PopupAnchor::ParentRect(anchor), layer)
        .kind(PopupKind::Popup)
        .open_memory(None)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .gap(2.0)
        .frame(popup_frame(ctx))
        .show(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
            let out = list.show(ui);
            picked = out.item;
            close = out.close || out.item.is_some();
        });
    if shown.is_some() && close {
        Popup::close_id(ctx, popup_id);
    }
    picked
}

/// A value with the kit's chevron that opens a [`PickList`] under it.
#[must_use = "a combo does nothing until it is shown"]
pub struct Combo<'a> {
    id_salt: Id,
    shown: &'a str,
    icon: Icon,
    width: Option<f32>,
    enabled: bool,
    why: &'a str,
    mixed: bool,
    error: bool,
    warn: bool,
    placeholder: bool,
    busy: bool,
    tip: &'a str,
}

impl<'a> Combo<'a> {
    /// A combo showing `shown` (the chosen value's name).
    pub fn new(id_salt: impl Hash + std::fmt::Debug, shown: &'a str) -> Self {
        Self {
            id_salt: Id::unique(("gz-ui.combo", id_salt)),
            shown,
            icon: Icon::None,
            width: None,
            enabled: true,
            why: "",
            mixed: false,
            error: false,
            warn: false,
            placeholder: false,
            busy: false,
            tip: "",
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = icon;
        self
    }

    /// Its width; left out, what is left of the row.
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }

    pub fn enabled(mut self, enabled: bool, why: &'a str) -> Self {
        self.enabled = enabled;
        self.why = why;
        self
    }

    /// The values of a selection differ: it says so.
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    /// The value is wrong: the error outline.
    pub fn error(mut self, error: bool) -> Self {
        self.error = error;
        self
    }

    /// The value is one the list does not hold, kept as it was written: the warning tone.
    pub fn warn(mut self, warn: bool) -> Self {
        self.warn = warn;
        self
    }

    /// Nothing is chosen: what is shown is the words for that, in the secondary tone.
    pub fn placeholder(mut self, placeholder: bool) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// The list is still being read: a spinner where the chevron is, and no press; `why` says
    /// what is read.
    pub fn busy(mut self, busy: bool, why: &'a str) -> Self {
        self.busy = busy;
        if busy {
            self.enabled = false;
            self.why = why;
        }
        self
    }

    /// What hovering the value says (a value cut short says itself).
    pub fn tip(mut self, tip: &'a str) -> Self {
        self.tip = tip;
        self
    }

    /// The id the combo's popup lives under (`Popup::open_id` opens it from outside).
    #[must_use]
    pub fn popup_id(&self) -> Id {
        self.id_salt.with("popup")
    }

    /// Draws the field and, while open, `list`. Returns the index picked.
    pub fn show(self, ui: &mut Ui, list: PickList<'_>) -> Option<usize> {
        let ctx = ui.ctx().clone();
        let popup_id = self.popup_id();
        let open = Popup::is_id_open(&ctx, popup_id);
        let resp = self.field(ui, open);
        if resp.clicked() && self.enabled {
            if open {
                Popup::close_id(&ctx, popup_id);
            } else {
                // a list that opens starts from an empty search, on what is chosen
                ctx.data_mut(|d| d.remove::<State>(list.id));
                Popup::open_id(&ctx, popup_id);
            }
        }
        let width = list.width.max(resp.rect.width());
        pick_popup(&ctx, popup_id, resp.rect, ui.layer_id(), list.width(width))
    }

    /// The field alone: an input fill, the 1 pt outline, the value, the chevron. For a caller
    /// whose popup is not a pick list (ticks over names): it opens its own under
    /// [`Combo::popup_id`].
    pub fn field(&self, ui: &mut Ui, open: bool) -> Response {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let w = self.width.unwrap_or_else(|| ui.available_width()).max(em(4.0));
        let (rect, resp) = ui.allocate_exact_size(
            Vec2::new(w, metrics::FRAME),
            if self.enabled { Sense::click() } else { Sense::hover() },
        );
        resp.widget_info(|| WidgetInfo::labeled(Role::ComboBox, self.enabled, self.shown));
        let mut cut_value = false;
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let hot = self.enabled && (resp.hovered() || open);
            let outline = if self.error {
                p.error
            } else if open || resp.has_focus() {
                p.accent_bright
            } else if hot {
                p.border_strong
            } else {
                p.border
            };
            painter.rect(
                rect,
                CornerRadius::same(metrics::RADIUS),
                if !self.enabled {
                    p.input
                } else if hot {
                    p.input_hover
                } else {
                    p.input
                },
                Stroke::new(if open || resp.has_focus() { metrics::FOCUS_RING * 0.5 } else { 1.0 }, outline),
                StrokeKind::Inside,
            );
            let mut x = rect.min.x + 8.0;
            if self.icon != Icon::None {
                icons::draw(
                    painter,
                    self.icon,
                    Rect::from_min_size(pos2(x, rect.center().y - 8.0), Vec2::splat(16.0)),
                    if self.enabled { p.text_secondary } else { p.text_disabled },
                );
                x += 22.0;
            }
            let chevron = Rect::from_min_size(
                pos2(rect.max.x - 8.0 - metrics::GLYPH_STATUS, rect.center().y - metrics::GLYPH_STATUS * 0.5),
                Vec2::splat(metrics::GLYPH_STATUS),
            );
            let (shown, color) = if self.mixed {
                ("Multiple values", p.text_muted)
            } else {
                let color = if !self.enabled && !self.busy {
                    p.text_disabled
                } else if self.warn {
                    p.warning
                } else if self.placeholder {
                    p.text_secondary
                } else {
                    p.text
                };
                (self.shown, color)
            };
            let cut = text::paint(
                painter,
                &Font::Body.id(),
                pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                color,
                shown,
                Some((chevron.min.x - 6.0 - x).max(0.0)),
                Clip::End,
            );
            if self.busy {
                crate::state::spinner_at(&ctx, painter, chevron);
            } else {
                icons::draw(
                    painter,
                    Icon::ChevronDown,
                    chevron,
                    if self.enabled { p.text_secondary } else { p.text_disabled },
                );
            }
            if cut {
                audit::truncated(&ctx, shown, shown, rect);
                cut_value = true;
            }
        }
        let why = (!self.enabled && !self.why.is_empty()).then_some(self.why);
        audit::control(&ctx, self.shown, self.enabled, why, why, rect);
        match why {
            Some(w) => resp.on_hover_text(w),
            None if !self.tip.is_empty() => resp.on_hover_text(self.tip),
            None if cut_value => resp.on_hover_text(self.shown),
            None => resp,
        }
    }
}

/// One of a few named choices as a combo: the plain case. Returns whether the choice changed.
pub fn combo(
    ui: &mut Ui,
    id_salt: impl Hash + std::fmt::Debug + Copy,
    current: &mut usize,
    options: &[&str],
) -> bool {
    let items: Vec<PickItem> =
        options.iter().enumerate().map(|(i, o)| PickItem::new(i.to_string(), *o)).collect();
    let current_id = current.to_string();
    let shown = options.get(*current).copied().unwrap_or("");
    let mixed = crate::grid::is_mixed(ui);
    let mut list = PickList::new(id_salt, &items).current(&current_id);
    if options.len() <= 8 {
        list = list.without_search();
    }
    match Combo::new(id_salt, shown).mixed(mixed).show(ui, list) {
        Some(i) if i != *current => {
            *current = i;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{Heading, Line, PickItem, first_item, lines_for};

    fn items() -> Vec<PickItem> {
        ["Wait", "Spawn NPC", "Give Item", "App Message", "Spawn Squad"]
            .iter()
            .enumerate()
            .map(|(i, l)| PickItem::new(format!("k{i}"), *l))
            .collect()
    }

    #[test]
    fn a_query_ranks_and_sections_only_without_one() {
        let it = items();
        let recent = vec!["k3".to_owned()];
        let favs = vec!["k2".to_owned()];
        // no query: Favourites, Recent, then every item
        let all = lines_for(&it, "", &recent, &favs);
        assert_eq!(all[0], Line::Head(Heading::Favourites));
        assert_eq!(all[1], Line::Item(2));
        assert_eq!(all[2], Line::Head(Heading::Recent));
        assert_eq!(all[3], Line::Item(3));
        assert_eq!(all[4], Line::Head(Heading::All));
        assert_eq!(all.len(), 5 + 5);
        // a query: the matches alone, a word's start first
        let spawn = lines_for(&it, "spa", &recent, &favs);
        assert_eq!(spawn, vec![Line::Item(1), Line::Item(4)]);
        let letters = lines_for(&it, "sn", &recent, &favs);
        assert_eq!(letters, vec![Line::Item(1)]);
    }

    #[test]
    fn the_keyboard_skips_headings() {
        let lines = vec![Line::Head(Heading::Recent), Line::Item(0), Line::Head(Heading::All), Line::Item(1)];
        assert_eq!(first_item(&lines, 0, 1), Some(1));
        assert_eq!(first_item(&lines, 2, 1), Some(3));
        assert_eq!(first_item(&lines, 2, -1), Some(1));
        assert_eq!(first_item(&lines, 4, 1), None);
    }
}
