//! The Text tab (`text:<rel>`; docs/UI_DESIGN.md, section 6, "Text"): a text file in the kit's
//! code editor, a Markdown file in its preview, side by side or as text; Find, Replace, Go to
//! Line and Go to Symbol; a script's completion and Go to Definition; the problems under it.

use std::cell::RefCell;
use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use egui::{Align, Key, Layout, Rect, Sense, Ui, UiBuilder, Vec2, pos2};
use gz_app::{Chord, DocCx, Document, OpenRequest};
use gz_editor::{Args, Editor, Value, json};
use gz_text::{Encoding, Eol, OfferKind, ScriptIndex, Symbol, TextService};
use gz_ui::code::{CodeCtl, CodeLang, CodeOpts, CompleteAsk, Completion, CompletionKind, LineMark};
use gz_ui::text::{self, Clip};
use gz_ui::{
    Button, Font, Icon, Kind, MenuItem, PickItem, PickList, SearchField, Tone, Toolbar, Tx, palette, tr,
};

use crate::find::{self, Query};
use crate::model::{self, Model, Shared, Source};
use crate::tabs::{Mode, Show, TextTabs};

/// The tab's kind.
pub const KIND: &str = "text";
/// `ui.popup` names of the tab's two menus.
pub const SYMBOLS_POPUP: &str = "text_symbols";
pub const MORE_POPUP: &str = "text_more";

/// Quiet this long after typing and the text is checked again.
const CHECK_AFTER_S: f64 = 0.4;
/// The problems drawer shows this many rows at most.
const PROBLEM_ROWS: usize = 6;

/// What the tab shows of its file, the view's own state.
#[derive(Default)]
struct View {
    mode: Mode,
    wrap: bool,
    find_open: bool,
    replace_open: bool,
    goto_open: bool,
    focus_find: bool,
    query: Query,
    replace_with: String,
    goto_text: String,
    current: Option<usize>,
    /// The matches of the query, by the hash of the text and the query.
    found: (u64, Vec<Range<usize>>),
    ctl: CodeCtl,
    md_goto: u32,
    top_line: u32,
    edited_at: f64,
    caret: (u32, u32),
    selection: Option<Range<usize>>,
    /// The file's own words for completion, by the text's hash.
    words: RefCell<(u64, Arc<Vec<String>>)>,
    symbols: (u64, Vec<Symbol>),
    lines: u32,
    /// The hash of the text, worked out once a frame (the frame's number beside it): what the
    /// caches above are kept by.
    hash: (u64, u64),
}

/// A Text tab.
pub struct TextDoc {
    editor: Arc<Editor>,
    m: Shared,
    id: String,
    started: bool,
    follow: Arc<AtomicU64>,
    v: View,
}

fn args(pairs: &[(&str, Value)]) -> Args {
    let mut a = Args::new();
    for (k, v) in pairs {
        a.insert((*k).to_owned(), v.clone());
    }
    a
}

impl TextDoc {
    /// A tab for the file a request names: `path` project-relative, or the game's with
    /// `source=game` (`arg`) or a `game:` prefix. Nothing is read here: the tab's first frame reads.
    pub fn open(editor: &Arc<Editor>, req: &OpenRequest) -> Result<Self, String> {
        let path = req
            .path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .ok_or("a text tab opens a file: name it with path")?;
        let source = Source::parse(path, req.arg.as_deref() == Some("game"));
        match &source {
            Source::Project(rel) => {
                editor.project_opt().ok_or("no project is open")?;
                gz_base::RelPath::new(rel).map_err(|_| format!("{path} is no path inside the project"))?;
            }
            Source::Game(_) => {
                editor.project_opt().ok_or("no project is open: a file of the game opens with one")?;
            }
        }
        let lang = model::lang_of(source.name());
        let id = source.tab_id();
        let v = View {
            mode: if lang == CodeLang::Markdown { Mode::Preview } else { Mode::Edit },
            wrap: matches!(lang, CodeLang::Markdown | CodeLang::Plain),
            ..View::default()
        };
        Ok(Self {
            editor: editor.clone(),
            m: Arc::new(parking_lot::Mutex::new(Model::new(source))),
            id,
            started: false,
            follow: Arc::new(AtomicU64::new(0)),
            v,
        })
    }

    /// The tab's id for a source: what `text.view` and Go to Definition ask by.
    #[must_use]
    pub fn id_of(source: &Source) -> String {
        source.tab_id()
    }

