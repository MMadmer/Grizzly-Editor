//! [`Section`] - the one fold header of the kit (docs/UI_DESIGN.md, section 4).
//!
//! A chevron, the title in Strong, an optional count after it, a trailing slot for small actions
//! (+, a menu); no fill, 28 pt tall. Its open state is kept by its key in the layout store, so a
//! section folded once stays folded next time the surface is drawn - and in the next run.

use egui::{Align2, Context, Rect, Response, Sense, Ui, Vec2, pos2};

use crate::audit;
use crate::button::icon_button_sized;
use crate::font::Font;
use crate::i18n::Tx;
use crate::icons::{self, Icon};
use crate::layout;
use crate::text::{self, Clip};
use crate::theme::{metrics, palette};

/// What a section header answers.
#[derive(Clone, Copy, Debug)]
pub struct SectionResponse {
    pub open: bool,
    /// The header was clicked and the section folded or unfolded.
    pub toggled: bool,
    /// The index of the trailing action pressed.
    pub action: Option<usize>,
    pub rect: Rect,
}

/// A fold header. `key` is stable across frames and runs: the layout store keeps
/// `section.<key>`.
#[must_use = "a section does nothing until it is shown: .show(ui) or .body(ui, ..)"]
pub struct Section<'a> {
    key: &'a str,
    title: Tx<'a>,
    count: Option<Tx<'a>>,
    open: bool,
    actions: Vec<(Icon, Tx<'a>)>,
    tip: Option<Tx<'a>>,
    /// What the section is waiting for: a spinner stands where the count would.
    busy: Option<Tx<'a>>,
}

fn store_key(key: &str) -> String {
    format!("section.{key}")
}

/// Whether the section of this key is open (what the author left it as, else `default`).
#[must_use]
pub fn is_open(ctx: &Context, key: &str, default: bool) -> bool {
    layout::get(ctx, &store_key(key), if default { 1.0 } else { 0.0 }) >= 0.5
}

/// Opens or folds the section of this key, as a click on its header would.
pub fn set_open(ctx: &Context, key: &str, open: bool) {
    layout::set(ctx, &store_key(key), if open { 1.0 } else { 0.0 });
}

impl<'a> Section<'a> {
    pub fn new(key: &'a str, title: impl Into<Tx<'a>>) -> Self {
        Self { key, title: title.into(), count: None, open: true, actions: Vec::new(), tip: None, busy: None }
    }

    /// Its state until the author folds or opens it. Open unless told.
    pub fn default_open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// A count after the title, muted: "Variables 11".
    pub fn count(mut self, count: impl Into<Tx<'a>>) -> Self {
        let count = count.into();
        self.count = (!count.is_empty()).then_some(count);
        self
    }

    /// The section's content is still being read: the kit's spinner stands after the title where
    /// the count would, and an audited frame reports `what` as loading (`ui.audit`), the section
    /// open or folded. A count is never shown with it - a number not known yet is no number.
    pub fn busy(mut self, what: impl Into<Tx<'a>>) -> Self {
        self.busy = Some(what.into());
        self.count = None;
        self
    }

    /// A small icon button in the trailing slot (+ adds, the menu glyph opens a menu); the
    /// first added stands at the right end.
    pub fn action(mut self, icon: Icon, tip: impl Into<Tx<'a>>) -> Self {
        self.actions.push((icon, tip.into()));
        self
    }

    /// The header's hover hint.
    pub fn tip(mut self, tip: impl Into<Tx<'a>>) -> Self {
        self.tip = Some(tip.into());
        self
    }

    /// Draws the header; the caller draws the body while `open`.
    pub fn show(self, ui: &mut Ui) -> SectionResponse {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let title = self.title.get(&ctx);
        let title: &str = &title;
        let count = self.count.as_ref().map(|c| c.get(&ctx));
        let tip = self.tip.as_ref().map(|t| t.get(&ctx));
        let mut open = is_open(&ctx, self.key, self.open);
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), metrics::SECTION_HEADER), Sense::hover());
        let slot = metrics::ICON_BUTTON;
        let actions_w = self.actions.len() as f32 * (slot + 2.0);
        let head = Rect::from_min_max(rect.min, pos2(rect.max.x - actions_w, rect.max.y));
        let id = ui.make_persistent_id(("gz-ui.section", self.key));
        let mut resp: Response = ui.interact(head, id, Sense::click());
        let toggled = resp.clicked();
        if toggled {
            open = !open;
            set_open(&ctx, self.key, open);
        }
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let hot = resp.hovered();
            if hot {
                painter.line_segment(
                    [pos2(rect.min.x, rect.max.y - 0.5), pos2(head.max.x, rect.max.y - 0.5)],
                    egui::Stroke::new(1.0, p.border_subtle),
                );
            }
            let chevron = Rect::from_min_size(
                pos2(rect.min.x + 2.0, rect.center().y - metrics::GLYPH_STATUS * 0.5),
                Vec2::splat(metrics::GLYPH_STATUS),
            );
            icons::draw(
                painter,
                if open { Icon::ChevronDown } else { Icon::ChevronRight },
                chevron,
                if hot { p.text } else { p.text_secondary },
            );
            let x = chevron.max.x + 6.0;
            let font = Font::Strong.id();
            let mark = metrics::GLYPH;
            let count_w = if self.busy.is_some() {
                mark + metrics::S8
            } else {
                count.as_deref().map_or(0.0, |c| text::measure(&ctx, &Font::Small.id(), c) + metrics::S8)
            };
            let room = (head.max.x - x - count_w - 4.0).max(0.0);
            text::paint(
                painter,
                &font,
                pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                p.text,
                self.title.borrow(),
                Some(room),
                Clip::End,
            );
            if self.busy.is_some() {
                let w = text::measure(&ctx, &font, title).min(room);
                let at = pos2(x + w + metrics::S8 + mark * 0.5, rect.center().y);
                crate::state::spinner_at(&ctx, painter, Rect::from_center_size(at, Vec2::splat(mark)));
            }
            if let Some(count) = count.as_deref() {
                let w = text::measure(&ctx, &font, title).min(room);
                painter.text(
                    pos2(x + w + metrics::S8, rect.center().y),
                    Align2::LEFT_CENTER,
                    count,
                    Font::Small.id(),
                    p.text_muted,
                );
            }
        }
        let mut action = None;
        let mut x = rect.max.x;
        for (i, (icon, tip)) in self.actions.iter().enumerate() {
            x -= slot;
            let cell = Rect::from_min_size(pos2(x, rect.center().y - slot * 0.5), Vec2::splat(slot));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(cell).id_salt(("action", i)));
            if icon_button_sized(&mut child, *icon, tip.borrow(), false, true, slot, metrics::GLYPH_ROW)
                .clicked()
            {
                action = Some(i);
            }
            x -= 2.0;
        }
        audit::control(&ctx, title, true, None, tip.as_deref(), rect);
        if let Some(what) = &self.busy {
            audit::loading(&ctx, &what.get(&ctx), rect);
        }
        if let Some(tip) = tip {
            resp = resp.on_hover_text(tip.as_ref());
        }
        let _ = resp;
        SectionResponse { open, toggled, action, rect }
    }

    /// The header and, while open, the body under it, indented by nothing (a section is a
    /// heading, not a box).
    pub fn body<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> (SectionResponse, Option<R>) {
        let head = self.show(ui);
        let inner = head.open.then(|| add(ui));
        (head, inner)
    }
}
