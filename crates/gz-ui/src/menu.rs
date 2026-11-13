//! Menus (docs/UI_DESIGN.md, section 4, "Menus").
//!
//! - [`menu_item`] with [`MenuItem`] - a row of a menu: an icon, the label, the shortcut flush
//!   right in muted text (at least 24 pt after the longest label), a danger item in the error
//!   colour. A popup has ONE icon column: when any of its rows has an icon or a check, every row
//!   reserves it, so the labels of a popup start at one x.
//! - [`popup_frame`], [`popup_body`], [`menu_popup`] - the theme's popup: overlay fill, a hairline,
//!   the popup shadow, rows that touch, at least 200 pt wide.
//! - [`show_popup`] / [`take_popup_request`] - a popup asked for by name (MCP `ui_popup`); the
//!   widget that owns the name opens it on its next frame.

use egui::{
    Align, Align2, Context, CornerRadius, Frame, Id, LayerId, Layout, Margin, Popup, PopupAnchor,
    PopupCloseBehavior, PopupKind, Rect, Role, Sense, Stroke, Ui, UiKind, Vec2, WidgetInfo, pos2,
};

use crate::audit;
use crate::font::Font;
use crate::icons::{self, Icon};
use crate::text::{self, Clip};
use crate::theme::{metrics, palette};
use crate::units::em;

/// How a menu row looks. `MenuItem::default()` is a plain, enabled row.
#[derive(Clone, Copy, Debug)]
pub struct MenuItem<'a> {
    pub icon: Icon,
    /// What is bound NOW, as text ("Ctrl+Shift+P"); empty for none.
    pub keys: &'a str,
    /// A check mark: a toggle that is on, the radio item picked.
    pub selected: bool,
    pub enabled: bool,
    /// Leads to a deletion: the label in the error colour.
    pub danger: bool,
    /// The row's hover hint; on a disabled row, why it is off.
    pub tip: &'a str,
}

impl Default for MenuItem<'_> {
    fn default() -> Self {
        Self { icon: Icon::None, keys: "", selected: false, enabled: true, danger: false, tip: "" }
    }
}

/// The height of a menu row.
const ROW_H: f32 = 24.0;
/// The gap the shortcut column opens after the longest label.
const KEYS_GAP: f32 = 24.0;

/// A menu row. Returns whether it was picked; a picked row closes its menu.
pub fn menu_item(ui: &mut Ui, label: &str, item: MenuItem<'_>) -> bool {
    let tip = item.tip;
    row(ui, label, item, false, tip)
}

/// What the popup being drawn knows of its rows: whether one of them had an icon or a check last
/// frame (then every row reserves the column), and whether one has this frame.
#[derive(Clone, Debug, Default)]
struct Column {
    reserve: bool,
    seen: bool,
    /// Where each row's label started, for `label_columns`.
    labels: Vec<f32>,
}

fn column_stack_id() -> Id {
    Id::unique("gz-ui.menu.columns")
}

/// Whether this row keeps the icon column: it has an icon or a check, or another row of its
/// popup did last frame. A row outside a popup body keeps it when it needs it.
fn reserves_column(ctx: &Context, own: bool) -> bool {
    ctx.data_mut(|d| match d.get_temp_mut_or_default::<Vec<(Id, Column)>>(column_stack_id()).last_mut() {
        Some((_, c)) => {
            c.seen |= own;
            own || c.reserve
        }
        None => own,
    })
}

/// The width of a shortcut drawn in a menu row.
fn keys_width(ctx: &Context, keys: &str) -> f32 {
    if keys.is_empty() { 0.0 } else { text::width(ctx, &Font::Small.id(), keys) }
}

/// A menu row, optionally with a submenu chevron and a tooltip: the overflow menu's rows.
pub(crate) fn row(ui: &mut Ui, label: &str, item: MenuItem<'_>, submenu: bool, tip: &str) -> bool {
    let picked = row_response(ui, label, &item, submenu, tip).clicked() && item.enabled;
    if picked && (ui.stack().contained_in(UiKind::Menu) || ui.stack().contained_in(UiKind::Popup)) {
        ui.close();
    }
    picked
}

