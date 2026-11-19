//! A Markdown preview: a text drawn as the document it describes, read-only, in a scroll area of
//! its own ([`markdown_view`]).
//!
//! The text is parsed once per change (pulldown-cmark) into a flat list of blocks - a heading, a
//! paragraph, a list item's paragraph, a code block, a table, a rule - each knowing the source lines
//! it came from and the lists and quotes around it. A frame lays out only the blocks in view: a
//! block never seen is as tall as its length says, a block laid out keeps its height (per width,
//! theme, scale and language). The block at the top of the view holds it still: a height learned
//! above it, a resize or an edit of the text never moves what is being read.

use std::borrow::Cow;
use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use egui::style::ScrollAnimation;
use egui::text::{LayoutJob, TextFormat};
use egui::{
    Color32, Context, CornerRadius, CursorIcon, Galley, Id, Label, Margin, Pos2, Rect, ScrollArea, Shape,
    Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2, vec2,
};
use parking_lot::Mutex;
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use rustc_hash::{FxHashMap, FxHasher};

use crate::code::{self, CodeLang};
use crate::font::Font;
use crate::theme::{Palette, metrics, palette};
use crate::units::em;

/// What the preview did this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkdownOut {
    /// A link the author clicked, as the document writes it (`https://...`, `docs/a.md`, `#heading`).
    /// `#anchor` links are handled inside (scroll to the heading) and never returned.
    pub link: Option<String>,
    /// The 1-based source line of the first block on screen (the caller keeps an editor beside it in step).
    pub top_line: u32,
}

/// Draws `text` as Markdown filling the space left in `ui`, in a vertical scroll area of its own
/// (`id_salt` keeps its scroll). `goto_line` > 0 scrolls once so the block holding that 1-based source
/// line is at the top, then is set to 0.
///
/// In a code block the line itself comes to the top, and `top_line` names the code line at the top:
/// its source lines are its rows. Anywhere else `top_line` is the first line of the block at the top.
pub fn markdown_view(ui: &mut Ui, id_salt: &str, text: &str, goto_line: &mut u32) -> MarkdownOut {
    let id = view_id(ui, id_salt);
    let handle = handle(ui.ctx(), id);
    let goto = std::mem::take(goto_line);
    let mut view = handle.0.lock();
    view.load(text);
    ScrollArea::vertical()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .animated(false)
        .content_margin(Margin::ZERO)
        .show_viewport(ui, |ui, viewport| view.frame(ui, id, viewport, goto))
        .inner
}

// ---------------------------------------------------------------------------------------------
// The model: what the text says, worked out once per text.

/// How a run of text is set: bits of a [`Span`]'s style.
mod style {
    pub const STRONG: u8 = 1;
    pub const EMPH: u8 = 2;
    pub const STRIKE: u8 = 4;
    pub const CODE: u8 = 8;
    /// An image: its alt text, shown as a chip instead of the picture.
    pub const PICTURE: u8 = 16;
    pub const HTML: u8 = 32;
}

/// A run of text in one style, maybe a link (an index into [`Doc::links`]).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Span {
    text: String,
    style: u8,
    link: Option<u32>,
}

/// The text of a paragraph, a heading or a table cell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Inline {
    spans: Vec<Span>,
}

impl Inline {
    fn push(&mut self, text: &str, style: u8, link: Option<u32>) {
        let picture = style & style::PICTURE != 0;
        if text.is_empty() && !picture {
            return;
        }
        if !picture
            && let Some(last) = self.spans.last_mut()
            && last.style == style
            && last.link == link
        {
            last.text.push_str(text);
            return;
        }
        self.spans.push(Span { text: text.to_owned(), style, link });
    }

    fn plain(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    fn chars(&self) -> usize {
        self.spans.iter().map(|s| s.text.chars().count()).sum()
    }

    fn breaks(&self) -> usize {
        self.spans.iter().map(|s| s.text.matches('\n').count()).sum()
    }

    fn trim_end_newlines(&mut self) {
        while let Some(last) = self.spans.last_mut() {
            let keep = last.text.trim_end_matches('\n').len();
            last.text.truncate(keep);
            if !last.text.is_empty() || last.style & style::PICTURE != 0 {
                break;
            }
            self.spans.pop();
        }
    }
}

/// A container a block sits in, outermost first: a list, or a quote (its number, so the bars of
/// one quote join across its blocks).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Nest {
    List,
    Quote(u32),
}

/// What stands before the first block of a list item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Marker {
    None,
    /// A bullet of a list this deep (0 the outermost).
    Bullet(u8),
    Number(u64),
    /// A task list item: done or not.
    Task(bool),
}

#[derive(Clone, Debug, PartialEq)]
enum BlockKind {
    Heading {
        level: u8,
        text: Inline,
        slug: String,
    },
    /// A paragraph, a list item's text, an HTML block (in the HTML style).
    Para(Inline),
    Code {
        lang: CodeLang,
        text: String,
        fenced: bool,
    },
    Table {
        align: Vec<Alignment>,
        head: Vec<Inline>,
        rows: Vec<Vec<Inline>>,
    },
    Rule,
}

/// One block of the document, laid out as a whole.
#[derive(Clone, Debug, PartialEq)]
struct Block {
    kind: BlockKind,
    /// The 1-based source lines it came from, both included.
    first: u32,
    last: u32,
    nest: Vec<Nest>,
    marker: Marker,
    /// The room above it, in points.
    gap: f32,
    /// What its height depends on: its source, its containers, its marker and its gap.
    hash: u64,
    /// For the estimate of a block not laid out yet: its characters and its lines.
    size: usize,
    breaks: usize,
}

/// A parsed text.
#[derive(Clone, Debug, Default)]
struct Doc {
    blocks: Vec<Block>,
    links: Vec<String>,
}

impl Doc {
    /// The block holding a 1-based source line, or the first one after it (a blank line), or the
    /// last one.
    fn block_at_line(&self, line: u32) -> usize {
        self.blocks.partition_point(|b| b.last < line).min(self.blocks.len().saturating_sub(1))
    }
}

fn hasher() -> FxHasher {
    BuildHasherDefault::<FxHasher>::default().build_hasher()
}

fn hash_of(text: &str) -> u64 {
    let mut h = hasher();
    text.hash(&mut h);
    h.finish()
}

/// A heading's anchor as GitHub makes it: lowercase, a space a `-`, every other character that is
/// no letter, digit, `-` or `_` dropped. Cyrillic is a letter.
fn slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.trim().chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            out.extend(c.to_lowercase());
        } else if c == ' ' {
            out.push('-');
        }
    }
    out
}

