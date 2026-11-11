//! The Output Log: one body for the Level panel and the Log drawer (docs/UI_DESIGN.md, section 6,
//! "Output Log"), and the counts and the line the status bar shows.
//!
//! [`LogModel`] keeps a cursor into `gz_log::Log` and takes only the lines written since the last
//! look, so nothing copies the whole log per frame. It counts what the author acts on - the
//! errors and warnings since the project opened, each distinct line once - and marks with a row
//! where that count starts. The view lays out only the rows on screen. The C++ specification is
//! `UILogForm.cpp`.

use std::sync::Arc;

use egui::{Frame, Margin, ScrollArea, Sense, Ui, Vec2, pos2};
use gz_app::{Drawer, DrawerDesc, LogState, PanelDesc, ShellBuilder};
use gz_base::Rev;
use gz_editor::{Editor, SmolStr};
use gz_log::{Level, Log, Record};
use gz_ui::menu::MenuItem;
use gz_ui::state::empty_state;
use gz_ui::{Font, Icon, Kind, Toolbar, ToolbarStyle, menu_item, palette};
use parking_lot::Mutex;
use rustc_hash::FxHashSet;

use crate::local::local_rule;

/// The row where the counting starts.
pub const COUNT_MARK: &str = "The project opened - warnings and errors are counted from here";
/// The line the project area writes as a project opens: the counts start before it.
const PROJECT_OPENED: &str = "Project opened: ";
/// Notes of the editor's own machinery: never the status bar's line.
const CHATTER: [&str; 5] =
    ["startup: ", "file names: ", "control endpoint: ", "Grizzly Editor ", "DARF content mounted"];
/// The most lines the view keeps; past it the oldest go in a block.
const KEEP_MAX: usize = 50_000;
const KEEP_DROP: usize = 20_000;

/// A line's level as the view colours it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineLevel {
    Plain,
    Info,
    Warning,
    Error,
}

/// A line of the view.
#[derive(Clone, Debug)]
struct Line {
    text: Arc<str>,
    level: LineLevel,
    /// The counting window it was written in.
    epoch: u32,
    /// The row where a counting window starts.
    mark: bool,
    /// One of the status bar's errors or warnings: the first of its text in its window.
    counted: bool,
}

/// The level of a plain line by the engine's marks: `! ` an error, `~ ` a warning, `* ` a note.
#[must_use]
pub fn level_of(text: &str) -> LineLevel {
    if text.starts_with("! ") {
        LineLevel::Error
    } else if text.starts_with("~ ") {
        LineLevel::Warning
    } else if text.starts_with("* ") {
        LineLevel::Info
    } else {
        LineLevel::Plain
    }
}

/// `! [LUA]  2 : [Lua] ...script(32) : ...` - a frame of the traceback under a script error, not
/// an error of its own.
#[must_use]
pub fn is_trace_frame(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("! [LUA]") else { return false };
    let rest = rest.trim_start_matches(' ');
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    digits > 0 && rest.get(digits..).is_some_and(|r| r.starts_with(" :"))
}

/// A note the status bar may show: a report of the editor, not its machinery talking.
#[must_use]
pub fn notable_note(text: &str) -> bool {
    !text.starts_with(char::is_whitespace) && !CHATTER.iter().any(|c| text.starts_with(c))
}

fn hash_text(text: &str) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault};
    BuildHasherDefault::<rustc_hash::FxHasher>::default().hash_one(text)
}

/// Where the view is drawn: each host comes up at the end when it appears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogHost {
    /// The Level tab's panel.
    Panel,
    /// The drawer over any tab.
    Drawer,
}

struct Inner {
    cursor: u64,
    rev: Rev,
    lines: Vec<Line>,
    marks: u32,
    epoch: u32,
    counted_texts: FxHashSet<u64>,
    errors: u32,
    warnings: u32,
    last: String,
    // the view
    visible: Vec<u32>,
    filtered: usize,
    refilter: bool,
    shown_search: String,
    shown_level: usize,
    search: String,
    /// 0 all, 1 warnings, 2 errors.
    level: usize,
    auto_scroll: bool,
    /// Frames to come that show the end: the first one stops at the size the view had before.
    scroll_down: u8,
    hosts: [Option<u64>; 2],
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            cursor: 0,
            rev: Rev::ZERO,
            lines: Vec::new(),
            marks: 0,
            epoch: 0,
            counted_texts: FxHashSet::default(),
            errors: 0,
            warnings: 0,
            last: String::new(),
            visible: Vec::new(),
            filtered: 0,
            refilter: false,
            shown_search: String::new(),
            shown_level: 0,
            search: String::new(),
            level: 0,
            auto_scroll: true,
            scroll_down: 2,
            hosts: [None, None],
        }
    }
}

