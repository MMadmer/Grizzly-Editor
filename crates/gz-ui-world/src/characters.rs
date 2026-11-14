//! Characters: the `characters` singleton tab (docs/UI_DESIGN.md, section 6, "Characters").
//!
//! Three columns: a list of the project's characters (their faces, as their files say, sorted by
//! id), the form of the one selected, and - where there is room - the character as the game will
//! show it: its model in the model viewer, its portrait, its name and community. The toolbar holds
//! Save (the one Primary while the form has edits), Revert and Delete... The tab keeps VIEW state and the character being edited: the roster comes from
//! `gz_characters::Characters` on the pools; a save is `Characters::save` - the difference
//! between the character as the tab read it and as it is now, written into the file's own text
//! (a new character's file is written canonically); a deletion is `file.delete`. The form binds
//! to what is STORED (`text_raw`), never to the resolved character: the game falls back, the
//! form must not. The problems of the edited character are worked out on the pools when it
//! changes, resolved against the others the way the build sees them.

use std::sync::Arc;

use egui::{Align, CornerRadius, Layout, Rect, ScrollArea, Ui, UiBuilder, Vec2, pos2};
use gz_app::{DocCx, Document, ShellBuilder};
use gz_base::{RelPath, Rev};
use gz_characters::resolve::resolve;
use gz_characters::{Character, Characters, Checker, Roster, text_collapse, text_in, text_raw, text_set};
use gz_editor::{Editor, Value, json};
use gz_luadata::Value as LuaValue;
use gz_luadata::{Conflict, Limits, LuaDoc, set_at};
use gz_mod::ManifestService;
use gz_project::Project;
use gz_ui::dialog::{Answer, ConfirmSpec, confirm, unsaved_prompt};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Kind, Tone, Toolbar, Tx, palette, tr};
use gz_ui_previews::model_load::ModelSource;
use parking_lot::Mutex;

use crate::call::{self, args, s};
use crate::lists::{Lists, ListsView};
use crate::portraits::{Sheets, draw_portrait};

/// The narrowest the preview column is drawn; under it the form keeps the room.
const PREVIEW_MIN: f32 = 300.0;

/// The tab's id and kind.
pub const TAB: &str = "characters";
/// The scroll `ui.scroll id=characters` moves: the work area of the tab that is up.
pub(crate) const HOST: &str = "###doc.host";

/// What replaces the edited character once the unsaved question is answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Then {
    Nothing,
    /// The character of this file.
    Select(String),
    New,
}

/// What tasks leave for the tab; taken at the start of its next frame.
#[derive(Default)]
struct Inbox {
    roster: Option<(Arc<Roster>, String)>,
    problems: Option<(u64, Vec<String>)>,
    saved: Option<(Result<String, String>, Then)>,
    deleted: Option<Result<(), String>>,
    answered: Option<(Answer, Then)>,
    delete_yes: bool,
}

/// What the drawing asked for, done once it is drawn.
pub(crate) enum Action {
    New,
    Select(String),
    Delete,
    Preview,
    BaseChanged,
}

/// A save as it is carried out.
#[derive(Clone, Debug)]
pub(crate) struct SavePlan {
    /// The file the character was read from, empty for a new one.
    old: String,
    /// The file it goes to: the old one, or `characters/<id>.npcasset` when the file follows the
    /// id the author settled on.
    target: String,
    rename: bool,
    /// The character as the tab read it from its file; none for a new one.
    before: Option<Character>,
    /// The character as it is now.
    after: Character,
}

/// Carries a save out: the rename first when the file follows the id, then the difference
/// written into the file. The file written, project-relative.
pub(crate) async fn run_save(ed: &Arc<Editor>, plan: SavePlan) -> Result<String, String> {
    let origin = ed.ui_origin();
    let mut target = plan.target;
    if plan.rename {
        let leaf = target.rsplit('/').next().unwrap_or(&target).to_owned();
        let reply = ed.call("file.rename", args([("path", s(&plan.old)), ("name", s(&leaf))]), origin).await;
        // a rename refused leaves the file where it is: the character goes there
        if !call::ok(&reply) {
            target = plan.old;
        }
    }
    let svc = Characters::of(ed).await.map_err(|e| e.message)?;
    svc.save(ed, &target, plan.before.as_ref(), &plan.after).await.map_err(|e| e.message)
}

/// The form's edits (`edit`, read as `base`) put onto the character the file holds now
/// (`disk`): every value the form changed, the file's values elsewhere; the values both changed
/// keep the form's and come back as conflicts. None when the values do not make a character.
pub(crate) fn merge_character(
    base: &Character,
    edit: &Character,
    disk: &Character,
) -> Option<(Character, Vec<Conflict>)> {
    let rules = gz_characters::model::diff_rules();
    let (from, mine) = (gz_characters::model::to_value(base), gz_characters::model::to_value(edit));
    let theirs = gz_characters::model::write(disk);
    let (doc, conflicts) =
        LuaDoc::rebase_with(&from, &mine, &theirs, &rules, &disk.file, Limits::asset()).ok()?;
    let mut merged = gz_characters::model::from_value(&doc.value()).ok()?;
    merged.file.clone_from(&edit.file);
    Some((merged, conflicts))
}

