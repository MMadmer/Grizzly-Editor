//! The toolbar (docs/UI_DESIGN.md, section 4, "Toolbar").
//!
//! - [`Toolbar::begin(ui, id, style)`](Toolbar::begin) ... [`Toolbar::end`] (or drop) - a row
//!   that never wraps. Too wide, labels collapse to icons first, then items move into the
//!   overflow menu (`»`), rising priority first, rightmost first on a tie. It lays out from the
//!   last frame's measures; a folded label unfolds only with 24 pt to spare, and a row keeps its
//!   layout while it still fits.
//! - Items: [`Toolbar::group`], [`Toolbar::separator`], [`Toolbar::spring`], [`Toolbar::status`],
//!   [`Toolbar::action`], [`Toolbar::split`] (an action with a chevron menu), [`Toolbar::menu`],
//!   [`Toolbar::search`], [`Toolbar::segmented`], [`Toolbar::widget`] (hidden in the overflow).
//! - Styles: [`ToolbarStyle::Row`] (36 pt, buttons 28), `Panel` (32, a line under it),
//!   `Overlay` (each group a pill on the overlay colour at 92%).
//! - [`plan`] - the overflow planning, a pure function of the measured widths.
//! - Popups by name (`menu::show_popup`): `toolbar_overflow:<id>`, `toolbar_split:<id>`, and the
//!   name a [`Toolbar::menu`] is given.

use egui::{
    Align, Align2, Color32, Context, CornerRadius, Id, Layout, Popup, Pos2, Rect, Response, RichText, Role,
    Sense, Stroke, Ui, UiBuilder, Vec2, WidgetInfo, pos2,
};

use crate::audit;
use crate::button::{Kind, icon_button_sized, segmented};
use crate::font::Font;
use crate::icons::{self, Icon};
use crate::menu::{self, MenuItem, menu_item, menu_popup, take_popup_request};
use crate::search::SearchField;
use crate::text::{self, Clip};
use crate::theme::{Palette, Tone, metrics, palette};
use crate::units::{button_width, em};

/// How a toolbar sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolbarStyle {
    /// A tab's toolbar: 36 pt, buttons 28.
    #[default]
    Row,
    /// A panel's toolbar: 32 pt, buttons 24, a line under it.
    Panel,
    /// Over a canvas: 32 pt, buttons 24, each group on a pill.
    Overlay,
}

impl ToolbarStyle {
    #[must_use]
    pub fn height(self) -> f32 {
        if self == ToolbarStyle::Row { metrics::TOOLBAR } else { metrics::PANEL_TOOLBAR }
    }

    #[must_use]
    pub fn button(self) -> f32 {
        if self == ToolbarStyle::Row { metrics::TOOLBAR_BUTTON } else { metrics::ICON_BUTTON }
    }
}

/// What an item of a toolbar is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ItemKind {
    #[default]
    Action,
    Menu,
    Split,
    Search,
    Widget,
    Status,
    Segmented,
    Separator,
    Spring,
}

/// How an item is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Full,
    /// Its label folded: the icon alone.
    Icon,
    /// In the overflow menu.
    Hidden,
}

/// An item as the planning sees it: what it measured last frame and how it was shown.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlanItem {
    pub kind: ItemKind,
    pub group: u32,
    /// Lower gives way first.
    pub priority: i32,
    pub w_full: f32,
    pub w_icon: f32,
    /// Has an icon and a label: it can fold to the icon.
    pub collapsible: bool,
    /// What it was drawn as last frame.
    pub shown: Mode,
}

/// The layout of a row.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub modes: Vec<Mode>,
    /// Room a search field or a spring takes beyond its measure.
    pub extra: Vec<f32>,
    /// Something is in the overflow: the `»` button is shown.
    pub any_hidden: bool,
}

/// A folded label unfolds only with this much room beyond its own width.
pub const UNFOLD_SLACK: f32 = 24.0;

/// The gap between two neighbours.
#[must_use]
pub fn gap(style: ToolbarStyle, a: ItemKind, group_a: u32, b: ItemKind, group_b: u32) -> f32 {
    if a == ItemKind::Separator || b == ItemKind::Separator {
        6.0
    } else if group_a != group_b {
        if style == ToolbarStyle::Overlay { 8.0 } else { metrics::TOOLBAR_GROUP_GAP }
    } else {
        2.0
    }
}

fn is_spacer(kind: ItemKind) -> bool {
    matches!(kind, ItemKind::Separator | ItemKind::Spring)
}

fn visible(item: &PlanItem, mode: Mode) -> bool {
    mode != Mode::Hidden && !is_spacer(item.kind)
}

/// A separator shows only between two shown items, and never twice in a row.
fn resolve_separators(items: &[PlanItem], modes: &mut [Mode]) {
    let mut before = false;
    for i in 0..items.len().min(modes.len()) {
        let (Some(item), Some(mode)) = (items.get(i), modes.get(i).copied()) else { break };
        if item.kind != ItemKind::Separator {
            before |= visible(item, mode);
            continue;
        }
        let after = items
            .iter()
            .zip(modes.iter())
            .skip(i + 1)
            .take_while(|(it, _)| it.kind != ItemKind::Separator)
            .any(|(it, m)| visible(it, *m));
        let shown = before && after;
        if let Some(m) = modes.get_mut(i) {
            *m = if shown { Mode::Full } else { Mode::Hidden };
        }
        if shown {
            before = false;
        }
    }
}

/// The width a row takes in these modes (separators resolved in place).
#[must_use]
pub fn row_width(items: &[PlanItem], modes: &mut [Mode], style: ToolbarStyle) -> f32 {
    resolve_separators(items, modes);
    let mut w = 0.0;
    let mut prev: Option<(ItemKind, u32)> = None;
    for (item, mode) in items.iter().zip(modes.iter()) {
        if *mode == Mode::Hidden {
            continue;
        }
        if let Some((kind, group)) = prev {
            w += gap(style, kind, group, item.kind, item.group);
        }
        w += match (item.kind, mode) {
            (ItemKind::Spring, _) => 0.0,
            (_, Mode::Icon) => item.w_icon,
            _ => item.w_full,
        };
        prev = Some((item.kind, item.group));
    }
    w
}

