//! Widgets the kit lacks, local to this crate until a second surface wants them
//! (docs/UI_DESIGN.md, section 4: "A widget the kit lacks is written as a local one").

use egui::{
    Align2, CornerRadius, Frame, Key, Margin, Modifiers, Rect, Sense, Shape, Stroke, StrokeKind, TextEdit,
    Ui, Vec2, pos2,
};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Tone, icon_button, metrics, palette};

/// A list's header: its title in the strong face and a + at its right end. Returns whether the
/// + was pressed.
pub fn local_list_header(ui: &mut Ui, title: &str, tip: &str) -> bool {
    let p = palette(ui.ctx());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, metrics::ICON_BUTTON), Sense::hover());
    text::paint(
        ui.painter(),
        &Font::Strong.id(),
        pos2(rect.min.x, rect.center().y),
        Align2::LEFT_CENTER,
        p.text,
        title,
        Some((w - metrics::ICON_BUTTON - 4.0).max(0.0)),
        Clip::End,
    );
    let b = Rect::from_min_size(
        pos2(rect.max.x - metrics::ICON_BUTTON, rect.min.y),
        Vec2::splat(metrics::ICON_BUTTON),
    );
    let mut child =
        ui.new_child(egui::UiBuilder::new().id_salt(("gz-ui-pda.list_header", title)).max_rect(b));
    icon_button(&mut child, Icon::Plus, tip, false, true).clicked()
}

/// The kit's `Section` header; `open` is its state until it is clicked.
pub fn local_section(ui: &mut Ui, id_salt: &str, title: &str, open: bool) -> bool {
    gz_ui::Section::new(id_salt, title).default_open(open).show(ui).open
}

/// A strip of tabs, 26 pt (a panel's tab strip): the selected one in the text colour on the
/// base fill under an accent overline. Returns whether the choice changed.
pub fn local_tab_strip(ui: &mut Ui, id_salt: &str, labels: &[String], current: &mut usize) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let font = Font::Body.id();
    let w = ui.available_width();
    let (strip, _) = ui.allocate_exact_size(Vec2::new(w, metrics::PANEL_TABS), Sense::hover());
    ui.painter().line_segment(
        [pos2(strip.min.x, strip.max.y - 0.5), pos2(strip.max.x, strip.max.y - 0.5)],
        Stroke::new(1.0, p.border_subtle),
    );
    // the tabs share the strip when it is too narrow for their labels
    let wants: Vec<f32> = labels.iter().map(|l| text::width(&ctx, &font, l) + 24.0).collect();
    let total: f32 = wants.iter().sum();
    let shrink = if total > w && total > 0.0 { w / total } else { 1.0 };
    let mut x = strip.min.x;
    let mut changed = false;
    for (i, label) in labels.iter().enumerate() {
        let tw = wants.get(i).copied().unwrap_or(0.0) * shrink;
        let r = Rect::from_min_max(pos2(x, strip.min.y), pos2(x + tw, strip.max.y));
        x += tw;
        let resp = ui.interact(r, ui.make_persistent_id(("gz-ui-pda.tab", id_salt, i)), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, *current == i, label));
        if resp.clicked() && *current != i {
            *current = i;
            changed = true;
        }
        let on = *current == i;
        let painter = ui.painter();
        if on {
            painter.rect_filled(r, CornerRadius::ZERO, p.base);
            painter.rect_filled(
                Rect::from_min_max(r.min, pos2(r.max.x, r.min.y + 2.0)),
                CornerRadius::ZERO,
                p.accent_bright,
            );
        } else if resp.hovered() {
            painter.rect_filled(r, CornerRadius::ZERO, p.hover);
        }
        let shown_cut = text::paint(
            painter,
            &font,
            r.center(),
            Align2::CENTER_CENTER,
            if on { p.text } else { p.text_secondary },
            label,
            Some((tw - 12.0).max(0.0)),
            Clip::End,
        );
        if shown_cut {
            gz_ui::audit::truncated(&ctx, label, label, r);
        }
        gz_ui::audit::control(&ctx, label, true, None, None, r);
    }
    changed
}