/// The problems of the edited character as the build will see it: resolved against the others
/// (its base, the ids they take), then validated. Blocking: run it on the IO pool (a model's
/// bones may be read).
pub(crate) fn own_problems(
    roster: &Roster,
    sel: Option<usize>,
    edit: &Character,
    checker: &Checker,
) -> Vec<String> {
    let mut all = roster.raw.clone();
    let at = match sel.filter(|i| *i < all.len()) {
        Some(i) => {
            if let Some(slot) = all.get_mut(i) {
                *slot = edit.clone();
            }
            i
        }
        None => {
            all.push(edit.clone());
            all.len() - 1
        }
    };
    let place = if edit.file.is_empty() { edit.id.clone() } else { edit.file.clone() };
    let mut chain = Vec::new();
    resolve(&mut all, &mut chain);
    let needle = format!(" {place}: ");
    let mut own: Vec<String> = chain.into_iter().filter(|p| p.contains(&needle)).collect();
    for (i, c) in all.iter().enumerate() {
        if i != at && c.id == edit.id {
            own.push(format!("E101 {place}: another character has the id '{}' - {}", edit.id, c.file));
        }
    }
    if let Some(c) = all.get(at) {
        checker.validate(c, &mut own);
    }
    own
}

/// The problems of the edited character, worked out on the pools.
pub(crate) async fn problems_of(
    ed: &Arc<Editor>,
    roster: Arc<Roster>,
    sel: Option<usize>,
    edit: Character,
) -> Vec<String> {
    let checker = match Characters::of(ed).await {
        Ok(svc) => svc.checker(ed).await,
        Err(_) => Checker::unlinked(),
    };
    ed.exec.io(move || own_problems(&roster, sel, &edit, &checker)).await
}

/// The row a coded problem belongs under; empty: the header's.
#[must_use]
pub fn field_of(problem: &str) -> &'static str {
    const MAP: [(&str, &str); 21] = [
        ("E101", "id"),
        ("E102", "id"),
        ("E111", "base"),
        ("E112", "base"),
        ("E103", "name"),
        ("W107", "text"),
        ("E104", "visual"),
        ("W105", "visual"),
        ("E123", "visual"),
        ("W115", "icon"),
        ("W114", "community"),
        ("W116", "dialog"),
        ("E110", "money"),
        ("W117", "kit"),
        ("E108", "kit"),
        ("E109", "kit"),
        ("E118", "trade"),
        ("E119", "trade"),
        ("W120", "trade"),
        ("W121", "trade"),
        ("W122", "trade"),
    ];
    MAP.iter().find(|(code, _)| problem.starts_with(code)).map_or("", |(_, f)| *f)
}

/// "E103 characters/x.npcasset: no name - ..." as the sentence the author reads: "No name -
/// ...". The code and the file are for MCP and the log.
#[must_use]
pub fn problem_text(problem: &str) -> String {
    let mut text_ = match problem.find(": ") {
        Some(at) => problem.get(at + 2..).unwrap_or("").to_owned(),
        None => problem.get(problem.len().min(5)..).unwrap_or("").to_owned(),
    };
    if let Some(first) = text_.chars().next()
        && first.is_ascii_lowercase()
    {
        text_.replace_range(..1, &first.to_ascii_uppercase().to_string());
    }
    if !text_.is_empty() && !text_.ends_with(['.', '!', '?']) {
        text_.push('.');
    }
    text_
}

/// The tone of a coded problem.
#[must_use]
pub fn problem_tone(problem: &str) -> Tone {
    if problem.starts_with('E') { Tone::Error } else { Tone::Warning }
}

/// The Characters tab.
pub struct CharactersDoc {
    pub(crate) editor: Arc<Editor>,
    root: String,
    pub(crate) roster: Option<Arc<Roster>>,
    /// The revisions of the file tree and the manifest the roster was last asked for.
    roster_key: Option<Rev>,
    loading: bool,
    /// The next read waits for the file tree to hold every change (after a write).
    fresh_wanted: bool,
    pub(crate) module_id: String,
    pub(crate) sel: Option<usize>,
    pub(crate) edit: Character,
    /// The character as the form read it from its file: what a save takes the difference from,
    /// and what an outside change is merged against. None for a new one.
    base: Option<Character>,
    /// The values both the form and the file on disk changed since the form read it: the
    /// form keeps its own, Take Theirs takes the file's.
    pub(crate) conflicts: Vec<Conflict>,
    pub(crate) dirty: bool,
    pub(crate) name_rus: String,
    pub(crate) name_eng: String,
    pub(crate) bio_rus: String,
    pub(crate) bio_eng: String,
    /// The base resolved: what this character starts from.
    pub(crate) inherited: Option<Character>,
    pub(crate) own: Vec<String>,
    edit_gen: u64,
    own_gen: u64,
    asked_gen: u64,
    lists_gen: u64,
    pub(crate) status: String,
    pub(crate) list_filter: String,
    /// The rows the list filter shows: (filter, roster stamp, rows).
    pub(crate) shown: (String, Rev, Vec<usize>),
    inbox: Arc<Mutex<Inbox>>,
    /// A file asked for (the content browser, MCP): selected once the roster holds it.
    pending_open: Option<String>,
    saving: bool,
    /// What stopped the last save: the answer of a command that asked for it.
    save_error: Option<String>,
    pub(crate) lists: ListsView,
    pub(crate) sheets: Option<Arc<Sheets>>,
    pub(crate) actions: Vec<Action>,
    /// How far the form scrolled at most, the last frame: where `ui.scroll to=bottom` goes.
    scroll_max: f32,
    /// The model viewer of the preview, by `source|visual`.
    model: Option<(String, Box<dyn Document>)>,
}