/// Lowest priority first, the rightmost of equals first.
fn give_way_order(items: &[PlanItem], eligible: impl Fn(&PlanItem) -> bool) -> Vec<usize> {
    let mut order: Vec<usize> = (0..items.len()).filter(|&i| items.get(i).is_some_and(&eligible)).collect();
    order.sort_by(|&a, &b| {
        let pa = items.get(a).map_or(0, |i| i.priority);
        let pb = items.get(b).map_or(0, |i| i.priority);
        pa.cmp(&pb).then(b.cmp(&a))
    });
    order
}

/// Items shown, labels counting twice: the layouts weighed are all folds of one order, so more
/// is more.
fn shows(items: &[PlanItem], modes: &[Mode]) -> usize {
    items
        .iter()
        .zip(modes)
        .filter(|(it, _)| !is_spacer(it.kind))
        .map(|(_, m)| match m {
            Mode::Full => 2,
            Mode::Icon => 1,
            Mode::Hidden => 0,
        })
        .sum()
}

/// The layout for a row `room` wide: labels fold first, then items move into the overflow.
fn fold(items: &[PlanItem], room: f32, overflow: f32, style: ToolbarStyle) -> (Vec<Mode>, f32, bool) {
    let mut modes = vec![Mode::Full; items.len()];
    let mut any_hidden = false;
    let mut w = row_width(items, &mut modes, style);
    if w > room {
        for i in give_way_order(items, |it| it.collapsible) {
            if let Some(m) = modes.get_mut(i) {
                *m = Mode::Icon;
            }
            w = row_width(items, &mut modes, style);
            if w <= room {
                break;
            }
        }
    }
    if w > room {
        for i in give_way_order(items, |it| !is_spacer(it.kind)) {
            if let Some(m) = modes.get_mut(i) {
                *m = Mode::Hidden;
            }
            any_hidden = true;
            w = row_width(items, &mut modes, style);
            if w + overflow <= room {
                break;
            }
        }
    }
    (modes, w, any_hidden)
}

/// Plans a row `width` wide from last frame's items, `button` being the overflow button's side.
///
/// Hysteresis: the fold is planned for `width - UNFOLD_SLACK`, and what the row showed last frame
/// is kept while it still fits and shows more. Decided afresh every frame, a value one character
/// wider elsewhere on the row would fold another item's label, and the next change unfold it.
/// What is left goes to the search fields first (up to twice their minimum), then to the springs.
#[must_use]
pub fn plan(items: &[PlanItem], width: f32, button: f32, style: ToolbarStyle) -> Plan {
    let overflow = button + gap(style, ItemKind::Action, 0, ItemKind::Action, 1);
    let (mut modes, mut w, mut any_hidden) = fold(items, width - UNFOLD_SLACK, overflow, style);
    let mut before: Vec<Mode> = items.iter().map(|i| i.shown).collect();
    let before_hidden = items.iter().zip(&before).any(|(it, m)| *m == Mode::Hidden && !is_spacer(it.kind));
    let w_before = row_width(items, &mut before, style);
    if w_before + if before_hidden { overflow } else { 0.0 } <= width
        && shows(items, &before) > shows(items, &modes)
    {
        modes = before;
        w = w_before;
        any_hidden = before_hidden;
    }
    let mut extra = vec![0.0; items.len()];
    let mut free = width - w - if any_hidden { overflow } else { 0.0 };
    if free > 0.0 {
        let mut searches = items
            .iter()
            .zip(&modes)
            .filter(|(it, m)| it.kind == ItemKind::Search && **m != Mode::Hidden)
            .count();
        let springs = items.iter().filter(|it| it.kind == ItemKind::Spring).count();
        for ((it, m), e) in items.iter().zip(&modes).zip(extra.iter_mut()) {
            if searches > 0 && it.kind == ItemKind::Search && *m != Mode::Hidden {
                *e = (free / searches as f32).min(it.w_full);
                free -= *e;
                searches -= 1;
            }
        }
        if springs > 0 {
            for (it, e) in items.iter().zip(extra.iter_mut()) {
                if it.kind == ItemKind::Spring {
                    *e = free / springs as f32;
                }
            }
        }
    }
    Plan { modes, extra, any_hidden }
}

/// What a toolbar keeps between frames.
#[derive(Clone, Debug)]
struct TbState {
    items: Vec<PlanItem>,
    ids: Vec<Id>,
    /// Overlay: last frame's group pills, x ranges relative to the origin.
    pills: Vec<(f32, f32)>,
    overflow_rect: Rect,
    pending_click: Option<Id>,
    pending_menu: Option<Id>,
    pending_segment: Option<(Id, usize)>,
}

impl Default for TbState {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            ids: Vec::new(),
            pills: Vec::new(),
            overflow_rect: Rect::NOTHING,
            pending_click: None,
            pending_menu: None,
            pending_segment: None,
        }
    }
}

/// An item that went into the overflow menu, kept while that menu is open.
enum Entry<'a> {
    Action { id: Id, icon: Icon, label: String, tip: String, enabled: bool, checked: bool },
    Menu { id: Id, icon: Icon, label: String, tip: String, enabled: bool },
    Split { id: Id, icon: Icon, label: String, tip: String, enabled: bool },
    Search { text: &'a mut String, hint: String },
    Status { text: String, tone: Tone },
    Segmented { id: Id, caption: String, options: Vec<String>, current: usize, enabled: bool },
}