impl Inner {
    /// The lines written since the last look come over.
    fn sync(&mut self, log: &Log) {
        let rev = log.rev();
        if rev == self.rev {
            return;
        }
        self.rev = rev;
        for record in log.since(self.cursor) {
            self.cursor = record.seq + 1;
            self.intake(&record);
        }
        if self.lines.len() > KEEP_MAX {
            self.lines.drain(..KEEP_DROP);
            self.marks = self.lines.iter().filter(|l| l.mark).count() as u32;
            self.refilter = true;
        }
    }

    fn intake(&mut self, record: &Record) {
        let text: Arc<str> = if record.text.contains(['\r', '\n']) {
            record
                .text
                .chars()
                .filter(|&c| c != '\r')
                .map(|c| if c == '\n' { ' ' } else { c })
                .collect::<String>()
                .into()
        } else {
            record.text.as_str().into()
        };
        let level = match record.level {
            Level::Error => LineLevel::Error,
            Level::Warning => LineLevel::Warning,
            Level::Info => LineLevel::Info,
            Level::Plain => level_of(&text),
        };
        if level == LineLevel::Info && text.starts_with(PROJECT_OPENED) {
            self.count_from_here(COUNT_MARK);
        }
        // the status bar's counts and its line: what an author acts on, not the chatter
        let serious = matches!(level, LineLevel::Error | LineLevel::Warning) && !is_trace_frame(&text);
        let counted = serious && self.counted_texts.insert(hash_text(&text));
        if counted {
            if level == LineLevel::Error {
                self.errors += 1;
            } else {
                self.warnings += 1;
            }
        }
        if serious || (level == LineLevel::Info && notable_note(&text)) {
            self.last = text.to_string();
        }
        self.lines.push(Line { text, level, epoch: self.epoch, mark: false, counted });
    }

    /// A counting window starts: the counts and the line begin again, and the log shows where,
    /// unless nothing came since the last mark.
    fn count_from_here(&mut self, mark: &str) {
        self.errors = 0;
        self.warnings = 0;
        self.counted_texts.clear();
        self.last.clear();
        self.epoch += 1;
        self.refilter = true;
        if mark.is_empty() || self.lines.last().is_none_or(|l| l.mark) {
            return;
        }
        self.lines.push(Line {
            text: mark.into(),
            level: LineLevel::Info,
            epoch: self.epoch,
            mark: true,
            counted: false,
        });
        self.marks += 1;
    }

    fn shows(&self, line: &Line) -> bool {
        // a mark belongs to the whole, unfiltered log
        if line.mark {
            return self.level == 0 && self.search.is_empty();
        }
        // Warnings and Errors list what the status bar counts: this window's lines, each text once
        if self.level > 0 && (line.epoch != self.epoch || !line.counted) {
            return false;
        }
        if self.level == 1 && line.level != LineLevel::Warning {
            return false;
        }
        if self.level == 2 && line.level != LineLevel::Error {
            return false;
        }
        gz_ui::search::matches(&self.search, &line.text)
    }

    /// The new lines through the filter; a changed filter looks at every line again.
    fn refilter_now(&mut self) {
        if self.refilter || self.shown_search != self.search || self.shown_level != self.level {
            self.refilter = false;
            self.shown_search.clone_from(&self.search);
            self.shown_level = self.level;
            self.visible.clear();
            self.filtered = 0;
            self.scroll_down = 2;
        }
        while self.filtered < self.lines.len() {
            if let Some(line) = self.lines.get(self.filtered)
                && self.shows(line)
            {
                self.visible.push(self.filtered as u32);
            }
            self.filtered += 1;
        }
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.visible.clear();
        self.filtered = 0;
        self.marks = 0;
        self.errors = 0;
        self.warnings = 0;
        self.counted_texts.clear();
        self.last.clear();
    }

    fn shown_text(&self) -> String {
        let mut out = String::new();
        for line in self.visible.iter().filter_map(|&i| self.lines.get(i as usize)).filter(|l| !l.mark) {
            out.push_str(&line.text);
            out.push_str("\r\n");
        }
        out
    }

    fn state(&self) -> LogState {
        LogState { last: self.last.clone(), errors: self.errors, warnings: self.warnings }
    }
}