/// The kit's `Switch` with its label after it: a setting of the tab that applies at once.
pub fn local_check(ui: &mut Ui, label: &str, value: &mut bool) -> bool {
    ui.push_id(("gz-ui-pda.check", label), |ui| gz_ui::Switch::new(value, label).show(ui).changed()).inner
}

/// What a [`local_code_view`] was asked to do this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodeOut {
    pub changed: bool,
    /// Ctrl+Enter.
    pub apply: bool,
    /// Ctrl+S while the text has the keys.
    pub save: bool,
}

/// A code text: Cascadia Mono, line numbers in a gutter, an error line marked; the field is as
/// tall as the text, so the pane scrolls - the gutter with it - and follows the caret. `goto`
/// scrolls to a line (1-based) once.
pub fn local_code_view(
    ui: &mut Ui,
    id_salt: &str,
    value: &mut String,
    error_line: u32,
    goto: &mut u32,
) -> CodeOut {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let font = Font::Mono.id();
    let line_h = Font::Mono.line();
    let edit_id = ui.make_persistent_id(("gz-ui-pda.code", id_salt));
    let mut out = CodeOut::default();
    // the keys the field would take: Ctrl+Enter applies, Ctrl+S saves
    if ctx.memory(|m| m.has_focus(edit_id)) {
        out.apply = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Enter));
        out.save = ui.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::S));
    }
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.inset);
    let gutter = text::width(&ctx, &font, "00000") + 12.0;
    let mut scroll =
        egui::ScrollArea::both().id_salt(("gz-ui-pda.code_scroll", id_salt)).auto_shrink([false, false]);
    if *goto > 0 {
        scroll = scroll
            .vertical_scroll_offset((4.0 + (*goto as f32 - 1.0) * line_h - rect.height() / 3.0).max(0.0));
        *goto = 0;
    }
    scroll.show(ui, |ui| {
        let under = ui.painter().add(Shape::Noop);
        let lines = 1 + value.bytes().filter(|&b| b == b'\n').count();
        let min_h = (lines as f32 * line_h + 8.0 + 2.0 * line_h).max(rect.height());
        let left = ui.cursor().min.x;
        ui.horizontal_top(|ui| {
            // the gutter is a column of its own: the field starts after it
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(gutter);
            let mut layouter = |ui: &Ui, text_: &dyn egui::TextBuffer, wrap: f32| {
                lua_galley(ui, text_.as_str(), &font, wrap)
            };
            let edit = TextEdit::multiline(value)
                .id(edit_id)
                .font(font.clone())
                .code_editor()
                .layouter(&mut layouter)
                .frame(Frame::NONE)
                .margin(Margin { left: 4, right: 6, top: 4, bottom: 4 })
                .desired_width(f32::INFINITY)
                .min_size(Vec2::new(ui.available_width(), min_h))
                .lock_focus(true);
            let shown = edit.show(ui);
            out.changed = shown.response.response.changed();
            let caret = shown.cursor_range.map(|c| c.primary.index.0);
            let origin = shown.galley_pos;
            let area = shown.response.response.rect;
            // the line of every row: a new line starts after a row that ends one
            let mut line = 1u32;
            let mut starts = true;
            let mut chars = 0usize;
            let mut caret_line = 0u32;
            let painter = ui.painter();
            for row in &shown.galley.rows {
                let r = row.rect().translate(origin.to_vec2());
                let n = row.row.char_count_excluding_newline().0;
                if caret.is_some_and(|c| c >= chars && c <= chars + n) && caret_line == 0 {
                    caret_line = line;
                }
                if starts {
                    if line == error_line {
                        let band = Rect::from_min_max(
                            pos2(left, r.min.y),
                            pos2(area.max.x, r.min.y + line_h.max(r.height())),
                        );
                        painter.set(under, Shape::rect_filled(band, CornerRadius::ZERO, p.tint(Tone::Error)));
                        // the reader's line underlined where its text is, the way an editor marks an error
                        let y = r.min.y + line_h - 2.0;
                        let (x0, x1) = (r.min.x, r.max.x.max(r.min.x + 24.0));
                        let mut points = Vec::new();
                        let mut x = x0;
                        let mut up = true;
                        while x <= x1 {
                            points.push(pos2(x, if up { y - 1.5 } else { y + 1.5 }));
                            x += 3.0;
                            up = !up;
                        }
                        painter.add(Shape::line(points, Stroke::new(1.0, p.error)));
                    }
                    let color = if line == error_line {
                        p.error
                    } else if caret.is_some() && caret_line == line {
                        p.text_secondary
                    } else {
                        p.text_disabled
                    };
                    painter.text(
                        pos2(left + gutter - 8.0, r.min.y),
                        Align2::RIGHT_TOP,
                        line.to_string(),
                        font.clone(),
                        color,
                    );
                }
                chars += n + usize::from(row.ends_with_newline);
                starts = row.ends_with_newline;
                if row.ends_with_newline {
                    line += 1;
                }
            }
            let x = left + gutter - 3.0;
            painter.line_segment(
                [pos2(x, area.min.y), pos2(x, area.max.y.max(rect.max.y))],
                Stroke::new(1.0, p.border_subtle),
            );
        });
    });
    out
}