/// A toolbar being drawn. Items are laid out where last frame's plan put them; [`Toolbar::end`]
/// (or dropping it) draws the overflow and saves the measures.
pub struct Toolbar<'u, 'a> {
    ui: &'u mut Ui,
    ctx: Context,
    id: Id,
    name: String,
    style: ToolbarStyle,
    origin: Pos2,
    width: f32,
    height: f32,
    button: f32,
    state: TbState,
    plan: Plan,
    cur_items: Vec<PlanItem>,
    cur_ids: Vec<Id>,
    x: f32,
    group: u32,
    priority: i32,
    prev: Option<(ItemKind, u32)>,
    hidden: usize,
    pill: Option<(u32, f32, f32)>,
    pills: Vec<(f32, f32)>,
    serial: u32,
    overflow_open: bool,
    entries: Vec<Entry<'a>>,
    finished: bool,
}

/// How a toolbar button looks.
fn look(
    p: &Palette,
    kind: Kind,
    enabled: bool,
    hovered: bool,
    pressed: bool,
    _checked: bool,
) -> (Color32, Color32, Font) {
    if !enabled {
        let fill =
            if matches!(kind, Kind::Ghost | Kind::GhostDanger) { Color32::TRANSPARENT } else { p.input };
        return (fill, p.text_disabled, Font::Body);
    }
    let pick = |rest: Color32, hover: Color32, press: Color32| {
        if pressed {
            press
        } else if hovered {
            hover
        } else {
            rest
        }
    };
    match kind {
        Kind::Secondary => (pick(p.button, p.button_hover, p.pressed), p.text, Font::Body),
        Kind::Primary => (pick(p.primary, p.primary_hover, p.primary_pressed), p.on_fill, Font::Strong),
        Kind::Danger => (pick(p.danger, p.danger_hover, p.danger_pressed), p.on_fill, Font::Strong),
        // a toggle that is on keeps no fill: the line under it says so
        Kind::Ghost => (pick(Color32::TRANSPARENT, p.hover, p.pressed), p.text, Font::Body),
        Kind::GhostDanger => (pick(Color32::TRANSPARENT, p.hover, p.pressed), p.error, Font::Body),
    }
}

/// What one toolbar button shows.
struct Face<'s> {
    icon: Icon,
    glyph: f32,
    /// The label drawn: empty when folded.
    label: &'s str,
    /// The folded label, as the tooltip's title.
    title: &'s str,
    tip: &'s str,
    kind: Kind,
    enabled: bool,
    checked: bool,
    caret: bool,
    corners: CornerRadius,
}

fn tool_button(ui: &mut Ui, rect: Rect, id: Id, f: &Face<'_>) -> Response {
    let p = palette(ui.ctx());
    let sense = if f.enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.interact(rect, id, sense);
    let name = if f.label.is_empty() { if f.title.is_empty() { f.tip } else { f.title } } else { f.label };
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, f.enabled, name));
    if ui.is_rect_visible(rect) {
        let (fill, color, font) =
            look(&p, f.kind, f.enabled, resp.hovered(), resp.is_pointer_button_down_on(), f.checked);
        let painter = ui.painter();
        if fill != Color32::TRANSPARENT {
            painter.rect_filled(rect, f.corners, fill);
        }
        if f.kind == Kind::Secondary {
            painter.rect_stroke(
                rect,
                f.corners,
                Stroke::new(1.0, if resp.hovered() { p.border_strong } else { p.border }),
                egui::StrokeKind::Inside,
            );
        }
        if f.checked && f.enabled {
            crate::button::toggle_line(painter, rect, p.accent_bright);
        }
        let font_id = font.id();
        let text_w = if f.label.is_empty() { 0.0 } else { text::width(ui.ctx(), &font_id, f.label) };
        let icon_w =
            if f.icon == Icon::None { 0.0 } else { f.glyph + if f.label.is_empty() { 0.0 } else { 6.0 } };
        let caret_w =
            if f.caret { metrics::GLYPH_STATUS + if text_w + icon_w > 0.0 { 4.0 } else { 0.0 } } else { 0.0 };
        let mut x = rect.center().x - (icon_w + text_w + caret_w) * 0.5;
        if f.icon != Icon::None {
            let icon_color = if !f.enabled {
                p.text_disabled
            } else if f.checked {
                p.accent_bright
            } else if matches!(f.kind, Kind::Primary | Kind::Danger | Kind::GhostDanger) {
                color
            } else if resp.hovered() {
                p.text
            } else {
                p.text_secondary
            };
            icons::draw(
                painter,
                f.icon,
                Rect::from_min_size(pos2(x, rect.center().y - f.glyph * 0.5), Vec2::splat(f.glyph)),
                icon_color,
            );
            x += icon_w;
        }
        if !f.label.is_empty() {
            painter.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, f.label, font_id, color);
            x += text_w;
        }
        if f.caret {
            let c = Rect::from_min_size(
                pos2(x + caret_w - metrics::GLYPH_STATUS, rect.center().y - 6.0),
                Vec2::splat(metrics::GLYPH_STATUS),
            );
            icons::draw(
                painter,
                Icon::ChevronDown,
                c,
                if f.enabled { p.text_secondary } else { p.text_disabled },
            );
        }
        if resp.has_focus() {
            painter.rect_stroke(
                rect.expand(1.0),
                CornerRadius::same(metrics::RADIUS + 1),
                Stroke::new(metrics::FOCUS_RING, p.accent_bright),
                egui::StrokeKind::Outside,
            );
        }
    }
    let why = (!f.enabled && !f.tip.is_empty()).then_some(f.tip);
    audit::control(ui.ctx(), name, f.enabled, why, if f.tip.is_empty() { None } else { Some(f.tip) }, rect);
    if !f.title.is_empty() && f.enabled {
        let (title, tip, text_c, tip_c) = (f.title, f.tip, p.text, p.text_secondary);
        resp.on_hover_ui(|ui| {
            ui.label(RichText::new(title).font(Font::Strong.id()).color(text_c));
            if !tip.is_empty() {
                ui.label(RichText::new(tip).font(Font::Body.id()).color(tip_c));
            }
        })
    } else if !f.tip.is_empty() {
        resp.on_hover_text(f.tip)
    } else {
        resp
    }
}

