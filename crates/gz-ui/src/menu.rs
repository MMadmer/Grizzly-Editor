//! Menus (docs/UI_DESIGN.md, section 4, "Menus").
//!
//! - [`menu_item`] with [`MenuItem`] - a row of a menu: an icon, the label, keycaps flush right
//!   in their column (at least 24 pt after the longest label), a danger item in the error colour.
//! - [`popup_frame`], [`popup_body`], [`menu_popup`] - the theme's popup: overlay fill, padding,
//!   rows that touch.
//! - [`show_popup`] / [`take_popup_request`] - a popup asked for by name (MCP `ui_popup`); the
//!   widget that owns the name opens it on its next frame.

use egui::{
    Align, Align2, Context, CornerRadius, Frame, Id, LayerId, Layout, Margin, Popup, PopupAnchor,
    PopupCloseBehavior, PopupKind, Rect, Role, Sense, Stroke, Ui, UiKind, Vec2, WidgetInfo, pos2,
};

use crate::audit;
use crate::button::{keycaps, keycaps_width};
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

/// A menu row, optionally with a submenu chevron and a tooltip: the overflow menu's rows.
pub(crate) fn row(ui: &mut Ui, label: &str, item: MenuItem<'_>, submenu: bool, tip: &str) -> bool {
    let p = palette(ui.ctx());
    let font = Font::Body.id();
    let has_slot = item.icon != Icon::None || item.selected;
    let label_w = text::width(ui.ctx(), &font, label);
    let keys_w = keycaps_width(ui, item.keys);
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
            let (icon, color) = match (item.icon, item.selected) {
                (Icon::None, _) => (Icon::Check, p.accent_bright),
                (icon, true) => (icon, p.accent_bright),
                (icon, false) if !item.enabled => (icon, p.text_disabled),
                (icon, false) if item.danger => (icon, p.error),
                (icon, false) => (icon, p.text_secondary),
            };
            icons::draw(painter, icon, glyph, color);
            x += metrics::GLYPH + 8.0;
        }
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
            keycaps(
                ui,
                pos2(rect.max.x - 8.0 - if submenu { 16.0 } else { 0.0 }, rect.center().y),
                item.keys,
                item.enabled,
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
    let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };
    let picked = resp.clicked() && item.enabled;
    if picked && (ui.stack().contained_in(UiKind::Menu) || ui.stack().contained_in(UiKind::Popup)) {
        ui.close();
    }
    picked
}

/// The frame of a menu or popup: overlay fill, the border, 6 pt corners, 4 pt padding.
pub fn popup_frame(ctx: &Context) -> Frame {
    let p = palette(ctx);
    Frame::new()
        .fill(p.overlay)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(metrics::RADIUS_POPUP))
        .inner_margin(Margin::same(4))
}

/// The body of a popup: rows that touch, at least 10 em wide, laid out top-down.
pub fn popup_body<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 0.0);
        ui.set_min_width(em(10.0));
        add(ui)
    })
    .inner
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
        .show(|ui| popup_body(ui, add))
        .map(|r| r.inner)
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