/// The frame round the row a dragged row would land on.
pub fn local_drop_mark(ui: &Ui, rect: Rect) {
    let p = palette(ui.ctx());
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(metrics::RADIUS),
        Stroke::new(2.0, p.accent_bright),
        StrokeKind::Inside,
    );
}

/// Above this the text is drawn in one colour: colouring is worked out for the visible text of a
/// usual layout, and a file this large is no usual layout.
const COLOUR_LIMIT: usize = 256 * 1024;

/// Above this the coloured text is laid out off the UI thread; until it is, the text shows in one
/// colour.
const COLOUR_SYNC_LIMIT: usize = 8 * 1024;

/// The text laid out for the field: coloured at once when it is short, else coloured on a thread
/// of its own (a layout of thousands of coloured runs takes longer than a frame).
fn lua_galley(ui: &Ui, text_: &str, font: &egui::FontId, wrap: f32) -> std::sync::Arc<egui::Galley> {
    use std::hash::{Hash, Hasher};
    let ctx = ui.ctx().clone();
    if text_.len() <= COLOUR_SYNC_LIMIT {
        let mut job = lua_job(&ctx, text_, font);
        job.wrap.max_width = wrap;
        return ui.fonts_mut(|f| f.layout_job(job));
    }
    let mut h = rustc_hash::FxHasher::default();
    text_.hash(&mut h);
    wrap.to_bits().hash(&mut h);
    let want = h.finish();
    // one slot: the galley of the last text coloured, and the text being coloured now
    let slot = egui::Id::unique("gz-ui-pda.lua_galley");
    let pending = slot.with("pending");
    if let Some((k, g)) = ctx.data(|d| d.get_temp::<(u64, std::sync::Arc<egui::Galley>)>(slot))
        && k == want
    {
        return g;
    }
    if ctx.data(|d| d.get_temp::<u64>(pending)) != Some(want) {
        ctx.data_mut(|d| d.insert_temp(pending, want));
        let (c, t, f) = (ctx.clone(), text_.to_owned(), font.clone());
        let _ = std::thread::Builder::new().name("gz-ui-pda.colour".to_owned()).spawn(move || {
            let mut job = lua_job(&c, &t, &f);
            job.wrap.max_width = wrap;
            let g = c.fonts_mut(|fonts| fonts.layout_job(job));
            c.data_mut(|d| d.insert_temp(slot, (want, g)));
            c.request_repaint();
        });
    }
    let p = palette(&ctx);
    let mut plain = egui::text::LayoutJob::simple(text_.to_owned(), font.clone(), p.text, wrap);
    plain.wrap.max_width = wrap;
    ui.fonts_mut(|f| f.layout_job(plain))
}