impl<'u, 'a> Toolbar<'u, 'a> {
    /// Starts a toolbar at the cursor, as wide as what is left of the row.
    pub fn begin(ui: &'u mut Ui, id_salt: &str, style: ToolbarStyle) -> Self {
        let ctx = ui.ctx().clone();
        let id = ui.make_persistent_id(("gz-ui.toolbar", id_salt));
        let height = style.height();
        let button = style.button();
        let origin = ui.cursor().min;
        let width = ui.available_width().max(0.0);
        let state = ctx.data(|d| d.get_temp::<TbState>(id)).unwrap_or_default();
        let plan = plan(&state.items, width, button, style);
        let mut name = String::with_capacity(17 + id_salt.len());
        name.push_str("toolbar_overflow:");
        name.push_str(id_salt);
        let overflow_id = id.with("overflow");
        if take_popup_request(&ctx, &name) {
            Popup::open_id(&ctx, overflow_id);
        }
        let overflow_open = Popup::is_id_open(&ctx, overflow_id);
        // the pills go under the items, so they are last frame's: the layout they follow is too
        if style == ToolbarStyle::Overlay {
            let p = palette(&ctx);
            let top = (height - button) * 0.5 - 2.0;
            for (a, b) in &state.pills {
                let r = Rect::from_min_max(
                    pos2(origin.x + a, origin.y + top),
                    pos2(origin.x + b, origin.y + height - top),
                );
                ui.painter().rect_filled(
                    r,
                    CornerRadius::same(metrics::RADIUS_POPUP),
                    p.overlay.gamma_multiply(0.92),
                );
            }
        }
        let n = state.items.len();
        Self {
            ui,
            ctx,
            id,
            name,
            style,
            origin,
            width,
            height,
            button,
            state,
            plan,
            cur_items: Vec::with_capacity(n),
            cur_ids: Vec::with_capacity(n),
            x: origin.x,
            group: 0,
            priority: 0,
            prev: None,
            hidden: 0,
            pill: None,
            pills: Vec::new(),
            serial: 0,
            overflow_open,
            entries: Vec::new(),
            finished: false,
        }
    }

    /// Ends the toolbar: the overflow button and menu, the measures for the next frame.
    pub fn end(self) {}

    /// Starts a group: a wider gap before it; its items give way at `priority` (lower first).
    pub fn group(&mut self, priority: i32) {
        self.group += 1;
        self.priority = priority;
    }

    /// Finds this frame's item in last frame's plan and places it; `None` when it is in the
    /// overflow.
    fn place(&mut self, id: Id, mut item: PlanItem) -> Option<(f32, f32, Mode)> {
        item.group = self.group;
        item.priority = self.priority;
        let hint = self.cur_ids.len();
        let found = if self.state.ids.get(hint) == Some(&id) {
            Some(hint)
        } else {
            self.state.ids.iter().position(|x| *x == id)
        };
        let mode = found.and_then(|i| self.plan.modes.get(i).copied()).unwrap_or(Mode::Full);
        let extra = found.and_then(|i| self.plan.extra.get(i).copied()).unwrap_or(0.0);
        item.shown = mode;
        self.cur_items.push(item);
        self.cur_ids.push(id);
        if mode == Mode::Hidden {
            if !is_spacer(item.kind) {
                self.hidden += 1;
            }
            return None;
        }
        let w = match (item.kind, mode) {
            (ItemKind::Spring, _) => 0.0,
            (_, Mode::Icon) => item.w_icon,
            _ => item.w_full,
        } + extra;
        if let Some((kind, group)) = self.prev {
            self.x += gap(self.style, kind, group, item.kind, item.group);
        }
        let x = self.x;
        self.x += w;
        self.prev = Some((item.kind, item.group));
        // overlay: each group sits on a pill of its own
        if self.style == ToolbarStyle::Overlay && !is_spacer(item.kind) {
            match self.pill {
                Some((g, a, b)) if g == item.group => self.pill = Some((g, a.min(x), b.max(x + w))),
                Some((_, a, b)) => {
                    self.pills.push((a, b));
                    self.pill = Some((item.group, x, x + w));
                }
                None => self.pill = Some((item.group, x, x + w)),
            }
        }
        Some((x, w, mode))
    }

