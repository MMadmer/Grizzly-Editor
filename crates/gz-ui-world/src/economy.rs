//! Economy: the `economy` singleton tab (docs/UI_DESIGN.md, section 6, "Economy").
//!
//! Who sells an item, who carries it and what makes it - three pages over the module's traders
//! (`.tradeasset`), loot tables (`.lootasset`) and recipes (`.recipeasset`): the files of the
//! page's kind on the left, the rows of the one selected on the right. An item never says any of
//! this (`docs/ASSETS.md`, "One thing, one place"); its card only shows it, and opens this tab.
//!
//! The tab is saved as it is edited: every edit of a row is the file written through
//! [`Items::save_table`] (the difference alone reaches the author's text), one save after
//! another. What a frame shows is the roster as the pools read it, with the edit a save in flight
//! carries; the problems are the check's, as sentences under the rows.

use std::sync::Arc;

use egui::{Align, Layout, Rect, ScrollArea, Ui, UiBuilder, Vec2, pos2};
use gz_app::{DocCx, Document, ShellBuilder};
use gz_base::Rev;
use gz_editor::{Editor, Value, json};
use gz_items::validate::Lookups;
use gz_items::{Count, Drop, Issue, Items, Recipe, Roster, Rows, Stock, TableFile, TableKind};
use gz_mod::ManifestService;
use gz_project::Project;
use gz_ui::dialog::{Answer, ConfirmSpec, confirm};
use gz_ui::list::list;
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{Button, Col, Font, Icon, ItemRows, Kind, TipExt, Tone, Toolbar, Tx, palette, tr};
use parking_lot::Mutex;

use crate::call::{self, args, s};
use crate::item_lists::{IconSheets, ItemLists, ItemListsView, Pictures};
use crate::items::IconSource;
use crate::lists::{Indexed, Lists, ListsView};
use crate::local::{local_value_combo, local_value_text};
use crate::picker::list_picker;

/// The tab's id and kind.
pub const TAB: &str = "economy";
/// What the toolbar says while every edit is in its file.
pub const SAVED: &str = "Saved to the file as you edit";
/// The three pages, in the order of the toolbar.
pub const PAGES: [(&str, TableKind); 3] =
    [("Traders", TableKind::Trade), ("Loot", TableKind::Loot), ("Recipes", TableKind::Recipes)];
/// The folder a file the tab makes goes into, by kind.
const FOLDERS: [&str; 3] = ["trade", "loot", "craft"];
const LIST_W: f32 = 260.0;
/// The widest the rows are drawn: the last column takes what is left of it.
const COLUMN_W: f32 = 1120.0;
/// The narrowest: under it the rows scroll sideways instead of squeezing their last column.
const COLUMN_MIN: f32 = 1000.0;
/// An item's icon in a row: fitted into this many points.
const THUMB: Vec2 = Vec2::new(30.0, 22.0);

/// The page of a kind, from 0.
#[must_use]
pub fn page_of(kind: TableKind) -> usize {
    PAGES.iter().position(|(_, k)| *k == kind).unwrap_or(0)
}

/// The roster and the problems of its table files, by entry.
pub(crate) struct Loaded {
    roster: Arc<Roster>,
    problems: Vec<Vec<Issue>>,
}

/// Reads the roster and the problems of its table files, on the pools.
pub(crate) async fn load(ed: &Arc<Editor>, fresh: bool) -> Loaded {
    match Items::of(ed).await {
        Ok(items) => {
            let roster = if fresh { items.fresh_roster(ed).await } else { items.roster(ed).await };
            let problems = items.all_table_problems(ed, roster.clone()).await;
            Loaded { roster, problems }
        }
        Err(_) => Loaded { roster: Arc::new(Roster::default()), problems: Vec::new() },
    }
}

#[derive(Default)]
struct Inbox {
    loaded: Option<Loaded>,
    saved: Option<Result<String, String>>,
    deleted: Option<Result<(), String>>,
    /// The stock sections of a trader's config: `(trader, sections)`.
    supplies: Option<(String, Vec<String>)>,
    delete_yes: bool,
}

/// What a frame asked for, run after it is drawn.
pub(crate) enum Action {
    Page(usize),
    Select(String),
    /// A new file of the page's kind; for a trader, the trader it is about.
    New(String),
    /// The selected file with a row added, changed or taken away.
    Edit(TableFile),
    AskDelete,
}

/// The Economy tab.
pub struct EconomyDoc {
    editor: Arc<Editor>,
    root: String,
    pub(crate) kind: TableKind,
    /// The file selected, project-relative.
    pub(crate) sel: Option<String>,
    loaded: Option<Loaded>,
    key: Option<Rev>,
    loading: bool,
    fresh_wanted: bool,
    /// The selected file as it is shown: what was read, with the edits a save in flight carries.
    pub(crate) shown: Option<TableFile>,
    saving: bool,
    /// An edit made while a save is in flight: written when that one has ended.
    queued: Option<TableFile>,
    pub(crate) status: String,
    inbox: Arc<Mutex<Inbox>>,
    lists: ListsView,
    item_lists: ItemListsView,
    /// What an item is picked from: the linked game's items and the module's own, by the names
    /// the player reads; made again when either list moves.
    items: Arc<Indexed>,
    items_key: (u64, Rev),
    /// The icons of the items the rows name, by section: worked out once per roster.
    icons: std::cell::RefCell<rustc_hash::FxHashMap<String, Option<IconSource>>>,
    pictures: Option<Arc<Pictures>>,
    icon_sheets: Option<Arc<IconSheets>>,
    /// The stock sections of the selected trader's config.
    supplies: Option<(String, Vec<String>)>,
    supplies_asked: Option<String>,
    /// A file to select once the roster has it.
    pending_open: Option<String>,
    pub(crate) actions: Vec<Action>,
    /// The delete question put up to be looked at (`economy.view ask_delete=true`).
    look_delete: bool,
}