    /// The first frame: the file read, followed, and the scripts' index made for completion.
    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        let (ed, m, follow) = (self.editor.clone(), self.m.clone(), self.follow.clone());
        let lua = self.m.lock().lang == CodeLang::Lua;
        let _task = self.editor.exec.spawn(async move {
            model::load(&ed, &m).await;
            model::follow(&ed, &m, follow);
            model::check(&ed, &m).await;
            if lua && let Ok(session) = ed.project() {
                let _ = TextService::of_session(&ed, &session).index(&ed).await;
                ed.exec.wake().ping();
            }
        });
    }

    /// What `text.view` and other tabs asked of this one.
    fn take_asks(&mut self, ctx: &egui::Context) {
        let Some(ask) = TextTabs::of(&self.editor).take(&self.id) else { return };
        if let Some(at) = ask.goto {
            self.v.ctl.goto = Some(at);
            self.v.md_goto = at.0;
            if self.v.mode == Mode::Preview {
                self.v.mode = Mode::Split;
            }
        }
        if let Some(q) = ask.find {
            self.v.query = q;
            self.v.find_open = true;
            self.v.current = None;
        }
        if let Some(mode) = ask.mode
            && self.m.lock().lang == CodeLang::Markdown
        {
            self.v.mode = mode;
        }
        if let Some(w) = ask.wrap {
            self.v.wrap = w;
        }
        match ask.show {
            Some(Show::Find) => {
                self.v.find_open = true;
                self.v.focus_find = true;
            }
            Some(Show::Symbols) => gz_ui::menu::show_popup(ctx, SYMBOLS_POPUP),
            Some(Show::Complete) => self.v.ctl.focus = true,
            Some(Show::None) => {
                self.v.find_open = false;
                self.v.goto_open = false;
            }
            None => {}
        }
    }

    fn spawn_check(&self) {
        let (ed, m) = (self.editor.clone(), self.m.clone());
        let _task = self.editor.exec.spawn(async move { model::check(&ed, &m).await });
    }

    /// The hash of the text as it is at the start of this frame (`frame`: egui's frame number; a
    /// command outside a frame passes `u64::MAX` and always hashes).
    fn text_hash(&mut self, frame: u64) -> u64 {
        if self.v.hash.0 != frame || frame == u64::MAX {
            self.v.hash = (frame, model::hash(&self.m.lock().text));
        }
        self.v.hash.1
    }

    /// The text is checked once it has been quiet for a moment.
    fn check_when_quiet(&mut self, now: f64, ctx: &egui::Context) {
        let h = self.text_hash(ctx.cumulative_frame_nr());
        let due = {
            let g = self.m.lock();
            !g.checking && g.file.is_some() && h != g.checked
        };
        if !due {
            return;
        }
        let quiet = now - self.v.edited_at;
        if quiet >= CHECK_AFTER_S {
            self.spawn_check();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(CHECK_AFTER_S - quiet));
        }
    }

    /// The matches of the find bar's query in the text as it is.
    fn found(&mut self, text: &str) -> Vec<Range<usize>> {
        use std::hash::{Hash, Hasher};
        if !self.v.find_open || self.v.query.text.is_empty() {
            return Vec::new();
        }
        let mut h = rustc_hash::FxHasher::default();
        // the text as this frame has it, or as a command passes it
        let at = if self.v.hash.0 == u64::MAX { model::hash(text) } else { self.v.hash.1 };
        at.hash(&mut h);
        text.len().hash(&mut h);
        self.v.query.hash(&mut h);
        let key = h.finish();
        if self.v.found.0 != key {
            self.v.found = (key, find::matches(text, &self.v.query));
            if self.v.current.is_some_and(|c| c >= self.v.found.1.len()) {
                self.v.current = None;
            }
        }
        self.v.found.1.clone()
    }

    fn step_find(&mut self, back: bool) {
        let text = self.m.lock().text.clone();
        let found = self.found(&text);
        let caret = gz_ui::code::edit::byte_of(&text, self.v.caret.0, self.v.caret.1);
        if let Some(i) = find::next(&found, caret, back, self.v.current) {
            self.v.current = Some(i);
            self.v.ctl.select = Some(found[i].clone());
        }
    }

    fn symbols(&mut self) -> Vec<Symbol> {
        let h = self.v.hash.1.max(1);
        if self.v.symbols.0 != h {
            let g = self.m.lock();
            let made = gz_text::symbols(g.source.name(), &g.text);
            drop(g);
            // an empty list is kept too: a file without symbols is not read again every frame
            self.v.symbols = (h, made);
        }
        self.v.symbols.1.clone()
    }

    fn revert(&self, ctx: &egui::Context) {
        let (dirty, name) = {
            let g = self.m.lock();
            (g.dirty(), g.source.name().to_owned())
        };
        let (ed, m) = (self.editor.clone(), self.m.clone());
        if !dirty {
            let _task = self.editor.exec.spawn(async move { model::load(&ed, &m).await });
            return;
        }
        let spec = gz_ui::ConfirmSpec {
            id: "text_revert".to_owned(),
            title: "Revert".into(),
            headline: tr!("Drop the unsaved edits of {name}?", name = name),
            items: vec![Tx::from("The tab's edits since the last save")],
            total: 0,
            consequence: "The file stays as it is on disk; the edits are lost.".into(),
            action: "Revert".into(),
            destructive: true,
        };
        let asker = if self.editor.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
        let line = gz_ui::dialog::confirm(ctx, asker, spec, move |answer| {
            if answer == gz_ui::Answer::Yes {
                let _task = ed.exec.clone().spawn(async move { model::load(&ed, &m).await });
            }
        });
        if let Some(f) = line {
            self.editor.log.finding(&f);
        }
    }

    /// Go to Definition: the file's own function first, then the scripts' index.
    fn definition(&mut self, name: String) {
        let own = name.rsplit(['.', ':']).next().unwrap_or(&name).to_owned();
        let script = self
            .m
            .lock()
            .source
            .name()
            .rsplit_once('.')
            .map_or(String::new(), |(s, _)| s.to_ascii_lowercase());
        let qualified = name.contains(['.', ':']);
        let mine =
            !qualified || name.split(['.', ':']).next().is_some_and(|q| q.eq_ignore_ascii_case(&script));
        if mine
            && let Some(s) = self.symbols().into_iter().find(|s| {
                s.name == own || s.name.ends_with(&format!(".{own}")) || s.name.ends_with(&format!(":{own}"))
            })
        {
            self.v.ctl.goto = Some((s.line, 1));
            return;
        }
        let ed = self.editor.clone();
        let _task = self.editor.exec.spawn(async move {
            let Ok(session) = ed.project() else { return };
            let index = TextService::of_session(&ed, &session).index(&ed).await;
            let Some(decl) = index.lookup(&name).into_iter().next() else {
                ed.notices.note(format!(
                    "Nothing declares {name}: no script of the game or of the project has it."
                ));
                return;
            };
            let source = match &decl.source {
                gz_text::DeclSource::Project(rel) => Source::Project(rel.as_str().replace('\\', "/")),
                gz_text::DeclSource::Game(p) => Source::Game(p.clone()),
            };
            open_at(&ed, &source, (decl.line, 1)).await;
        });
    }

    /// A Markdown link: a web address in the browser, `#heading` in the preview (the kit's), a
    /// file of the project in its tab.
    fn link(&self, target: &str, ctx: &egui::Context) {
        let t = target.trim();
        if t.starts_with("http://") || t.starts_with("https://") || t.starts_with("mailto:") {
            ctx.open_url(egui::OpenUrl::new_tab(t));
            return;
        }
        let Source::Project(rel) = self.m.lock().source.clone() else { return };
        let file = t.split('#').next().unwrap_or(t).replace('\\', "/");
        if file.is_empty() {
            return;
        }
        let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
        let joined = if dir.is_empty() || file.starts_with('/') {
            file.trim_start_matches('/').to_owned()
        } else {
            format!("{dir}/{file}")
        };
        let mut parts: Vec<&str> = Vec::new();
        for p in joined.split('/') {
            match p {
                "" | "." => {}
                ".." => {
                    parts.pop();
                }
                p => parts.push(p),
            }
        }
        let path = parts.join("/");
        let ed = self.editor.clone();
        if gz_text::is_text_name(&path) {
            let _task =
                self.editor.exec.spawn(async move { open_at(&ed, &Source::Project(path), (1, 1)).await });
        } else {
            ed.post(
                "content.view",
                args(&[("action", json!("reveal")), ("path", json!(path)), ("open", json!(true))]),
            );
        }
    }

    /// The completion offers for an ask: the scripts' index, then the file's own words.
    fn completions(index: Option<&ScriptIndex>, words: &[String], ask: &CompleteAsk) -> Vec<Completion> {
        let mut out: Vec<Completion> = index
            .map(|i| i.complete(ask.qualifier.as_deref(), &ask.prefix, 60))
            .unwrap_or_default()
            .into_iter()
            .map(|o| Completion {
                text: o.text,
                detail: o.detail,
                kind: match o.kind {
                    OfferKind::Keyword => CompletionKind::Keyword,
                    OfferKind::Function => CompletionKind::Function,
                    OfferKind::Value => CompletionKind::Value,
                    OfferKind::Module => CompletionKind::Module,
                },
            })
            .collect();
        if ask.qualifier.is_none() && !ask.prefix.is_empty() {
            let low = ask.prefix.to_lowercase();
            for w in words {
                if out.len() >= 80 {
                    break;
                }
                if w != &ask.prefix && w.to_lowercase().starts_with(&low) && !out.iter().any(|c| &c.text == w)
                {
                    out.push(Completion {
                        text: w.clone(),
                        detail: String::new(),
                        kind: CompletionKind::Word,
                    });
                }
            }
        }
        out
    }

    fn words(&self, text: &str) -> Arc<Vec<String>> {
        let h = self.v.hash.1;
        let mut cache = self.v.words.borrow_mut();
        if cache.0 != h || cache.1.is_empty() {
            let mut seen = rustc_hash::FxHashSet::default();
            let mut words = Vec::new();
            for w in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                if w.len() >= 3 && !w.as_bytes()[0].is_ascii_digit() && seen.insert(w) {
                    words.push(w.to_owned());
                }
                if words.len() > 4000 {
                    break;
                }
            }
            *cache = (h, Arc::new(words));
        }
        cache.1.clone()
    }

    fn draw_find_bar(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        let text = self.m.lock().text.clone();
        let found = self.found(&text);
        let read_only = self.m.lock().source.read_only();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let resp = SearchField::new(&mut self.v.query.text)
                .hint("Find")
                .id_salt("text.find")
                .width(gz_ui::units::em(18.0))
                .show(ui);
            if std::mem::take(&mut self.v.focus_find) {
                resp.request_focus();
            }
            if resp.changed() {
                self.v.current = None;
            }
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if enter {
                let back = ui.input(|i| i.modifiers.shift);
                self.step_find(back);
                resp.request_focus();
            }
            if Button::new("Match Case")
                .kind(Kind::Ghost)
                .checked(self.v.query.case)
                .tip("Tell capitals from small letters")
                .show(ui)
                .clicked()
            {
                self.v.query.case = !self.v.query.case;
                self.v.current = None;
            }
            if Button::new("Whole Word")
                .kind(Kind::Ghost)
                .checked(self.v.query.whole_word)
                .tip("Only where it stands as a word of its own")
                .show(ui)
                .clicked()
            {
                self.v.query.whole_word = !self.v.query.whole_word;
                self.v.current = None;
            }
            let count = if self.v.query.text.is_empty() {
                Tx::EMPTY
            } else if found.is_empty() {
                Tx::from("No matches")
            } else {
                match self.v.current {
                    Some(c) => tr!("{n} of {total}", n = c + 1, total = found.len()),
                    None => tr!("{total} {total:match|matches}", total = found.len()),
                }
            };
            text::label(ui, count, Font::Small, p.text_secondary, Clip::End);
            let any = !found.is_empty();
            if gz_ui::icon_button(ui, Icon::ChevronUp, "Previous Match (Shift+F3)", false, any).clicked() {
                self.step_find(true);
            }
            if gz_ui::icon_button(ui, Icon::ChevronDown, "Next Match (F3)", false, any).clicked() {
                self.step_find(false);
            }
            if !read_only
                && Button::new("Replace")
                    .kind(Kind::Ghost)
                    .checked(self.v.replace_open)
                    .tip("Replace what is found (Ctrl+H)")
                    .show(ui)
                    .clicked()
            {
                self.v.replace_open = !self.v.replace_open;
            }
            if gz_ui::icon_button(ui, Icon::Close, "Close (Esc)", false, true).clicked() {
                self.v.find_open = false;
                self.v.replace_open = false;
            }
        });
        if self.v.replace_open && !read_only {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let _ = gz_ui::form::hint_field(
                    ui,
                    ui.make_persistent_id("gz-ui-text.replace_with"),
                    &mut self.v.replace_with,
                    "Replace with",
                    gz_ui::units::em(18.0),
                );
                let any = !found.is_empty();
                if Button::new("Replace")
                    .enabled(any, "Nothing is found")
                    .tip("Replace the current match and go to the next")
                    .show(ui)
                    .clicked()
                {
                    let at =
                        self.v.current.and_then(|c| found.get(c).cloned()).or_else(|| found.first().cloned());
                    if let Some(at) = at {
                        let (new, caret) = find::replace_one(&text, &at, &self.v.replace_with);
                        self.v.ctl.set_text = Some((new, caret));
                        self.v.current = None;
                        self.v.edited_at = ui.input(|i| i.time);
                    }
                }
                if Button::new("Replace All")
                    .enabled(any, "Nothing is found")
                    .tip("Replace every match, as one step Ctrl+Z takes back")
                    .show(ui)
                    .clicked()
                {
                    let (new, n) = find::replace_all(&text, &found, &self.v.replace_with);
                    self.v.ctl.set_text = Some((new, 0));
                    self.v.current = None;
                    self.v.edited_at = ui.input(|i| i.time);
                    gz_ui::state::toast(
                        ui.ctx(),
                        Tone::Success,
                        tr!("{n} {n:match|matches} replaced", n = n),
                    );
                }
            });
        }
    }

    fn draw_goto_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let id = ui.make_persistent_id("gz-ui-text.goto_line");
            let resp = gz_ui::form::hint_field(
                ui,
                id,
                &mut self.v.goto_text,
                tr!("Line (1 to {n})", n = self.v.lines.max(1)),
                gz_ui::units::em(10.0),
            );
            if std::mem::take(&mut self.v.focus_find) {
                resp.request_focus();
            }
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            let go = Button::new("Go").show(ui).clicked();
            if enter || go {
                let mut parts = self.v.goto_text.split([':', ',']).map(|s| s.trim().parse::<u32>().ok());
                if let Some(Some(line)) = parts.next() {
                    let col = parts.next().flatten().unwrap_or(1);
                    self.v.ctl.goto = Some((line.clamp(1, self.v.lines.max(1)), col.max(1)));
                    self.v.goto_open = false;
                    self.v.goto_text.clear();
                }
            }
            if gz_ui::icon_button(ui, Icon::Close, "Close (Esc)", false, true).clicked() {
                self.v.goto_open = false;
            }
        });
    }

    fn draw_problems(&mut self, ui: &mut Ui, problems: &[gz_base::Problem]) {
        let p = palette(ui.ctx());
        let h = (problems.len().min(PROBLEM_ROWS) as f32) * gz_ui::metrics::ROW + 4.0;
        gz_ui::list::list(ui, "text.problems", Vec2::new(0.0, h), |l| {
            for (k, pr) in problems.iter().enumerate() {
                let tone = match pr.severity {
                    gz_base::Severity::Error => Tone::Error,
                    gz_base::Severity::Warning => Tone::Warning,
                    _ => Tone::Info,
                };
                let line = pr.line.unwrap_or(0);
                let clicked = l
                    .row(&format!("{k}"), false, |ui| {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                            gz_ui::icons::draw(ui.painter(), gz_ui::state::tone_icon(tone), r, p.tone(tone));
                            let where_ = if line > 0 { tr!("Line {n}", n = line) } else { Tx::EMPTY };
                            text::label(ui, where_, Font::Small, p.text_secondary, Clip::End);
                            // the sentence names its line itself: the column says it once
                            let said = format!("Line {line}: ");
                            let message = pr.message.strip_prefix(&said).unwrap_or(&pr.message).to_owned();
                            text::label(ui, Tx::ui(message), Font::Body, p.text, Clip::End);
                        });
                    })
                    .clicked();
                if clicked && line > 0 {
                    self.v.ctl.goto = Some((line, pr.column.unwrap_or(1)));
                }
            }
        });
    }

    /// The text in the code editor, in `rect`.
    fn draw_code(&mut self, ui: &mut Ui, rect: Rect, cx: &mut DocCx<'_>) {
        let m = self.m.clone();
        let g = m.lock();
        let lang = g.lang;
        let read_only = g.source.read_only();
        let marks: Vec<LineMark> = g
            .problems
            .iter()
            .filter_map(|p| {
                let tone = match p.severity {
                    gz_base::Severity::Error => Tone::Error,
                    gz_base::Severity::Warning => Tone::Warning,
                    _ => return None,
                };
                Some(LineMark { line: p.line?, column: p.column.unwrap_or(0), tone })
            })
            .collect();
        let found = self.found(&g.text);
        let words = if lang == CodeLang::Lua { self.words(&g.text) } else { Arc::new(Vec::new()) };
        drop(g);
        let index = if lang == CodeLang::Lua {
            self.editor.project_opt().and_then(|s| TextService::of_session(&self.editor, &s).index_now())
        } else {
            None
        };
        let complete = move |ask: &CompleteAsk| Self::completions(index.as_deref(), &words, ask);
        let mut opts = CodeOpts::new(lang);
        opts.read_only = read_only;
        opts.wrap = self.v.wrap;
        opts.marks = &marks;
        opts.found = &found;
        opts.found_current = self.v.current;
        if lang == CodeLang::Lua {
            opts.complete = Some(&complete);
        }
        let mut child = ui.new_child(
            UiBuilder::new().id_salt("gz-ui-text.code").max_rect(rect).layout(Layout::top_down(Align::Min)),
        );
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        let mut g = m.lock();
        let out = gz_ui::code::code_view(&mut child, &self.id, &mut g.text, &opts, &mut self.v.ctl);
        // what is selected, for Find to start from
        let picked = out
            .selection
            .clone()
            .and_then(|r| g.text.get(r).filter(|t| !t.contains('\n')).map(str::to_owned));
        drop(g);
        self.v.caret = out.caret;
        self.v.selection = out.selection.clone();
        self.v.lines = out.lines;
        if out.changed {
            self.v.edited_at = ui.input(|i| i.time);
            if self.v.find_open {
                self.v.current = None;
            }
        }
        if out.top_line != self.v.top_line {
            self.v.top_line = out.top_line;
            self.v.md_goto = out.top_line;
        }
        if out.save {
            cx.requests.action("file.save");
        }
        if out.find || out.replace {
            self.v.find_open = true;
            self.v.goto_open = false;
            self.v.focus_find = true;
            self.v.replace_open |= out.replace;
            if let Some(t) = picked {
                self.v.query.text = t;
                self.v.current = None;
            }
        }
        if out.goto_line {
            self.v.goto_open = true;
            self.v.find_open = false;
            self.v.focus_find = true;
        }
        if out.symbols {
            gz_ui::menu::show_popup(cx.ctx, SYMBOLS_POPUP);
        }
        if let Some(back) = out.find_next {
            self.v.find_open = true;
            self.step_find(back);
        }
        if out.escape {
            self.v.find_open = false;
            self.v.goto_open = false;
        }
        if let Some(name) = out.definition {
            self.definition(name);
        }
    }

    fn draw_preview(&mut self, ui: &mut Ui, rect: Rect) {
        let text = self.m.lock().text.clone();
        let mut child = ui.new_child(
            UiBuilder::new().id_salt("gz-ui-text.md").max_rect(rect).layout(Layout::top_down(Align::Min)),
        );
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        let out = gz_ui::markdown::markdown_view(&mut child, &self.id, &text, &mut self.v.md_goto);
        if let Some(link) = out.link {
            self.link(&link, ui.ctx());
        }
    }

    fn state_json(&self) -> Value {
        let g = self.m.lock();
        let (source, path) = match &g.source {
            Source::Project(rel) => ("project", rel.clone()),
            Source::Game(p) => ("game", p.replace('\\', "/")),
        };
        let problems: Vec<Value> = g
            .problems
            .iter()
            .map(|p| json!({ "code": p.code.as_str(), "severity": format!("{:?}", p.severity).to_lowercase(), "line": p.line, "column": p.column, "text": p.message }))
            .collect();
        json!({
            "tab": self.id,
            "path": path,
            "source": source,
            "language": g.lang.id(),
            "encoding": g.encoding.id(),
            "line_endings": g.eol.id(),
            "lines": 1 + g.text.bytes().filter(|&b| b == b'\n').count(),
            "dirty": g.dirty(),
            "read_only": g.source.read_only(),
            "loading": g.loading,
            "error": g.read_error,
            "caret": { "line": self.v.caret.0, "column": self.v.caret.1 },
            "mode": if g.lang == CodeLang::Markdown { self.v.mode.id() } else { "edit" },
            "wrap": self.v.wrap,
            "problems": problems,
            "checking": g.checking,
            "symbols": self.v.symbols.1.iter().map(|s| json!({ "name": s.name, "line": s.line, "detail": s.detail })).collect::<Vec<_>>(),
            "find": {
                "open": self.v.find_open,
                "query": self.v.query.text,
                "case": self.v.query.case,
                "whole_word": self.v.query.whole_word,
                "count": self.v.found.1.len(),
                "current": self.v.current.map(|c| c + 1),
            },
            "disk_note": g.disk_note,
            "top_line": self.v.top_line,
        })
    }

    /// The state `text.view` answers: the symbols and the matches made fresh for it.
    pub fn state_for_command(&mut self) -> Value {
        self.text_hash(u64::MAX);
        let _ = self.symbols();
        let text = self.m.lock().text.clone();
        let _ = self.found(&text);
        self.state_json()
    }

    /// Take Theirs, for `text.view take_theirs=true`.
    pub fn take_theirs(&mut self) -> bool {
        if self.m.lock().disk_note.is_empty() {
            return false;
        }
        let text = model::take_theirs(&self.m);
        self.v.ctl.set_text = Some((text, 0));
        true
    }
}

