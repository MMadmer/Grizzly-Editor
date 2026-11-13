//! [`Section`] - the one fold header of the kit (docs/UI_DESIGN.md, section 4).
//!
//! A chevron, the title in Strong, an optional count after it, a trailing slot for small actions
//! (+, a menu); no fill, 28 pt tall. Its open state is kept by its key in the layout store, so a
//! section folded once stays folded next time the surface is drawn - and in the next run.

use egui::{Align2, Context, Rect, Response, Sense, Ui, Vec2, pos2};

use crate::audit;
use crate::button::icon_button_sized;
use crate::font::Font;
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
    title: &'a str,
    count: Option<&'a str>,
    open: bool,
    actions: Vec<(Icon, &'a str)>,
    tip: Option<&'a str>,
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
    pub fn new(key: &'a str, title: &'a str) -> Self {
        Self { key, title, count: None, open: true, actions: Vec::new(), tip: None }
    }

    /// Its state until the author folds or opens it. Open unless told.
    pub fn default_open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// A count after the title, muted: "Variables 11".
    pub fn count(mut self, count: &'a str) -> Self {
        self.count = (!count.is_empty()).then_some(count);
        self
    }

    /// A small icon button in the trailing slot (+ adds, the menu glyph opens a menu); the
    /// first added stands at the right end.
    pub fn action(mut self, icon: Icon, tip: &'a str) -> Self {
        self.actions.push((icon, tip));
        self
    }

    /// The header's hover hint.
    pub fn tip(mut self, tip: &'a str) -> Self {
        self.tip = Some(tip);
        self
    }

    /// Draws the header; the caller draws the body while `open`.
    pub fn show(self, ui: &mut Ui) -> SectionResponse {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
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
            let count_w = self.count.map_or(0.0, |c| text::width(&ctx, &Font::Small.id(), c) + metrics::S8);
            let room = (head.max.x - x - count_w - 4.0).max(0.0);
            text::paint(
                painter,
                &font,
                pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                p.text,
                self.title,
                Some(room),
                Clip::End,
            );
            if let Some(count) = self.count {
                let w = text::width(&ctx, &font, self.title).min(room);
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
            if icon_button_sized(&mut child, *icon, tip, false, true, slot, metrics::GLYPH_ROW).clicked() {
                action = Some(i);
            }
            x -= 2.0;
        }
        audit::control(&ctx, self.title, true, None, self.tip, rect);
        if let Some(tip) = self.tip {
            resp = resp.on_hover_text(tip);
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