impl CharactersDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>, open: Option<String>) -> Self {
        Self {
            editor: editor.clone(),
            root: String::new(),
            roster: None,
            roster_key: None,
            loading: false,
            fresh_wanted: true,
            module_id: String::new(),
            sel: None,
            edit: Character::default(),
            base: None,
            conflicts: Vec::new(),
            dirty: false,
            name_rus: String::new(),
            name_eng: String::new(),
            bio_rus: String::new(),
            bio_eng: String::new(),
            inherited: None,
            own: Vec::new(),
            edit_gen: 1,
            own_gen: 0,
            asked_gen: 0,
            lists_gen: 0,
            status: String::new(),
            list_filter: String::new(),
            shown: (String::new(), Rev::ZERO, Vec::new()),
            inbox: Arc::new(Mutex::new(Inbox::default())),
            pending_open: open,
            saving: false,
            save_error: None,
            lists: ListsView::default(),
            sheets: None,
            actions: Vec::new(),
            scroll_max: 0.0,
            model: None,
        }
    }

    // ---- the roster -----------------------------------------------------------------------------

    /// Follows the project's files: a frame only compares revisions; a change reads the roster
    /// again on the pools.
    fn sync(&mut self) {
        let Ok((session, project)) = Project::of(&self.editor) else { return };
        let root = session.root.to_string_lossy().into_owned();
        if root != self.root {
            self.root = root;
            self.roster = None;
            self.roster_key = None;
            self.sel = None;
            self.edit = Character::default();
            self.dirty = false;
            self.pull();
        }
        let manifest = ManifestService::get(&session).map_or(Rev::ZERO, |m| m.rev());
        let key = project.tree.rev().mix(manifest);
        if self.loading || self.roster_key == Some(key) {
            return;
        }
        self.roster_key = Some(key);
        self.load_roster();
    }

    /// Reads the roster on the pools; `fresh` waits for the file tree to hold every change first.
    fn load_roster(&mut self) {
        self.loading = true;
        let fresh = std::mem::take(&mut self.fresh_wanted);
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            if let Ok(svc) = Characters::of(&ed).await {
                let roster = if fresh { svc.fresh_roster().await } else { svc.roster().await };
                let module = svc.manifest().snapshot().manifest().map(|m| m.id.clone()).unwrap_or_default();
                inbox.lock().roster = Some((roster, module));
            } else {
                inbox.lock().roster = Some((Arc::new(Roster::default()), String::new()));
            }
            call::wake_ui(&ed);
        });
    }

    /// Reads the roster again once the file tree holds what was just written.
    fn reload_fresh(&mut self) {
        self.fresh_wanted = true;
        self.roster_key = None;
    }

    /// A roster arrived: the list as the disk has it, edits in progress kept - the edited
    /// character is found again by its file; a file asked for is selected.
    pub(crate) fn take_roster(&mut self, roster: Arc<Roster>, module: String) {
        let file = self.edit.file.clone();
        self.module_id = module;
        self.roster = Some(roster.clone());
        let found = if file.is_empty() {
            None
        } else {
            roster.raw.iter().position(|c| c.file.eq_ignore_ascii_case(&file))
        };
        if self.dirty {
            self.sel = found;
            if let Some(disk) = found.and_then(|i| roster.raw.get(i)) {
                self.rebase(disk);
            }
            self.recompute_inherited();
        } else {
            self.select(found.or(if roster.raw.is_empty() { None } else { Some(0) }));
        }
        self.edit_gen += 1;
        if let Some(path) = self.pending_open.take() {
            let index = self.find(None, Some(&path));
            if let Some(i) = index
                && Some(i) != self.sel
                && let Some(file) = roster.raw.get(i).map(|c| c.file.clone())
            {
                self.ask_then(Then::Select(file));
            }
        }
    }

    /// The character of an id or a project path, any slash, any case.
    #[must_use]
    pub(crate) fn find(&self, id: Option<&str>, path: Option<&str>) -> Option<usize> {
        let roster = self.roster.as_ref()?;
        let norm = |p: &str| p.replace('\\', "/").to_lowercase();
        roster
            .raw
            .iter()
            .position(|c| id.is_some_and(|i| c.id == i) || path.is_some_and(|p| norm(&c.file) == norm(p)))
    }

    /// Selects a character of the roster (`None`: nothing), its edits gone.
    pub(crate) fn select(&mut self, index: Option<usize>) {
        let roster = self.roster.clone();
        let picked = index.and_then(|i| roster.as_ref().and_then(|r| r.raw.get(i)).map(|c| (i, c.clone())));
        match picked {
            Some((i, c)) => {
                self.sel = Some(i);
                self.base = Some(c.clone());
                self.edit = c;
            }
            None => {
                self.sel = None;
                self.base = None;
                self.edit = Character::default();
            }
        }
        self.conflicts.clear();
        self.dirty = false;
        self.pull();
        self.recompute_inherited();
        self.edit_gen += 1;
    }

    /// The file of the edited character changed on disk while the form holds edits: the form's
    /// changes go onto the file's new values, so what the file gained shows and stays; a value
    /// both changed keeps the form's and is listed in `conflicts`.
    fn rebase(&mut self, disk: &Character) {
        let Some(base) = self.base.clone() else { return };
        if *disk == base {
            return;
        }
        if let Some((c, conflicts)) = merge_character(&base, &self.edit, disk) {
            self.edit = c;
            self.conflicts = conflicts;
            self.pull();
        }
        self.base = Some(disk.clone());
        self.dirty = self.base.as_ref() != Some(&self.edit);
    }

    /// The form takes the file's value of one conflict (Take Theirs).
    pub(crate) fn take_theirs(&mut self, index: usize) {
        if index >= self.conflicts.len() {
            return;
        }
        let c = self.conflicts.remove(index);
        let mut v = gz_characters::model::to_value(&self.edit);
        set_at(&mut v, &c.path, c.theirs.unwrap_or(LuaValue::Nil));
        if let Ok(mut taken) = gz_characters::model::from_value(&v) {
            taken.file.clone_from(&self.edit.file);
            self.edit = taken;
            self.pull();
            self.recompute_inherited();
        }
        self.dirty = self.base.as_ref() != Some(&self.edit);
        self.edit_gen += 1;
    }

    /// The text fields take what is stored.
    fn pull(&mut self) {
        self.name_rus = text_raw(&self.edit.name, "rus");
        self.name_eng = text_raw(&self.edit.name, "eng");
        self.bio_rus = text_raw(&self.edit.bio, "rus");
        self.bio_eng = text_raw(&self.edit.bio, "eng");
    }

    /// The text fields go back into the character, every language kept, a `rus` alone a plain
    /// string again.
    pub(crate) fn push_texts(&mut self) {
        text_set(&mut self.edit.name, "rus", &self.name_rus);
        text_set(&mut self.edit.name, "eng", &self.name_eng);
        text_set(&mut self.edit.bio, "rus", &self.bio_rus);
        text_set(&mut self.edit.bio, "eng", &self.bio_eng);
        text_collapse(&mut self.edit.name);
        text_collapse(&mut self.edit.bio);
    }

    /// What the base chain gives this character: the base resolved.
    pub(crate) fn recompute_inherited(&mut self) {
        self.inherited = if self.edit.base.is_empty() {
            None
        } else {
            self.roster.as_ref().and_then(|r| r.get(&self.edit.base)).cloned()
        };
    }

    /// The form changed the character.
    pub(crate) fn changed(&mut self) {
        self.push_texts();
        if self.edit.money_max < self.edit.money_min {
            self.edit.money_max = self.edit.money_min;
        }
        self.dirty = true;
        self.edit_gen += 1;
    }

    /// The problems are worked out again when the character or the lists changed.
    fn revalidate(&mut self) {
        if self.lists.serial != self.lists_gen {
            self.lists_gen = self.lists.serial;
            self.edit_gen += 1;
        }
        if self.sel.is_none() && !self.dirty {
            self.own.clear();
            self.own_gen = self.edit_gen;
            return;
        }
        if self.own_gen == self.edit_gen || self.asked_gen == self.edit_gen {
            return;
        }
        let Some(roster) = self.roster.clone() else { return };
        self.asked_gen = self.edit_gen;
        let (ed, inbox, serial, sel, edit) =
            (self.editor.clone(), self.inbox.clone(), self.edit_gen, self.sel, self.edit.clone());
        let _task = self.editor.exec.spawn(async move {
            let problems = problems_of(&ed, roster, sel, edit).await;
            inbox.lock().problems = Some((serial, problems));
            call::wake_ui(&ed);
        });
    }

    /// Problems worked out elsewhere (MCP) for the character as it is now.
    pub(crate) fn set_problems(&mut self, problems: Vec<String>) {
        self.own = problems;
        self.own_gen = self.edit_gen;
    }

    /// What `characters.view` validates: the roster, the selection, the character.
    pub(crate) fn validation_input(&self) -> Option<(Arc<Roster>, Option<usize>, Character)> {
        Some((self.roster.clone()?, self.sel, self.edit.clone()))
    }

    // ---- the inbox ------------------------------------------------------------------------------

    fn take_inbox(&mut self, ctx: Option<&egui::Context>) {
        let mut inbox = self.inbox.lock();
        let roster = inbox.roster.take();
        let problems = inbox.problems.take();
        let saved = inbox.saved.take();
        let deleted = inbox.deleted.take();
        let answered = inbox.answered.take();
        let delete_yes = std::mem::take(&mut inbox.delete_yes);
        drop(inbox);
        if let Some((roster, module)) = roster {
            self.loading = false;
            self.take_roster(roster, module);
        }
        if let Some((serial, problems)) = problems
            && serial == self.edit_gen
        {
            self.own = problems;
            self.own_gen = serial;
        }
        if let Some((result, then)) = saved {
            self.saved(result, then, ctx);
        }
        if let Some(result) = deleted {
            match result {
                Ok(()) => {
                    self.status.clear();
                    self.dirty = false;
                    self.sel = None;
                    self.edit = Character::default();
                    self.pull();
                    self.inherited = None;
                    self.edit_gen += 1;
                }
                Err(e) => self.status = format!("Not deleted: {e}"),
            }
            self.reload_fresh();
        }
        if let Some((answer, then)) = answered {
            match answer {
                Answer::Cancel => {}
                Answer::Yes => {
                    if let Err(e) = self.start_save(then) {
                        self.status = format!("Not saved: {e}");
                    }
                }
                Answer::No => {
                    self.discard_edits();
                    self.run_then(then);
                }
            }
        }
        if delete_yes {
            self.delete_now();
        }
    }

    // ---- saving ---------------------------------------------------------------------------------

    /// How the edited character is saved: a new one goes to
    /// `characters/<id>.npcasset`, and a file of `characters/` follows the id the author settles
    /// on while nothing has that name yet.
    pub(crate) fn save_plan(&mut self) -> Result<SavePlan, String> {
        if self.edit.id.is_empty() {
            return Err("the character needs an id".into());
        }
        self.push_texts();
        let old = self.edit.file.clone();
        let wanted = format!("characters/{}.npcasset", self.edit.id);
        let taken = |rel: &str| {
            Project::of(&self.editor).ok().is_some_and(|(_, p)| {
                RelPath::new(rel).ok().is_some_and(|r| p.tree.snapshot().entry(&r).is_some())
            })
        };
        let rename = !old.is_empty()
            && old.to_ascii_lowercase().starts_with("characters/")
            && !old.eq_ignore_ascii_case(&wanted)
            && !taken(&wanted);
        let target = if old.is_empty() || rename { wanted } else { old.clone() };
        Ok(SavePlan { old, target, rename, before: self.base.clone(), after: self.edit.clone() })
    }

    /// Starts a save on the pools; `then` runs once it is written.
    fn start_save(&mut self, then: Then) -> Result<(), String> {
        if self.saving {
            return Ok(());
        }
        let plan = self.save_plan()?;
        self.saving = true;
        self.save_error = None;
        // the shell may close the tab right after it asked to save
        self.dirty = false;
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            let result = run_save(&ed, plan).await;
            inbox.lock().saved = Some((result, then));
            call::wake_ui(&ed);
        });
        Ok(())
    }

    /// A save answered: the character keeps its file, the list is read again.
    pub(crate) fn saved(&mut self, result: Result<String, String>, then: Then, _ctx: Option<&egui::Context>) {
        self.saving = false;
        match result {
            Ok(file) => {
                self.status.clear();
                self.dirty = false;
                self.edit.file = file;
                self.base = Some(self.edit.clone());
                self.conflicts.clear();
                self.reload_fresh();
                self.run_then(then);
            }
            Err(e) => {
                self.dirty = true;
                self.status = format!("Not saved: {e}");
                self.save_error = Some(e);
            }
        }
    }

    /// The unsaved edits go: the character as its file says.
    pub(crate) fn discard_edits(&mut self) {
        let sel = self.sel;
        self.select(sel);
        self.status.clear();
    }

    fn run_then(&mut self, then: Then) {
        match then {
            Then::Nothing => {}
            Then::New => self.new_character(),
            Then::Select(file) => {
                let index = self.find(None, Some(&file));
                self.select(index);
            }
        }
    }

    /// Unsaved edits ask before anything replaces them: Save, Don't Save, or stay.
    pub(crate) fn ask_then(&mut self, then: Then) {
        if !self.dirty {
            self.run_then(then);
            return;
        }
        let what: Vec<Tx<'static>> = vec![if self.edit.id.is_empty() {
            Tx::from("the new character")
        } else {
            Tx::raw(self.edit.id.clone())
        }];
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let Ok(queue) = gz_app::UiQueue::of(&self.editor) else { return };
        // the kit question needs the context: it is asked at the start of the next frame
        queue.post(move |_, ctx| {
            let inbox2 = inbox.clone();
            let ed2 = ed.clone();
            let line = unsaved_prompt(ctx, call::asker(&ed), "character", &what, move |answer| {
                inbox2.lock().answered = Some((answer, then));
                call::wake_ui(&ed2);
            });
            call::log_line(&ed, line);
            ctx.request_repaint();
        });
    }

    /// A new character, unsaved, heading the list.
    pub(crate) fn new_character(&mut self) {
        let taken = |id: &str| self.roster.as_ref().is_some_and(|r| r.raw.iter().any(|c| c.id == id));
        let mut id = "new_character".to_owned();
        for n in 2..1000 {
            if !taken(&id) {
                break;
            }
            id = format!("new_character_{n}");
        }
        self.edit = Character {
            id,
            community: "stalker".into(),
            name: LuaValue::from("New character"),
            visual: "actors\\stalker_neutral\\stalker_neutral_1".into(),
            icon: "ui_inGame2_neutral_2".into(),
            has_rank: true,
            rank: 300,
            ..Character::default()
        };
        self.sel = None;
        self.dirty = true;
        self.pull();
        self.recompute_inherited();
        self.edit_gen += 1;
    }

    /// Names what disappears and asks: the file goes for good.
    fn ask_delete(&mut self, ctx: &egui::Context) {
        if self.edit.file.is_empty() {
            return;
        }
        let spec = ConfirmSpec {
            id: "delete_character".to_owned(),
            title: "Delete Character".into(),
            headline: tr!("Delete the character \"{id}\"?", id = self.edit.id.as_str()),
            items: vec![Tx::raw(self.edit.file.clone())],
            total: 0,
            consequence: "Deleted for good - this cannot be undone. The next build takes the character out of the module, and a quest that spawns it has nothing to spawn."
                .into(),
            action: "Delete".into(),
            destructive: true,
        };
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let line = confirm(ctx, call::asker(&self.editor), spec, move |answer| {
            if answer == Answer::Yes {
                inbox.lock().delete_yes = true;
                call::wake_ui(&ed);
            }
        });
        call::log_line(&self.editor, line);
    }

    fn delete_now(&mut self) {
        let file = self.edit.file.clone();
        if file.is_empty() {
            return;
        }
        let inbox = self.inbox.clone();
        call::call_then(&self.editor, "file.delete", args([("path", s(&file))]), move |reply, _| {
            inbox.lock().deleted = Some(if call::ok(reply) { Ok(()) } else { Err(call::error_text(reply)) });
        });
    }

    /// Where the model viewer finds a visual of the form: the project's own, else the game's.
    fn model_source(&self, visual: &str) -> ModelSource {
        if self.lists.visuals.get(&visual.to_lowercase()).is_some_and(|e| e.extra == "project") {
            ModelSource::Project
        } else {
            ModelSource::Game
        }
    }

    /// Opens the model viewer on the model of the form: the project's own, else the game's.
    fn preview(&self) {
        let visual = self.edit.visual.clone();
        if visual.is_empty() {
            return;
        }
        let source = self.model_source(&visual);
        let ed = self.editor.clone();
        call::call_then(
            &self.editor,
            "preview.model",
            args([("name", s(&visual)), ("source", s(source.arg()))]),
            move |reply, ctx| {
                if !call::ok(reply) {
                    gz_ui::state::toast(ctx, Tone::Error, Tx::ui(call::error_text(reply)));
                    ed.log.warn(format!("the model preview did not open: {}", call::error_text(reply)));
                }
            },
        );
    }

    fn run_actions(&mut self, ctx: &egui::Context) {
        for action in std::mem::take(&mut self.actions) {
            match action {
                Action::New => self.ask_then(Then::New),
                Action::Select(file) => self.ask_then(Then::Select(file)),
                Action::Delete => self.ask_delete(ctx),
                Action::Preview => self.preview(),
                Action::BaseChanged => self.recompute_inherited(),
            }
        }
    }

    // ---- MCP ------------------------------------------------------------------------------------

    /// The `characters.view` answer of the open tab.
    #[must_use]
    pub fn view_json(&self, active: bool) -> Value {
        let selected = if self.sel.is_some() || self.dirty { self.edit.id.clone() } else { String::new() };
        json!({
            "tab": TAB,
            "open": true,
            "active": active,
            "selected": selected,
            "file": self.edit.file.replace('\\', "/"),
            "dirty": self.dirty,
            "count": self.roster.as_ref().map_or(0, |r| r.raw.len()),
            "problems": self.own,
            "conflicts": self.conflicts.iter().map(crate::char_form::conflict_line).collect::<Vec<_>>(),
            // the model viewer beside the form: its source word and what it shows (`error` is empty
            // when the model is shown); null until a frame has made it
            "model": self.model.as_ref().map(|(key, d)| {
                let source = key.split('|').next().unwrap_or_default();
                json!({ "source": source, "viewer": d.mcp_state() })
            }),
        })
    }

    /// The edits in progress, as the unsaved answer names them.
    #[must_use]
    pub(crate) fn dirty_names(&self) -> Vec<String> {
        vec![self.edit.id.clone()]
    }

    #[must_use]
    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    // ---- drawing --------------------------------------------------------------------------------

    /// The character as the game will show it: the model in the model viewer, the portrait at a
    /// readable size, the name and the community the player reads.
    fn draw_preview(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let p = palette(ui.ctx());
        if self.sel.is_none() && !self.dirty {
            return;
        }
        let c = self.resolved_view();
        let visual = c.visual.clone();
        let source = self.model_source(&visual);
        let want = format!("{}|{visual}", source.arg());
        if self.model.as_ref().is_none_or(|(k, _)| *k != want) {
            self.model = (!visual.is_empty())
                .then(|| {
                    let req = gz_ui_previews::model::open_request(&visual, source);
                    let doc = gz_ui_previews::model::factory(&self.editor, &req).ok().map(|mut d| {
                        if let Some(m) =
                            d.as_any_mut().and_then(|a| a.downcast_mut::<gz_ui_previews::model::ModelDoc>())
                        {
                            m.show_details = false;
                        }
                        d
                    });
                    (want.clone(), doc)
                })
                .and_then(|(k, d)| d.map(|d| (k, d)));
        }
        let full = ui.max_rect();
        let model_h = (full.height() * 0.6).clamp(180.0, 520.0);
        let model_rect = Rect::from_min_size(full.min, Vec2::new(full.width(), model_h));
        match self.model.as_mut() {
            Some((_, doc)) => {
                let mut child = ui
                    .new_child(UiBuilder::new().id_salt("gz-ui-world.characters.model").max_rect(model_rect));
                doc.body(&mut child, cx);
            }
            None => {
                ui.painter().rect_filled(model_rect, CornerRadius::same(4), p.canvas);
                let mut child = ui
                    .new_child(UiBuilder::new().max_rect(model_rect).layout(Layout::top_down(Align::Center)));
                let _ = gz_ui::state::empty_state(
                    &mut child,
                    Icon::FileModel,
                    "No model",
                    "Pick one under Appearance.",
                    None,
                );
            }
        }
        let below = Rect::from_min_max(pos2(full.min.x, model_rect.max.y + 12.0), full.max);
        let mut info = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.characters.seen")
                .max_rect(below)
                .layout(Layout::top_down(Align::Min)),
        );
        let _ = text::label(&mut info, "In the Game", Font::Strong, p.text, Clip::End);
        info.add_space(6.0);
        info.horizontal(|ui| {
            let _ = draw_portrait(ui, &self.editor, self.sheets.as_ref(), &self.lists, &c.icon, 96.0);
            ui.add_space(12.0);
            ui.vertical(|ui| {
                let name = text_in(&c.name, "rus");
                let shown = if name.is_empty() { Tx::from("No name") } else { Tx::raw(name) };
                let _ = text::label(ui, shown, Font::Title, p.text, Clip::End);
                let community =
                    if c.community.is_empty() { "stalker".to_owned() } else { c.community.clone() };
                let caption = self
                    .lists
                    .communities
                    .get(&community)
                    .map(|e| e.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or(community);
                let _ = text::label(ui, Tx::raw(caption), Font::Body, p.text_secondary, Clip::End);
                let eng = text_in(&c.name, "eng");
                if !eng.is_empty() {
                    let _ = text::label(ui, Tx::raw(eng), Font::Small, p.text_muted, Clip::End);
                }
            });
        });
    }

    /// The character as the form shows it, its base's values where it sets none.
    fn resolved_view(&self) -> gz_characters::Character {
        let mut c = self.edit.clone();
        if let Some(b) = &self.inherited {
            for (mine, theirs) in
                [(&mut c.visual, &b.visual), (&mut c.icon, &b.icon), (&mut c.community, &b.community)]
            {
                if mine.is_empty() {
                    mine.clone_from(theirs);
                }
            }
            if text_in(&c.name, "rus").is_empty() {
                c.name = b.name.clone();
            }
        }
        c
    }

    fn draw_body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let mut head = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.characters.split")
                .max_rect(full)
                .layout(Layout::left_to_right(Align::Min)),
        );
        let split = gz_ui::splitter::splitter(&mut head, "characters.list", true, 220.0, 400.0, 260.0);
        let gap = head.spacing().item_spacing.x;
        let list_rect =
            Rect::from_min_max(full.min, pos2((full.min.x + split.size).min(full.max.x), full.max.y));
        let mut form_rect =
            Rect::from_min_max(pos2((list_rect.max.x + gap).min(full.max.x), full.min.y), full.max);
        // the preview takes the room the form leaves, while there is enough of it
        let form_w = gz_ui::metrics::FORM_W + 24.0;
        if form_rect.width() >= form_w + PREVIEW_MIN {
            let preview = Rect::from_min_max(
                pos2(form_rect.min.x + form_w + gap, full.min.y + 8.0),
                pos2(full.max.x - 12.0, full.max.y - 8.0),
            );
            form_rect.max.x = form_rect.min.x + form_w;
            let mut side = ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-ui-world.characters.preview")
                    .max_rect(preview)
                    .layout(Layout::top_down(Align::Min)),
            );
            self.draw_preview(&mut side, cx);
        }

        let mut left = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.characters.side")
                .max_rect(list_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        self.draw_list(&mut left);

        let mut right = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.characters.form")
                .max_rect(form_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        let mut scroll =
            ScrollArea::vertical().id_salt("gz-ui-world.characters.scroll").auto_shrink([false, false]);
        match gz_app::take_scroll(&ctx, HOST) {
            Some(gz_app::ScrollTo::Top) => scroll = scroll.vertical_scroll_offset(0.0),
            Some(gz_app::ScrollTo::Bottom) => scroll = scroll.vertical_scroll_offset(self.scroll_max),
            Some(gz_app::ScrollTo::Offset(y)) => scroll = scroll.vertical_scroll_offset(y),
            _ => {}
        }
        let out = scroll.show(&mut right, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 0)).show(ui, |ui| {
                let column = ui.available_width().clamp(1.0, gz_ui::metrics::FORM_W);
                ui.set_max_width(column);
                ui.set_min_width(column);
                if self.sel.is_none() && !self.dirty {
                    if gz_ui::state::empty_state(
                        ui,
                        Icon::Character,
                        "No character selected",
                        "Pick one in the list, or add one with +.",
                        Some("New Character".into()),
                    ) {
                        self.actions.push(Action::New);
                    }
                } else {
                    self.draw_form(ui, column);
                }
            });
        });
        let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
        self.scroll_max = max;
        gz_app::report_scroll(&ctx, HOST, out.state.clamped_offset().y.min(max), max);
        ui.allocate_rect(full, egui::Sense::hover());
    }
}

