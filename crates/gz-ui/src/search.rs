//! The search field and the one text filter of the editor (docs/UI_DESIGN.md, section 4).
//!
//! - [`SearchField`] - a magnifier, the text, a clear cross; Esc clears, Ctrl+F focuses.
//! - [`matches()`] - space-separated terms that must all occur, ignoring case (Cyrillic too), with
//!   "ё" read as "е".

use egui::{CornerRadius, Frame, Id, Key, Margin, Modifiers, Rect, Response, Sense, Stroke, StrokeKind, TextEdit, Ui, Vec2, pos2};

use crate::audit;
use crate::button::icon_button_sized;
use crate::font::Font;
use crate::icons::{self, Icon};
use crate::theme::{metrics, palette};
use crate::units::em;

/// The folded form of a character: lower case, "ё" as "е".
fn fold(c: char) -> char {
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    let lower = c.to_lowercase().next().unwrap_or(c);
    match lower {
        'ё' => 'е',
        other => other,
    }
}

/// Whether `term` occurs in `text`, both compared folded. No allocation.
fn contains_folded(text: &str, term: &str) -> bool {
    if term.is_empty() {
        return true;
    }
    let mut starts = text.char_indices();
    loop {
        let rest = match starts.next() {
            Some((i, _)) => text.get(i..).unwrap_or(""),
            None => return false,
        };
        let mut hay = rest.chars();
        let mut hit = true;
        for t in term.chars() {
            match hay.next() {
                Some(h) if fold(h) == fold(t) => {}
                Some(_) => {
                    hit = false;
                    break;
                }
                None => return false,
            }
        }
        if hit {
            return true;
        }
    }
}

/// Whether `text` matches `query`: every space-separated term of the query occurs in the text,
/// ignoring case (Cyrillic included), "ё" read as "е". An empty query matches everything.
#[must_use]
pub fn matches(query: &str, text: &str) -> bool {
    query.split_whitespace().all(|term| contains_folded(text, term))
}

/// A search field. Returns its response; `changed()` says the text changed.
#[must_use = "a search field does nothing until it is shown: .show(ui)"]
pub struct SearchField<'a> {
    text: &'a mut String,
    hint: &'a str,
    min_em: f32,
    width: Option<f32>,
    id_salt: Option<&'a str>,
}

impl<'a> SearchField<'a> {
    pub fn new(text: &'a mut String) -> Self {
        Self { text, hint: "Search", min_em: 8.0, width: None, id_salt: None }
    }

    /// The hint shown while it is empty; also its name for the audit.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = hint;
        self
    }

    /// The narrowest it gets, in em.
    pub fn min_em(mut self, min_em: f32) -> Self {
        self.min_em = min_em;
        self
    }

    /// Its width; by default what the row has left.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Tells two fields of one surface apart; by default the hint does.
    pub fn id_salt(mut self, salt: &'a str) -> Self {
        self.id_salt = Some(salt);
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let p = palette(ui.ctx());
        let width = self.width.unwrap_or_else(|| ui.available_width()).max(em(self.min_em));
        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, metrics::FRAME), Sense::hover());
        let id = ui.make_persistent_id(("gz-ui.search", self.id_salt.unwrap_or(self.hint)));
        let edit_id = id.with("edit");

        // Ctrl+F goes to the first search field drawn in a frame
        let ctx = ui.ctx().clone();
        let frame = ctx.cumulative_frame_nr();
        let ctrl_f_id = Id::unique("gz-ui.search.ctrl_f");
        let claimed = ctx.data(|d| d.get_temp::<u64>(ctrl_f_id)) == Some(frame);
        if !claimed && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::F)) {
            ctx.data_mut(|d| d.insert_temp(ctrl_f_id, frame));
            ctx.memory_mut(|m| m.request_focus(edit_id));
        }

        let focused = ctx.memory(|m| m.has_focus(edit_id));
        let hovered = ui.rect_contains_pointer(rect);
        if ui.is_rect_visible(rect) {
            let fill = if hovered || focused { p.input_hover } else { p.input };
            ui.painter().rect_filled(rect, CornerRadius::same(metrics::RADIUS), fill);
            if focused {
                ui.painter().rect_stroke(rect, CornerRadius::same(metrics::RADIUS), Stroke::new(1.0, p.accent_bright), StrokeKind::Inside);
            }
            let glass = Rect::from_min_size(pos2(rect.min.x + 8.0, rect.center().y - 7.0), Vec2::splat(metrics::GLYPH_ROW));
            icons::draw(ui.painter(), Icon::Search, glass, if focused { p.text } else { p.text_muted });
        }

        // the padding hosts the magnifier on the left and the clear cross on the right
        let side = 8.0 + metrics::GLYPH_ROW + 6.0;
        let clear_w: f32 = if self.text.is_empty() { 0.0 } else { 22.0 };
        let edit_rect = Rect::from_min_max(pos2(rect.min.x + side, rect.min.y), pos2(rect.max.x - clear_w.max(8.0), rect.max.y));
        let mut resp = ui.put(
            edit_rect,
            TextEdit::singleline(&mut *self.text)
                .id(edit_id)
                .frame(Frame::NONE)
                .margin(Margin::symmetric(0, 4))
                .font(Font::Body.id())
                .hint_text(self.hint)
                .desired_width(edit_rect.width()),
        );

        // Esc clears: egui lets the field go on Esc, so the field that just lost the focus asks
        if resp.lost_focus() && ctx.input(|i| i.key_pressed(Key::Escape)) && !self.text.is_empty() {
            self.text.clear();
            resp.mark_changed();
        }
        if !self.text.is_empty() {
            let side_c = 20.0;
            let clear_rect = Rect::from_min_size(pos2(rect.max.x - side_c - 2.0, rect.center().y - side_c * 0.5), Vec2::splat(side_c));
            let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id.with("clear")).max_rect(clear_rect));
            if icon_button_sized(&mut child, Icon::Clear, "Clear", false, true, side_c, metrics::GLYPH_STATUS).clicked() {
                self.text.clear();
                resp.mark_changed();
            }
        }
        audit::control(&ctx, self.hint, true, None, Some("Esc clears, Ctrl+F focuses"), rect);
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn empty_query_matches_everything() {
        assert!(matches("", "anything"));
        assert!(matches("   ", ""));
    }

    #[test]
    fn every_term_must_occur() {
        assert!(matches("lev map", "Level Map Editor"));
        assert!(!matches("lev quest", "Level Map Editor"));
        assert!(matches("map lev", "Level Map Editor"));
    }

    #[test]
    fn case_is_ignored_cyrillic_too() {
        assert!(matches("КВЕСТ", "Новый квест"));
        assert!(matches("новый", "НОВЫЙ КВЕСТ"));
        assert!(matches("ABC", "xabcx"));
    }

    #[test]
    fn yo_reads_as_ye() {
        assert!(matches("ёж", "Ежик"));
        assert!(matches("еж", "Ёжик"));
        assert!(matches("ЁЖ", "ежик"));
    }

    #[test]
    fn no_match_past_the_end() {
        assert!(!matches("abcd", "abc"));
        assert!(!matches("x", ""));
    }
}