/// A row that opens a submenu to its right while hovered: a row like its neighbours - the same
/// icon column, the same label start - with a chevron at its end. `add` draws the submenu's rows.
pub fn submenu<R>(
    ui: &mut Ui,
    label: &str,
    icon: Icon,
    enabled: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let item = MenuItem { icon, enabled, ..MenuItem::default() };
    let resp = row_response(ui, label, &item, true, "");
    if !enabled {
        return None;
    }
    egui::containers::menu::SubMenu::new().show(ui, &resp, |ui| popup_body(ui, add)).map(|r| r.inner)
}

/// Lays out and paints one row and answers its response; what a click means is the caller's.
fn row_response(ui: &mut Ui, label: &str, item: &MenuItem<'_>, submenu: bool, tip: &str) -> egui::Response {
    let p = palette(ui.ctx());
    let font = Font::Body.id();
    let has_slot = reserves_column(ui.ctx(), item.icon != Icon::None || item.selected);
    let label_w = text::width(ui.ctx(), &font, label);
    let keys_w = keys_width(ui.ctx(), item.keys);
    let desired = 8.0
        + if has_slot { metrics::GLYPH + 8.0 } else { 0.0 }
        + label_w
        + if keys_w > 0.0 { KEYS_GAP + keys_w } else { 0.0 }
        + if submenu { 16.0 } else { 0.0 }
        + 8.0;
    let sense = if item.enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.allocate_response(Vec2::new(desired, ROW_H), sense);
    resp.widget_info(|| WidgetInfo::selected(Role::MenuItem, item.enabled, item.selected, label));
    let rect = resp.rect;
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if item.enabled && (resp.hovered() || resp.has_focus()) {
            painter.rect_filled(
                rect,
                CornerRadius::same(metrics::RADIUS),
                if resp.is_pointer_button_down_on() { p.pressed } else { p.hover },
            );
        }
        let text_color = if !item.enabled {
            p.text_disabled
        } else if item.danger {
            p.error
        } else {
            p.text
        };
        let mut x = rect.min.x + 8.0;
        if has_slot {
            let glyph = Rect::from_min_size(
                pos2(x, rect.center().y - metrics::GLYPH * 0.5),
                Vec2::splat(metrics::GLYPH),
            );
            // a check takes the icon's place
            let shown = match (item.icon, item.selected) {
                (_, true) => Some((Icon::Check, p.accent_bright)),
                (Icon::None, false) => None,
                (icon, false) if !item.enabled => Some((icon, p.text_disabled)),
                (icon, false) if item.danger => Some((icon, p.error)),
                (icon, false) => Some((icon, p.text_secondary)),
            };
            if let Some((icon, color)) = shown {
                icons::draw(painter, icon, glyph, color);
            }
            x += metrics::GLYPH + 8.0;
        }
        #[cfg(test)]
        ui.ctx().data_mut(|d| d.insert_temp(Id::unique("gz-ui.menu.test_label_x"), x));
        ui.ctx().data_mut(|d| {
            if let Some((_, c)) = d.get_temp_mut_or_default::<Vec<(Id, Column)>>(column_stack_id()).last_mut()
            {
                c.labels.push(x);
            }
        });
        // a menu wider than this row's needs leaves the label whole; a narrower one cuts it
        let right = rect.max.x
            - 8.0
            - if keys_w > 0.0 { KEYS_GAP + keys_w } else { 0.0 }
            - if submenu { 16.0 } else { 0.0 };
        text::paint(
            painter,
            &font,
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            text_color,
            label,
            Some((right - x).max(0.0)),
            Clip::End,
        );
        if keys_w > 0.0 {
            painter.text(
                pos2(rect.max.x - 8.0 - if submenu { 16.0 } else { 0.0 }, rect.center().y),
                Align2::RIGHT_CENTER,
                item.keys,
                Font::Small.id(),
                if item.enabled { p.text_muted } else { p.text_disabled },
            );
        }
        if submenu {
            let chevron = Rect::from_min_size(
                pos2(rect.max.x - 20.0, rect.center().y - 6.0),
                Vec2::splat(metrics::GLYPH_STATUS),
            );
            icons::draw(ui.painter(), Icon::ChevronRight, chevron, p.text_secondary);
        }
    }
    let said = (!tip.is_empty()).then_some(tip);
    audit::control(ui.ctx(), label, item.enabled, if item.enabled { None } else { said }, said, rect);
    if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
}