/// A file's name without its folder and extension.
fn stem(file: &str) -> &str {
    gz_items::model::stem(file)
}

/// The sentence a problem of a table file shows: the check's message with a capital and a
/// period; a file that does not read names itself first, and the tab is about that file already.
#[must_use]
pub fn sentence(issue: &Issue) -> String {
    let mut i = issue.clone();
    if i.code == gz_items::roster::UNREADABLE
        && let Some(at) = i.message.find("asset: ")
    {
        i.message = i.message.get(at + "asset: ".len()..).unwrap_or("").to_owned();
    }
    crate::item_model::sentence(&i)
}

/// The row a problem is about, from 0, when it is about one.
fn row_of(issue: &Issue) -> Option<usize> {
    match issue.path.get(1) {
        Some(gz_luadata::Seg::Index(i)) => Some(*i),
        _ => None,
    }
}

fn tone_of(issue: &Issue) -> Tone {
    match issue.severity {
        gz_base::Severity::Error => Tone::Error,
        gz_base::Severity::Warning => Tone::Warning,
        _ => Tone::Muted,
    }
}

/// A number as a row's field shows it; empty for one the file leaves out.
fn num(v: Option<f64>) -> Option<String> {
    v.map(gz_luadata::number_to_text)
}

impl EconomyDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>, open: Option<String>) -> Self {
        let kind = open.as_deref().and_then(TableKind::of_file).unwrap_or(TableKind::Trade);
        Self {
            editor: editor.clone(),
            root: String::new(),
            kind,
            sel: None,
            loaded: None,
            key: None,
            loading: false,
            fresh_wanted: true,
            shown: None,
            saving: false,
            queued: None,
            status: String::new(),
            inbox: Arc::new(Mutex::new(Inbox::default())),
            lists: ListsView::default(),
            item_lists: ItemListsView::default(),
            items: Arc::new(Indexed::default()),
            items_key: (u64::MAX, Rev::ZERO),
            icons: std::cell::RefCell::default(),
            pictures: None,
            icon_sheets: None,
            supplies: None,
            supplies_asked: None,
            pending_open: open.map(|p| p.replace('\\', "/")),
            actions: Vec::new(),
            look_delete: false,
        }
    }

    // ---- the roster -----------------------------------------------------------------------------

    fn sync(&mut self) {
        let Ok((session, project)) = Project::of(&self.editor) else { return };
        let root = session.root.to_string_lossy().into_owned();
        if root != self.root {
            self.root = root;
            self.loaded = None;
            self.key = None;
            self.sel = None;
            self.shown = None;
        }
        let manifest = ManifestService::get(&session).map_or(Rev::ZERO, |m| m.rev());
        let key = project.tree.rev().mix(manifest).mix(project.game_rev());
        if self.loading || self.key == Some(key) {
            return;
        }
        self.key = Some(key);
        self.loading = true;
        let fresh = std::mem::take(&mut self.fresh_wanted);
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            let loaded = load(&ed, fresh).await;
            inbox.lock().loaded = Some(loaded);
            call::wake_ui(&ed);
        });
    }

    pub(crate) fn reload_fresh(&mut self) {
        self.fresh_wanted = true;
        self.key = None;
    }

    /// The table files of the page's kind: `(roster index, file)`.
    fn files(&self) -> Vec<(usize, String)> {
        let Some(l) = &self.loaded else { return Vec::new() };
        l.roster
            .tables
            .iter()
            .enumerate()
            .filter(|(_, t)| t.table.kind() == self.kind)
            .map(|(i, t)| (i, t.table.file.clone()))
            .collect()
    }

    /// What the list calls a file: a trader by the name its config goes by, any other by its name.
    fn title(&self, table: &TableFile) -> Tx<'static> {
        match &table.rows {
            Rows::Trade { trader, .. } if !trader.is_empty() => self.trader_label(trader),
            _ => Tx::raw(stem(&table.file).to_owned()),
        }
    }

    fn trader_label(&self, trader: &str) -> Tx<'static> {
        match self.item_lists.traders.get(trader).filter(|e| !e.name.is_empty()) {
            Some(e) => crate::lists::Indexed::label_tx(e),
            None => Tx::raw(trader.to_owned()),
        }
    }

    fn take_inbox(&mut self) {
        let mut inbox = self.inbox.lock();
        let loaded = inbox.loaded.take();
        let saved = inbox.saved.take();
        let deleted = inbox.deleted.take();
        let supplies = inbox.supplies.take();
        let delete_yes = std::mem::take(&mut inbox.delete_yes);
        drop(inbox);
        if let Some(supplies) = supplies {
            self.supplies = Some(supplies);
        }
        if let Some(result) = saved {
            self.saving = false;
            match result {
                Ok(file) => {
                    self.status.clear();
                    if self.sel.is_none() {
                        self.pending_open = Some(file);
                    }
                }
                Err(e) => {
                    self.status = format!("Not saved: {e}");
                    self.queued = None;
                    // what the file says is shown again
                    self.shown = None;
                }
            }
            match self.queued.take() {
                Some(next) => self.start_save(next),
                None => self.reload_fresh(),
            }
        }
        if let Some(result) = deleted {
            match result {
                Ok(()) => {
                    self.status.clear();
                    self.sel = None;
                    self.shown = None;
                }
                Err(e) => self.status = format!("Not deleted: {e}"),
            }
            self.reload_fresh();
        }
        if let Some(loaded) = loaded {
            self.loading = false;
            self.take_loaded(loaded);
        }
        if delete_yes {
            self.delete_now();
        }
    }

    /// A roster arrived: the files as the disk has them.
    pub(crate) fn take_loaded(&mut self, loaded: Loaded) {
        self.loaded = Some(loaded);
        self.icons.borrow_mut().clear();
        self.settle();
    }

    /// The items a row picks from: the linked game's, then the items the module makes itself -
    /// what the rows are mostly about -, each by the name the player reads.
    fn sync_items(&mut self) {
        let stamp = self.loaded.as_ref().map_or(Rev::ZERO, |l| l.roster.stamp);
        let key = (self.lists.items.serial, stamp);
        if key == self.items_key {
            return;
        }
        self.items_key = key;
        let mut entries = self.lists.items.entries.clone();
        if let Some(l) = &self.loaded {
            for (e, a) in l.roster.entries.iter().zip(&l.roster.composition.applied) {
                let new = e.readable && e.item.kind() == gz_items::Kind::New && !e.item.is_abstract();
                if !new || a.section.is_empty() || a.left_out || self.lists.items.get(&a.section).is_some() {
                    continue;
                }
                let mut name = gz_characters::text_in(&e.item.name, "eng");
                if name.is_empty() {
                    name = gz_characters::text_in(&e.item.name, "rus");
                }
                entries.push(gz_nq::PickerEntry { id: a.section.clone(), name, extra: "project".to_owned() });
            }
        }
        self.items = Arc::new(Indexed::new(entries, self.lists.items.serial.wrapping_add(stamp.0)));
    }

    /// The item picker of a row: the item's icon, then the pick list over [`EconomyDoc::items`].
    /// Returns whether the value changed.
    fn item_cell(&self, ui: &mut Ui, salt: &str, value: &mut String, none: &'static str) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // an empty cell has no picture to keep room for
            if !value.trim().is_empty() {
                let (rect, _) = ui.allocate_exact_size(THUMB, egui::Sense::hover());
                let key = value.trim().to_ascii_lowercase();
                let src = self.icons.borrow().get(&key).cloned();
                let src = match src {
                    Some(src) => src,
                    None => {
                        let found = self.loaded.as_ref().and_then(|l| {
                            crate::items::roster_icon(
                                &self.editor,
                                std::path::Path::new(&self.root),
                                &l.roster,
                                &key,
                            )
                        });
                        self.icons.borrow_mut().insert(key, found.clone());
                        found
                    }
                };
                if src.is_some() {
                    let _ = crate::item_form::paint_item_icon(
                        ui,
                        rect,
                        src.as_ref(),
                        &self.editor,
                        self.pictures.as_ref(),
                        self.icon_sheets.as_ref(),
                    );
                }
            }
            changed = list_picker(
                ui,
                salt,
                value,
                &self.items,
                &self.items,
                Some(Tx::from(none)),
                !self.lists.loaded,
            );
        });
        changed
    }

    /// Shows a page: its first file is selected.
    pub(crate) fn set_page(&mut self, kind: TableKind) {
        if kind != self.kind {
            self.kind = kind;
            self.sel = None;
            self.shown = None;
            self.queued = None;
            self.settle();
        }
    }

    /// The selection against the roster that arrived: a file asked for is selected, one that is
    /// gone is let go, and the file shown is what was read unless a save is still to be heard.
    fn settle(&mut self) {
        let Some(roster) = self.loaded.as_ref().map(|l| l.roster.clone()) else { return };
        if let Some(path) = self.pending_open.take() {
            match roster.table_index(&path) {
                Some(i) => {
                    if let Some(t) = roster.tables.get(i) {
                        self.kind = t.table.kind();
                        self.sel = Some(t.table.file.clone());
                        self.shown = None;
                    }
                }
                // not there yet: the next read finds it
                None if self.loading || self.saving => self.pending_open = Some(path),
                None => {}
            }
        }
        let files = self.files();
        let found = self.sel.as_deref().and_then(|f| roster.table_index(f));
        if found.is_none() {
            self.sel = files.first().map(|(_, f)| f.clone());
            self.shown = None;
        }
        if !self.saving && self.queued.is_none() {
            self.shown = self
                .sel
                .as_deref()
                .and_then(|f| roster.table_index(f))
                .and_then(|i| roster.tables.get(i))
                .map(|t| t.table.clone());
        }
        self.want_supplies();
    }

    /// The stock sections of the selected trader's config, read once per trader.
    fn want_supplies(&mut self) {
        let Some(Rows::Trade { trader, .. }) = self.shown.as_ref().map(|t| &t.rows) else { return };
        if trader.is_empty() || self.supplies_asked.as_deref() == Some(trader.as_str()) {
            return;
        }
        self.supplies_asked = Some(trader.clone());
        let (ed, inbox, trader) = (self.editor.clone(), self.inbox.clone(), trader.clone());
        let _task = self.editor.exec.spawn(async move {
            let Some(file) = gz_items::tables::trader_file(&trader) else { return };
            let Ok(items) = Items::of(&ed).await else { return };
            let lookups = items.lookups(&ed).await;
            let sections = ed
                .exec
                .io(move || {
                    let mut out: Vec<String> = lookups
                        .config_sections(&file)
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|s| s.starts_with("supplies"))
                        .collect();
                    out.sort();
                    out.dedup();
                    out
                })
                .await;
            inbox.lock().supplies = Some((trader, sections));
            call::wake_ui(&ed);
        });
    }

    // ---- edits ----------------------------------------------------------------------------------

    fn start_save(&mut self, table: TableFile) {
        self.saving = true;
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            let result = match Items::of(&ed).await {
                Ok(items) => items.save_table(&ed, None, &table).await.map_err(|e| e.message),
                Err(e) => Err(e.message),
            };
            inbox.lock().saved = Some(result);
            call::wake_ui(&ed);
        });
    }

    /// The selected file as edited: shown at once, written as soon as no other write is under way.
    pub(crate) fn edit(&mut self, table: TableFile) {
        if self.shown.as_ref() == Some(&table) {
            return;
        }
        self.shown = Some(table.clone());
        if self.saving {
            self.queued = Some(table);
        } else {
            self.start_save(table);
        }
    }

    /// The first free file of the page's kind in its folder: `<folder>/<name>`, `<name>_2`, ...
    fn free_file(&self, name: &str) -> String {
        let folder = FOLDERS[page_of(self.kind)];
        let ext = self.kind.extension();
        let name: String = name
            .trim()
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
            .collect();
        let name = if name.is_empty() { self.kind.header().to_owned() } else { name };
        let taken = |file: &str| {
            self.loaded.as_ref().is_some_and(|l| l.roster.table_index(file).is_some())
                || self.shown.as_ref().is_some_and(|t| t.file.eq_ignore_ascii_case(file))
        };
        let mut file = format!("{folder}/{name}{ext}");
        let mut n = 2;
        while taken(&file) {
            file = format!("{folder}/{name}_{n}{ext}");
            n += 1;
        }
        file
    }

    /// A new file of the page's kind, not written yet: a trader's is about `trader`.
    pub(crate) fn plan_new(&self, trader: &str) -> Result<TableFile, String> {
        let (name, rows) = match self.kind {
            TableKind::Trade => {
                let t = trader.trim().to_ascii_lowercase();
                if t.is_empty() || gz_items::tables::trader_file(&t).is_none() {
                    return Err("Pick the trader the file is about.".to_owned());
                }
                let name = gz_items::tables::trader_name(&t);
                (
                    stem(&name).trim_start_matches("trade_").to_owned(),
                    Rows::Trade { trader: name, stock: Vec::new() },
                )
            }
            TableKind::Loot => ("loot".to_owned(), Rows::Loot(Vec::new())),
            TableKind::Recipes => ("recipes".to_owned(), Rows::Recipes(Vec::new())),
        };
        Ok(TableFile { file: self.free_file(&name), rows, editor: gz_luadata::Value::Nil })
    }

    /// A new file of the page's kind, written at once and selected.
    pub(crate) fn make(&mut self, trader: &str) {
        match self.plan_new(trader) {
            Ok(table) => {
                self.sel = Some(table.file.clone());
                self.pending_open = Some(table.file.clone());
                self.edit(table);
            }
            Err(e) => self.status = e,
        }
    }

    /// Names what disappears and asks: the file goes for good.
    fn ask_delete(&mut self, ctx: &egui::Context, asker: gz_ui::Asker) {
        let Some(table) = self.shown.clone() else { return };
        let spec = ConfirmSpec {
            id: "delete_table".to_owned(),
            title: "Delete File".into(),
            headline: tr!("Delete \"{name}\"?", name = stem(&table.file)),
            items: vec![Tx::raw(table.file.clone())],
            total: 0,
            consequence: match table.kind() {
                TableKind::Trade => "Deleted for good - this cannot be undone. The next build takes these rows off the trader's shelf.",
                TableKind::Loot => "Deleted for good - this cannot be undone. The next build takes these rows out of what the dead carry.",
                TableKind::Recipes => "Deleted for good - this cannot be undone. The next build takes these recipes out of the module.",
            }
            .into(),
            action: "Delete".into(),
            destructive: true,
        };
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let line = confirm(ctx, asker, spec, move |answer| {
            if answer == Answer::Yes {
                inbox.lock().delete_yes = true;
                call::wake_ui(&ed);
            }
        });
        call::log_line(&self.editor, line);
    }

    fn delete_now(&mut self) {
        let Some(file) = self.sel.clone() else { return };
        let inbox = self.inbox.clone();
        call::call_then(&self.editor, "file.delete", args([("path", s(&file))]), move |reply, _| {
            inbox.lock().deleted = Some(if call::ok(reply) { Ok(()) } else { Err(call::error_text(reply)) });
        });
    }

    pub(crate) fn run_actions(&mut self, ctx: &egui::Context) {
        // put up to be looked at: a person's window answers it, whoever asked
        if std::mem::take(&mut self.look_delete) {
            self.ask_delete(ctx, gz_ui::Asker::Ui);
        }
        for a in std::mem::take(&mut self.actions) {
            match a {
                Action::Page(i) => {
                    if let Some((_, kind)) = PAGES.get(i) {
                        self.set_page(*kind);
                    }
                }
                Action::Select(file) => {
                    if self.sel.as_deref() != Some(file.as_str()) {
                        self.sel = Some(file);
                        self.shown = None;
                        self.queued = None;
                        self.settle();
                    }
                }
                Action::New(trader) => self.make(&trader),
                Action::Edit(table) => self.edit(table),
                Action::AskDelete => self.ask_delete(ctx, call::asker(&self.editor)),
            }
        }
    }

    // ---- drawing --------------------------------------------------------------------------------

    fn draw_list(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        let files = self.files();
        let Some(loaded) = &self.loaded else { return };
        let sel = self.sel.clone();
        let mut pick = None;
        list(ui, "gz-ui-world.economy.list", Vec2::ZERO, |l| {
            for (i, file) in &files {
                let Some(t) = loaded.roster.tables.get(*i) else { continue };
                let selected = sel.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(file));
                let worst = loaded.problems.get(*i).and_then(|ps| {
                    if ps.iter().any(Issue::is_error) {
                        Some(Tone::Error)
                    } else {
                        ps.iter().any(|x| x.severity == gz_base::Severity::Warning).then_some(Tone::Warning)
                    }
                });
                let title = self.title(&t.table);
                let count = t.table.rows.len();
                let resp = l.row(file, selected, |ui| {
                    let r = ui.max_rect();
                    let cy = r.center().y;
                    let right = em(2.5) + if worst.is_some() { 12.0 } else { 0.0 };
                    let room = (r.width() - right - 6.0).max(0.0);
                    text::paint(
                        ui.painter(),
                        &Font::Body.id(),
                        pos2(r.min.x + 6.0, cy),
                        egui::Align2::LEFT_CENTER,
                        p.text,
                        title.clone(),
                        Some(room),
                        Clip::End,
                    );
                    text::paint(
                        ui.painter(),
                        &Font::Small.id(),
                        pos2(r.max.x - 6.0 - if worst.is_some() { 12.0 } else { 0.0 }, cy),
                        egui::Align2::RIGHT_CENTER,
                        p.text_muted,
                        Tx::raw(count.to_string()),
                        None,
                        Clip::End,
                    );
                    if let Some(tone) = worst {
                        ui.painter().circle_filled(pos2(r.max.x - 7.0, cy), 3.0, p.tone(tone));
                    }
                });
                if resp.clicked() {
                    pick = Some(file.clone());
                }
                let _ = resp.tip(Tx::raw(file.clone()));
            }
        });
        if let Some(file) = pick {
            self.actions.push(Action::Select(file));
        }
    }

    /// The Add control of the page: a trader is picked from the game's, the other two are made.
    fn draw_add(&mut self, ui: &mut Ui) {
        match self.kind {
            TableKind::Trade => {
                let mut picked = String::new();
                if list_picker(
                    ui,
                    "economy.new_trader",
                    &mut picked,
                    &self.item_lists.traders,
                    &self.item_lists.traders,
                    Some(Tx::from("Add a Trader...")),
                    !self.item_lists.loaded,
                ) && !picked.is_empty()
                {
                    self.actions.push(Action::New(picked));
                }
            }
            TableKind::Loot | TableKind::Recipes => {
                let label =
                    if self.kind == TableKind::Loot { "Add a Loot Table" } else { "Add a Recipes File" };
                if Button::new(label).icon(Icon::Plus).show(ui).clicked() {
                    self.actions.push(Action::New(String::new()));
                }
            }
        }
    }

    fn draw_rows(&mut self, ui: &mut Ui) {
        let Some(table) = self.shown.clone() else { return };
        let p = palette(ui.ctx());
        ui.horizontal(|ui| {
            let _ = text::label(ui, self.title(&table), Font::Title, p.text, Clip::End);
            let _ = text::label(ui, Tx::raw(table.file.clone()), Font::Small, p.text_muted, Clip::Middle);
        });
        let what = match table.kind() {
            TableKind::Trade => {
                "What the module puts on this trader's shelf. A count of 0 takes an item off it."
            }
            TableKind::Loot => {
                "What the dead carry: how many, the chance for everybody, and for a community of its own."
            }
            TableKind::Recipes => "What is made, at which tool or workbench, and from one to four parts.",
        };
        let _ = text::label(ui, what, Font::Small, p.text_secondary, Clip::End);
        ui.add_space(8.0);
        let edited = match &table.rows {
            Rows::Trade { trader, stock } => {
                self.stock_rows(ui, stock).map(|stock| Rows::Trade { trader: trader.clone(), stock })
            }
            Rows::Loot(drops) => self.drop_rows(ui, drops).map(Rows::Loot),
            Rows::Recipes(recipes) => self.recipe_rows(ui, recipes).map(Rows::Recipes),
        };
        if let Some(rows) = edited {
            self.actions.push(Action::Edit(TableFile { rows, ..table.clone() }));
        }
        // the problems of the file, as sentences
        ui.add_space(10.0);
        let index = self.loaded.as_ref().and_then(|l| l.roster.table_index(&table.file));
        let issues: Vec<Issue> = match (index, &self.loaded) {
            (Some(i), Some(l)) => l.problems.get(i).cloned().unwrap_or_default(),
            _ => Vec::new(),
        };
        for issue in issues.iter().filter(|i| i.severity != gz_base::Severity::Note) {
            let line = sentence(issue);
            let shown = match row_of(issue) {
                Some(row) => tr!("Row {n}: {problem}", n = row + 1, problem = Tx::ui(line.as_str())),
                None => Tx::ui(line),
            };
            let _ = text::label(ui, shown, Font::Small, p.tone(tone_of(issue)), Clip::End);
        }
        ui.add_space(10.0);
        if Button::new("Delete...")
            .kind(Kind::GhostDanger)
            .tip("Delete this file for good")
            .show(ui)
            .clicked()
        {
            self.actions.push(Action::AskDelete);
        }
    }

    /// The stock sections a row's Stock column offers: the trader's own, `supplies_1` at least.
    fn stock_sections(&self, trader: &str) -> Vec<String> {
        let mut out = match &self.supplies {
            Some((t, s)) if t == trader => s.clone(),
            _ => Vec::new(),
        };
        if out.is_empty() {
            out.push("supplies_1".to_owned());
        }
        out
    }

    fn stock_rows(&self, ui: &mut Ui, stock: &[Stock]) -> Option<Vec<Stock>> {
        let columns = [
            Col::new("Item", em(24.0)),
            Col::new("Stock", em(9.0)),
            Col::new("Count", em(6.0)),
            Col::new("Chance", em(6.0)),
        ];
        let trader = match self.shown.as_ref().map(|t| &t.rows) {
            Some(Rows::Trade { trader, .. }) => trader.clone(),
            _ => String::new(),
        };
        let sections = self.stock_sections(&trader);
        let mut out = stock.to_vec();
        let mut changed = false;
        let result = ItemRows::new("economy.stock", &columns, stock.len())
            .add_label("Add Item")
            .fixed_order()
            .empty_text("Nothing of the module on this shelf yet.")
            .show(ui, |ui, row, col| {
                let Some(r) = out.get_mut(row) else { return };
                let salt = format!("economy.stock.{row}.{col}");
                match col {
                    0 => changed |= self.item_cell(ui, &salt, &mut r.item, "Pick an item..."),
                    1 => {
                        let shown = r.sections().join(", ");
                        if let Some(word) =
                            local_value_combo(ui, &salt, &shown, !r.supplies.is_empty(), &sections)
                        {
                            r.supplies = if word == "supplies_1" { Vec::new() } else { vec![word] };
                            changed = true;
                        }
                    }
                    2 | 3 => {
                        let v = if col == 2 { r.count } else { r.chance };
                        if let (Some(t), _) =
                            local_value_text(ui, &salt, num(v).as_deref(), Tx::raw("1"), ui.available_width())
                        {
                            let n = gz_ui::grid::parse_number(&t);
                            if col == 2 {
                                r.count = n;
                            } else {
                                r.chance = n;
                            }
                            changed = true;
                        }
                    }
                    _ => {}
                }
            });
        if let Some(i) = result.removed.filter(|i| *i < out.len()) {
            out.remove(i);
            changed = true;
        }
        if result.added {
            out.push(Stock::default());
            changed = true;
        }
        changed.then_some(out)
    }

    fn drop_rows(&self, ui: &mut Ui, drops: &[Drop]) -> Option<Vec<Drop>> {
        let columns = [
            Col::new("Item", em(22.0)),
            Col::new("From", em(4.0)),
            Col::new("To", em(4.0)),
            Col::new("Chance", em(5.0)),
            Col::new("Community", em(12.0)),
            Col::new("Its Chance", em(5.0)),
        ];
        // a drop is one row for everybody, then one row for each community of its own
        let mut lines: Vec<(usize, Option<usize>)> = Vec::new();
        for (i, d) in drops.iter().enumerate() {
            lines.push((i, None));
            lines.extend((0..d.by.len()).map(|b| (i, Some(b))));
        }
        let mut out = drops.to_vec();
        let mut changed = false;
        let mut add_community: Option<usize> = None;
        let result = ItemRows::new("economy.drops", &columns, lines.len())
            .add_label("Add Item")
            .fixed_order()
            .empty_text("The module adds nothing to what the dead carry yet.")
            .show(ui, |ui, row, col| {
                let Some(&(i, by)) = lines.get(row) else { return };
                let Some(d) = out.get_mut(i) else { return };
                let salt = format!("economy.drops.{row}.{col}");
                match (by, col) {
                    (None, 0) => {
                        changed |= self.item_cell(ui, &salt, &mut d.item, "Pick an item...");
                    }
                    (None, 1 | 2) => {
                        let (lo, hi) = match d.count {
                            Some(Count::One(n)) => (Some(n), Some(n)),
                            Some(Count::Range(a, b)) => (Some(a), Some(b)),
                            None => (None, None),
                        };
                        let v = if col == 1 { lo } else { hi };
                        if let (Some(t), _) =
                            local_value_text(ui, &salt, num(v).as_deref(), Tx::EMPTY, ui.available_width())
                        {
                            let n = gz_ui::grid::parse_number(&t);
                            let (a, b) = if col == 1 { (n, hi) } else { (lo, n) };
                            d.count = match (a, b) {
                                (Some(a), Some(b)) if (a - b).abs() < f64::EPSILON => Some(Count::One(a)),
                                (Some(a), Some(b)) => Some(Count::Range(a, b)),
                                (Some(a), None) | (None, Some(a)) => Some(Count::One(a)),
                                (None, None) => None,
                            };
                            changed = true;
                        }
                    }
                    (None, 3) => {
                        if let (Some(t), _) = local_value_text(
                            ui,
                            &salt,
                            num(d.chance).as_deref(),
                            Tx::EMPTY,
                            ui.available_width(),
                        ) {
                            d.chance = gz_ui::grid::parse_number(&t);
                            changed = true;
                        }
                    }
                    (None, 4) => {
                        if Button::new("Add Community").kind(Kind::Ghost).icon(Icon::Plus).show(ui).clicked()
                        {
                            add_community = Some(i);
                        }
                    }
                    (Some(b), 4) => {
                        if let Some(entry) = d.by.get_mut(b) {
                            changed |= list_picker(
                                ui,
                                &salt,
                                &mut entry.0,
                                &self.lists.communities,
                                &self.lists.stalker_communities,
                                None,
                                !self.lists.loaded,
                            );
                        }
                    }
                    (Some(b), 5) => {
                        if let Some(entry) = d.by.get_mut(b)
                            && let (Some(t), _) = local_value_text(
                                ui,
                                &salt,
                                Some(&gz_luadata::number_to_text(entry.1)),
                                Tx::EMPTY,
                                ui.available_width(),
                            )
                            && let Some(c) = gz_ui::grid::parse_number(&t)
                        {
                            entry.1 = c;
                            changed = true;
                        }
                    }
                    _ => {}
                }
            });
        if let Some(i) = add_community
            && let Some(d) = out.get_mut(i)
        {
            let first = self
                .lists
                .stalker_communities
                .entries
                .iter()
                .map(|e| e.id.clone())
                .find(|id| !d.by.iter().any(|(w, _)| w == id))
                .unwrap_or_else(|| "stalker".to_owned());
            d.by.push((first, d.chance.unwrap_or(0.01)));
            changed = true;
        }
        if let Some(&(i, by)) = result.removed.and_then(|row| lines.get(row)) {
            match by {
                // the cross of a community's row takes that community away
                Some(b) => {
                    if let Some(d) = out.get_mut(i)
                        && b < d.by.len()
                    {
                        d.by.remove(b);
                    }
                }
                None => {
                    if i < out.len() {
                        out.remove(i);
                    }
                }
            }
            changed = true;
        }
        if result.added {
            out.push(Drop { count: Some(Count::One(1.0)), chance: Some(0.01), ..Drop::default() });
            changed = true;
        }
        changed.then_some(out)
    }

    fn recipe_rows(&self, ui: &mut Ui, recipes: &[Recipe]) -> Option<Vec<Recipe>> {
        let columns = [
            Col::new("Makes", em(13.0)),
            Col::new("Count", em(5.5)),
            Col::new("Made At", em(11.0)),
            Col::new("Part", em(8.5)),
            Col::new("Part", em(8.5)),
            Col::new("Part", em(8.5)),
            Col::new("Part", em(8.5)),
        ];
        let mut out = recipes.to_vec();
        let mut changed = false;
        let result = ItemRows::new("economy.recipes", &columns, recipes.len())
            .add_label("Add Recipe")
            .empty_text("No recipe of the module yet.")
            .show(ui, |ui, row, col| {
                let Some(r) = out.get_mut(row) else { return };
                let salt = format!("economy.recipes.{row}.{col}");
                match col {
                    0 => changed |= self.item_cell(ui, &salt, &mut r.makes, "Pick an item..."),
                    1 => {
                        if let (Some(t), _) = local_value_text(
                            ui,
                            &salt,
                            r.count.map(|c| c.to_string()).as_deref(),
                            Tx::raw("1"),
                            ui.available_width(),
                        ) {
                            r.count = t.trim().parse::<u32>().ok();
                            changed = true;
                        }
                    }
                    2 => changed |= self.item_cell(ui, &salt, &mut r.at, "Pick a tool..."),
                    _ => {
                        let at = col - 3;
                        let mut v = r.parts.get(at).cloned().unwrap_or_default();
                        if self.item_cell(ui, &salt, &mut v, "None") {
                            while r.parts.len() <= at {
                                r.parts.push(String::new());
                            }
                            r.parts[at] = v;
                            r.parts.retain(|p| !p.is_empty());
                            changed = true;
                        }
                    }
                }
            });
        if let Some(i) = result.removed.filter(|i| *i < out.len()) {
            out.remove(i);
            changed = true;
        }
        if let Some((from, to)) = result.moved
            && from < out.len()
        {
            let r = out.remove(from);
            out.insert(to.min(out.len()), r);
            changed = true;
        }
        if result.added {
            out.push(Recipe::default());
            changed = true;
        }
        changed.then_some(out)
    }

    fn draw_body(&mut self, ui: &mut Ui) {
        let full = ui.max_rect();
        let gap = ui.spacing().item_spacing.x;
        let list_rect = Rect::from_min_max(full.min, pos2((full.min.x + LIST_W).min(full.max.x), full.max.y));
        let rows_rect =
            Rect::from_min_max(pos2((list_rect.max.x + gap).min(full.max.x), full.min.y), full.max);
        let mut left = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.economy.side")
                .max_rect(list_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        self.draw_add(&mut left);
        left.add_space(6.0);
        self.draw_list(&mut left);

        let mut right = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.economy.rows")
                .max_rect(rows_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        ScrollArea::both().id_salt("gz-ui-world.economy.scroll").auto_shrink([false, false]).show(
            &mut right,
            |ui| {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 0)).show(ui, |ui| {
                    let column = ui.available_width().clamp(COLUMN_MIN, COLUMN_W);
                    ui.set_min_width(column);
                    ui.set_max_width(column);
                    if self.shown.is_none() {
                        let (title, text_) = match self.kind {
                            TableKind::Trade => (
                                "No trader yet",
                                "Add a trader of the game to put the module's items on its shelf. A character of the module keeps its shop in its own file.",
                            ),
                            TableKind::Loot => (
                                "No loot table yet",
                                "A loot table says what the dead carry: an item, how many, and how often.",
                            ),
                            TableKind::Recipes => (
                                "No recipes yet",
                                "A recipes file says what is made at a tool or a workbench, and from what.",
                            ),
                        };
                        let icon = match self.kind {
                            TableKind::Trade => Icon::Trader,
                            TableKind::Loot => Icon::Loot,
                            TableKind::Recipes => Icon::Recipe,
                        };
                        let _ = gz_ui::state::empty_state(ui, icon, title, text_, None);
                    } else {
                        self.draw_rows(ui);
                    }
                });
            },
        );
    }

    // ---- what the tab answers -------------------------------------------------------------------

    /// The `economy.view` answer: the page, its files, the one selected with its rows.
    #[must_use]
    pub fn view_json(&self, active: bool) -> Value {
        let files: Vec<Value> = self
            .files()
            .iter()
            .filter_map(|(i, file)| {
                let l = self.loaded.as_ref()?;
                let t = l.roster.tables.get(*i)?;
                let issues = l.problems.get(*i).cloned().unwrap_or_default();
                Some(json!({
                    "file": file,
                    "rows": t.table.rows.len(),
                    "errors": issues.iter().filter(|x| x.is_error()).count(),
                    "problems": issues.iter().filter(|x| x.severity != gz_base::Severity::Note).map(sentence).collect::<Vec<_>>(),
                }))
            })
            .collect();
        let rows = self.shown.as_ref().map(|t| match gz_items::tables::to_value(t) {
            gz_luadata::Value::Table(v) => gz_luadata::to_json(&gz_luadata::Value::Table(v)),
            _ => Value::Null,
        });
        json!({
            "tab": TAB,
            "open": true,
            "active": active,
            "page": PAGES[page_of(self.kind)].0.to_ascii_lowercase(),
            "kind": self.kind.name(),
            "files": files,
            "selected": self.sel,
            "shown": rows,
            "saving": self.saving || self.queued.is_some(),
            "loading": self.loading || self.loaded.is_none(),
            "status": if self.status.is_empty() { SAVED } else { self.status.as_str() },
        })
    }

    /// Puts the delete question up to be looked at.
    pub(crate) fn look_delete(&mut self) {
        self.look_delete = true;
    }

    /// Asks for a file to be selected once the roster has it.
    pub(crate) fn open_file(&mut self, file: &str) {
        self.pending_open = Some(file.replace('\\', "/"));
        self.settle();
    }
}