impl Document for CharactersDoc {
    fn id(&self) -> &str {
        TAB
    }
    fn kind(&self) -> &'static str {
        TAB
    }
    fn label(&self) -> Tx<'static> {
        Tx::from("Characters")
    }
    fn icon(&self) -> Icon {
        Icon::Character
    }
    fn tooltip(&self) -> Option<Tx<'static>> {
        Some(Tx::from("The module's own NPCs"))
    }
    fn dirty(&self) -> bool {
        // a save under way is not saved yet: who waits for it waits for its file
        self.dirty || self.saving
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        let project = cx.editor.project_opt().is_some();
        let some = self.sel.is_some() && !self.edit.file.is_empty();
        tb.group(3);
        let kind = if self.dirty { Kind::Primary } else { Kind::Ghost };
        if tb.action(
            "save",
            Icon::Save,
            "Save",
            "Write the character into its file (Ctrl+S)",
            self.dirty,
            false,
            kind,
        ) && let Err(e) = self.start_save(Then::Nothing)
        {
            self.status = e;
        }
        if tb.action(
            "revert",
            Icon::Undo,
            "Revert",
            "Drop the edits: the character as its file says",
            self.dirty,
            false,
            Kind::Ghost,
        ) {
            self.discard_edits();
        }
        if tb.action(
            "delete",
            Icon::Delete,
            "Delete...",
            "Delete the character's file for good",
            some,
            false,
            Kind::GhostDanger,
        ) {
            self.actions.push(Action::Delete);
        }
        tb.group(1);
        if tb.action(
            "reload",
            Icon::Refresh,
            "Reload",
            "Read the characters, the linked game's lists and the project's models again",
            project,
            false,
            Kind::Ghost,
        ) {
            self.reload_fresh();
            if let Some(lists) = Lists::of(cx.editor) {
                lists.force(cx.editor);
            }
        }
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        self.take_inbox(Some(&ctx));
        if cx.editor.project_opt().is_none() {
            gz_ui::state::empty_state(
                ui,
                Icon::Character,
                "No project",
                "Characters are .npcasset files of the open project.",
                None,
            );
            return;
        }
        self.sync();
        if let Some(lists) = Lists::of(cx.editor) {
            lists.want(cx.editor);
            self.lists = lists.view();
        }
        if self.sheets.is_none() {
            self.sheets = Sheets::of(cx.editor);
        }
        self.revalidate();
        self.draw_body(ui, cx);
        self.run_actions(&ctx);
    }

    fn save(&mut self, _cx: &mut DocCx<'_>) -> Result<(), String> {
        self.start_save(Then::Nothing)
    }

    fn save_failed(&mut self) -> Option<gz_editor::CmdError> {
        // the save's answer is taken here too: the tab may not be the one drawn
        self.take_inbox(None);
        self.save_error.take().map(|e| gz_editor::CmdError::new("failed", e))
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
        self.discard_edits();
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

/// The `characters.view` answer of a tab that is not open.
#[must_use]
pub fn closed_json() -> Value {
    json!({ "tab": TAB, "open": false, "active": false, "selected": "", "file": "", "dirty": false, "count": 0, "problems": [] })
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.document(TAB, |editor, req| {
        if editor.project_opt().is_none() {
            return Err("no project is open".to_owned());
        }
        Ok(Box::new(CharactersDoc::new(editor, req.path.clone())))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_reads_as_a_sentence_under_its_field() {
        let p = "E103 characters/x.npcasset: no name - it is what the player reads";
        assert_eq!(field_of(p), "name");
        assert_eq!(problem_text(p), "No name - it is what the player reads.");
        assert_eq!(problem_tone(p), Tone::Error);
        assert_eq!(field_of("W120 a: x"), "trade");
        assert_eq!(field_of("E999 a: x"), "");
        assert_eq!(problem_text("W105 a: done!"), "Done!");
    }

    #[test]
    fn edits_in_the_form_merge_with_a_change_on_disk() {
        let base = Character {
            id: "trader".into(),
            file: "characters/trader.npcasset".into(),
            name: LuaValue::from("Sidorovich"),
            community: "stalker".into(),
            ..Character::default()
        };
        // the form renames him, the file on disk moves him to another community
        let edit = Character { name: LuaValue::from("Sid"), ..base.clone() };
        let disk = Character { community: "dolg".into(), ..base.clone() };
        let (merged, conflicts) = merge_character(&base, &edit, &disk).expect("merged");
        assert_eq!((merged.name.clone(), merged.community.as_str()), (LuaValue::from("Sid"), "dolg"));
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert_eq!(merged.file, base.file);
        // both change the name: the form keeps its own and the conflict names the file's
        let disk = Character { name: LuaValue::from("Old Sid"), ..base.clone() };
        let (merged, conflicts) = merge_character(&base, &edit, &disk).expect("merged");
        assert_eq!(merged.name, LuaValue::from("Sid"));
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].theirs, Some(LuaValue::from("Old Sid")));
    }

    #[test]
    fn the_edited_character_is_checked_against_the_others() {
        let mut roster = Roster::default();
        let a = Character { id: "a".into(), file: "characters/a.npcasset".into(), ..Character::default() };
        let b = Character {
            id: "b".into(),
            file: "characters/b.npcasset".into(),
            base: "nobody".into(),
            ..Character::default()
        };
        roster.raw = vec![a, b.clone()];
        let problems = own_problems(&roster, Some(1), &b, &Checker::unlinked());
        assert!(problems.iter().any(|p| p.starts_with("E111 characters/b.npcasset: ")), "{problems:?}");
        assert!(!problems.iter().any(|p| p.starts_with("E101")));
        let twin = Character { id: "a".into(), ..Character::default() };
        let problems = own_problems(&roster, None, &twin, &Checker::unlinked());
        assert!(
            problems.iter().any(|p| p.starts_with("E101 a: another character has the id 'a'")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_closed_tab_reports_nothing_selected() {
        let v = closed_json();
        assert_eq!(v["tab"], "characters");
        assert_eq!(v["open"], false);
        assert_eq!(v["selected"], "");
    }
}
