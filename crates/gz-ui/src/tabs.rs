//! [`PanelTab`] and [`tab_strip`] - the one panel tab of the kit (docs/UI_DESIGN.md, section 4).
//!
//! A tab is a label on the 26 pt panel strip: the tab in front on the base fill under an accent
//! overline, the others in the secondary text colour with a hover wash. Two settings of the one
//! widget exist: a [`TabStyle::Docked`] tab of a dock slot (rounded top corners, the label from
//! the left, a cross on hover and on the tab in front, laid out by the dock that owns the
//! strip), and a [`TabStyle::Flat`] tab of a panel's own strip ([`tab_strip`]: square, the label
//! centred, the tabs sharing the width when it is too narrow for their labels).

use egui::{Align2, CornerRadius, Id, Rect, Role, Sense, Stroke, Ui, Vec2, WidgetInfo, pos2};

use crate::icons::{self, Icon};
use crate::text::{self, Clip};
use crate::theme::{metrics, palette};
use crate::{Font, audit};

/// Which of the two tabs it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabStyle {
    /// A tab of a dock slot: rounded top corners, the label from the left, a cross.
    #[default]
    Docked,
    /// A tab of a panel's own strip: square, the label centred.
    Flat,
}

/// What a tab was asked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TabResponse {
    pub clicked: bool,
    /// The cross was pressed.
    pub close: bool,
    /// Right-clicked: the tab's menu.
    pub menu: bool,
}

/// The padding of a docked tab: left of the label, its gap to the cross, the cross's slot.
const DOCKED_PAD: f32 = 12.0 + 4.0 + 18.0;
/// The padding of a flat tab: 12 pt each side of the label.
const FLAT_PAD: f32 = 24.0;

/// The width a tab takes for its label: `closable` leaves room for the cross.
#[must_use]
pub fn tab_width(ctx: &egui::Context, label: &str, closable: bool) -> f32 {
    text::width(ctx, &Font::Body.id(), label) + if closable { DOCKED_PAD } else { FLAT_PAD }
}

/// A panel tab in `rect` (pt); `id` keeps its click.
#[must_use = "a tab does nothing until it is shown: .show(ui)"]
pub struct PanelTab<'a> {
    id: Id,
    rect: Rect,
    label: &'a str,
    front: bool,
    closable: bool,
    style: TabStyle,
}

impl<'a> PanelTab<'a> {
    pub fn new(id: Id, rect: Rect, label: &'a str) -> Self {
        Self { id, rect, label, front: false, closable: false, style: TabStyle::Docked }
    }

    /// The tab in front of its strip: the base fill, the text colour, the accent overline.
    pub fn front(mut self, front: bool) -> Self {
        self.front = front;
        self
    }

    /// A cross at its right end, on hover and on the tab in front.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    pub fn style(mut self, style: TabStyle) -> Self {
        self.style = style;
        self
    }

    pub fn show(self, ui: &mut Ui) -> TabResponse {
        let Self { id, rect, label, front, closable, style } = self;
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let flat = style == TabStyle::Flat;
        let resp = ui.interact(rect, id, Sense::click());
        resp.widget_info(|| WidgetInfo::selected(Role::Tab, true, front, label));
        let cross_rect =
            Rect::from_min_size(pos2(rect.max.x - 20.0, rect.center().y - 8.0), Vec2::splat(16.0));
        let cross_resp = closable.then(|| ui.interact(cross_rect, id.with("close"), Sense::click()));
        let hovered = resp.hovered() || cross_resp.as_ref().is_some_and(egui::Response::hovered);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let radius = if flat { CornerRadius::ZERO } else { CornerRadius { nw: 3, ne: 3, sw: 0, se: 0 } };
            let fill = if front {
                Some(p.base)
            } else if hovered {
                Some(p.hover)
            } else {
                // a docked tab lies on the dock's strip, a flat one on its panel
                (!flat).then_some(p.chrome)
            };
            if let Some(fill) = fill {
                painter.rect_filled(rect, radius, fill);
            }
            if front {
                painter.rect_filled(
                    Rect::from_min_size(rect.min, Vec2::new(rect.width(), 2.0)),
                    CornerRadius::ZERO,
                    p.accent_bright,
                );
            }
            let show_cross = closable && (hovered || front);
            let color = if front { p.text } else { p.text_secondary };
            let cut = if flat {
                text::paint(
                    painter,
                    &Font::Body.id(),
                    rect.center(),
                    Align2::CENTER_CENTER,
                    color,
                    label,
                    Some((rect.width() - 12.0).max(0.0)),
                    Clip::End,
                )
            } else {
                let room = rect.width() - 12.0 - if show_cross { 22.0 } else { 6.0 };
                text::paint(
                    painter,
                    &Font::Body.id(),
                    pos2(rect.min.x + 10.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    color,
                    label,
                    Some(room.max(0.0)),
                    Clip::End,
                )
            };
            if cut {
                audit::truncated(&ctx, label, label, rect);
            }
            if show_cross && let Some(c) = &cross_resp {
                if c.hovered() {
                    painter.rect_filled(cross_rect, CornerRadius::same(3), p.pressed);
                }
                icons::draw(
                    painter,
                    Icon::Close,
                    cross_rect.shrink(3.0),
                    if c.hovered() { p.text } else { p.text_secondary },
                );
            }
        }
        audit::control(&ctx, label, true, None, None, rect);
        let close = cross_resp.as_ref().is_some_and(egui::Response::clicked);
        if let Some(c) = cross_resp {
            c.on_hover_text("Close");
        }
        TabResponse { clicked: resp.clicked() && !close, close, menu: resp.secondary_clicked() }
    }
}

/// The widths of the tabs of a flat strip `width` wide: each wants its label and the padding, and
/// they all give way by the same share when together they want more than the strip has.
#[must_use]
pub fn flat_widths(wants: &[f32], width: f32) -> Vec<f32> {
    let total: f32 = wants.iter().sum();
    let shrink = if total > width && total > 0.0 { width / total } else { 1.0 };
    wants.iter().map(|w| w * shrink).collect()
}

/// A strip of flat tabs across `ui`, 26 pt, a hairline under it: a panel's own pages. Returns
/// whether the choice changed.
pub fn tab_strip(ui: &mut Ui, id_salt: &str, labels: &[String], current: &mut usize) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let w = ui.available_width();
    let (strip, _) = ui.allocate_exact_size(Vec2::new(w, metrics::PANEL_TABS), Sense::hover());
    ui.painter().line_segment(
        [pos2(strip.min.x, strip.max.y - 0.5), pos2(strip.max.x, strip.max.y - 0.5)],
        Stroke::new(1.0, p.border_subtle),
    );
    let wants: Vec<f32> = labels.iter().map(|l| tab_width(&ctx, l, false)).collect();
    let widths = flat_widths(&wants, w);
    let mut x = strip.min.x;
    let mut changed = false;
    for (i, (label, tw)) in labels.iter().zip(widths).enumerate() {
        let r = Rect::from_min_max(pos2(x, strip.min.y), pos2(x + tw, strip.max.y));
        x += tw;
        let resp = PanelTab::new(ui.make_persistent_id(("gz-ui.tab", id_salt, i)), r, label)
            .front(*current == i)
            .style(TabStyle::Flat)
            .show(ui);
        if resp.clicked && *current != i {
            *current = i;
            changed = true;
        }
    }
    changed
}