/// Opens a file in its Text tab with the caret at a line: the ask waits for the tab.
pub async fn open_at(editor: &Arc<Editor>, source: &Source, at: (u32, u32)) {
    let id = source.tab_id();
    TextTabs::of(editor).ask(&id, |a| a.goto = Some(at));
    let req = match source {
        Source::Project(rel) => OpenRequest::file(KIND, rel),
        Source::Game(p) => OpenRequest { arg: Some("game".to_owned()), ..OpenRequest::file(KIND, p) },
    };
    let opened = gz_app::on_ui(editor, move |shell, ctx| {
        shell.with_cx(ctx, |cx| cx.requests.open(req));
        Ok(())
    })
    .await;
    if let Err(e) = opened {
        editor.notices.error(format!("The file could not be opened: {}", e.message));
    }
}

impl Document for TextDoc {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &'static str {
        KIND
    }

    fn label(&self) -> Tx<'static> {
        Tx::raw(self.m.lock().source.name().to_owned())
    }

    fn icon(&self) -> Icon {
        match self.m.lock().lang {
            CodeLang::Lua => Icon::FileScript,
            CodeLang::Ltx => Icon::FileConfig,
            CodeLang::Xml => Icon::FileXml,
            _ => Icon::FileText,
        }
    }

    fn path(&self) -> Option<String> {
        Some(self.m.lock().source.restore())
    }

    fn tooltip(&self) -> Option<Tx<'static>> {
        let g = self.m.lock();
        Some(match &g.source {
            Source::Project(rel) => Tx::raw(rel.clone()),
            Source::Game(p) => {
                tr!("{path} - a file of the game, read-only", path = p.replace('\\', "/")).into_owned()
            }
        })
    }

    fn dirty(&self) -> bool {
        self.m.lock().dirty()
    }

    fn padded(&self) -> bool {
        false
    }

    fn alive(&self) -> bool {
        !self.m.lock().closed && self.editor.project_opt().is_some()
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        self.text_hash(cx.ctx.cumulative_frame_nr());
        let (dirty, read_only, lang, has_file, encoding, eol) = {
            let g = self.m.lock();
            (g.dirty(), g.source.read_only(), g.lang, g.file.is_some(), g.encoding, g.eol)
        };
        tb.group(3);
        gz_app::toolbar_button(tb, cx, "file.save", Kind::Ghost, None);
        let clean = "No unsaved edits";
        if tb.action(
            "revert",
            Icon::Undo,
            "Revert",
            if dirty { "Drop the unsaved edits and read the file again" } else { clean },
            dirty,
            false,
            Kind::Ghost,
        ) {
            self.revert(cx.ctx);
        }
        if lang == CodeLang::Markdown {
            tb.group(5);
            let modes = [Mode::Edit, Mode::Split, Mode::Preview];
            let mut at = modes.iter().position(|m| *m == self.v.mode).unwrap_or(0);
            if tb.segmented(
                "mode",
                "View",
                &mut at,
                &[
                    Tx::ctx("text_mode", "Edit"),
                    Tx::ctx("text_mode", "Split"),
                    Tx::ctx("text_mode", "Preview"),
                ],
                "The text, the text beside its preview, or the preview",
                true,
            ) {
                self.v.mode = modes[at.min(2)];
            }
        }
        tb.group(2);
        if tb.action(
            "find",
            Icon::Search,
            "Find",
            "Find and replace in the text (Ctrl+F)",
            has_file,
            self.v.find_open,
            Kind::Ghost,
        ) {
            self.v.find_open = !self.v.find_open;
            self.v.goto_open = false;
            self.v.focus_find = self.v.find_open;
        }
        if tb.action(
            "goto_line",
            Icon::Forward,
            "Go to Line",
            "The caret to a line (Ctrl+G)",
            has_file,
            self.v.goto_open,
            Kind::Ghost,
        ) {
            self.v.goto_open = !self.v.goto_open;
            self.v.find_open = false;
            self.v.focus_find = self.v.goto_open;
        }
        let symbols = self.symbols();
        let what = match lang {
            CodeLang::Ltx => "The file's sections (Ctrl+Shift+O)",
            CodeLang::Markdown => "The file's headings (Ctrl+Shift+O)",
            _ => "The file's functions (Ctrl+Shift+O)",
        };
        let items: Vec<PickItem> = symbols
            .iter()
            .enumerate()
            .map(|(k, s)| {
                let mut it = PickItem::new(k.to_string(), Tx::raw(s.name.clone()));
                it.detail = tr!("Line {n} {detail}", n = s.line, detail = s.detail.clone()).into_owned();
                it.icon = match s.kind {
                    gz_text::symbols::SymbolKind::Section => Icon::FileConfig,
                    gz_text::symbols::SymbolKind::Heading => Icon::FileText,
                    _ => Icon::FileScript,
                };
                it
            })
            .collect();
        let mut picked: Option<u32> = None;
        tb.menu(
            "symbols",
            Icon::Menu,
            "Go to Symbol",
            what,
            Some(SYMBOLS_POPUP),
            !symbols.is_empty(),
            |ui| {
                let out = PickList::new("gz-ui-text.symbols", &items)
                    .hint("Search")
                    .width(gz_ui::units::em(22.0))
                    .show(ui);
                if let Some(i) = out.item {
                    picked = symbols.get(i).map(|s| s.line);
                    ui.close();
                } else if out.close {
                    ui.close();
                }
            },
        );
        if let Some(line) = picked {
            self.v.ctl.goto = Some((line, 1));
            self.v.md_goto = line;
        }
        tb.group(1);
        let mut set_enc: Option<Encoding> = None;
        let mut set_eol: Option<Eol> = None;
        let mut wrap = self.v.wrap;
        let mut reveal = false;
        tb.menu(
            "more",
            Icon::More,
            "More",
            "Word Wrap, the encoding and the line endings the file is saved in",
            Some(MORE_POPUP),
            true,
            |ui| {
                if gz_ui::menu_item(ui, "Word Wrap", MenuItem { selected: wrap, ..MenuItem::default() }) {
                    wrap = !wrap;
                    ui.close();
                }
                ui.separator();
                let why = if read_only {
                    Tx::from("A file of the game: the editor never writes it.")
                } else {
                    Tx::EMPTY
                };
                for (e, label) in [
                    (Encoding::Utf8, "Save as UTF-8"),
                    (Encoding::Utf8Bom, "Save as UTF-8 with BOM"),
                    (Encoding::Cp1251, "Save as Windows-1251"),
                ] {
                    if gz_ui::menu_item(
                        ui,
                        label,
                        MenuItem {
                            selected: encoding == e,
                            enabled: !read_only,
                            tip: why.clone(),
                            ..MenuItem::default()
                        },
                    ) {
                        set_enc = Some(e);
                        ui.close();
                    }
                }
                ui.separator();
                for (e, label) in
                    [(Eol::Crlf, "Windows Line Endings (CRLF)"), (Eol::Lf, "Unix Line Endings (LF)")]
                {
                    if gz_ui::menu_item(
                        ui,
                        label,
                        MenuItem {
                            selected: eol == e,
                            enabled: !read_only,
                            tip: why.clone(),
                            ..MenuItem::default()
                        },
                    ) {
                        set_eol = Some(e);
                        ui.close();
                    }
                }
                ui.separator();
                if gz_ui::menu_item(
                    ui,
                    "Show in Explorer",
                    MenuItem { icon: Icon::Reveal, enabled: !read_only, ..MenuItem::default() },
                ) {
                    reveal = true;
                    ui.close();
                }
            },
        );
        self.v.wrap = wrap;
        if let Some(e) = set_enc {
            self.m.lock().encoding = e;
        }
        if let Some(e) = set_eol {
            self.m.lock().eol = e;
        }
        if reveal && let Source::Project(rel) = self.m.lock().source.clone() {
            cx.editor.post("ui.reveal", args(&[("path", json!(rel))]));
        }
        if read_only {
            tb.status("Read-only: a file of the game", Tone::Muted);
        }
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        self.start();
        let ctx = ui.ctx().clone();
        self.text_hash(ctx.cumulative_frame_nr());
        self.take_asks(&ctx);
        let p = palette(&ctx);
        let full = ui.max_rect();
        ui.painter().rect_filled(full, egui::CornerRadius::ZERO, p.base);
        let (loading, error, note, problems, lang) = {
            let g = self.m.lock();
            (g.loading, g.read_error.clone(), g.disk_note.clone(), g.problems.clone(), g.lang)
        };
        let mut area = ui.new_child(
            UiBuilder::new().id_salt("gz-ui-text.body").max_rect(full).layout(Layout::top_down(Align::Min)),
        );
        if loading {
            let mut c = area
                .new_child(UiBuilder::new().max_rect(full.shrink(16.0)).layout(Layout::top_down(Align::Min)));
            gz_ui::state::busy_line(&mut c, "Reading the file...");
            ui.allocate_rect(full, Sense::hover());
            return;
        }
        if let Some(e) = error {
            let mut c = area
                .new_child(UiBuilder::new().max_rect(full.shrink(16.0)).layout(Layout::top_down(Align::Min)));
            gz_ui::state::error_state(&mut c, "The file cannot be opened", Tx::ui(e), None);
            ui.allocate_rect(full, Sense::hover());
            return;
        }
        if !note.is_empty() {
            let theirs = note.starts_with("Changed on disk");
            if gz_ui::state::banner(
                &mut area,
                Tone::Warning,
                Tx::ui(note),
                theirs.then(|| Tx::from("Take Theirs")),
                None,
            ) == Some(0)
            {
                self.take_theirs();
            }
        }
        if self.v.find_open {
            area.add_space(4.0);
            self.draw_find_bar(&mut area);
            area.add_space(4.0);
        } else if self.v.goto_open {
            area.add_space(4.0);
            self.draw_goto_bar(&mut area);
            area.add_space(4.0);
        }
        let top = area.cursor().min.y;
        let drawer_h = if problems.is_empty() {
            0.0
        } else {
            (problems.len().min(PROBLEM_ROWS) as f32) * gz_ui::metrics::ROW + 12.0
        };
        let work = Rect::from_min_max(
            pos2(full.min.x, top),
            pos2(full.max.x, (full.max.y - drawer_h).max(top + 40.0)),
        );
        let markdown = lang == CodeLang::Markdown;
        match (markdown, self.v.mode) {
            (true, Mode::Preview) => self.draw_preview(ui, work),
            (true, Mode::Split) => {
                let mid = work.center().x;
                let left = Rect::from_min_max(work.min, pos2(mid - 3.0, work.max.y));
                let right = Rect::from_min_max(pos2(mid + 3.0, work.min.y), work.max);
                self.draw_code(ui, left, cx);
                ui.painter().vline(mid, work.y_range(), egui::Stroke::new(1.0, p.border_subtle));
                self.draw_preview(ui, right);
            }
            _ => self.draw_code(ui, work, cx),
        }
        if !problems.is_empty() {
            let rect = Rect::from_min_max(pos2(full.min.x, work.max.y + 4.0), full.max);
            let mut d = ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-ui-text.problems")
                    .max_rect(rect.shrink2(Vec2::new(8.0, 4.0)))
                    .layout(Layout::top_down(Align::Min)),
            );
            self.draw_problems(&mut d, &problems);
        }
        ui.allocate_rect(full, Sense::hover());
        self.check_when_quiet(ctx.input(|i| i.time), &ctx);
    }

    fn status(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let p = palette(ui.ctx());
        let (lang, enc, eol, n, errors) = {
            let g = self.m.lock();
            let errors = g.problems.iter().filter(|p| p.severity == gz_base::Severity::Error).count();
            (g.lang.title(), g.encoding.title(), g.eol.title(), g.problems.len(), errors)
        };
        let shown = if n == 0 {
            tr!(
                "{lang} · {enc} · {eol} · Ln {line}, Col {col}",
                lang = lang,
                enc = enc,
                eol = eol,
                line = self.v.caret.0.max(1),
                col = self.v.caret.1.max(1)
            )
        } else {
            tr!(
                "{lang} · {enc} · {eol} · Ln {line}, Col {col} · {n} {n:problem|problems}",
                lang = lang,
                enc = enc,
                eol = eol,
                line = self.v.caret.0.max(1),
                col = self.v.caret.1.max(1),
                n = n
            )
        };
        text::label(ui, shown, Font::Small, if errors > 0 { p.error } else { p.text_secondary }, Clip::End);
    }

    fn shortcut(&mut self, cx: &mut DocCx<'_>, chord: &Chord) -> bool {
        let plain = !chord.ctrl && !chord.alt;
        match chord.key {
            Key::F if chord.ctrl && !chord.shift => {
                self.v.find_open = true;
                self.v.goto_open = false;
                self.v.focus_find = true;
            }
            Key::H if chord.ctrl => {
                self.v.find_open = true;
                self.v.replace_open = true;
                self.v.focus_find = true;
            }
            Key::G if chord.ctrl => {
                self.v.goto_open = true;
                self.v.find_open = false;
                self.v.focus_find = true;
            }
            Key::O if chord.ctrl && chord.shift => gz_ui::menu::show_popup(cx.ctx, SYMBOLS_POPUP),
            Key::F3 if plain => self.step_find(chord.shift),
            Key::Escape if plain && (self.v.find_open || self.v.goto_open) => {
                self.v.find_open = false;
                self.v.goto_open = false;
                self.v.ctl.focus = true;
            }
            _ => return false,
        }
        true
    }

    fn edit(&mut self, cx: &mut DocCx<'_>, action: gz_app::EditAction, run: bool) -> bool {
        use gz_app::EditAction as E;
        let read_only = self.m.lock().source.read_only();
        let there = match action {
            E::Find | E::Copy => true,
            E::Undo | E::Redo | E::Cut | E::Duplicate | E::Delete => !read_only,
            E::Paste => false,
        };
        if !there || !run {
            return there;
        }
        let selected = || {
            let g = self.m.lock();
            self.v.selection.clone().and_then(|r| g.text.get(r).map(str::to_owned))
        };
        match action {
            E::Undo => self.v.ctl.undo = true,
            E::Redo => self.v.ctl.redo = true,
            E::Duplicate => self.v.ctl.duplicate = true,
            E::Delete => self.v.ctl.delete = true,
            E::Copy => {
                if let Some(t) = selected() {
                    cx.ctx.copy_text(t);
                }
            }
            E::Cut => {
                if let Some(t) = selected() {
                    cx.ctx.copy_text(t);
                    self.v.ctl.delete = true;
                }
            }
            E::Find => {
                self.v.find_open = true;
                self.v.goto_open = false;
                self.v.focus_find = true;
            }
            E::Paste => {}
        }
        true
    }

    fn save(&mut self, _cx: &mut DocCx<'_>) -> Result<(), String> {
        {
            let g = self.m.lock();
            if g.source.read_only() {
                return Err("A file of the game: the editor never writes it.".to_owned());
            }
            if !g.dirty() || g.saving {
                return Ok(());
            }
        }
        let (ed, m) = (self.editor.clone(), self.m.clone());
        let _task = self.editor.exec.spawn(async move {
            let _ = model::save(&ed, &m).await;
        });
        Ok(())
    }

    fn save_failed(&mut self) -> Option<gz_editor::CmdError> {
        self.m.lock().save_error.take()
    }

    fn reload(&mut self, _cx: &mut DocCx<'_>, discard: bool) -> Result<(), String> {
        if self.m.lock().dirty() && !discard {
            return Err("The tab holds unsaved edits: reload with discard to drop them.".to_owned());
        }
        let (ed, m) = (self.editor.clone(), self.m.clone());
        let _task = self.editor.exec.spawn(async move { model::load(&ed, &m).await });
        Ok(())
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
        let mut g = self.m.lock();
        g.text = g.disk.clone();
        if let Some((e, l)) = g.file.as_ref().map(|f| (f.encoding, f.eol)) {
            g.encoding = e;
            g.eol = l;
        }
        g.disk_note.clear();
    }

    fn on_activate(&mut self, _cx: &mut DocCx<'_>) {
        self.start();
    }

    fn inactive(&mut self, _cx: &mut DocCx<'_>) {
        // the file follows the disk whether the tab is up or not
        self.start();
    }

    fn on_close(&mut self, _cx: &mut DocCx<'_>) {
        self.follow.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }

    fn restore_arg(&self) -> Option<String> {
        Some(self.m.lock().source.restore())
    }

    fn mcp_state(&self) -> Value {
        self.state_json()
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}