    fn centred(&self, x: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, (self.origin.y + (self.height - h) * 0.5).round()), Vec2::new(w, h))
    }

    fn child(&mut self, salt: Id, rect: Rect) -> Ui {
        self.ui.new_child(
            UiBuilder::new().id_salt(salt).max_rect(rect).layout(Layout::left_to_right(Align::Center)),
        )
    }

    /// A thin line between items; shown only between two shown items.
    pub fn separator(&mut self) {
        let id = self.id.with(("sep", self.serial));
        self.serial += 1;
        let item = PlanItem { kind: ItemKind::Separator, w_full: 1.0, w_icon: 1.0, ..PlanItem::default() };
        if let Some((x, _, _)) = self.place(id, item) {
            let p = palette(&self.ctx);
            let x = x.round() + 0.5;
            self.ui.painter().line_segment(
                [pos2(x, self.origin.y + 8.0), pos2(x, self.origin.y + self.height - 8.0)],
                Stroke::new(1.0, p.border),
            );
        }
    }

    /// How many items were added so far (a separator or a spring counts).
    #[must_use]
    pub fn items(&self) -> usize {
        self.cur_items.len()
    }

    /// Where the next item would start, in pt: the right edge of what is on the row.
    #[must_use]
    pub fn cursor_x(&self) -> f32 {
        self.x
    }

    /// The row's rect.
    #[must_use]
    pub fn rect(&self) -> egui::Rect {
        egui::Rect::from_min_size(self.origin, Vec2::new(self.width, self.height))
    }

    /// Takes the room the row has left (shared with the other springs).
    pub fn spring(&mut self) {
        let id = self.id.with(("spring", self.serial));
        self.serial += 1;
        self.place(id, PlanItem { kind: ItemKind::Spring, ..PlanItem::default() });
    }

    /// A text on the row in a tone's colour: the problems count, a state.
    pub fn status(&mut self, text_: &str, tone: Tone) {
        let id = self.id.with(("status", self.serial));
        self.serial += 1;
        let w = text::width(&self.ctx, &Font::Body.id(), text_);
        let item = PlanItem { kind: ItemKind::Status, w_full: w, w_icon: w, ..PlanItem::default() };
        match self.place(id, item) {
            Some((x, w, _)) => {
                let p = palette(&self.ctx);
                let color = match tone {
                    Tone::Muted => p.text_muted,
                    Tone::Info => p.text_secondary,
                    other => p.tone(other),
                };
                let y = self.origin.y + self.height * 0.5;
                text::paint(
                    self.ui.painter(),
                    &Font::Body.id(),
                    pos2(x, y),
                    Align2::LEFT_CENTER,
                    color,
                    text_,
                    Some(w + 0.5),
                    Clip::End,
                );
            }
            None if self.overflow_open => self.entries.push(Entry::Status { text: text_.to_owned(), tone }),
            None => {}
        }
    }

    /// A button: an icon, a label (folded to the icon when the row is short), its tip; off, the
    /// tip says why. Returns whether it was pressed, here or from the overflow menu.
    #[allow(clippy::too_many_arguments)]
    pub fn action(
        &mut self,
        id_salt: &str,
        icon: Icon,
        label: &str,
        tip: &str,
        enabled: bool,
        checked: bool,
        kind: Kind,
    ) -> bool {
        let id = self.id.with(("action", id_salt));
        if kind == Kind::Primary {
            crate::button::note_primary(&self.ctx, self.id, "toolbar", label);
        }
        let w_full = if label.is_empty() {
            self.button
        } else {
            button_width(&self.ctx, label, kind, icon != Icon::None, 0.0)
        };
        let w_icon = if icon == Icon::None { w_full } else { self.button };
        let item = PlanItem {
            kind: ItemKind::Action,
            w_full,
            w_icon,
            collapsible: icon != Icon::None && !label.is_empty(),
            ..PlanItem::default()
        };
        let Some((x, w, mode)) = self.place(id, item) else {
            if self.overflow_open {
                self.entries.push(Entry::Action {
                    id,
                    icon,
                    label: label.to_owned(),
                    tip: tip.to_owned(),
                    enabled,
                    checked,
                });
            }
            // picked from the overflow menu last frame
            if self.state.pending_click == Some(id) {
                self.state.pending_click = None;
                return enabled;
            }
            return false;
        };
        let rect = self.centred(x, w, self.button);
        let folded = mode == Mode::Icon;
        let face = Face {
            icon,
            glyph: metrics::GLYPH,
            label: if folded { "" } else { label },
            title: if folded { label } else { "" },
            tip,
            kind,
            enabled,
            checked,
            caret: false,
            corners: CornerRadius::same(metrics::RADIUS),
        };
        tool_button(self.ui, rect, id, &face).clicked() && enabled
    }

    /// A button that opens a menu (a caret after its label). `request` is the name `ui.popup`
    /// opens it by. Off, it keeps its caret and width, its tip says why, and it never opens.
    /// `add` draws the menu's rows while it is open; returns whether it is open.
    #[allow(clippy::too_many_arguments)]
    pub fn menu(
        &mut self,
        id_salt: &str,
        icon: Icon,
        label: &str,
        tip: &str,
        request: Option<&str>,
        enabled: bool,
        add: impl FnOnce(&mut Ui),
    ) -> bool {
        let id = self.id.with(("menu", id_salt));
        let popup_id = id.with("popup");
        let caret = metrics::GLYPH_STATUS + 4.0;
        let w_full = button_width(&self.ctx, label, Kind::Ghost, icon != Icon::None, 0.0) + caret;
        let w_icon = if icon == Icon::None { w_full } else { self.button + caret };
        let item = PlanItem {
            kind: ItemKind::Menu,
            w_full,
            w_icon,
            collapsible: icon != Icon::None && !label.is_empty(),
            ..PlanItem::default()
        };
        let asked = request.is_some_and(|r| take_popup_request(&self.ctx, r));
        let open_now = Popup::is_id_open(&self.ctx, popup_id);
        let anchor = match self.place(id, item) {
            None => {
                if self.overflow_open {
                    self.entries.push(Entry::Menu {
                        id,
                        icon,
                        label: label.to_owned(),
                        tip: tip.to_owned(),
                        enabled,
                    });
                }
                // picked from the overflow menu last frame, or asked for over MCP while it is there
                if self.state.pending_menu == Some(id) || asked {
                    self.state.pending_menu = None;
                    if enabled {
                        Popup::open_id(&self.ctx, popup_id);
                    }
                }
                self.state.overflow_rect
            }
            Some((x, w, mode)) => {
                let rect = self.centred(x, w, self.button);
                let folded = mode == Mode::Icon;
                let face = Face {
                    icon,
                    glyph: metrics::GLYPH,
                    label: if folded { "" } else { label },
                    title: if folded { label } else { "" },
                    tip,
                    kind: Kind::Ghost,
                    enabled,
                    checked: open_now,
                    caret: true,
                    corners: CornerRadius::same(metrics::RADIUS),
                };
                let clicked = tool_button(self.ui, rect, id, &face).clicked();
                // off, a request is taken and dropped
                if enabled && clicked {
                    Popup::toggle_id(&self.ctx, popup_id);
                } else if enabled && asked {
                    Popup::open_id(&self.ctx, popup_id);
                }
                rect
            }
        };
        if !enabled {
            // a menu that goes off takes its open popup with it
            Popup::close_id(&self.ctx, popup_id);
            return false;
        }
        menu_popup(&self.ctx, popup_id, anchor, self.ui.layer_id(), add).is_some()
    }

    /// An action with a chevron that opens its menu (Build Mod). In the overflow it is the action
    /// plus `<label> Options`. `add` draws the menu's rows while it is open. Returns whether the
    /// action was pressed.
    #[allow(clippy::too_many_arguments)]
    pub fn split(
        &mut self,
        id_salt: &str,
        icon: Icon,
        label: &str,
        tip: &str,
        enabled: bool,
        kind: Kind,
        add: impl FnOnce(&mut Ui),
    ) -> bool {
        let id = self.id.with(("split", id_salt));
        if kind == Kind::Primary {
            crate::button::note_primary(&self.ctx, self.id, "toolbar", label);
        }
        let popup_id = id.with("popup");
        let chevron = 20.0;
        let seam = 1.0;
        let main_w = if label.is_empty() {
            self.button
        } else {
            button_width(&self.ctx, label, kind, icon != Icon::None, 0.0)
        };
        let w_full = main_w + seam + chevron;
        let w_icon = if icon == Icon::None { w_full } else { self.button + seam + chevron };
        let item = PlanItem {
            kind: ItemKind::Split,
            w_full,
            w_icon,
            collapsible: icon != Icon::None && !label.is_empty(),
            ..PlanItem::default()
        };
        let mut pressed = false;
        let anchor = match self.place(id, item) {
            None => {
                if self.overflow_open {
                    self.entries.push(Entry::Split {
                        id,
                        icon,
                        label: label.to_owned(),
                        tip: tip.to_owned(),
                        enabled,
                    });
                }
                if self.state.pending_click == Some(id) {
                    self.state.pending_click = None;
                    pressed = enabled;
                }
                if self.state.pending_menu == Some(id) {
                    self.state.pending_menu = None;
                    Popup::open_id(&self.ctx, popup_id);
                }
                self.state.overflow_rect
            }
            Some((x, w, mode)) => {
                let folded = mode == Mode::Icon;
                let main = self.centred(x, w - seam - chevron, self.button);
                let r = metrics::RADIUS;
                let face = Face {
                    icon,
                    glyph: metrics::GLYPH,
                    label: if folded { "" } else { label },
                    title: if folded { label } else { "" },
                    tip,
                    kind,
                    enabled,
                    checked: false,
                    caret: false,
                    corners: CornerRadius { nw: r, sw: r, ne: 0, se: 0 },
                };
                pressed = tool_button(self.ui, main, id, &face).clicked() && enabled;
                // the seam between the halves is the row showing through
                let more =
                    Rect::from_min_size(pos2(main.max.x + seam, main.min.y), Vec2::new(chevron, self.button));
                let more_face = Face {
                    icon: Icon::ChevronDown,
                    glyph: metrics::GLYPH_STATUS,
                    label: "",
                    title: "",
                    tip: "More options",
                    kind,
                    enabled: true,
                    checked: Popup::is_id_open(&self.ctx, popup_id),
                    caret: false,
                    corners: CornerRadius { nw: 0, sw: 0, ne: r, se: r },
                };
                let request = format!("toolbar_split:{id_salt}");
                if tool_button(self.ui, more, id.with("more"), &more_face).clicked() {
                    Popup::toggle_id(&self.ctx, popup_id);
                } else if take_popup_request(&self.ctx, &request) {
                    Popup::open_id(&self.ctx, popup_id);
                }
                main.union(more)
            }
        };
        menu_popup(&self.ctx, popup_id, anchor, self.ui.layer_id(), add);
        pressed
    }

    /// A search field: at least `min_em` wide, up to twice that when the row has room. Returns
    /// whether the text changed.
    pub fn search(&mut self, text_: &'a mut String, hint: &str, min_em: f32) -> bool {
        let id = self.id.with(("search", self.serial));
        self.serial += 1;
        let w = em(min_em);
        let item = PlanItem { kind: ItemKind::Search, w_full: w, w_icon: w, ..PlanItem::default() };
        match self.place(id, item) {
            Some((x, w, _)) => {
                let rect = self.centred(x, w, metrics::FRAME);
                let mut child = self.child(id, rect);
                SearchField::new(text_).hint(hint).min_em(min_em).width(w).show(&mut child).changed()
            }
            None => {
                if self.overflow_open {
                    self.entries.push(Entry::Search { text: text_, hint: hint.to_owned() });
                }
                false
            }
        }
    }

    /// One choice out of a few under a caption; in the overflow its choices are radio items
    /// under the caption. Returns whether the choice changed.
    #[allow(clippy::too_many_arguments)]
    pub fn segmented(
        &mut self,
        id_salt: &str,
        caption: &str,
        current: &mut usize,
        labels: &[&str],
        tip: &str,
        enabled: bool,
    ) -> bool {
        let id = self.id.with(("segmented", id_salt));
        let font = Font::Body.id();
        // the width button::segmented takes at its natural size
        let w: f32 = labels.iter().map(|l| 20.0 + text::width(&self.ctx, &font, l)).sum();
        let item = PlanItem { kind: ItemKind::Segmented, w_full: w, w_icon: w, ..PlanItem::default() };
        let mut changed = false;
        if let Some((pid, choice)) = self.state.pending_segment
            && pid == id
        {
            self.state.pending_segment = None;
            if enabled && choice < labels.len() && choice != *current {
                *current = choice;
                changed = true;
            }
        }
        match self.place(id, item) {
            Some((x, w, _)) => {
                let rect = self.centred(x, w, metrics::FRAME);
                let mut child = self.child(id, rect);
                if !enabled {
                    child.disable();
                }
                changed |= segmented(&mut child, current, labels, None);
                if !tip.is_empty() {
                    self.ui.interact(rect, id.with("tip"), Sense::hover()).on_hover_text(tip);
                }
            }
            None => {
                if self.overflow_open {
                    self.entries.push(Entry::Segmented {
                        id,
                        caption: caption.to_owned(),
                        options: labels.iter().map(|l| (*l).to_owned()).collect(),
                        current: *current,
                        enabled,
                    });
                }
            }
        }
        changed
    }

    /// Room `width` points wide for a widget of the caller's; it has no overflow form and is
    /// hidden there. Returns whether it was drawn.
    pub fn widget(&mut self, id_salt: &str, width: f32, add: impl FnOnce(&mut Ui)) -> bool {
        let id = self.id.with(("widget", id_salt));
        let item = PlanItem { kind: ItemKind::Widget, w_full: width, w_icon: width, ..PlanItem::default() };
        match self.place(id, item) {
            Some((x, w, _)) => {
                let rect = self.centred(x, w, metrics::FRAME);
                let mut child = self.child(id, rect);
                add(&mut child);
                true
            }
            None => false,
        }
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        let ctx = self.ctx.clone();
        let p = palette(&ctx);
        let overflow_id = self.id.with("overflow");

        // everything that did not fit is one click away
        if self.hidden > 0 {
            let rect = self.centred(self.origin.x + self.width - self.button, self.button, self.button);
            let open = Popup::is_id_open(&ctx, overflow_id);
            let mut child = self.child(overflow_id.with("button"), rect);
            if icon_button_sized(&mut child, Icon::Overflow, "More", open, true, self.button, metrics::GLYPH)
                .clicked()
            {
                Popup::toggle_id(&ctx, overflow_id);
            }
            self.state.overflow_rect = rect;
            if self.style == ToolbarStyle::Overlay {
                self.pills.push((rect.min.x, rect.max.x));
            }
            let entries = std::mem::take(&mut self.entries);
            let mut click = None;
            let mut menu_pick = None;
            let mut segment = None;
            menu_popup(&ctx, overflow_id, rect, self.ui.layer_id(), |ui| {
                for entry in entries {
                    match entry {
                        Entry::Action { id, icon, label, tip, enabled, checked } => {
                            let name = if label.is_empty() { tip.as_str() } else { label.as_str() };
                            if menu::row(
                                ui,
                                name,
                                MenuItem { icon, selected: checked, enabled, ..MenuItem::default() },
                                false,
                                &tip,
                            ) {
                                click = Some(id);
                            }
                        }
                        Entry::Menu { id, icon, label, tip, enabled } => {
                            let name = if label.is_empty() { tip.as_str() } else { label.as_str() };
                            if menu::row(
                                ui,
                                name,
                                MenuItem { icon, enabled, ..MenuItem::default() },
                                true,
                                &tip,
                            ) {
                                menu_pick = Some(id);
                            }
                        }
                        Entry::Split { id, icon, label, tip, enabled } => {
                            let name = if label.is_empty() { tip.as_str() } else { label.as_str() };
                            if menu::row(
                                ui,
                                name,
                                MenuItem { icon, enabled, ..MenuItem::default() },
                                false,
                                &tip,
                            ) {
                                click = Some(id);
                            }
                            let options = format!("{name} Options");
                            if menu::row(ui, &options, MenuItem::default(), true, "") {
                                menu_pick = Some(id);
                            }
                        }
                        Entry::Search { text, hint } => {
                            SearchField::new(text).hint(&hint).width(em(12.0)).id_salt("overflow").show(ui);
                        }
                        Entry::Status { text, tone } => {
                            text::status(ui, tone, &text);
                        }
                        Entry::Segmented { id, caption, options, current, enabled } => {
                            let muted = palette(ui.ctx()).text_muted;
                            text::label(ui, &caption, Font::Small, muted, Clip::End);
                            for (k, option) in options.iter().enumerate() {
                                if menu_item(
                                    ui,
                                    option,
                                    MenuItem { selected: k == current, enabled, ..MenuItem::default() },
                                ) {
                                    segment = Some((id, k));
                                }
                            }
                        }
                    }
                }
            });
            if click.is_some() {
                self.state.pending_click = click;
            }
            if menu_pick.is_some() {
                self.state.pending_menu = menu_pick;
                // a menu picked from the overflow opens on the next frame, under the overflow button
                Popup::close_id(&ctx, overflow_id);
            }
            if segment.is_some() {
                self.state.pending_segment = segment;
            }
            if click.is_some() || menu_pick.is_some() || segment.is_some() {
                ctx.request_repaint();
            }
        } else {
            Popup::close_id(&ctx, overflow_id);
        }

        // overlay pills for the next frame, relative to the origin
        match self.style {
            ToolbarStyle::Overlay => {
                if let Some((_, a, b)) = self.pill.take() {
                    self.pills.push((a, b));
                }
                let ox = self.origin.x;
                self.state.pills = self.pills.iter().map(|(a, b)| (a - ox - 2.0, b - ox + 2.0)).collect();
            }
            ToolbarStyle::Panel => {
                let y = self.origin.y + self.height - 0.5;
                self.ui.painter().line_segment(
                    [pos2(self.origin.x, y), pos2(self.origin.x + self.width, y)],
                    Stroke::new(1.0, p.border_subtle),
                );
            }
            ToolbarStyle::Row => {}
        }

        // the row as one item of its ui
        let row = Rect::from_min_size(self.origin, Vec2::new(self.width, self.height));
        self.ui.allocate_rect(row, Sense::hover());
        if self.x > self.origin.x + self.width + 1.0 {
            audit::escaped(
                &ctx,
                &self.name,
                Rect::from_min_max(self.origin, pos2(self.x, self.origin.y + self.height)),
            );
        }
        self.state.items = std::mem::take(&mut self.cur_items);
        self.state.ids = std::mem::take(&mut self.cur_ids);
        let state = std::mem::take(&mut self.state);
        ctx.data_mut(|d| d.insert_temp(self.id, state));
    }
}

