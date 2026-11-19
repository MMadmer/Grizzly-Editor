//! Code text: the languages the editor colours, the coloured layout of a text in one of them
//! ([`highlight`]) and the code editor ([`code_view`]): Mono, a gutter of line numbers, the
//! caret's line, problem lines in their tone, find marks, the bracket pair, and the keys a code
//! editor has - indent and outdent, comment, duplicate, move, Enter that keeps the indent,
//! completion. The Text tab's editor, the PDA tab's Source pane and the code blocks of a
//! Markdown preview all draw through here.
//!
//! What a frame costs: the tokens of a text are made once per text (kept by its hash), the
//! layout is egui's galley cache (one entry per line, so an edit lays out its own line again),
//! a text over 16 KB is laid out a few kilobytes a frame before it is shown, and only the line
//! numbers and marks on screen are painted.

pub mod edit;
pub mod lex;

use std::ops::Range;
use std::sync::Arc;

use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::text_edit::TextEditState;
use egui::{
    Align, Align2, Context, CornerRadius, FontId, Frame, Id, Key, Margin, Modifiers, Pos2, Rect, Sense,
    Shape, Stroke, StrokeKind, TextEdit, TextFormat, Ui, Vec2, pos2,
};

use self::edit::Sel;
use self::lex::Tok;
use crate::font::Font;
use crate::theme::{Palette, Tone, palette};

/// A language the editor colours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CodeLang {
    /// Text without a language: one colour.
    #[default]
    Plain,
    /// A Lua program: a game script, a shader script.
    Lua,
    /// Lua data (`return { ... }`): the editor's own assets.
    LuaData,
    /// LTX: the game's configs, `mod.ltx`, a module's patches.
    Ltx,
    /// XML and HTML.
    Xml,
    Markdown,
    Json,
    /// A unified diff.
    Diff,
}

impl CodeLang {
    /// The language of a file by its extension (with or without the dot, any case).
    #[must_use]
    pub fn from_extension(ext: &str) -> Self {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "script" | "lua" | "s" => Self::Lua,
            "nqasset" | "behasset" | "pdaasset" | "npcasset" | "itemasset" | "tradeasset" | "lootasset"
            | "recipeasset" | "meshasset" | "texasset" | "sndasset" | "scene" | "grizzly" => Self::LuaData,
            "ltx" | "ltxp" | "ini" | "cfg" => Self::Ltx,
            "xml" | "xmlp" | "html" | "htm" | "svg" => Self::Xml,
            "md" | "markdown" => Self::Markdown,
            "json" => Self::Json,
            "diff" | "patch" => Self::Diff,
            _ => Self::Plain,
        }
    }

    /// The language a Markdown code fence names (` ```lua `), by its first word.
    #[must_use]
    pub fn from_fence(info: &str) -> Self {
        let word = info.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        match word.as_str() {
            "lua" | "script" | "luajit" => Self::Lua,
            "ltx" | "ini" | "ltxp" => Self::Ltx,
            "xml" | "html" => Self::Xml,
            "md" | "markdown" => Self::Markdown,
            "json" => Self::Json,
            "diff" | "patch" => Self::Diff,
            _ => Self::Plain,
        }
    }

    /// What the status bar calls it.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Plain => "Text",
            Self::Lua => "Lua",
            Self::LuaData => "Lua Data",
            Self::Ltx => "LTX",
            Self::Xml => "XML",
            Self::Markdown => "Markdown",
            Self::Json => "JSON",
            Self::Diff => "Diff",
        }
    }

    /// The word for answers and tests: `lua`, `ltx`, ...
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Lua => "lua",
            Self::LuaData => "lua_data",
            Self::Ltx => "ltx",
            Self::Xml => "xml",
            Self::Markdown => "markdown",
            Self::Json => "json",
            Self::Diff => "diff",
        }
    }
}

/// Above this a text is drawn in one colour: colouring is worked out for the text people edit,
/// and a file this large is no such text.
pub const COLOUR_LIMIT: usize = 256 * 1024;
/// Above this a text is laid out over several frames before the field shows it.
const WARM_FROM: usize = 16 * 1024;
/// What one frame lays out of it.
const WARM_STEP: usize = 8 * 1024;

fn colour(p: &Palette, tok: Tok) -> egui::Color32 {
    let s = &p.syntax;
    match tok {
        Tok::Keyword => s.keyword,
        Tok::Str => s.string,
        Tok::Number => s.number,
        Tok::Comment => s.comment,
        Tok::Function => s.function,
        Tok::Name => s.name,
        Tok::Tag => s.tag,
        Tok::Attribute => s.attribute,
        Tok::Punct => s.punct,
        Tok::Added => s.added,
        Tok::Removed => s.removed,
    }
}

/// The jobs made lately, by the hash of what they were made from: several views (the Text tab,
/// the PDA's Source, a Markdown preview's blocks) colour at once.
#[derive(Clone, Default)]
struct JobCache {
    jobs: Vec<(u64, Arc<LayoutJob>)>,
}

const JOBS_KEPT: usize = 96;

fn job_key(lang: CodeLang, text: &str, font: &FontId, p: &Palette) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    text.hash(&mut h);
    lang.hash(&mut h);
    font.size.to_bits().hash(&mut h);
    font.family.hash(&mut h);
    p.key().hash(&mut h);
    h.finish()
}

/// The text laid out in its language's colours, in `font`, unwrapped.
#[must_use]
pub fn highlight(ctx: &Context, lang: CodeLang, text: &str, font: &FontId) -> LayoutJob {
    (*highlight_arc(ctx, lang, text, font)).clone()
}