/// The frame of a menu or popup: overlay fill, a hairline, the popup shadow, 6 pt corners, 4 pt
/// padding.
pub fn popup_frame(ctx: &Context) -> Frame {
    let p = palette(ctx);
    Frame::new()
        .fill(p.overlay)
        .stroke(Stroke::new(1.0, p.border_subtle))
        .shadow(p.shadow_popup)
        .corner_radius(CornerRadius::same(metrics::RADIUS_POPUP))
        .inner_margin(Margin::same(4))
}

/// The body of a popup: rows that touch, at least 200 pt wide, laid out top-down, one icon
/// column for all of its rows.
pub fn popup_body<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    popup_body_labels(ui, add).0
}

/// [`popup_body`], answering where its rows' labels started too.
fn popup_body_labels<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> (R, Vec<f32>) {
    let ctx = ui.ctx().clone();
    let id = ui.scope_id().with("gz-ui.menu.column");
    // what this popup's rows said last frame; unknown (its first frame): keep the column
    let reserve = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<(Id, Column)>>(column_stack_id())
            .push((id, Column { reserve, seen: false, labels: Vec::new() }));
    });
    let out = ui
        .with_layout(Layout::top_down_justified(Align::Min), |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 0.0);
            ui.set_min_width(metrics::MENU_MIN_W.max(em(10.0)));
            add(ui)
        })
        .inner;
    let (seen, labels) = ctx.data_mut(|d| {
        let stack = d.get_temp_mut_or_default::<Vec<(Id, Column)>>(column_stack_id());
        match stack.iter().rposition(|(i, _)| *i == id) {
            Some(at) => {
                let c = stack.remove(at).1;
                (c.seen, c.labels)
            }
            None => (reserve, Vec::new()),
        }
    });
    if seen != reserve {
        ctx.data_mut(|d| d.insert_temp(id, seen));
        ctx.request_repaint();
    }
    (out, labels)
}

fn labels_id(popup_id: Id) -> Id {
    popup_id.with("gz-ui.menu.labels")
}

/// Where the labels of the rows of the menu `popup_id` started, the last frame it was drawn
/// (`ui.list` names them: a menu with one label column has one x).
#[must_use]
pub fn label_columns(ctx: &Context, popup_id: Id) -> Vec<f32> {
    ctx.data(|d| d.get_temp::<Vec<f32>>(labels_id(popup_id))).unwrap_or_default()
}

/// Whether the rows of the popup drawn with `id` keep an icon column (what its last frame
/// found). For a test and the audit.
#[must_use]
pub fn reserves_icon_column(ctx: &Context, body_id: Id) -> Option<bool> {
    ctx.data(|d| d.get_temp::<bool>(body_id.with("gz-ui.menu.column")))
}

/// A menu kept open in egui's memory, under `anchor`. Open it with `Popup::open_id` or
/// `Popup::toggle_id`; a picked [`menu_item`] closes it, so does a click outside or Esc.
pub fn menu_popup<R>(
    ctx: &Context,
    popup_id: Id,
    anchor: Rect,
    layer: LayerId,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    Popup::new(popup_id, ctx.clone(), PopupAnchor::ParentRect(anchor), layer)
        .kind(PopupKind::Menu)
        .open_memory(None)
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .gap(2.0)
        .frame(popup_frame(ctx))
        .show(|ui| popup_body_labels(ui, add))
        .map(|r| {
            let (out, labels) = r.inner;
            ctx.data_mut(|d| d.insert_temp(labels_id(popup_id), labels));
            out
        })
}

fn requests_id() -> Id {
    Id::unique("gz-ui.menu.popup_requests")
}

/// Asks the widget whose popup is called `name` to open it on its next frame.
pub fn show_popup(ctx: &Context, name: &str) {
    ctx.data_mut(|d| {
        let list = d.get_temp_mut_or_default::<Vec<String>>(requests_id());
        if !list.iter().any(|n| n == name) {
            list.push(name.to_owned());
        }
    });
}

/// Whether a popup called `name` was asked for; the request is taken.
#[must_use]
pub fn take_popup_request(ctx: &Context, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    ctx.data_mut(|d| {
        let list = d.get_temp_mut_or_default::<Vec<String>>(requests_id());
        match list.iter().position(|n| n == name) {
            Some(i) => {
                list.remove(i);
                true
            }
            None => false,
        }
    })
}