impl Drop for Toolbar<'_, '_> {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::{ItemKind, Mode, PlanItem, ToolbarStyle, UNFOLD_SLACK, plan, row_width};

    fn action(group: u32, priority: i32, full: f32) -> PlanItem {
        PlanItem {
            kind: ItemKind::Action,
            group,
            priority,
            w_full: full,
            w_icon: 28.0,
            collapsible: true,
            shown: Mode::Full,
        }
    }

    #[test]
    fn a_row_that_fits_stays_whole() {
        let items = [action(1, 0, 80.0), action(1, 0, 80.0)];
        let p = plan(&items, 400.0, 28.0, ToolbarStyle::Row);
        assert_eq!(p.modes, vec![Mode::Full, Mode::Full]);
        assert!(!p.any_hidden);
    }

    #[test]
    fn labels_fold_first_rising_priority_rightmost_first() {
        // 3 x 100 + 2 x 2 = 304; room 280 - 24 = 256
        let items = [action(1, 0, 100.0), action(1, 0, 100.0), action(1, 0, 100.0)];
        let p = plan(&items, 280.0, 28.0, ToolbarStyle::Row);
        // the rightmost folds: 100 + 100 + 28 + 4 = 232
        assert_eq!(p.modes, vec![Mode::Full, Mode::Full, Mode::Icon]);
        let high = [action(1, 5, 100.0), action(1, 0, 100.0), action(1, 5, 100.0)];
        let p = plan(&high, 280.0, 28.0, ToolbarStyle::Row);
        assert_eq!(p.modes, vec![Mode::Full, Mode::Icon, Mode::Full]);
    }