fn highlight_arc(ctx: &Context, lang: CodeLang, text: &str, font: &FontId) -> Arc<LayoutJob> {
    let p = palette(ctx);
    let key = job_key(lang, text, font, &p);
    let id = Id::unique("gz-ui.code.jobs");
    if let Some(job) = ctx.data(|d| {
        d.get_temp::<Arc<parking_lot::Mutex<JobCache>>>(id)
            .and_then(|c| c.lock().jobs.iter().find(|(k, _)| *k == key).map(|(_, j)| j.clone()))
    }) {
        return job;
    }
    let mut job = LayoutJob::default();
    let fmt = |color| TextFormat { font_id: font.clone(), color, ..TextFormat::default() };
    if text.len() > COLOUR_LIMIT || lang == CodeLang::Plain {
        job.append(text, 0.0, fmt(p.text));
    } else {
        let mut at = 0usize;
        for (s, e, tok) in lex::spans(lang, text) {
            if s > at {
                job.append(&text[at..s], 0.0, fmt(p.text));
            }
            job.append(&text[s..e], 0.0, fmt(colour(&p, tok)));
            at = e;
        }
        if at < text.len() {
            job.append(&text[at..], 0.0, fmt(p.text));
        }
    }
    job.wrap.max_width = f32::INFINITY;
    let job = Arc::new(job);
    let cache = ctx.data_mut(|d| d.get_temp_mut_or_default::<Arc<parking_lot::Mutex<JobCache>>>(id).clone());
    let mut c = cache.lock();
    if c.jobs.len() >= JOBS_KEPT {
        c.jobs.remove(0);
    }
    c.jobs.push((key, job.clone()));
    job
}

/// The spans of a text that are strings or comments: brackets there do not pair.
fn quiet_spans(lang: CodeLang, text: &str) -> Vec<(usize, usize)> {
    if text.len() > COLOUR_LIMIT {
        return Vec::new();
    }
    lex::spans(lang, text)
        .into_iter()
        .filter(|s| matches!(s.2, Tok::Str | Tok::Comment))
        .map(|s| (s.0, s.1))
        .collect()
}

/// A problem on a line of the text: 1-based line and column (0: the whole line), its tone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineMark {
    pub line: u32,
    pub column: u32,
    pub tone: Tone,
}

/// What a completion row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Keyword,
    Function,
    Value,
    /// A script: what comes after its dot.
    Module,
    /// A word of the text itself.
    Word,
}

/// A row of the completion list: what it writes, what it says beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub text: String,
    pub detail: String,
    pub kind: CompletionKind,
}

/// What completion is asked for: the word typed so far and the name before its dot (`xr_logic`
/// of `xr_logic.pi`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompleteAsk {
    pub qualifier: Option<String>,
    pub prefix: String,
}

/// What answers completion's asks with the rows it offers.
pub type Completer = dyn Fn(&CompleteAsk) -> Vec<Completion>;

/// How a code view shows its text this frame.
#[derive(Clone, Copy)]
pub struct CodeOpts<'a> {
    pub lang: CodeLang,
    pub read_only: bool,
    pub wrap: bool,
    /// The problems' lines.
    pub marks: &'a [LineMark],
    /// Find's matches (byte ranges) and the one that is current.
    pub found: &'a [Range<usize>],
    pub found_current: Option<usize>,
    /// The rows completion offers for an ask; `None`: the view completes nothing.
    pub complete: Option<&'a Completer>,
}

impl<'a> CodeOpts<'a> {
    #[must_use]
    pub fn new(lang: CodeLang) -> Self {
        Self {
            lang,
            read_only: false,
            wrap: false,
            marks: &[],
            found: &[],
            found_current: None,
            complete: None,
        }
    }
}

/// What the caller asks of the view, taken once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeCtl {
    /// The caret to a 1-based line and column (0: the line's start), scrolled into view.
    pub goto: Option<(u32, u32)>,
    /// A byte range selected and scrolled into view.
    pub select: Option<Range<usize>>,
    /// The whole text replaced as one undo step (Replace All, Take Theirs), with the caret at a
    /// byte of the new text.
    pub set_text: Option<(String, usize)>,
    /// The keyboard to the view.
    pub focus: bool,
    /// Undo and Redo of the text's own history (the Edit menu, `doc.undo`), Duplicate of the
    /// lines the selection touches, Delete of the selection.
    pub undo: bool,
    pub redo: bool,
    pub duplicate: bool,
    pub delete: bool,
}

/// What the view did this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeOut {
    pub changed: bool,
    /// Ctrl+Enter.
    pub apply: bool,
    /// Ctrl+S.
    pub save: bool,
    /// Ctrl+F, Ctrl+H, Ctrl+G, Ctrl+Shift+O.
    pub find: bool,
    pub replace: bool,
    pub goto_line: bool,
    pub symbols: bool,
    /// F3 (`Some(false)`), Shift+F3 (`Some(true)`: backwards).
    pub find_next: Option<bool>,
    /// F12, or Ctrl and a click: the dotted name there.
    pub definition: Option<String>,
    /// Esc with nothing of the view's own to close.
    pub escape: bool,
    /// The caret: 1-based line and column (characters).
    pub caret: (u32, u32),
    /// The selection as bytes, when there is one.
    pub selection: Option<Range<usize>>,
    /// The 1-based line of the first row on screen.
    pub top_line: u32,
    pub focused: bool,
    pub lines: u32,
}