/// `%D0%B0` and the like back to text: an anchor link may be written encoded.
fn percent_decode(s: &str) -> String {
    let hex = |b: u8| char::from(b).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(hi), Some(lo)) =
                (bytes.get(i + 1).copied().and_then(hex), bytes.get(i + 2).copied().and_then(hex))
        {
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse(src: &str) -> Doc {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut b = Builder::new(src);
    for (event, range) in Parser::new_ext(src, opts).into_offset_iter() {
        b.event(event, range);
    }
    b.finish()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeafKind {
    Para,
    Heading(u8),
    Code(CodeLang, bool),
    Html,
}

/// A block being read.
struct Leaf {
    kind: LeafKind,
    inline: Inline,
    code: String,
    span: Range<usize>,
    /// Opened by its own tag; a tight list item's text has none and ends where its item does.
    explicit: bool,
    marker: Marker,
    gap: f32,
}

struct TableBuild {
    align: Vec<Alignment>,
    head: Vec<Inline>,
    rows: Vec<Vec<Inline>>,
    row: Vec<Inline>,
    cell: Option<Inline>,
    span: Range<usize>,
    marker: Marker,
    gap: f32,
}

struct Builder<'a> {
    src: &'a str,
    line_starts: Vec<usize>,
    blocks: Vec<Block>,
    links: Vec<String>,
    nest: Vec<Nest>,
    /// The next number of each open list (none for a bulleted one).
    lists: Vec<Option<u64>>,
    marker: Marker,
    leaf: Option<Leaf>,
    table: Option<TableBuild>,
    strong: u32,
    emph: u32,
    strike: u32,
    link: Vec<u32>,
    /// An image being read: its path and its alt text.
    image: Option<(String, String)>,
    quotes: u32,
    /// The next block is the first of a list that follows other blocks.
    list_start: bool,
    slugs: FxHashMap<String, u32>,
}

impl<'a> Builder<'a> {
    fn new(src: &'a str) -> Self {
        let line_starts =
            std::iter::once(0).chain(src.match_indices('\n').map(|(i, _)| i + 1)).collect::<Vec<_>>();
        Self {
            src,
            line_starts,
            blocks: Vec::new(),
            links: Vec::new(),
            nest: Vec::new(),
            lists: Vec::new(),
            marker: Marker::None,
            leaf: None,
            table: None,
            strong: 0,
            emph: 0,
            strike: 0,
            link: Vec::new(),
            image: None,
            quotes: 0,
            list_start: false,
            slugs: FxHashMap::default(),
        }
    }

    fn line_of(&self, offset: usize) -> u32 {
        u32::try_from(self.line_starts.partition_point(|&s| s <= offset)).unwrap_or(u32::MAX)
    }

    fn in_list(&self) -> bool {
        self.nest.contains(&Nest::List)
    }

    /// The room above the next block: none above the first, more above a heading, little between
    /// the items of a tight list.
    fn gap(&mut self, heading: Option<u8>, explicit: bool) -> f32 {
        let first_of_list = std::mem::take(&mut self.list_start);
        if self.blocks.is_empty() {
            return 0.0;
        }
        match heading {
            Some(1 | 2) => metrics::S24,
            Some(3) => metrics::S16,
            Some(_) => metrics::S12,
            None if first_of_list || !self.in_list() => metrics::S12,
            None if explicit => metrics::S8,
            None => metrics::S4,
        }
    }

    fn style(&self) -> u8 {
        let mut s = 0;
        if self.strong > 0 {
            s |= style::STRONG;
        }
        if self.emph > 0 {
            s |= style::EMPH;
        }
        if self.strike > 0 {
            s |= style::STRIKE;
        }
        s
    }

    fn add_link(&mut self, url: String) -> u32 {
        self.links.push(url);
        u32::try_from(self.links.len() - 1).unwrap_or(u32::MAX)
    }

    fn begin(&mut self, kind: LeafKind, span: Range<usize>, explicit: bool) {
        self.close_implicit();
        let heading = if let LeafKind::Heading(l) = kind { Some(l) } else { None };
        let gap = self.gap(heading, explicit);
        let marker = std::mem::replace(&mut self.marker, Marker::None);
        self.leaf =
            Some(Leaf { kind, inline: Inline::default(), code: String::new(), span, explicit, marker, gap });
    }

    fn close_implicit(&mut self) {
        if self.leaf.as_ref().is_some_and(|l| !l.explicit) {
            self.finish_leaf();
        }
    }

    fn finish_leaf(&mut self) {
        let Some(mut leaf) = self.leaf.take() else { return };
        let kind = match leaf.kind {
            LeafKind::Para => BlockKind::Para(leaf.inline),
            LeafKind::Html => {
                leaf.inline.trim_end_newlines();
                BlockKind::Para(leaf.inline)
            }
            LeafKind::Heading(level) => {
                let slug = self.unique_slug(&leaf.inline.plain());
                BlockKind::Heading { level, text: leaf.inline, slug }
            }
            LeafKind::Code(lang, fenced) => {
                if leaf.code.ends_with('\n') {
                    leaf.code.pop();
                }
                BlockKind::Code { lang, text: leaf.code, fenced }
            }
        };
        self.push(kind, leaf.span, leaf.marker, leaf.gap);
    }

    fn unique_slug(&mut self, text: &str) -> String {
        let base = slug(text);
        let seen = self.slugs.entry(base.clone()).or_insert(0);
        let out = if *seen == 0 { base } else { format!("{base}-{seen}") };
        *seen += 1;
        out
    }

    fn push(&mut self, kind: BlockKind, span: Range<usize>, marker: Marker, gap: f32) {
        let first = self.line_of(span.start);
        let last = self.line_of(span.end.saturating_sub(1).max(span.start)).max(first);
        let mut h = hasher();
        self.src.get(span).unwrap_or("").hash(&mut h);
        for n in &self.nest {
            matches!(n, Nest::Quote(_)).hash(&mut h);
        }
        marker.hash(&mut h);
        gap.to_bits().hash(&mut h);
        let (tag, size, breaks) = match &kind {
            BlockKind::Para(t) => (0, t.chars(), t.breaks()),
            BlockKind::Heading { level, text, .. } => (10 + level, text.chars(), text.breaks()),
            BlockKind::Code { text, .. } => (1, 0, text.lines().count()),
            BlockKind::Table { rows, .. } => (2, 0, rows.len() + 1),
            BlockKind::Rule => (3, 0, 0),
        };
        tag.hash(&mut h);
        self.blocks.push(Block {
            kind,
            first,
            last,
            nest: self.nest.clone(),
            marker,
            gap,
            hash: h.finish(),
            size,
            breaks,
        });
    }

    /// Where inline content goes: the open table cell, or the open block (a tight list item's text
    /// opens one).
    fn target(&mut self, span: &Range<usize>) -> Option<&mut Inline> {
        if self.table.is_some() {
            return self.table.as_mut().and_then(|t| t.cell.as_mut());
        }
        if self.leaf.is_none() {
            self.begin(LeafKind::Para, span.clone(), false);
        }
        let leaf = self.leaf.as_mut()?;
        leaf.span.end = leaf.span.end.max(span.end);
        Some(&mut leaf.inline)
    }

    fn text(&mut self, text: &str, extra: u8, span: &Range<usize>) {
        let text = if text.contains('\r') { Cow::Owned(text.replace('\r', "")) } else { Cow::Borrowed(text) };
        if let Some((_, alt)) = &mut self.image {
            alt.push_str(&text);
            return;
        }
        if let Some(leaf) = &mut self.leaf
            && let LeafKind::Code(..) = leaf.kind
        {
            leaf.code.push_str(&text);
            return;
        }
        let style = self.style() | extra;
        let link = self.link.last().copied();
        if let Some(inline) = self.target(span) {
            inline.push(&text, style, link);
        }
    }

    fn event(&mut self, event: Event<'_>, span: Range<usize>) {
        match event {
            Event::Start(tag) => self.start(tag, span),
            Event::End(tag) => self.end(tag, span),
            Event::Text(t) => self.text(&t, 0, &span),
            Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => {
                self.text(&t, style::CODE, &span)
            }
            Event::Html(t) => {
                if let Some(leaf) = &mut self.leaf
                    && leaf.kind == LeafKind::Html
                {
                    leaf.inline.push(&t.replace('\r', ""), style::HTML, None);
                } else {
                    self.text(&t, style::HTML, &span);
                }
            }
            Event::InlineHtml(t) => self.text(&t, style::HTML, &span),
            Event::FootnoteReference(t) => self.text(&format!("[^{t}]"), 0, &span),
            Event::SoftBreak => self.text(" ", 0, &span),
            Event::HardBreak => self.text("\n", 0, &span),
            Event::Rule => {
                self.close_implicit();
                let gap = self.gap(None, true);
                let marker = std::mem::replace(&mut self.marker, Marker::None);
                self.push(BlockKind::Rule, span, marker, gap);
            }
            Event::TaskListMarker(done) => {
                let task = Marker::Task(done);
                match &mut self.leaf {
                    // a loose item's paragraph has taken the item's marker already
                    Some(leaf)
                        if leaf.inline.spans.is_empty()
                            && matches!(leaf.marker, Marker::Bullet(_) | Marker::Number(_)) =>
                    {
                        leaf.marker = task;
                    }
                    _ if self.marker != Marker::None => self.marker = task,
                    _ => {}
                }
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>, span: Range<usize>) {
        match tag {
            Tag::Paragraph => self.begin(LeafKind::Para, span, true),
            Tag::Heading { level, .. } => self.begin(LeafKind::Heading(level as u8), span, true),
            Tag::BlockQuote(_) => {
                self.close_implicit();
                self.quotes += 1;
                self.nest.push(Nest::Quote(self.quotes));
            }
            Tag::CodeBlock(kind) => {
                let (lang, fenced) = match kind {
                    CodeBlockKind::Fenced(info) => (CodeLang::from_fence(&info), true),
                    CodeBlockKind::Indented => (CodeLang::Plain, false),
                };
                self.begin(LeafKind::Code(lang, fenced), span, true);
            }
            Tag::HtmlBlock => self.begin(LeafKind::Html, span, true),
            Tag::List(first) => {
                self.close_implicit();
                if !self.in_list() {
                    self.list_start = true;
                }
                self.lists.push(first);
                self.nest.push(Nest::List);
            }
            Tag::Item => {
                self.close_implicit();
                let depth = u8::try_from(self.lists.len().saturating_sub(1)).unwrap_or(u8::MAX);
                self.marker = match self.lists.last_mut() {
                    Some(Some(next)) => {
                        let m = Marker::Number(*next);
                        *next += 1;
                        m
                    }
                    _ => Marker::Bullet(depth),
                };
            }
            Tag::Table(align) => {
                self.close_implicit();
                let gap = self.gap(None, true);
                let marker = std::mem::replace(&mut self.marker, Marker::None);
                self.table = Some(TableBuild {
                    align,
                    head: Vec::new(),
                    rows: Vec::new(),
                    row: Vec::new(),
                    cell: None,
                    span,
                    marker,
                    gap,
                });
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = &mut self.table {
                    t.row.clear();
                }
            }
            Tag::TableCell => {
                if let Some(t) = &mut self.table {
                    t.cell = Some(Inline::default());
                }
            }
            Tag::Emphasis => self.emph += 1,
            Tag::Strong => self.strong += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => {
                let id = self.add_link(dest_url.to_string());
                self.link.push(id);
            }
            Tag::Image { dest_url, .. } => {
                // the image opens the paragraph it stands in
                let _ = self.target(&span);
                self.image = Some((dest_url.to_string(), String::new()));
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd, span: Range<usize>) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::HtmlBlock => {
                self.finish_leaf();
            }
            TagEnd::BlockQuote(_) => {
                self.close_implicit();
                self.nest.pop();
            }
            TagEnd::List(_) => {
                self.close_implicit();
                self.lists.pop();
                self.nest.pop();
            }
            TagEnd::Item => {
                self.close_implicit();
                // an empty item still shows its marker
                if self.marker != Marker::None {
                    let gap = self.gap(None, false);
                    let marker = std::mem::replace(&mut self.marker, Marker::None);
                    self.push(BlockKind::Para(Inline::default()), span, marker, gap);
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    let kind = BlockKind::Table { align: t.align, head: t.head, rows: t.rows };
                    self.push(kind, t.span, t.marker, t.gap);
                }
            }
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    t.head = std::mem::take(&mut t.row);
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            TagEnd::TableCell => {
                if let Some(t) = &mut self.table {
                    let cell = t.cell.take().unwrap_or_default();
                    t.row.push(cell);
                }
            }
            TagEnd::Emphasis => self.emph = self.emph.saturating_sub(1),
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link => {
                self.link.pop();
            }
            TagEnd::Image => {
                if let Some((path, alt)) = self.image.take() {
                    // a picture inside a link goes where the link does
                    let link = match self.link.last() {
                        Some(&l) => l,
                        None => self.add_link(path),
                    };
                    if let Some(inline) = self.target(&span) {
                        inline.push(&alt, style::PICTURE, Some(link));
                    }
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Doc {
        self.close_implicit();
        self.finish_leaf();
        Doc { blocks: self.blocks, links: self.links }
    }
}

// ---------------------------------------------------------------------------------------------
// Layout: a block as galleys and shapes, relative to the top-left of its slot (its gap included).

/// Room around the document.
const PAD: f32 = metrics::S12;
/// A list's indent: the column its markers stand in.
const LIST_INDENT: f32 = em(1.5);
const QUOTE_INDENT: f32 = metrics::S16;
const QUOTE_BAR: f32 = 3.0;
const CODE_PAD: f32 = metrics::S8;
const CELL_PAD: Vec2 = Vec2::new(metrics::S8, metrics::S4);
/// The narrowest a text column gets, however deep the lists go.
const MIN_TEXT_W: f32 = em(4.0);
/// A table column narrower than its text gets at least this much of it.
const MIN_COLUMN_W: f32 = em(5.0);

fn heading_font(level: u8) -> Font {
    match level {
        1 => Font::Page,
        2 => Font::Title,
        _ => Font::Strong,
    }
}

fn indent(nest: &[Nest]) -> f32 {
    nest.iter().map(|n| if *n == Nest::List { LIST_INDENT } else { QUOTE_INDENT }).sum()
}

/// The height of a block nobody has laid out yet, from its length.
fn estimate(b: &Block, w: f32) -> f32 {
    let tw = (w - indent(&b.nest)).max(MIN_TEXT_W);
    let prose = |font: Font| {
        let per_line = (tw / (font.size() * 0.5)).max(1.0);
        ((b.size as f32 / per_line).ceil().max(1.0) + b.breaks as f32) * font.line()
    };
    b.gap
        + match &b.kind {
            BlockKind::Para(_) => prose(Font::Body),
            BlockKind::Heading { level, .. } => {
                prose(heading_font(*level)) + if *level <= 2 { metrics::S4 + 1.0 } else { 0.0 }
            }
            BlockKind::Code { .. } => b.breaks.max(1) as f32 * Font::Mono.line() + 2.0 * CODE_PAD,
            BlockKind::Table { .. } => b.breaks as f32 * (Font::Body.line() + 2.0 * CELL_PAD.y),
            BlockKind::Rule => metrics::S8 + 1.0,
        }
}

/// The links of a galley: the first character, the one past the last, the link.
type Links = Vec<(u32, u32, u32)>;

/// A text of a block, selectable, with its links.
struct Text {
    at: Vec2,
    galley: Arc<Galley>,
    links: Links,
}

/// A code block's text, in a box that scrolls sideways when a line is wider.
struct CodeBox {
    rect: Rect,
    galley: Arc<Galley>,
}

/// A block laid out.
#[derive(Default)]
struct Laid {
    /// The slot: the gap and the block.
    h: f32,
    shapes: Vec<Shape>,
    texts: Vec<Text>,
    code: Option<CodeBox>,
    task: Option<(Rect, bool)>,
}

struct InlineJob {
    job: LayoutJob,
    links: Links,
}

fn format(p: &Palette, s: &Span, base: Font, color: Color32, hovered: bool) -> TextFormat {
    let st = s.style;
    let font_id = if st & style::CODE != 0 {
        if base == Font::Body { Font::Mono.id() } else { Font::Mono.at(base.size()) }
    } else if st & style::STRONG != 0 && base == Font::Body {
        Font::Strong.id()
    } else {
        base.id()
    };
    let color = if st & (style::PICTURE | style::HTML) != 0 {
        p.text_muted
    } else if s.link.is_some() {
        p.accent_bright
    } else {
        color
    };
    let mut f = TextFormat { font_id, line_height: Some(base.line()), color, ..TextFormat::default() };
    f.italics = st & style::EMPH != 0;
    if st & style::CODE != 0 {
        f.background = p.inset;
    } else if st & style::PICTURE != 0 {
        f.background = p.raised;
    }
    if st & style::STRIKE != 0 {
        f.strikethrough = Stroke::new(1.0, color);
    }
    if hovered {
        f.underline = Stroke::new(1.0, color);
    }
    f
}

fn inline_job(
    ctx: &Context,
    p: &Palette,
    text: &Inline,
    base: Font,
    color: Color32,
    width: f32,
    hover: Option<u32>,
) -> InlineJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    let mut links = Vec::new();
    let mut at = 0u32;
    for s in &text.spans {
        let picture;
        let shown: &str = if s.style & style::PICTURE == 0 {
            &s.text
        } else {
            picture = if s.text.is_empty() {
                crate::i18n::t(ctx, "[picture]")
            } else {
                crate::tr!("[picture: {alt}]", alt = s.text.as_str()).text(ctx)
            };
            &picture
        };
        job.append(shown, 0.0, format(p, s, base, color, s.link.is_some() && s.link == hover));
        let n = u32::try_from(shown.chars().count()).unwrap_or(u32::MAX);
        if let Some(link) = s.link {
            links.push((at, at.saturating_add(n), link));
        }
        at = at.saturating_add(n);
    }
    InlineJob { job, links }
}

fn galley(ctx: &Context, job: LayoutJob) -> Arc<Galley> {
    ctx.fonts_mut(|f| f.layout_job(job))
}

/// The half height of a galley's first row: where a list marker stands.
fn first_mid(g: &Galley, fallback: f32) -> f32 {
    g.rows.first().map_or(fallback, |r| r.rect().height()) * 0.5
}

/// The link under a point of a galley (relative to it): only over a character, not the room
/// beside a line.
fn link_at(g: &Galley, links: &[(u32, u32, u32)], pos: Vec2) -> Option<u32> {
    let mut ci = 0u32;
    for row in &g.rows {
        let r = row.rect();
        if pos.y >= r.min.y && pos.y < r.max.y {
            let k = row.glyphs.iter().position(|gl| {
                let x0 = row.pos.x + gl.pos.x;
                pos.x >= x0 && pos.x < x0 + gl.advance_width
            })?;
            let c = ci.saturating_add(u32::try_from(k).unwrap_or(u32::MAX));
            return links.iter().find(|(s, e, _)| c >= *s && c < *e).map(|l| l.2);
        }
        let n = row.glyphs.len() + usize::from(row.ends_with_newline);
        ci = ci.saturating_add(u32::try_from(n).unwrap_or(u32::MAX));
    }
    None
}

/// What a table needs to lay itself out.
struct TableIn<'a> {
    x: f32,
    top: f32,
    w: f32,
    align: &'a [Alignment],
    head: &'a [Inline],
    rows: &'a [Vec<Inline>],
    color: Color32,
    hover: Option<u32>,
}

/// Lays out a table: columns as wide as their text when it fits, shared out by it when not. Returns
/// its height and where its first line's middle is.
fn table(ctx: &Context, p: &Palette, t: &TableIn<'_>, l: &mut Laid) -> (f32, f32) {
    let empty = Inline::default();
    let ncols = t.rows.iter().map(Vec::len).chain([t.head.len()]).max().unwrap_or(0).max(1);
    let cell = |r: usize, c: usize| -> &Inline {
        if r == 0 { t.head.get(c) } else { t.rows.get(r - 1).and_then(|row| row.get(c)) }.unwrap_or(&empty)
    };
    let font = |r: usize| if r == 0 { Font::Strong } else { Font::Body };
    let nrows = t.rows.len() + 1;
    // the cells in one line each: their natural widths
    let mut natural: Vec<Option<Text>> = Vec::with_capacity(nrows * ncols);
    let mut nat = vec![0.0f32; ncols];
    for r in 0..nrows {
        for (c, n) in nat.iter_mut().enumerate() {
            let j = inline_job(ctx, p, cell(r, c), font(r), t.color, f32::INFINITY, t.hover);
            let g = galley(ctx, j.job);
            *n = n.max(g.size().x);
            natural.push(Some(Text { at: Vec2::ZERO, galley: g, links: j.links }));
        }
    }
    let pads = 2.0 * CELL_PAD.x;
    let need: f32 = nat.iter().map(|n| n + pads).sum();
    let widths: Vec<f32> = if need <= t.w {
        nat.iter().map(|n| n + pads).collect()
    } else {
        nat.iter().map(|n| ((n + pads) * t.w / need).max((n + pads).min(MIN_COLUMN_W))).collect()
    };
    let table_w: f32 = widths.iter().sum();
    let stroke = Stroke::new(1.0, p.border_subtle);
    let radius = CornerRadius::same(metrics::RADIUS);
    let mut y = t.top;
    for (r, row) in natural.chunks_mut(ncols).enumerate() {
        let mut cells = Vec::with_capacity(ncols);
        let mut row_h = font(r).line();
        for (c, slot) in row.iter_mut().enumerate() {
            let room = widths[c] - pads;
            let text = match slot.take() {
                Some(text) if text.galley.size().x <= room + 0.01 => text,
                _ => {
                    let j = inline_job(ctx, p, cell(r, c), font(r), t.color, room.max(1.0), t.hover);
                    Text { at: Vec2::ZERO, galley: galley(ctx, j.job), links: j.links }
                }
            };
            row_h = row_h.max(text.galley.size().y);
            cells.push(text);
        }
        let rh = row_h + 2.0 * CELL_PAD.y;
        if r == 0 {
            let head = Rect::from_min_size(pos2(t.x, y), vec2(table_w, rh));
            let top_only = CornerRadius { nw: metrics::RADIUS, ne: metrics::RADIUS, sw: 0, se: 0 };
            l.shapes.push(Shape::rect_filled(head, top_only, p.raised));
        }
        let mut cx = t.x;
        for (c, mut text) in cells.into_iter().enumerate() {
            let room = widths[c] - pads;
            let w = text.galley.size().x;
            let dx = match t.align.get(c) {
                Some(Alignment::Center) => (room - w) * 0.5,
                Some(Alignment::Right) => room - w,
                _ => 0.0,
            }
            .max(0.0);
            text.at = vec2(cx + CELL_PAD.x + dx, y + CELL_PAD.y);
            l.texts.push(text);
            cx += widths[c];
        }
        y += rh;
        if r + 1 < nrows {
            l.shapes.push(Shape::hline(t.x..=t.x + table_w, y, stroke));
        }
    }
    let mut cx = t.x;
    for w in &widths[..ncols - 1] {
        cx += w;
        l.shapes.push(Shape::vline(cx, t.top..=y, stroke));
    }
    let frame = Rect::from_min_max(pos2(t.x, t.top), pos2(t.x + table_w, y));
    l.shapes.push(Shape::rect_stroke(frame, radius, stroke, StrokeKind::Inside));
    (y - t.top, CELL_PAD.y + Font::Strong.line() * 0.5)
}

// ---------------------------------------------------------------------------------------------
// The view: what a preview keeps between frames.

/// The block at the top of the view and how far into its slot the view starts.
#[derive(Clone, Copy, Debug)]
struct Anchor {
    block: usize,
    /// The block's first line: where to look for it again after the text changed.
    line: u32,
    rel: f32,
}

#[derive(Default)]
struct View {
    text_hash: u64,
    doc: Option<Arc<Doc>>,
    /// What the measured heights hold for: the width, the theme, the scale, the language.
    key: u64,
    synced: bool,
    /// The height of every block laid out, by its hash.
    measured: FxHashMap<u64, f32>,
    /// The height of each block of the document: measured, or estimated.
    heights: Vec<f32>,
    /// Coloured code, by block and theme: colouring is worth keeping.
    jobs: FxHashMap<u64, Arc<LayoutJob>>,
    anchor: Option<Anchor>,
    /// The scroll offset this view left the scroll area at.
    last_offset: Option<f32>,
    /// A heading an anchor link asked for.
    jump: Option<usize>,
    /// The link under the pointer in the last frame: it is drawn underlined.
    hover: Option<u32>,
    /// How many blocks the last frame laid out.
    laid: usize,
}

#[derive(Clone, Default)]
struct Handle(Arc<Mutex<View>>);

fn view_id(ui: &Ui, id_salt: &str) -> Id {
    ui.make_persistent_id(("gz-ui.markdown", id_salt))
}

fn handle(ctx: &Context, id: Id) -> Handle {
    ctx.data(|d| d.get_temp::<Handle>(id)).unwrap_or_else(|| {
        let made = Handle::default();
        ctx.data_mut(|d| d.insert_temp(id, made.clone()));
        made
    })
}

fn measure_key(ctx: &Context, p: &Palette, w: f32) -> u64 {
    let mut h = hasher();
    ((w * 2.0).round() as i64).hash(&mut h);
    p.key().hash(&mut h);
    ctx.pixels_per_point().to_bits().hash(&mut h);
    crate::i18n::lang(ctx).hash(&mut h);
    h.finish()
}

/// The top of each block and, last, the whole height.
fn prefix(heights: &[f32]) -> Vec<f32> {
    let mut tops = Vec::with_capacity(heights.len() + 1);
    let mut y = 0.0;
    tops.push(y);
    for h in heights {
        y += h;
        tops.push(y);
    }
    tops
}

/// The block of a changed text that the anchor was: the same block near where it was, else the one
/// at its line.
fn remap(doc: &Doc, old: usize, hash: Option<u64>, line: u32) -> usize {
    if let Some(h) = hash {
        let lo = old.saturating_sub(64);
        let hi = old.saturating_add(64).min(doc.blocks.len());
        if let Some(i) = (lo..hi).filter(|&i| doc.blocks[i].hash == h).min_by_key(|&i| i.abs_diff(old)) {
            return i;
        }
    }
    doc.block_at_line(line)
}

/// The source line at the view's top: in a code block the row there, else the block's first line.
fn top_line(b: &Block, l: &Laid, rel: f32) -> u32 {
    if let (BlockKind::Code { fenced, .. }, Some(cb)) = (&b.kind, &l.code) {
        let y = rel - cb.rect.min.y;
        if y >= 0.0 {
            let row = cb.galley.rows.iter().take_while(|r| r.pos.y <= y + 0.01).count().saturating_sub(1);
            let first = b.first + u32::from(*fenced);
            let last = if *fenced { b.last.saturating_sub(1).max(first) } else { b.last };
            return first.saturating_add(u32::try_from(row).unwrap_or(u32::MAX)).min(last);
        }
    }
    b.first
}

/// Where the view starts in a block it jumps to: a code line at the top, else a little above the
/// block's text.
fn jump_rel(b: &Block, l: &Laid, i: usize, line: u32) -> f32 {
    if let (BlockKind::Code { fenced, .. }, Some(cb)) = (&b.kind, &l.code) {
        let first = b.first + u32::from(*fenced);
        if line >= first && !cb.galley.rows.is_empty() {
            let k = usize::try_from(line - first).unwrap_or(usize::MAX).min(cb.galley.rows.len() - 1);
            return cb.rect.min.y + cb.galley.rows[k].pos.y;
        }
    }
    if i == 0 { -PAD } else { (b.gap - metrics::S8).max(0.0) }
}

impl View {
    /// Reads the text when it changed, keeping the anchor on the block it was on.
    fn load(&mut self, text: &str) {
        let h = hash_of(text);
        if self.doc.is_some() && h == self.text_hash {
            return;
        }
        let doc = Arc::new(parse(text));
        if let (Some(an), Some(old)) = (self.anchor.as_mut(), self.doc.as_ref()) {
            let was = old.blocks.get(an.block).map(|b| b.hash);
            an.block = remap(&doc, an.block, was, an.line);
            an.line = doc.blocks.get(an.block).map_or(1, |b| b.first);
        }
        self.doc = Some(doc);
        self.text_hash = h;
        self.hover = None;
        self.jump = None;
        self.synced = false;
    }

    /// The heights for this document at this width: what was measured, else estimates.
    fn sync(&mut self, doc: &Doc, key: u64, w: f32) {
        if self.synced && key == self.key {
            return;
        }
        if key != self.key {
            self.measured.clear();
            self.key = key;
        }
        let n = doc.blocks.len();
        // what an edit left behind is dropped now and then
        if self.measured.len() > 4 * n + 256 {
            let live: rustc_hash::FxHashSet<u64> = doc.blocks.iter().map(|b| b.hash).collect();
            self.measured.retain(|h, _| live.contains(h));
        }
        if self.jobs.len() > 2 * n + 64 {
            self.jobs.clear();
        }
        self.heights = doc
            .blocks
            .iter()
            .map(|b| self.measured.get(&b.hash).copied().unwrap_or_else(|| estimate(b, w)))
            .collect();
        self.synced = true;
    }

    fn code_job(
        &mut self,
        ctx: &Context,
        p: &Palette,
        b: &Block,
        lang: CodeLang,
        text: &str,
    ) -> Arc<LayoutJob> {
        let key = b.hash ^ p.key().rotate_left(17);
        self.jobs
            .entry(key)
            .or_insert_with(|| Arc::new(code::highlight(ctx, lang, text, &Font::Mono.id())))
            .clone()
    }

    /// Lays a block out at the document's width `w` and keeps its height.
    fn lay(&mut self, ctx: &Context, p: &Palette, doc: &Doc, i: usize, w: f32, bar: f32) -> Laid {
        self.laid += 1;
        let b = &doc.blocks[i];
        let mut l = Laid::default();
        let quoted = b.nest.iter().any(|n| matches!(n, Nest::Quote(_)));
        let color = if quoted { p.text_secondary } else { p.text };
        let mut x = 0.0;
        let mut marker_x = None;
        let mut bars = Vec::new();
        let last_list = b.nest.iter().rposition(|n| *n == Nest::List);
        for (k, n) in b.nest.iter().enumerate() {
            match n {
                Nest::List => {
                    if last_list == Some(k) {
                        marker_x = Some(x);
                    }
                    x += LIST_INDENT;
                }
                Nest::Quote(q) => {
                    bars.push((x, *q));
                    x += QUOTE_INDENT;
                }
            }
        }
        let tw = (w - x).max(MIN_TEXT_W);
        let top = b.gap;
        let hover = self.hover;
        let (content, mid) = match &b.kind {
            BlockKind::Para(text) => {
                let j = inline_job(ctx, p, text, Font::Body, color, tw, hover);
                let g = galley(ctx, j.job);
                let mid = first_mid(&g, Font::Body.line());
                let h = g.size().y.max(Font::Body.line());
                l.texts.push(Text { at: vec2(x, top), galley: g, links: j.links });
                (h, mid)
            }
            BlockKind::Heading { level, text, .. } => {
                let font = heading_font(*level);
                let j = inline_job(ctx, p, text, font, color, tw, hover);
                let g = galley(ctx, j.job);
                let mid = first_mid(&g, font.line());
                let mut h = g.size().y.max(font.line());
                l.texts.push(Text { at: vec2(x, top), galley: g, links: j.links });
                if *level <= 2 {
                    let y = top + h + metrics::S4 + 0.5;
                    l.shapes.push(Shape::hline(x..=x + tw, y, Stroke::new(1.0, p.border_subtle)));
                    h += metrics::S4 + 1.0;
                }
                (h, mid)
            }
            BlockKind::Code { lang, text, .. } => {
                let job = self.code_job(ctx, p, b, *lang, text);
                let g = galley(ctx, (*job).clone());
                let inner = (tw - 2.0 * CODE_PAD).max(1.0);
                let gh = g.size().y.max(Font::Mono.line());
                let scroll = if g.size().x > inner { bar } else { 0.0 };
                let h = gh + scroll + 2.0 * CODE_PAD;
                let mid = CODE_PAD + first_mid(&g, Font::Mono.line());
                let back = Rect::from_min_size(pos2(x, top), vec2(tw, h));
                l.shapes.push(Shape::rect_filled(back, CornerRadius::same(metrics::RADIUS), p.inset));
                let rect = Rect::from_min_size(pos2(x + CODE_PAD, top + CODE_PAD), vec2(inner, gh + scroll));
                l.code = Some(CodeBox { rect, galley: g });
                (h, mid)
            }
            BlockKind::Table { align, head, rows } => {
                let t = TableIn { x, top, w: tw, align, head, rows, color, hover };
                table(ctx, p, &t, &mut l)
            }
            BlockKind::Rule => {
                let y = top + metrics::S4 + 0.5;
                l.shapes.push(Shape::hline(x..=x + tw, y, Stroke::new(1.0, p.border)));
                (metrics::S8 + 1.0, 0.0)
            }
        };
        let bottom = top + content;
        // a quote's bar runs on through the gaps between its blocks
        for (bx, q) in bars {
            let joined = i > 0 && doc.blocks[i - 1].nest.contains(&Nest::Quote(q));
            let y0 = if joined { 0.0 } else { top };
            let r = Rect::from_min_max(pos2(bx, y0), pos2(bx + QUOTE_BAR, bottom));
            l.shapes.push(Shape::rect_filled(r, CornerRadius::same(1), p.border_strong));
        }
        if let Some(mx) = marker_x {
            let at = pos2(mx + metrics::S8 + 2.0, top + mid);
            match b.marker {
                Marker::Bullet(depth) => l.shapes.push(match depth % 3 {
                    0 => Shape::circle_filled(at, 2.5, p.text_secondary),
                    1 => Shape::circle_stroke(at, 2.5, Stroke::new(1.0, p.text_secondary)),
                    _ => Shape::rect_filled(
                        Rect::from_center_size(at, Vec2::splat(4.5)),
                        CornerRadius::ZERO,
                        p.text_secondary,
                    ),
                }),
                Marker::Number(n) => {
                    let g = ctx
                        .fonts_mut(|f| f.layout_no_wrap(format!("{n}."), Font::Body.id(), p.text_secondary));
                    let right = mx + LIST_INDENT - metrics::S6;
                    let pos = pos2(right - g.size().x, at.y - g.size().y * 0.5);
                    l.shapes.push(Shape::galley(pos, g, p.text_secondary));
                }
                Marker::Task(done) => l.task = Some((Rect::from_center_size(at, Vec2::splat(13.0)), done)),
                Marker::None => {}
            }
        }
        l.h = bottom;
        self.measured.insert(b.hash, bottom);
        if let Some(h) = self.heights.get_mut(i) {
            *h = bottom;
        }
        l
    }

    /// One frame of the scroll area's content.
    fn frame(&mut self, ui: &mut Ui, base: Id, viewport: Rect, goto: u32) -> MarkdownOut {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let mut out = MarkdownOut { link: None, top_line: 1 };
        let Some(doc) = self.doc.clone() else { return out };
        let n = doc.blocks.len();
        let full_w = ui.available_width();
        let w = (full_w - 2.0 * PAD).max(MIN_TEXT_W);
        let bar = ui.spacing().scroll.allocated_width();
        self.sync(&doc, measure_key(&ctx, &p, w), w);
        self.laid = 0;
        let o = viewport.min.y;
        let view_top = ui.max_rect().top() + o;
        let view_bottom = (view_top + viewport.height()).min(ui.clip_rect().bottom());
        let left = ui.max_rect().left() + PAD;
        if n == 0 {
            ui.set_min_size(vec2(full_w, 2.0 * PAD));
            self.anchor = None;
            self.last_offset = Some(o);
            return out;
        }

        let tops = prefix(&self.heights);
        let mut ready: Vec<(usize, Laid)> = Vec::new();
        let jump =
            if goto > 0 { Some((doc.block_at_line(goto), goto)) } else { self.jump.take().map(|b| (b, 0)) };
        let (mut a, mut rel) = if let Some((b, line)) = jump {
            let l = self.lay(&ctx, &p, &doc, b, w, bar);
            let rel = jump_rel(&doc.blocks[b], &l, b, line);
            ready.push((b, l));
            (b, rel)
        } else if let Some(an) = self.anchor.filter(|an| an.block < n) {
            (an.block, an.rel + self.last_offset.map_or(0.0, |last| o - last))
        } else {
            let y = o - PAD;
            let a = tops.partition_point(|t| *t <= y).saturating_sub(1).min(n - 1);
            (a, y - tops[a])
        };
        // a block coming into view from above is measured before anything is drawn, so what was at
        // the top stays where it was
        while rel < 0.0 && a > 0 {
            a -= 1;
            let l = self.lay(&ctx, &p, &doc, a, w, bar);
            rel += l.h;
            ready.push((a, l));
        }
        while a + 1 < n && rel >= self.heights[a] {
            rel -= self.heights[a];
            a += 1;
        }
        if a == 0 {
            rel = rel.max(-PAD);
        }

        let mut hover = None;
        let mut click = None;
        let mut y = view_top - rel;
        let mut i = a;
        while i < n && (i == a || y < view_bottom) {
            let l = match ready.iter().position(|(k, _)| *k == i) {
                Some(k) => ready.swap_remove(k).1,
                None => self.lay(&ctx, &p, &doc, i, w, bar),
            };
            if i == a {
                out.top_line = top_line(&doc.blocks[a], &l, rel);
            }
            paint(ui, &l, pos2(left, y), base.with(i), &mut hover, &mut click);
            y += l.h;
            i += 1;
        }

        if hover != self.hover {
            self.hover = hover;
            ctx.request_repaint();
        }
        if let Some(link) = click
            && let Some(url) = doc.links.get(link as usize)
        {
            if let Some(fragment) = url.strip_prefix('#') {
                let want = percent_decode(fragment).to_lowercase();
                let found = doc
                    .blocks
                    .iter()
                    .position(|b| matches!(&b.kind, BlockKind::Heading { slug, .. } if *slug == want));
                if let Some(b) = found {
                    self.jump = Some(b);
                    ctx.request_repaint();
                }
            } else {
                out.link = Some(url.clone());
            }
        }

        // the content is as tall as the blocks say now; the scroll offset follows the anchor
        let tops = prefix(&self.heights);
        let content_h = tops[n] + 2.0 * PAD;
        ui.set_min_size(vec2(full_w, content_h));
        let max_e = (content_h - viewport.height()).max(0.0);
        // past an edge egui is springing back on its own: leave the offset to it
        let e = if (-0.5..=max_e + 0.5).contains(&o) { (PAD + tops[a] + rel).clamp(0.0, max_e) } else { o };
        rel = e - PAD - tops[a];
        let corrected = (e - o).abs() > 0.5;
        if corrected {
            ui.scroll_with_delta_animation(vec2(0.0, o - e), ScrollAnimation::none());
            ctx.request_repaint();
        }
        self.anchor = Some(Anchor { block: a, line: doc.blocks[a].first, rel });
        self.last_offset = Some(if corrected { e } else { o });
        out
    }
}

/// Paints a laid-out block with its slot's top-left at `at`: the shapes, then each text as a
/// selectable label that answers its links.
fn paint(ui: &mut Ui, l: &Laid, at: Pos2, id: Id, hover: &mut Option<u32>, click: &mut Option<u32>) {
    let off = at.to_vec2();
    let clip = ui.clip_rect();
    for s in &l.shapes {
        let mut s = s.clone();
        s.translate(off);
        ui.painter().add(s);
    }
    if let Some((r, done)) = l.task {
        crate::check::paint_box(ui, r.translate(off), done, false, false, true);
    }
    if let Some(cb) = &l.code {
        let rect = cb.rect.translate(off);
        if clip.intersects(rect) {
            let mut child = ui.new_child(UiBuilder::new().scope_id(id.with("code")).max_rect(rect));
            ScrollArea::horizontal()
                .id_salt("code")
                .auto_shrink([false, true])
                .content_margin(Margin::ZERO)
                .show(&mut child, |ui| ui.add(Label::new(cb.galley.clone()).selectable(true)));
        }
    }
    for (k, t) in l.texts.iter().enumerate() {
        let rect = Rect::from_min_size(at + t.at, t.galley.size());
        if !clip.intersects(rect) {
            continue;
        }
        let mut child = ui.new_child(UiBuilder::new().scope_id(id.with(k)).max_rect(rect));
        let resp = child.add(Label::new(t.galley.clone()).selectable(true));
        if t.links.is_empty() {
            continue;
        }
        if let Some(pos) = resp.hover_pos()
            && let Some(link) = link_at(&t.galley, &t.links, pos - rect.min)
        {
            *hover = Some(link);
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            if resp.clicked() {
                *click = Some(link);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::style::ScrollAnimation;
    use egui::{Rect, UiBuilder, Vec2, pos2, vec2};
    use pulldown_cmark::Alignment;

    use super::{
        BlockKind, MarkdownOut, Marker, Nest, PAD, handle, markdown_view, metrics, parse, percent_decode,
        slug, style, view_id,
    };
    use crate::testkit::{click_at, context, frame};

    const SAMPLE: &str = "# Title\n\nSome *text*\nand more.\n\n- one\n- two\n  - deep\n\n3. three\n4. four\n\n\
        - [x] done\n- [ ] open\n\n> quoted\n> still\n\n```lua\nx = 1\ny = 2\n```\n\n| a | b |\n|:--|--:|\n\
        | 1 | 2 |\n\n---\n";

    fn para(b: &super::Block) -> String {
        match &b.kind {
            BlockKind::Para(t) | BlockKind::Heading { text: t, .. } => t.plain().trim().to_owned(),
            other => panic!("not a paragraph: {other:?}"),
        }
    }

    #[test]
    fn the_blocks_say_what_they_are_and_where_they_came_from() {
        let doc = parse(SAMPLE);
        let b = &doc.blocks;
        assert_eq!(b.len(), 13, "{b:#?}");
        let lines: Vec<(u32, u32)> = b.iter().map(|b| (b.first, b.last)).collect();
        assert_eq!(
            lines,
            [
                (1, 1),
                (3, 4),
                (6, 6),
                (7, 7),
                (8, 8),
                (10, 10),
                (11, 11),
                (13, 13),
                (14, 14),
                (16, 17),
                (19, 22),
                (24, 26),
                (28, 28)
            ]
        );
        assert!(matches!(&b[0].kind, BlockKind::Heading { level: 1, slug, .. } if slug == "title"));
        assert_eq!(b[0].gap, 0.0);
        assert_eq!(para(&b[1]), "Some text and more.");
        // a list: the first item keeps the room of a block, the rest are tight
        assert_eq!((para(&b[2]), b[2].marker, b[2].gap), ("one".to_owned(), Marker::Bullet(0), metrics::S12));
        assert_eq!((para(&b[3]), b[3].marker, b[3].gap), ("two".to_owned(), Marker::Bullet(0), metrics::S4));
        assert_eq!(b[3].nest, [Nest::List]);
        assert_eq!((para(&b[4]), b[4].marker), ("deep".to_owned(), Marker::Bullet(1)));
        assert_eq!(b[4].nest, [Nest::List, Nest::List]);
        assert_eq!((b[5].marker, b[6].marker), (Marker::Number(3), Marker::Number(4)));
        assert_eq!((para(&b[7]), b[7].marker), ("done".to_owned(), Marker::Task(true)));
        assert_eq!((para(&b[8]), b[8].marker), ("open".to_owned(), Marker::Task(false)));
        assert_eq!(para(&b[9]), "quoted still");
        assert!(matches!(b[9].nest[..], [Nest::Quote(_)]));
        assert_eq!(
            b[10].kind,
            BlockKind::Code {
                lang: crate::code::CodeLang::Lua,
                text: "x = 1\ny = 2".to_owned(),
                fenced: true
            }
        );
        let BlockKind::Table { align, head, rows } = &b[11].kind else { panic!("{:?}", b[11].kind) };
        assert_eq!(align, &[Alignment::Left, Alignment::Right]);
        let plain =
            |row: &[super::Inline]| row.iter().map(|c| c.plain().trim().to_owned()).collect::<Vec<_>>();
        assert_eq!(plain(head), ["a", "b"]);
        assert_eq!(rows.iter().map(|r| plain(r)).collect::<Vec<_>>(), [["1", "2"]]);
        assert_eq!(b[12].kind, BlockKind::Rule);
    }

    #[test]
    fn inline_runs_keep_their_style_and_their_links() {
        let doc =
            parse("A [link](https://x.org) and ![the hut](p/hut.png) ~~gone~~ <b>html</b> **`code`**\n");
        let BlockKind::Para(t) = &doc.blocks[0].kind else { panic!() };
        let runs: Vec<(&str, u8, Option<u32>)> =
            t.spans.iter().map(|s| (s.text.as_str(), s.style, s.link)).collect();
        assert_eq!(
            runs,
            [
                ("A ", 0, None),
                ("link", 0, Some(0)),
                (" and ", 0, None),
                ("the hut", style::PICTURE, Some(1)),
                (" ", 0, None),
                ("gone", style::STRIKE, None),
                (" ", 0, None),
                ("<b>", style::HTML, None),
                ("html", 0, None),
                ("</b>", style::HTML, None),
                (" ", 0, None),
                ("code", style::STRONG | style::CODE, None),
            ]
        );
        assert_eq!(doc.links, ["https://x.org", "p/hut.png"]);
        // a picture inside a link goes where the link does
        let doc = parse("[![badge](b.svg)](https://ci.example)\n");
        let BlockKind::Para(t) = &doc.blocks[0].kind else { panic!() };
        assert_eq!(t.spans[0].link.map(|l| doc.links[l as usize].as_str()), Some("https://ci.example"));
    }

    #[test]
    fn headings_get_the_anchors_github_gives_them() {
        assert_eq!(slug("Hello, World!"), "hello-world");
        assert_eq!(slug("Раздел два: итоги"), "раздел-два-итоги");
        assert_eq!(slug("API v2.0 — notes"), "api-v20--notes");
        assert_eq!(slug("snake_case and-dash"), "snake_case-and-dash");
        let doc = parse("# A\n\n## A\n\n### A\n");
        let slugs: Vec<&str> = doc
            .blocks
            .iter()
            .filter_map(|b| {
                if let BlockKind::Heading { slug, .. } = &b.kind { Some(slug.as_str()) } else { None }
            })
            .collect();
        assert_eq!(slugs, ["a", "a-1", "a-2"]);
        assert_eq!(percent_decode("%D1%80%D0%B0%D0%B7"), "раз");
        assert_eq!(percent_decode("a%zz%4"), "a%zz%4");
    }

    #[test]
    fn a_line_finds_the_block_that_holds_it() {
        let doc = parse(SAMPLE);
        assert_eq!(doc.block_at_line(1), 0);
        // a blank line belongs to the block after it
        assert_eq!(doc.block_at_line(2), 1);
        assert_eq!(doc.block_at_line(4), 1);
        assert_eq!(doc.block_at_line(9), 5);
        assert_eq!(doc.block_at_line(21), 10);
        assert_eq!(doc.block_at_line(999), doc.blocks.len() - 1);
    }

    /// A text of 400 sections: a heading, a paragraph, a list, a table and code each.
    fn long_text() -> String {
        let mut s = String::new();
        for k in 0..400 {
            s.push_str(&format!(
                "## Section {k}\n\nThe paragraph of section {k} says what the section is about, with **strong** \
                 words, a [link](docs/{k}.md) and `code` in it.\n\n- first point\n- second point\n  - a nested \
                 point\n\n| Key | Value |\n|---|---:|\n| a{k} | {k} |\n\n```lua\nlocal x = {k}\nreturn x\n```\n\n"
            ));
        }
        s
    }

    fn line_of(text: &str, needle: &str) -> u32 {
        let at = text.find(needle).unwrap_or_else(|| panic!("{needle} is not in the text"));
        u32::try_from(text[..at].matches('\n').count() + 1).unwrap()
    }

    struct Run {
        ctx: egui::Context,
        t: f64,
        /// What the next frame scrolls by before the preview draws: what a wheel would do.
        scroll: f32,
    }

    impl Run {
        fn new() -> Self {
            Self { ctx: context(), t: 0.0, scroll: 0.0 }
        }

        /// One frame of the preview in a box of `size` at the screen's top-left; what it answered,
        /// how many blocks it laid out and where its scroll area's content starts.
        fn frame(
            &mut self,
            text: &str,
            size: Vec2,
            goto: &mut u32,
            events: Vec<egui::Event>,
        ) -> (MarkdownOut, usize, egui::Pos2) {
            self.t += 0.05;
            let mut got = (MarkdownOut::default(), 0, pos2(0.0, 0.0));
            frame(&self.ctx, self.t, events, |ui| {
                let rect = Rect::from_min_size(ui.max_rect().min, size);
                ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                    let origin = ui.max_rect().min;
                    let scroll = std::mem::take(&mut self.scroll);
                    if scroll != 0.0 {
                        ui.scroll_with_delta_animation(vec2(0.0, -scroll), ScrollAnimation::none());
                    }
                    let out = markdown_view(ui, "test", text, goto);
                    let laid = handle(ui.ctx(), view_id(ui, "test")).0.lock().laid;
                    got = (out, laid, origin);
                });
            });
            got
        }
    }

    #[test]
    fn a_long_text_lays_out_only_what_is_in_view_and_keeps_its_place() {
        let text = long_text();
        assert!(text.len() > 50_000, "{} bytes", text.len());
        let blocks = parse(&text).blocks.len();
        let mut run = Run::new();
        let size = vec2(800.0, 600.0);
        let mut goto = 0;
        let (out, laid, _) = run.frame(&text, size, &mut goto, Vec::new());
        assert_eq!(out.top_line, 1);
        assert!(laid > 0 && laid * 20 < blocks, "{laid} of {blocks} blocks laid out");

        // to the middle: the heading comes to the top, and stays there
        let line = line_of(&text, "## Section 200\n");
        goto = line;
        let (out, laid, _) = run.frame(&text, size, &mut goto, Vec::new());
        assert_eq!((goto, out.top_line), (0, line));
        assert!(laid * 20 < blocks, "{laid} of {blocks} blocks laid out");
        for _ in 0..3 {
            let (out, laid, _) = run.frame(&text, size, &mut goto, Vec::new());
            assert_eq!(out.top_line, line);
            assert!(laid * 20 < blocks, "{laid} of {blocks} blocks laid out");
        }

        // another width measures everything again; the block at the top stays
        for _ in 0..2 {
            let (out, _, _) = run.frame(&text, vec2(560.0, 600.0), &mut goto, Vec::new());
            assert_eq!(out.top_line, line);
        }

        // an edit above it moves its lines, not the view
        let edited = format!("An introduction.\n\n{text}");
        for _ in 0..2 {
            let (out, _, _) = run.frame(&edited, vec2(560.0, 600.0), &mut goto, Vec::new());
            assert_eq!(out.top_line, line + 2);
        }

        // a code line comes to the top itself
        let code_line = line_of(&edited, "local x = 321\n");
        goto = code_line;
        for _ in 0..2 {
            let (out, _, _) = run.frame(&edited, vec2(560.0, 600.0), &mut goto, Vec::new());
            assert_eq!(out.top_line, code_line);
        }
    }

    /// The section a source line of [`long_text`] is in.
    fn section_at(text: &str, line: u32) -> usize {
        let upto: String = text.split_inclusive('\n').take(line as usize).collect();
        let at = upto.rfind("## Section ").expect("a section above the line");
        upto[at + 11..].split_whitespace().next().and_then(|n| n.parse().ok()).expect("its number")
    }

    #[test]
    fn scrolling_up_past_estimates_moves_as_far_as_asked() {
        let text = long_text();
        let size = vec2(800.0, 600.0);
        let mut run = Run::new();
        let start = line_of(&text, "## Section 200\n");
        let mut goto = start;
        let (mut out, ..) = run.frame(&text, size, &mut goto, Vec::new());
        for _ in 0..20 {
            run.scroll = -50.0;
            let (now, laid, _) = run.frame(&text, size, &mut goto, Vec::new());
            assert!(now.top_line <= out.top_line, "{} after {}", now.top_line, out.top_line);
            assert!(laid < 40, "{laid} blocks laid out");
            out = now;
        }
        // a scroll asked for in a frame shows in the next one
        let (mut out, ..) = run.frame(&text, size, &mut goto, Vec::new());
        // a section is about 280 pt: 1000 pt up is three or four sections, whatever was estimated
        let section = section_at(&text, out.top_line);
        assert!((195..=198).contains(&section), "section {section}");
        // and back down again, to the very place
        for _ in 0..20 {
            run.scroll = 50.0;
            let (now, ..) = run.frame(&text, size, &mut goto, Vec::new());
            assert!(now.top_line >= out.top_line, "{} after {}", now.top_line, out.top_line);
            out = now;
        }
        let (out, ..) = run.frame(&text, size, &mut goto, Vec::new());
        assert_eq!(out.top_line, start);
    }

    #[test]
    fn a_link_comes_back_and_an_anchor_scrolls_to_its_heading() {
        let mut text = String::from("[Open the format](docs/nq/NQ_FORMAT.md) or [jump](#раздел-два)\n\n");
        for k in 0..300 {
            text.push_str(&format!("Filler paragraph {k}.\n\n"));
        }
        text.push_str("## Раздел два\n\n");
        for k in 0..100 {
            text.push_str(&format!("After it {k}.\n\n"));
        }
        let size = vec2(800.0, 600.0);
        let mut run = Run::new();
        let mut goto = 0;
        let (_, _, origin) = run.frame(&text, size, &mut goto, Vec::new());

        // the first glyph of the first link
        let mut link = None;
        for events in click_at(origin + vec2(PAD + 3.0, PAD + 8.0)) {
            let (out, ..) = run.frame(&text, size, &mut goto, events);
            link = link.or(out.link);
        }
        assert_eq!(link.as_deref(), Some("docs/nq/NQ_FORMAT.md"));

        // the anchor link, further right on the same line: never returned, it scrolls
        let galley_x = {
            let ctx = run.ctx.clone();
            let mut w = 0.0;
            crate::testkit::frame(&ctx, run.t, Vec::new(), |ui| {
                w = crate::text::width(
                    ui.ctx(),
                    &crate::Font::Body.id(),
                    crate::Tx::raw("Open the format or "),
                );
            });
            w
        };
        let mut link = None;
        let mut top = 0;
        for events in click_at(origin + vec2(PAD + galley_x + 6.0, PAD + 8.0)) {
            let (out, ..) = run.frame(&text, size, &mut goto, events);
            link = link.or(out.link);
            top = out.top_line;
        }
        assert_eq!(link, None);
        let heading = line_of(&text, "## Раздел два");
        assert_eq!(top, heading);
        let (out, ..) = run.frame(&text, size, &mut goto, Vec::new());
        assert_eq!(out.top_line, heading);
    }

    #[test]
    fn an_empty_text_draws_nothing_and_answers_line_one() {
        let mut run = Run::new();
        let mut goto = 5;
        let (out, laid, _) = run.frame("", vec2(400.0, 300.0), &mut goto, Vec::new());
        assert_eq!((out, laid, goto), (MarkdownOut { link: None, top_line: 1 }, 0, 0));
    }
}