    #[test]
    fn then_items_move_into_the_overflow() {
        let items = [action(1, 1, 100.0), action(1, 0, 100.0), action(1, 0, 100.0)];
        // everything as icons is 3 x 28 + 4 = 88; room 100 - 24 = 76
        let p = plan(&items, 100.0, 28.0, ToolbarStyle::Row);
        assert!(p.any_hidden);
        // the lower priority, rightmost first: the last one goes, then the middle one
        assert_eq!(p.modes.get(2), Some(&Mode::Hidden));
        assert_eq!(p.modes.first(), Some(&Mode::Icon));
    }

    #[test]
    fn a_folded_label_unfolds_only_with_room_to_spare() {
        let mut items = [action(1, 0, 100.0), action(1, 0, 100.0)];
        // 202 wide: fits whole, but not with the slack
        items[1].shown = Mode::Icon;
        let p = plan(&items, 202.0 + UNFOLD_SLACK - 1.0, 28.0, ToolbarStyle::Row);
        assert_eq!(p.modes.get(1), Some(&Mode::Icon));
        let p = plan(&items, 202.0 + UNFOLD_SLACK, 28.0, ToolbarStyle::Row);
        assert_eq!(p.modes.get(1), Some(&Mode::Full));
    }

    #[test]
    fn a_row_keeps_its_layout_while_it_fits() {
        let mut items = [action(1, 0, 100.0), action(1, 0, 100.0)];
        items[0].shown = Mode::Full;
        items[1].shown = Mode::Full;
        // 202 fits 210 but not 210 - 24: still whole, because it was whole and fits
        let p = plan(&items, 210.0, 28.0, ToolbarStyle::Row);
        assert_eq!(p.modes, vec![Mode::Full, Mode::Full]);
    }