/// The completion list while it is up.
#[derive(Clone, Debug, Default)]
struct Completing {
    open: bool,
    /// Ctrl+Space put it up: it lists everything, even before a word is typed.
    asked: bool,
    /// Where the word being completed starts (bytes).
    from: usize,
    ask: CompleteAsk,
    rows: Vec<Completion>,
    pick: usize,
}

/// The selection of a text edit as bytes.
fn sel_of(text: &str, range: Option<CCursorRange>) -> Sel {
    let Some(r) = range else { return Sel::at(text.len()) };
    Sel {
        anchor: edit::byte_index(text, r.secondary.index.0),
        caret: edit::byte_index(text, r.primary.index.0),
    }
}

fn range_of(text: &str, sel: Sel) -> CCursorRange {
    CCursorRange::two(
        CCursor::new(edit::char_index(text, sel.anchor)),
        CCursor::new(edit::char_index(text, sel.caret)),
    )
}

/// One key, with exactly these modifiers (egui's own match lets a Shift or an Alt more through).
fn take(ui: &Ui, mods: Modifiers, key: Key) -> bool {
    ui.input_mut(|i| {
        let m = i.modifiers;
        if m.shift != mods.shift || m.alt != mods.alt || m.command != mods.command {
            return false;
        }
        i.consume_key(mods, key)
    })
}

/// A structural edit: written into the text with an undo point before it.
fn apply(
    ctx: &Context,
    id: Id,
    state: &mut TextEditState,
    text: &mut String,
    before: CCursorRange,
    e: edit::Edited,
) {
    let mut undo = state.undoer();
    undo.add_undo(&(before, text.clone()));
    state.set_undoer(undo);
    *text = e.text;
    state.cursor.set_char_range(Some(range_of(text, e.sel)));
    state.clone().store(ctx, id);
}