impl Document for EconomyDoc {
    fn id(&self) -> &str {
        TAB
    }
    fn kind(&self) -> &'static str {
        TAB
    }
    fn label(&self) -> Tx<'static> {
        Tx::from("Economy")
    }
    fn icon(&self) -> Icon {
        Icon::Trader
    }
    fn tooltip(&self) -> Option<Tx<'static>> {
        Some(Tx::from("Who sells an item, who carries it and what makes it - saved to the files as you edit"))
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, _cx: &mut DocCx<'_>) {
        tb.group(2);
        let mut page = page_of(self.kind);
        let labels: Vec<&str> = PAGES.iter().map(|(l, _)| *l).collect();
        if tb.segmented(
            "page",
            "",
            &mut page,
            &labels,
            "The module's traders, its loot tables or its recipes",
            true,
        ) {
            self.actions.push(Action::Page(page));
        }
        tb.group(0);
        tb.separator();
        if self.status.is_empty() {
            tb.status(SAVED, Tone::Muted);
        } else {
            tb.status(Tx::ui(self.status.as_str()), Tone::Error);
        }
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        self.take_inbox();
        if cx.editor.project_opt().is_none() {
            let _ = gz_ui::state::empty_state(
                ui,
                Icon::Trader,
                "No project",
                "Traders, loot tables and recipes are files of the open project.",
                None,
            );
            return;
        }
        self.sync();
        if let Some(lists) = Lists::of(cx.editor) {
            lists.want(cx.editor);
            self.lists = lists.view();
        }
        if let Some(lists) = ItemLists::of(cx.editor) {
            lists.want(cx.editor);
            self.item_lists = lists.view();
        }
        if self.pictures.is_none() {
            self.pictures = Pictures::of(cx.editor);
        }
        if self.icon_sheets.is_none() {
            self.icon_sheets = IconSheets::of(cx.editor);
        }
        self.sync_items();
        if self.loaded.is_none() {
            gz_ui::state::busy_line(ui, "Reading the module's files...");
            self.run_actions(&ctx);
            return;
        }
        self.draw_body(ui);
        self.run_actions(&ctx);
    }

    fn on_activate(&mut self, _cx: &mut DocCx<'_>) {
        // the files as they are now: a write over MCP may have come after the tab last looked
        self.reload_fresh();
    }

    fn mcp_state(&self) -> Value {
        self.view_json(false)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The `economy.view` answer of a tab that is not open.
#[must_use]
pub fn closed_json() -> Value {
    json!({ "tab": TAB, "open": false, "active": false, "page": "traders", "files": [], "selected": Value::Null, "status": SAVED })
}

/// Opens the tab, on a file of the module's when one is named: what an item's Where It Is Found
/// row does.
pub(crate) fn open(editor: &Arc<Editor>, file: &str) {
    let mut pairs = vec![("action", s("open")), ("kind", s(TAB))];
    if !file.is_empty() {
        pairs.push(("path", s(file)));
    }
    let mut a = gz_editor::Args::new();
    for (k, v) in pairs {
        a.insert(k.to_owned(), v);
    }
    call::call_then(editor, "ui.tab", a, move |reply, ctx| {
        if !call::ok(reply) {
            gz_ui::state::toast(ctx, Tone::Error, Tx::ui(call::error_text(reply)));
        }
    });
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.document(TAB, |editor, req| {
        if editor.project_opt().is_none() {
            return Err("no project is open".to_owned());
        }
        Ok(Box::new(EconomyDoc::new(editor, req.path.clone())))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_reads_as_a_sentence_about_its_row() {
        let unreadable = Issue::error(
            gz_items::roster::UNREADABLE,
            Vec::new(),
            "trade/a.tradeasset: line 3: '=' expected",
        );
        assert_eq!(sentence(&unreadable), "Line 3: '=' expected.");
        let at_row = Issue::warning(
            "I417",
            vec![gz_luadata::Seg::key("stock"), gz_luadata::Seg::Index(2), gz_luadata::Seg::key("item")],
            "'x' is no section of the game or of the module: there is no such item",
        );
        assert_eq!(row_of(&at_row), Some(2));
        assert_eq!(row_of(&unreadable), None);
        assert_eq!(
            sentence(&at_row),
            "'x' is no section of the game or of the module: there is no such item."
        );
    }

    #[test]
    fn the_pages_are_the_three_kinds() {
        for (i, (_, kind)) in PAGES.iter().enumerate() {
            assert_eq!(page_of(*kind), i);
        }
        assert_eq!(PAGES.len(), TableKind::ALL.len());
        assert_eq!(FOLDERS.len(), PAGES.len());
    }
}