    #[test]
    fn free_room_goes_to_search_then_springs() {
        let items = [
            action(1, 0, 50.0),
            PlanItem { kind: ItemKind::Spring, group: 1, ..PlanItem::default() },
            PlanItem {
                kind: ItemKind::Search,
                group: 1,
                w_full: 128.0,
                w_icon: 128.0,
                ..PlanItem::default()
            },
        ];
        // 50 + 2 + 0 + 2 + 128 = 182 of 500: the search takes 128 more, the spring the rest
        let p = plan(&items, 500.0, 28.0, ToolbarStyle::Row);
        assert_eq!(p.extra.get(2), Some(&128.0));
        assert_eq!(p.extra.get(1), Some(&(500.0 - 182.0 - 128.0)));
    }

    #[test]
    fn separators_show_only_between_shown_items() {
        let sep =
            PlanItem { kind: ItemKind::Separator, group: 1, w_full: 1.0, w_icon: 1.0, ..PlanItem::default() };
        let items = [sep, action(1, 0, 50.0), sep, sep, action(1, 0, 50.0), sep];
        let mut modes = vec![Mode::Full; items.len()];
        let w = row_width(&items, &mut modes, ToolbarStyle::Row);
        // a separator followed by another one has nothing shown after it: the second one stands
        assert_eq!(modes, vec![Mode::Hidden, Mode::Full, Mode::Hidden, Mode::Full, Mode::Full, Mode::Hidden]);
        // 50 + 6 + 1 + 6 + 50
        assert_eq!(w, 113.0);
    }

    #[test]
    fn groups_open_wider_gaps() {
        let items = [action(1, 0, 50.0), action(2, 0, 50.0)];
        let mut modes = vec![Mode::Full; 2];
        assert_eq!(row_width(&items, &mut modes, ToolbarStyle::Row), 112.0);
        assert_eq!(row_width(&items, &mut modes, ToolbarStyle::Overlay), 108.0);
    }
}