/// The code editor: `text` in its language's colours, filling the space left in `ui`, in a scroll
/// area of its own (`id_salt` keeps it). See the module's documentation for what it draws and the
/// keys it takes; [`CodeOut`] says what the caller is asked to do.
pub fn code_view(
    ui: &mut Ui,
    id_salt: &str,
    text: &mut String,
    opts: &CodeOpts<'_>,
    ctl: &mut CodeCtl,
) -> CodeOut {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let font = Font::Mono.id();
    let line_h = Font::Mono.line();
    let edit_id = ui.make_persistent_id(("gz-ui.code", id_salt));
    let done_id = edit_id.with("completing");
    let mut out = CodeOut {
        lines: 1 + u32::try_from(text.bytes().filter(|&b| b == b'\n').count()).unwrap_or(u32::MAX),
        ..CodeOut::default()
    };
    let mut state = TextEditState::load(&ctx, edit_id).unwrap_or_default();
    let mut completing: Completing = ctx.data(|d| d.get_temp(done_id)).unwrap_or_default();
    let mut moved = false;

    // what the caller asked for
    if let Some((new, at)) = ctl.set_text.take() {
        let before = state.cursor.char_range().unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
        let at = at.min(new.len());
        apply(&ctx, edit_id, &mut state, text, before, edit::Edited { text: new, sel: Sel::at(at) });
        out.changed = true;
        moved = true;
    }
    if let Some((line, col)) = ctl.goto.take() {
        let at = edit::byte_of(text, line, col);
        state.cursor.set_char_range(Some(range_of(text, Sel::at(at))));
        state.clone().store(&ctx, edit_id);
        ctl.focus = true;
        moved = true;
    }
    if let Some(r) = ctl.select.take() {
        let (s, e) = (r.start.min(text.len()), r.end.min(text.len()));
        state.cursor.set_char_range(Some(range_of(text, Sel { anchor: s, caret: e })));
        state.clone().store(&ctx, edit_id);
        moved = true;
    }
    if std::mem::take(&mut ctl.focus) {
        ctx.memory_mut(|m| m.request_focus(edit_id));
    }
    // the Edit menu's rows, from outside the field
    let before = state.cursor.char_range().unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
    if (ctl.undo || ctl.redo) && !opts.read_only {
        let mut undoer = state.undoer();
        let now = (before, text.clone());
        let back = if ctl.undo { undoer.undo(&now).cloned() } else { undoer.redo(&now).cloned() };
        if let Some((range, old)) = back {
            *text = old;
            state.cursor.set_char_range(Some(range));
            state.set_undoer(undoer);
            state.clone().store(&ctx, edit_id);
            out.changed = true;
            moved = true;
        }
    }
    if ctl.duplicate && !opts.read_only {
        apply(&ctx, edit_id, &mut state, text, before, edit::duplicate(text, sel_of(text, Some(before))));
        out.changed = true;
    }
    if ctl.delete && !opts.read_only {
        let sel = sel_of(text, Some(before));
        if !sel.is_empty() {
            let mut new = text.clone();
            new.replace_range(sel.start()..sel.end(), "");
            apply(
                &ctx,
                edit_id,
                &mut state,
                text,
                before,
                edit::Edited { text: new, sel: Sel::at(sel.start()) },
            );
            out.changed = true;
        }
    }
    (ctl.undo, ctl.redo, ctl.duplicate, ctl.delete) = (false, false, false, false);

    let focused = ctx.memory(|m| m.has_focus(edit_id));
    out.focused = focused;
    let unit = edit::indent_unit(text);
    if focused {
        let before = state.cursor.char_range().unwrap_or_else(|| CCursorRange::one(CCursor::new(0)));
        let sel = sel_of(text, Some(before));
        // the completion list takes its keys first
        if completing.open {
            let n = completing.rows.len();
            if take(ui, Modifiers::NONE, Key::Escape) {
                completing.open = false;
            } else if n > 0 && take(ui, Modifiers::NONE, Key::ArrowDown) {
                completing.pick = (completing.pick + 1) % n;
            } else if n > 0 && take(ui, Modifiers::NONE, Key::ArrowUp) {
                completing.pick = (completing.pick + n - 1) % n;
            } else if n > 0 && take(ui, Modifiers::NONE, Key::PageDown) {
                completing.pick = (completing.pick + 8).min(n - 1);
            } else if n > 0 && take(ui, Modifiers::NONE, Key::PageUp) {
                completing.pick = completing.pick.saturating_sub(8);
            } else if n > 0 && (take(ui, Modifiers::NONE, Key::Enter) || take(ui, Modifiers::NONE, Key::Tab))
            {
                let row = completing.rows[completing.pick.min(n - 1)].text.clone();
                let from = completing.from.min(sel.caret);
                let mut new = String::with_capacity(text.len() + row.len());
                new.push_str(&text[..from]);
                new.push_str(&row);
                new.push_str(&text[sel.caret..]);
                apply(
                    &ctx,
                    edit_id,
                    &mut state,
                    text,
                    before,
                    edit::Edited { text: new, sel: Sel::at(from + row.len()) },
                );
                completing.open = false;
                out.changed = true;
            }
        }
        out.save = take(ui, Modifiers::CTRL, Key::S);
        out.apply = take(ui, Modifiers::CTRL, Key::Enter);
        out.symbols = take(ui, Modifiers::CTRL | Modifiers::SHIFT, Key::O);
        out.find = take(ui, Modifiers::CTRL, Key::F);
        out.replace = take(ui, Modifiers::CTRL, Key::H);
        out.goto_line = take(ui, Modifiers::CTRL, Key::G);
        if take(ui, Modifiers::SHIFT, Key::F3) {
            out.find_next = Some(true);
        } else if take(ui, Modifiers::NONE, Key::F3) {
            out.find_next = Some(false);
        }
        if take(ui, Modifiers::NONE, Key::F12) {
            out.definition = edit::dotted_at(text, sel.caret).map(|d| d.2);
        }
        if !completing.open && take(ui, Modifiers::NONE, Key::Escape) {
            out.escape = true;
        }
        let ask_now = opts.complete.is_some() && take(ui, Modifiers::CTRL, Key::Space);
        if !opts.read_only && !out.changed {
            let lang = opts.lang;
            let edited = if take(ui, Modifiers::SHIFT, Key::Tab) {
                Some(edit::outdent(text, sel, &unit))
            } else if take(ui, Modifiers::NONE, Key::Tab) {
                Some(edit::indent(text, sel, &unit))
            } else if take(ui, Modifiers::CTRL, Key::Slash) {
                edit::toggle_comment(text, sel, lang)
            } else if take(ui, Modifiers::CTRL, Key::D) {
                Some(edit::duplicate(text, sel))
            } else if take(ui, Modifiers::ALT, Key::ArrowUp) {
                edit::move_lines(text, sel, true)
            } else if take(ui, Modifiers::ALT, Key::ArrowDown) {
                edit::move_lines(text, sel, false)
            } else if take(ui, Modifiers::NONE, Key::Enter) {
                // a new line is typing, not a step of its own: egui's undo takes it with the rest
                let e = edit::newline(text, sel, lang, &unit);
                *text = e.text;
                state.cursor.set_char_range(Some(range_of(text, e.sel)));
                state.clone().store(&ctx, edit_id);
                out.changed = true;
                None
            } else {
                None
            };
            if let Some(e) = edited {
                apply(&ctx, edit_id, &mut state, text, before, e);
                out.changed = true;
            }
        }
        if ask_now {
            completing.open = true;
            completing.asked = true;
        }
    }

    if !warm_up(ui, edit_id, opts.lang, text, &font) {
        return out;
    }

    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.inset);
    let digits = out.lines.max(999).to_string().len() + 1;
    let gutter = crate::text::width(&ctx, &font, crate::Tx::raw("0".repeat(digits))) + 12.0;
    let scroll = if opts.wrap { egui::ScrollArea::vertical() } else { egui::ScrollArea::both() };
    let mut caret_rect: Option<Rect> = None;
    let mut ctrl_click = false;
    let mut galley_info: Option<(Arc<egui::Galley>, Pos2)> = None;
    let shown_scroll =
        scroll.id_salt(("gz-ui.code_scroll", id_salt)).auto_shrink([false, false]).show(ui, |ui| {
            let under = ui.painter().add(Shape::Noop);
            let min_h = (out.lines as f32 * line_h + 8.0 + 2.0 * line_h).max(rect.height());
            let left = ui.cursor().min.x;
            let wrap_w = (rect.width() - gutter - 16.0).max(80.0);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(gutter);
                let lang = opts.lang;
                let wrap = opts.wrap;
                let mut layouter = |ui: &Ui, t: &dyn egui::TextBuffer, _w: f32| {
                    let mut job = (*highlight_arc(ui.ctx(), lang, t.as_str(), &font)).clone();
                    job.wrap.max_width = if wrap { wrap_w } else { f32::INFINITY };
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let width = if opts.wrap { wrap_w } else { f32::INFINITY };
                let min = Vec2::new(ui.available_width(), min_h);
                let shown = if opts.read_only {
                    let mut view: &str = text.as_str();
                    field(&mut view, edit_id, &font, &mut layouter, width, min).show(ui)
                } else {
                    field(text, edit_id, &font, &mut layouter, width, min).show(ui)
                };
                out.changed |= shown.response.response.changed();
                ctrl_click = shown.response.response.clicked() && ui.input(|i| i.modifiers.command);
                let caret = shown.cursor_range;
                let origin = shown.galley_pos;
                let area = shown.response.response.rect;
                let galley = shown.galley.clone();
                let painter = ui.painter().clone();
                let clip = ui.clip_rect();
                if let Some(c) = caret {
                    caret_rect = Some(galley.pos_from_cursor(c.primary).translate(origin.to_vec2()));
                    let s = sel_of(text, Some(c));
                    out.caret = edit::line_col(text, s.caret);
                    out.selection = (!s.is_empty()).then(|| s.start()..s.end());
                }
                // the rows on screen, each with the line it belongs to and the bytes it holds
                let mut line = 1u32;
                let mut starts = true;
                let mut chars = 0usize;
                let mut top_line = 0u32;
                let marks = opts.marks;
                let mut bands: Vec<Shape> = Vec::new();
                let caret_line = out.caret.0;
                let mut row_chars: Vec<(Rect, usize, usize)> = Vec::new();
                for row in &galley.rows {
                    let r = row.rect().translate(origin.to_vec2());
                    let n = row.row.char_count_excluding_newline().0;
                    let on_screen = r.max.y >= clip.min.y && r.min.y <= clip.max.y;
                    if on_screen {
                        if top_line == 0 {
                            top_line = line;
                        }
                        row_chars.push((r, chars, chars + n));
                        let band = Rect::from_min_max(
                            pos2(left, r.min.y),
                            pos2(area.max.x.max(rect.max.x), r.min.y + line_h.max(r.height())),
                        );
                        if let Some(m) = marks.iter().find(|m| m.line == line) {
                            bands.push(Shape::rect_filled(band, CornerRadius::ZERO, p.tint(m.tone)));
                            if starts {
                                squiggle(
                                    &painter,
                                    &galley,
                                    origin,
                                    text,
                                    row,
                                    chars,
                                    m,
                                    p.tone(m.tone),
                                    line_h,
                                );
                            }
                        } else if caret.is_some() && caret_line == line {
                            bands.push(Shape::rect_filled(
                                band,
                                CornerRadius::ZERO,
                                Palette::alpha(p.hover, 0.7),
                            ));
                        }
                        if starts {
                            let color = if let Some(m) = marks.iter().find(|m| m.line == line) {
                                p.tone(m.tone)
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
                    } else if r.min.y > clip.max.y {
                        break;
                    }
                    chars += n + usize::from(row.ends_with_newline);
                    starts = row.ends_with_newline;
                    if row.ends_with_newline {
                        line += 1;
                    }
                }
                out.top_line = top_line.max(1);
                // find's matches and the bracket pair, on the rows on screen
                let char_rect =
                    |ci: usize| galley.pos_from_cursor(CCursor::new(ci)).translate(origin.to_vec2());
                let span_rects = |s: usize, e: usize| -> Vec<Rect> {
                    let (cs, ce) = (edit::char_index(text, s), edit::char_index(text, e));
                    row_chars
                        .iter()
                        .filter(|(_, a, b)| ce > *a && cs <= *b)
                        .map(|(r, a, b)| {
                            let x0 = char_rect(cs.max(*a)).min.x;
                            let x1 = char_rect(ce.min(*b)).min.x;
                            Rect::from_min_max(
                                pos2(x0, r.min.y),
                                pos2(x1.max(x0 + 2.0), r.min.y + line_h.max(r.height())),
                            )
                        })
                        .collect()
                };
                if let (Some(first), Some(last)) = (row_chars.first(), row_chars.last()) {
                    let (lo, hi) = (edit::byte_index(text, first.1), edit::byte_index(text, last.2));
                    for (k, f) in opts.found.iter().enumerate() {
                        if f.end < lo || f.start > hi {
                            continue;
                        }
                        let current = opts.found_current == Some(k);
                        for r in span_rects(f.start, f.end) {
                            let fill =
                                if current { Palette::alpha(p.warning, 0.45) } else { p.tint(Tone::Warning) };
                            bands.push(Shape::rect_filled(r, CornerRadius::same(2), fill));
                            if current {
                                bands.push(Shape::rect_stroke(
                                    r,
                                    CornerRadius::same(2),
                                    Stroke::new(1.0, p.warning),
                                    StrokeKind::Inside,
                                ));
                            }
                        }
                    }
                    if let Some(c) = caret {
                        let at = edit::byte_index(text, c.primary.index.0);
                        if text.len() <= COLOUR_LIMIT
                            && let Some((a, b)) = edit::bracket_pair(text, at, &quiet_spans(opts.lang, text))
                        {
                            for i in [a, b] {
                                for r in span_rects(i, i + 1) {
                                    bands.push(Shape::rect_filled(
                                        r,
                                        CornerRadius::same(2),
                                        p.tint(Tone::Info),
                                    ));
                                    bands.push(Shape::rect_stroke(
                                        r,
                                        CornerRadius::same(2),
                                        Stroke::new(1.0, p.border_strong),
                                        StrokeKind::Inside,
                                    ));
                                }
                            }
                        }
                    }
                }
                painter.set(under, Shape::Vec(bands));
                let x = left + gutter - 3.0;
                painter.line_segment(
                    [pos2(x, area.min.y), pos2(x, area.max.y.max(rect.max.y))],
                    Stroke::new(1.0, p.border_subtle),
                );
                galley_info = Some((galley, origin));
            });
        });
    let _ = shown_scroll;
    if moved && let Some(r) = caret_rect {
        ui.scroll_to_rect(r.expand2(Vec2::new(24.0, line_h * 3.0)), Some(Align::Center));
    }
    if ctrl_click && out.definition.is_none() {
        let s = sel_of(text, state_range(&ctx, edit_id));
        out.definition = edit::dotted_at(text, s.caret).map(|d| d.2);
    }

    // completion: asked for, or kept up while the word is typed
    if let Some(complete) = opts.complete
        && focused
    {
        let caret = sel_of(text, state_range(&ctx, edit_id)).caret;
        let typed_dot =
            out.changed && caret > 0 && matches!(text.as_bytes().get(caret - 1), Some(b'.' | b':'));
        let typing = out.changed
            && caret > 0
            && text.as_bytes().get(caret - 1).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
        if typed_dot && opts.lang == CodeLang::Lua {
            completing.open = true;
        }
        if let (true, Some((from, qualifier, prefix))) =
            (completing.open || typing, edit::completion_at(text, caret))
        {
            let auto = !completing.open;
            let ask = CompleteAsk { qualifier, prefix };
            if ask != completing.ask || completing.rows.is_empty() {
                completing.rows = complete(&ask);
                completing.pick = 0;
                completing.ask = ask.clone();
            }
            completing.from = from;
            // as it is typed the list comes up once a word has three letters, or after a dot
            if auto && (ask.prefix.len() >= 3 || ask.qualifier.is_some()) && !completing.rows.is_empty() {
                completing.open = true;
            }
            // nothing left to offer, the one offer is what is typed already, or the word is over
            // (a space, an operator) and nobody asked for the whole list
            let over = ask.prefix.is_empty() && ask.qualifier.is_none() && !completing.asked;
            if completing.rows.is_empty()
                || over
                || (completing.rows.len() == 1 && completing.rows[0].text == ask.prefix && auto)
            {
                completing.open = false;
            }
        } else if completing.open && out.changed {
            completing.open = false;
        }
        if !completing.open {
            completing.asked = false;
        }
        if completing.open
            && let Some(at) = caret_rect
        {
            draw_completions(&ctx, edit_id, &completing, at, &p);
        }
    } else {
        completing.open = false;
    }
    ctx.data_mut(|d| d.insert_temp(done_id, completing));
    let _ = galley_info;
    out
}