/// The text as Lua data in four colours of the palette: keys, strings, numbers, comments; the
/// rest in the text colour. The job is kept per text, so a frame that changes nothing lexes
/// nothing.
fn lua_job(ctx: &egui::Context, text_: &str, font: &egui::FontId) -> egui::text::LayoutJob {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    text_.hash(&mut h);
    let want = h.finish();
    let key = egui::Id::unique("gz-ui-pda.lua_job");
    if let Some((k, job)) = ctx.data(|d| d.get_temp::<(u64, std::sync::Arc<egui::text::LayoutJob>)>(key))
        && k == want
    {
        return (*job).clone();
    }
    let p = palette(ctx);
    let mut job = egui::text::LayoutJob::default();
    let fmt = |color| egui::TextFormat { font_id: font.clone(), color, ..egui::TextFormat::default() };
    if text_.len() > COLOUR_LIMIT {
        job.append(text_, 0.0, fmt(p.text));
    } else {
        for (span, kind) in lua_spans(text_) {
            let color = match kind {
                LuaToken::Key => p.accent_bright,
                LuaToken::Str => p.success,
                LuaToken::Number => p.warning,
                LuaToken::Comment => p.text_muted,
                LuaToken::Plain => p.text,
            };
            job.append(span, 0.0, fmt(color));
        }
    }
    ctx.data_mut(|d| d.insert_temp(key, (want, std::sync::Arc::new(job.clone()))));
    job
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LuaToken {
    Key,
    Str,
    Number,
    Comment,
    Plain,
}

/// The text cut into its tokens, in order; together they are the whole text.
fn lua_spans<'t>(t: &'t str) -> Vec<(&'t str, LuaToken)> {
    let b = t.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut plain) = (0usize, 0usize);
    let push = |out: &mut Vec<(&'t str, LuaToken)>, from: usize, to: usize, kind| {
        if let Some(s) = t.get(from..to)
            && !s.is_empty()
        {
            out.push((s, kind));
        }
    };
    while i < b.len() {
        let c = b[i];
        let start = i;
        let kind = if c == b'-' && b.get(i + 1) == Some(&b'-') {
            // a block comment --[[ ... ]] or one to the end of the line
            if b.get(i + 2) == Some(&b'[') && b.get(i + 3) == Some(&b'[') {
                i = t.get(i + 4..).and_then(|r| r.find("]]")).map_or(b.len(), |e| i + 4 + e + 2);
            } else {
                i = t.get(i..).and_then(|r| r.find('\n')).map_or(b.len(), |e| i + e);
            }
            LuaToken::Comment
        } else if c == b'"' || c == b'\'' {
            i += 1;
            while i < b.len() && b[i] != c && b[i] != b'\n' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i = (i + 1).min(b.len());
            LuaToken::Str
        } else if c == b'[' && b.get(i + 1) == Some(&b'[') {
            i = t.get(i + 2..).and_then(|r| r.find("]]")).map_or(b.len(), |e| i + 2 + e + 2);
            LuaToken::Str
        } else if c.is_ascii_digit()
            || (c == b'-'
                && b.get(i + 1).is_some_and(u8::is_ascii_digit)
                && !b.get(i.wrapping_sub(1)).is_some_and(|p| p.is_ascii_alphanumeric()))
        {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') {
                i += 1;
            }
            LuaToken::Number
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            // a name followed by '=' (not '==') is a key of the table
            let mut j = i;
            while j < b.len() && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
            if b.get(j) == Some(&b'=') && b.get(j + 1) != Some(&b'=') {
                LuaToken::Key
            } else {
                LuaToken::Plain
            }
        } else {
            i += 1;
            while i < b.len() && !t.is_char_boundary(i) {
                i += 1;
            }
            LuaToken::Plain
        };
        if kind == LuaToken::Plain {
            continue;
        }
        push(&mut out, plain, start, LuaToken::Plain);
        push(&mut out, start, i, kind);
        plain = i;
    }
    push(&mut out, plain, b.len(), LuaToken::Plain);
    out
}

#[cfg(test)]
mod lua_tests {
    use super::{LuaToken, lua_spans};

    #[test]
    fn lua_data_in_four_colours() {
        let t = "-- note\nreturn { id = \"x\", n = -12, [[raw]], name == 1 }";
        let spans = lua_spans(t);
        let joined: String = spans.iter().map(|(s, _)| *s).collect();
        assert_eq!(joined, t);
        let kind = |s: &str| spans.iter().find(|(x, _)| *x == s).map(|(_, k)| *k);
        assert_eq!(kind("-- note"), Some(LuaToken::Comment));
        assert_eq!(kind("id"), Some(LuaToken::Key));
        assert_eq!(kind("\"x\""), Some(LuaToken::Str));
        assert_eq!(kind("-12"), Some(LuaToken::Number));
        assert_eq!(kind("[[raw]]"), Some(LuaToken::Str));
        assert!(kind("name").is_none());
    }
}