/// The Output Log's model: an `Editor` service.
#[derive(Default)]
pub struct LogModel {
    inner: Mutex<Inner>,
}

impl LogModel {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<LogModel> {
        editor.services.get_or_init(LogModel::default)
    }

    /// What the status bar says of the log: the last line worth reading, the errors and the
    /// warnings since the project opened, each distinct line once.
    #[must_use]
    pub fn state(&self, log: &Log) -> LogState {
        let mut m = self.inner.lock();
        m.sync(log);
        m.state()
    }
}

/// Draws the Output Log into `ui`: the toolbar, then the lines. The Level tab's panel and the Log
/// drawer both call it.
pub fn output_log(ui: &mut Ui, editor: &Editor, host: LogHost) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let model = LogModel::of(editor);
    let mut m = model.inner.lock();
    m.sync(&editor.log);
    // a view that comes up - the drawer opened, the panel's tab picked - shows the end
    let frame = ctx.cumulative_frame_nr();
    let slot = usize::from(host == LogHost::Drawer);
    let appearing = m.hosts.get(slot).copied().flatten().is_none_or(|f| f + 1 < frame);
    if appearing {
        m.scroll_down = 2;
    }
    if let Some(seen) = m.hosts.get_mut(slot) {
        *seen = Some(frame);
    }

    // ---- the toolbar: search, the level, what the view does
    let all = format!("All {}", m.lines.len().saturating_sub(m.marks as usize));
    let warnings = format!("Warnings {}", m.warnings);
    let errors = format!("Errors {}", m.errors);
    let labels = [all.as_str(), warnings.as_str(), errors.as_str()];
    let (any_shown, any_lines, auto) = (!m.visible.is_empty(), !m.lines.is_empty(), m.auto_scroll);
    let (toggle_auto, copy, clear);
    {
        let v = &mut *m;
        let mut tb = Toolbar::begin(ui, "output_log", ToolbarStyle::Panel);
        tb.group(2);
        tb.search(&mut v.search, "Search the log", 10.0);
        tb.group(1);
        tb.segmented(
            "level",
            "Show",
            &mut v.level,
            &labels,
            "Which lines the log shows: every line, or the warnings or errors since the project opened, each once - the ones the status bar counts",
            true,
        );
        tb.spring();
        tb.group(0);
        toggle_auto = tb.action(
            "autoscroll",
            Icon::MoveDown,
            "Auto-scroll",
            "Follow the new lines as they come",
            true,
            auto,
            Kind::Ghost,
        );
        copy = tb.action(
            "copy",
            Icon::Copy,
            "Copy",
            if any_shown { "Copy the lines shown to the clipboard" } else { "No lines are shown" },
            any_shown,
            false,
            Kind::Ghost,
        );
        clear = tb.action(
            "clear",
            Icon::Clear,
            "Clear",
            if any_lines {
                "Clear the lines shown here; the log file keeps them"
            } else {
                "The log is empty"
            },
            any_lines,
            false,
            Kind::Ghost,
        );
        tb.end();
    }
    if toggle_auto {
        m.auto_scroll = !m.auto_scroll;
        if m.auto_scroll {
            m.scroll_down = 2;
        }
    }
    if copy {
        ctx.copy_text(m.shown_text());
    }
    if clear {
        m.clear();
    }
    // after the toolbar, which may have changed the search or the level
    m.refilter_now();

    // ---- the lines: Mono, and only those on screen are laid out
    let mut show_all = false;
    let mut copy_line: Option<String> = None;
    let mut copy_shown = false;
    Frame::new().fill(p.inset).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        if m.lines.is_empty() {
            empty_state(ui, Icon::Log, "The log is empty", "", None);
            return;
        }
        if m.visible.is_empty() {
            show_all = empty_state(
                ui,
                Icon::Search,
                "No lines match",
                "Nothing in the log matches the search and the level.",
                Some("Show All"),
            );
            return;
        }
        let row_h = Font::Mono.line();
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 1.0);
        let total = m.visible.len();
        let mut area = ScrollArea::both()
            .id_salt("gz-ui-app.log.lines")
            .auto_shrink([false, false])
            .stick_to_bottom(m.auto_scroll);
        if m.scroll_down > 0 {
            area = area.vertical_scroll_offset(total as f32 * (row_h + 1.0));
            m.scroll_down -= 1;
        }
        let font = Font::Mono.id();
        let lines = &m.lines;
        let visible = &m.visible;
        area.show_rows(ui, row_h, total, |ui, range| {
            for k in range {
                let Some(line) = visible.get(k).and_then(|&i| lines.get(i as usize)) else { continue };
                let color = match (line.mark, line.level) {
                    (true, _) => p.text_muted,
                    (_, LineLevel::Error) => p.error,
                    (_, LineLevel::Warning) => p.warning,
                    (_, LineLevel::Info) => p.text_secondary,
                    (_, LineLevel::Plain) => p.text,
                };
                let galley = ui.painter().layout_no_wrap(line.text.to_string(), font.clone(), color);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(galley.size().x, row_h), Sense::click());
                if ui.is_rect_visible(rect) {
                    ui.painter().galley(
                        pos2(rect.min.x, rect.center().y - galley.size().y * 0.5),
                        galley,
                        color,
                    );
                }
                if line.mark {
                    // where the counts start: its reason, then a rule to the edge
                    local_rule(ui, rect.max.x + 8.0, ui.clip_rect().max.x, rect.center().y);
                    continue;
                }
                let _menu = resp.context_menu(|ui| {
                    gz_ui::menu::popup_body(ui, |ui| {
                        if menu_item(ui, "Copy Line", MenuItem::default()) {
                            copy_line = Some(line.text.to_string());
                        }
                        if menu_item(ui, "Copy Lines Shown", MenuItem::default()) {
                            copy_shown = true;
                        }
                    });
                });
            }
        });
    });
    if show_all {
        m.search.clear();
        m.level = 0;
    }
    if let Some(text_) = copy_line {
        ctx.copy_text(text_);
    }
    if copy_shown {
        ctx.copy_text(m.shown_text());
    }
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.panel(PanelDesc {
        id: SmolStr::new("output_log"),
        label: SmolStr::new("Output Log"),
        icon: Icon::Log,
        drawer: Some(Drawer::Log),
    });
    b.drawer(
        Drawer::Log,
        DrawerDesc {
            label: SmolStr::new("Output Log"),
            icon: Icon::Log,
            panel: Some(SmolStr::new("output_log")),
            body: Box::new(|ui: &mut Ui, cx: &mut gz_app::DocCx<'_>| {
                output_log(ui, cx.editor, LogHost::Drawer)
            }),
        },
    );
    b.log_state(|editor| LogModel::of(editor).state(&editor.log));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_and_trace_frames() {
        assert_eq!(level_of("! boom"), LineLevel::Error);
        assert_eq!(level_of("~ hm"), LineLevel::Warning);
        assert_eq!(level_of("* note"), LineLevel::Info);
        assert_eq!(level_of("plain"), LineLevel::Plain);
        assert!(is_trace_frame("! [LUA]  2 : [Lua] x.script(32) : y"));
        assert!(!is_trace_frame("! [LUA] error in x.script"));
        assert!(!is_trace_frame("! other"));
    }

    #[test]
    fn the_status_line_skips_the_machinery() {
        assert!(!notable_note("startup: 0.14 s, 120 file reads"));
        assert!(!notable_note("  Loading fonts   0.02 s"));
        assert!(!notable_note("control endpoint: 127.0.0.1:28017"));
        assert!(notable_note("Project opened: D:\\Mods\\A"));
    }

    #[test]
    fn counts_start_where_the_project_opened_each_text_once() {
        let log = Log::memory();
        log.warn("before");
        log.info("Project opened: D:\\p");
        log.warn("again and again");
        log.warn("again and again");
        log.error("broke");
        log.info("startup: 1 s");
        log.write(Level::Error, "! [LUA]  1 : [Lua] a.script(1) : f");
        let mut m = Inner::default();
        m.sync(&log);
        assert_eq!((m.errors, m.warnings), (1, 1));
        assert_eq!(m.last, "broke");
        // the mark stands before the line that opened the project
        assert!(m.lines.get(1).is_some_and(|l| l.mark && &*l.text == COUNT_MARK));
        m.level = 1;
        m.refilter_now();
        assert_eq!(m.visible.len(), 1);
        m.level = 0;
        m.search = "again".to_owned();
        m.refilter_now();
        assert_eq!(m.visible.len(), 2);
        m.clear();
        assert_eq!((m.errors, m.warnings, m.lines.len()), (0, 0, 0));
    }

    #[test]
    fn only_new_lines_come_over() {
        let log = Log::memory();
        log.info("one");
        let mut m = Inner::default();
        m.sync(&log);
        log.info("two");
        m.sync(&log);
        m.sync(&log);
        assert_eq!(m.lines.len(), 2);
    }
}