/// The text field of a code view.
fn field<'t>(
    buffer: &'t mut dyn egui::TextBuffer,
    id: Id,
    font: &FontId,
    layouter: &'t mut dyn FnMut(&Ui, &dyn egui::TextBuffer, f32) -> Arc<egui::Galley>,
    width: f32,
    min: Vec2,
) -> TextEdit<'t> {
    TextEdit::multiline(buffer)
        .id(id)
        .font(font.clone())
        .code_editor()
        .layouter(layouter)
        .frame(Frame::NONE)
        .margin(Margin { left: 4, right: 6, top: 4, bottom: 4 })
        .desired_width(width)
        .min_size(min)
        .lock_focus(true)
}

fn state_range(ctx: &Context, id: Id) -> Option<CCursorRange> {
    TextEditState::load(ctx, id).and_then(|s| s.cursor.char_range())
}

/// A problem's squiggle under the word at its column, or under the line's text.
#[allow(clippy::too_many_arguments)]
fn squiggle(
    painter: &egui::Painter,
    galley: &egui::Galley,
    origin: Pos2,
    text: &str,
    row: &egui::epaint::text::PlacedRow,
    row_start: usize,
    m: &LineMark,
    color: egui::Color32,
    line_h: f32,
) {
    let r = row.rect().translate(origin.to_vec2());
    let (x0, x1) = if m.column > 0 {
        let from = row_start + m.column as usize - 1;
        let byte = edit::byte_index(text, from);
        let end = text[byte..]
            .char_indices()
            .find(|(_, c)| !(c.is_alphanumeric() || *c == '_'))
            .map_or(text.len(), |(k, _)| byte + k)
            .max(byte + 1);
        let a = galley.pos_from_cursor(CCursor::new(from)).translate(origin.to_vec2()).min.x;
        let b = galley
            .pos_from_cursor(CCursor::new(edit::char_index(text, end)))
            .translate(origin.to_vec2())
            .min
            .x;
        (a, b.max(a + 8.0))
    } else {
        (r.min.x, r.max.x.max(r.min.x + 24.0))
    };
    let y = r.min.y + line_h - 2.0;
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = true;
    while x <= x1 {
        points.push(pos2(x, if up { y - 1.5 } else { y + 1.5 }));
        x += 3.0;
        up = !up;
    }
    painter.add(Shape::line(points, Stroke::new(1.0, color)));
}

/// The completion list under the caret: the kit's popup frame, a row per offer (what it writes in
/// Mono, what it is muted beside it), the picked one in the selection colour.
fn draw_completions(ctx: &Context, id: Id, c: &Completing, at: Rect, p: &Palette) {
    const SHOWN: usize = 10;
    let first = c.pick.saturating_sub(SHOWN - 1);
    egui::Area::new(id.with("completions"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(at.min.x, at.max.y + 2.0))
        .show(ctx, |ui| {
            crate::menu::popup_frame(ctx).show(ui, |ui| {
                ui.set_min_width(260.0);
                for (k, row) in c.rows.iter().enumerate().skip(first).take(SHOWN) {
                    let (rect, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width().max(260.0), 22.0),
                        Sense::hover(),
                    );
                    if k == c.pick {
                        ui.painter().rect_filled(rect, CornerRadius::same(4), p.selection_focused);
                    }
                    // the kind as a letter in its token's colour
                    let (glyph, tint) = match row.kind {
                        CompletionKind::Function => ("f", p.syntax.function),
                        CompletionKind::Module => ("m", p.syntax.name),
                        CompletionKind::Keyword => ("k", p.syntax.keyword),
                        CompletionKind::Value => ("v", p.syntax.number),
                        CompletionKind::Word => ("w", p.text_muted),
                    };
                    let mid = rect.center().y;
                    ui.painter().text(
                        pos2(rect.min.x + 8.0, mid),
                        Align2::LEFT_CENTER,
                        glyph,
                        Font::Mono.id(),
                        tint,
                    );
                    ui.painter().text(
                        pos2(rect.min.x + 24.0, mid),
                        Align2::LEFT_CENTER,
                        &row.text,
                        Font::Mono.id(),
                        p.text,
                    );
                    if !row.detail.is_empty() {
                        ui.painter().text(
                            pos2(rect.max.x - 8.0, mid),
                            Align2::RIGHT_CENTER,
                            &row.detail,
                            Font::Small.id(),
                            p.text_muted,
                        );
                    }
                }
                if c.rows.len() > SHOWN {
                    let shown = format!("{} / {}", c.pick + 1, c.rows.len());
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(ui.available_width(), 18.0), Sense::hover());
                    ui.painter().text(
                        rect.right_center() - Vec2::new(8.0, 0.0),
                        Align2::RIGHT_CENTER,
                        shown,
                        Font::Small.id(),
                        p.text_muted,
                    );
                }
            });
        });
}

/// The job up to `end` (a line's end): the very lines of the whole job, so the galley cache that
/// keeps each line apart has them when the whole is laid out.
fn prefix_job(job: &LayoutJob, end: usize) -> LayoutJob {
    use egui::epaint::text::ByteIndex;
    let mut out = job.clone();
    out.text.truncate(end);
    out.sections.retain(|sec| sec.byte_range.start.0 < end);
    for sec in &mut out.sections {
        sec.byte_range.end = ByteIndex(sec.byte_range.end.0.min(end));
    }
    out
}

/// Lays a large text out a few kilobytes a frame, each frame the part done so far again (the
/// galley cache keeps the lines a frame used, and hands them back), and shows how far it is;
/// `true` once the whole text is laid out, which the field then finds in the cache.
fn warm_up(ui: &mut Ui, id: Id, lang: CodeLang, value: &str, font: &FontId) -> bool {
    use std::hash::{Hash, Hasher};
    if value.len() <= WARM_FROM {
        return true;
    }
    let ctx = ui.ctx().clone();
    let mut h = rustc_hash::FxHasher::default();
    value.hash(&mut h);
    let want = h.finish();
    let key = id.with("warm");
    let done =
        ctx.data(|d| d.get_temp::<(u64, usize)>(key)).filter(|(k, _)| *k == want).map_or(0, |(_, n)| n);
    if done >= value.len() {
        return true;
    }
    // a text that changed after it was laid out once is laid out line by line as it is typed
    if done == 0 && ctx.data(|d| d.get_temp::<bool>(key.with("once"))).unwrap_or(false) {
        ctx.data_mut(|d| d.insert_temp(key, (want, value.len())));
        return true;
    }
    let mut job = (*highlight_arc(&ctx, lang, value, font)).clone();
    job.wrap.max_width = f32::INFINITY;
    let from = (done + WARM_STEP).min(value.len());
    let end = value
        .as_bytes()
        .get(from..)
        .and_then(|rest| rest.iter().position(|&b| b == b'\n'))
        .map_or(value.len(), |i| from + i + 1);
    let part = if end >= value.len() { job } else { prefix_job(&job, end) };
    let _ = ui.fonts_mut(|f| f.layout_job(part));
    ctx.data_mut(|d| d.insert_temp(key, (want, end)));
    if end >= value.len() {
        ctx.data_mut(|d| d.insert_temp(key.with("once"), true));
        return true;
    }
    ctx.request_repaint();
    let share = (end as f32 / value.len() as f32 * 100.0).round();
    crate::state::busy_line(ui, crate::tr!("Laying out the text: {share}%", share = format!("{share:.0}")));
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prefix_keeps_the_lines_of_the_whole() {
        let mut job = LayoutJob::default();
        let fmt = |color| TextFormat { color, ..TextFormat::default() };
        job.append("a = 1,\n", 0.0, fmt(egui::Color32::RED));
        job.append("b = \"x\",\n", 0.0, fmt(egui::Color32::GREEN));
        job.append("c = 3", 0.0, fmt(egui::Color32::BLUE));
        let cut = job.text.find('b').unwrap() + "b = \"x\",\n".len();
        let part = prefix_job(&job, cut);
        assert_eq!(part.text, "a = 1,\nb = \"x\",\n");
        assert_eq!(part.sections.len(), 2);
        assert!(part.sections.iter().all(|s| s.byte_range.end.0 <= cut));
    }

    #[test]
    fn a_file_names_its_language() {
        assert_eq!(CodeLang::from_extension("script"), CodeLang::Lua);
        assert_eq!(CodeLang::from_extension(".LTXP"), CodeLang::Ltx);
        assert_eq!(CodeLang::from_extension("itemasset"), CodeLang::LuaData);
        assert_eq!(CodeLang::from_extension("md"), CodeLang::Markdown);
        assert_eq!(CodeLang::from_extension("bin"), CodeLang::Plain);
        assert_eq!(CodeLang::from_fence("lua {.x}"), CodeLang::Lua);
    }

    #[test]
    fn a_coloured_job_holds_the_whole_text() {
        let ctx = Context::default();
        let t = "local x = 'a' -- c\n[s]\nk = 1\n";
        for lang in [CodeLang::Lua, CodeLang::Ltx, CodeLang::Plain] {
            let job = highlight(&ctx, lang, t, &Font::Mono.id());
            assert_eq!(job.text, t);
            let covered: usize = job.sections.iter().map(|s| s.byte_range.end.0 - s.byte_range.start.0).sum();
            assert_eq!(covered, t.len());
        }
    }

    /// The editor in a headless frame: `events` typed with `mods` held, the text and the caret as
    /// the view leaves them.
    struct Bench {
        ctx: Context,
        t: f64,
    }

    impl Bench {
        fn new() -> Self {
            Self { ctx: crate::testkit::context(), t: 0.0 }
        }

        fn frame(
            &mut self,
            text: &mut String,
            opts: &CodeOpts<'_>,
            ctl: &mut CodeCtl,
            mods: Modifiers,
            events: Vec<egui::Event>,
        ) -> CodeOut {
            self.t += 0.1;
            let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 600.0));
            let mut all = vec![egui::Event::ModifiersChanged(mods)];
            all.extend(events);
            let input = egui::RawInput {
                screen_rect: Some(screen),
                time: Some(self.t),
                events: all,
                ..egui::RawInput::default()
            };
            let mut out = CodeOut::default();
            let mut shown = self.ctx.run_ui(input, |ui| out = code_view(ui, "bench", text, opts, ctl));
            shown.textures_delta.clear();
            out
        }

        /// The view takes the keys, the caret at a line and column.
        fn caret_at(&mut self, text: &mut String, opts: &CodeOpts<'_>, line: u32, col: u32) {
            let mut ctl = CodeCtl { goto: Some((line, col)), focus: true, ..CodeCtl::default() };
            self.frame(text, opts, &mut ctl, Modifiers::NONE, Vec::new());
            self.frame(text, opts, &mut CodeCtl::default(), Modifiers::NONE, Vec::new());
        }

        fn key(&mut self, text: &mut String, opts: &CodeOpts<'_>, mods: Modifiers, k: Key) -> CodeOut {
            let ev = egui::Event::Key {
                key: k,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods,
            };
            self.frame(text, opts, &mut CodeCtl::default(), mods, vec![ev])
        }
    }

    #[test]
    fn enter_tab_and_comment_edit_the_text() {
        let mut b = Bench::new();
        let opts = CodeOpts::new(CodeLang::Lua);
        let mut text = "function f()\n\tif x then".to_owned();
        b.caret_at(&mut text, &opts, 2, 11);
        let out = b.key(&mut text, &opts, Modifiers::NONE, Key::Enter);
        assert!(out.changed);
        assert_eq!(text, "function f()\n\tif x then\n\t\t");
        let mut text = "x = 1\ny = 2".to_owned();
        b.caret_at(&mut text, &opts, 1, 1);
        b.key(&mut text, &opts, Modifiers::CTRL, Key::Slash);
        assert_eq!(text, "-- x = 1\ny = 2");
        // the caret stayed before `x`: Tab without a selection puts an indent there
        b.key(&mut text, &opts, Modifiers::NONE, Key::Tab);
        assert_eq!(text, "--     x = 1\ny = 2");
        // a selection over both lines: Tab indents them, Shift+Tab takes it back
        let mut ctl = CodeCtl { select: Some(0..text.len()), ..CodeCtl::default() };
        b.frame(&mut text, &opts, &mut ctl, Modifiers::NONE, Vec::new());
        b.key(&mut text, &opts, Modifiers::NONE, Key::Tab);
        assert_eq!(text, "    --     x = 1\n    y = 2");
        b.key(&mut text, &opts, Modifiers::SHIFT, Key::Tab);
        assert_eq!(text, "--     x = 1\ny = 2");
        let out = b.key(&mut text, &opts, Modifiers::CTRL, Key::S);
        assert!(out.save, "Ctrl+S is the caller's save");
    }

    #[test]
    fn completion_comes_up_as_a_word_is_typed_and_enter_takes_it() {
        let mut b = Bench::new();
        let offer = |ask: &CompleteAsk| -> Vec<Completion> {
            ["qa_text_twice", "qa_other"]
                .into_iter()
                .filter(|w| w.starts_with(&ask.prefix))
                .map(|w| Completion {
                    text: w.to_owned(),
                    detail: "(x)".to_owned(),
                    kind: CompletionKind::Function,
                })
                .collect()
        };
        let mut opts = CodeOpts::new(CodeLang::Lua);
        opts.complete = Some(&offer);
        let mut text = String::new();
        b.caret_at(&mut text, &opts, 1, 1);
        b.frame(
            &mut text,
            &opts,
            &mut CodeCtl::default(),
            Modifiers::NONE,
            vec![egui::Event::Text("qa_t".to_owned())],
        );
        assert_eq!(text, "qa_t");
        b.key(&mut text, &opts, Modifiers::NONE, Key::Enter);
        assert_eq!(text, "qa_text_twice", "Enter takes the offer instead of a new line");
        // a space ends the word: the list is down, Enter is a new line again
        b.frame(
            &mut text,
            &opts,
            &mut CodeCtl::default(),
            Modifiers::NONE,
            vec![egui::Event::Text(" ".to_owned())],
        );
        b.key(&mut text, &opts, Modifiers::NONE, Key::Enter);
        assert_eq!(text, "qa_text_twice \n");
    }
}
