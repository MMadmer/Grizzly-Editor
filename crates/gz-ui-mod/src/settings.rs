//! Mod Settings: the `mod_settings` singleton tab (docs/UI_DESIGN.md, section 6, "Mod Settings").
//!
//! A nav of 200 pt - one row per page with its icon and the dot of its worst problem - and the
//! page: its title, what it is for, then one label-left grid of up to 44 em with each field's
//! problem as a sentence under it, on a card. The tab saves as it is edited: once no field is
//! being typed in and nothing blocks, the form's changes go out as one `mod.set` (the toolbar
//! row says so); a change of `mod.ltx` made elsewhere reaches every field the author has not
//! touched. Commands is one table of the console commands, the selected one's help under it.

use std::path::Path;
use std::sync::Arc;

use egui::{
    Align, ColorImage, Id, Layout, Popup, Rect, ScrollArea, TextureHandle, TextureOptions, Ui, UiBuilder,
    Vec2, pos2,
};
use gz_app::{DocCx, Document, OpenRequest, ShellBuilder};
use gz_editor::{Editor, Rev, Value, json};
use gz_game::{GameLink, ModeScan};
use gz_mod::{Command, Manifest, ManifestDoc, ManifestService, normalize_github_repo, validate_id};
use gz_project::Project;
use gz_release::browser::{TAG_VOCABULARY, tag_label};
use gz_ui::dialog::{Asker, ConfirmSpec, confirm};
use gz_ui::grid::{Grid, chips, combo, read_only, switch, text_block};
use gz_ui::list::list;
use gz_ui::pick::{PickItem, PickList as KitPickList, pick_popup};
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{
    Button, Col, Font, Icon, ItemRows, Kind, NavRowSpec, PropFlags, PropertyGrid, Tone, icon_button, metrics,
    nav_row, palette, segmented,
};
use parking_lot::Mutex;

use crate::call::{self, Pick, Picking, args, s};
use crate::form::{self, Aux, Form, Kind as TargetKind, PAGES, Problem};
use crate::local::{local_picture, local_value_column};

/// The tab's id and kind.
pub const TAB: &str = "mod_settings";

/// The scratch file a chosen picture waits in.
const PENDING_NAME: &str = "thumbnail_pending.dds";

/// The waiting picture takes the place of `thumbnail.dds` (the read-only mark
/// cleared; on another volume a copy).
fn place_thumbnail(pending: &Path, target: &Path) -> Result<(), String> {
    let cant = || format!("can't write {}", target.display());
    if let Ok(md) = std::fs::metadata(target) {
        let mut perms = md.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = std::fs::set_permissions(target, perms);
        }
    }
    if std::fs::rename(pending, target).is_ok() {
        return Ok(());
    }
    std::fs::copy(pending, target).map_err(|_| cant())?;
    let _ = std::fs::remove_file(pending);
    Ok(())
}

/// Each page's icon, in [`PAGES`]' order.
const PAGE_ICONS: [Icon; 7] =
    [Icon::Info, Icon::Globe, Icon::Mode, Icon::Relations, Icon::Exclude, Icon::PdaInterface, Icon::Terminal];

const ROLE: &str = "mod_settings";
const LABEL_FRAC: f32 = 0.28;
const PICTURE_SIDE: f32 = 96.0;
const NAV_W: f32 = 200.0;
/// The largest picture read for the preview.
const PICTURE_READ_LIMIT: u64 = 64 << 20;

/// The project's script functions and the console commands the linked game's modules take.
type CommandsRead = (Vec<String>, Vec<(String, String)>);

/// A module installed in the linked game, for the Relations pickers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    /// Lower case, as the game compares it.
    pub id: String,
    pub name: String,
}

/// What tasks leave for the tab; taken at the start of its next frame.
#[derive(Default)]
struct Inbox {
    modes: Option<Arc<ModeScan>>,
    installed: Option<Vec<Installed>>,
    commands: Option<CommandsRead>,
    tree: Option<(Vec<String>, usize, usize)>,
    picture: Option<(String, Option<ColorImage>)>,
    picture_chosen: Option<String>,
    picture_done: Option<Result<(), String>>,
    /// A picture made into the scratch `thumbnail_pending.dds`: its path, or why not.
    picture_pending: Option<Result<String, String>>,
    saved: Option<Result<(), String>>,
    /// The footer's Save was pressed.
    save_asked: bool,
    updates_yes: bool,
    service_ready: bool,
}

/// What the Picture row shows.
#[derive(Default)]
struct PictureView {
    /// What the texture is of: the file and the manifest revision.
    key: String,
    texture: Option<TextureHandle>,
    loading: bool,
    /// The file does not read as a picture.
    unreadable: bool,
    /// A picture is being made into `thumbnail.dds`.
    making: bool,
    /// A save of the picture is under way, with what it took (the waiting picture, the
    /// removal): unsaved until it is answered, and put back when it fails.
    saving: Option<(Option<String>, bool)>,
    /// Moves when a picture was made: the file of the same name is read again.
    made: u64,
    /// The picture chosen and made into `temp\thumbnail_pending.dds`: it takes the place of
    /// `thumbnail.dds` when the tab is saved (`s_ThumbPending`).
    pending: Option<String>,
    /// Remove was pressed: the manifest stops naming the picture when the tab is saved, and the
    /// editor's own `thumbnail.dds` goes with it (`s_ThumbRemove`).
    remove: bool,
}

/// What a page asks of the shell once it is drawn.
enum Action {
    OpenPda(String),
    CreatePda,
    RescanModes,
    ReadScripts,
    ChoosePicture,
    RemovePicture,
    KeepPicture,
}

/// The Mod Settings tab.
pub struct ModSettingsDoc {
    editor: Arc<Editor>,
    root: String,
    page: usize,
    form: Form,
    base: Form,
    base_rev: Option<Rev>,
    exists: bool,
    thumbnail: String,
    aux: Aux,
    aux_gen: u64,
    problems: Vec<Problem>,
    checked: Option<(Form, u64)>,
    status: String,
    saving: Option<Form>,
    failed: Option<Form>,
    /// What stopped the last save: the answer of a command that asked for it.
    save_error: Option<String>,
    inbox: Arc<Mutex<Inbox>>,
    modes: Option<Arc<ModeScan>>,
    installed: Option<Vec<Installed>>,
    gated: (usize, usize),
    /// The command whose help shows under the table.
    command_sel: usize,
    /// The project's scripts were asked for since the project opened.
    scripts_asked: bool,
    picture: PictureView,
    ask_updates: bool,
    service_asked: bool,
    actions: Vec<Action>,
}

impl ModSettingsDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>) -> Self {
        let mut doc = Self {
            editor: editor.clone(),
            root: String::new(),
            page: form::GENERAL,
            form: Form::default(),
            base: Form::default(),
            base_rev: None,
            exists: true,
            thumbnail: String::new(),
            aux: Aux::default(),
            aux_gen: 0,
            problems: Vec::new(),
            checked: None,
            status: String::new(),
            saving: None,
            failed: None,
            save_error: None,
            inbox: Arc::new(Mutex::new(Inbox::default())),
            modes: None,
            installed: None,
            gated: (0, 0),
            command_sel: 0,
            scripts_asked: false,
            picture: PictureView::default(),
            ask_updates: false,
            service_asked: false,
            actions: Vec::new(),
        };
        doc.sync();
        doc
    }

    // ---- the data ------------------------------------------------------------------------------

    /// The form as `mod.ltx` says it, the reads of the checks started.
    fn reset(&mut self, root: String) {
        self.root = root;
        self.base_rev = None;
        self.form = Form::default();
        self.base = Form::default();
        self.status.clear();
        self.saving = None;
        self.failed = None;
        self.installed = None;
        self.picture = PictureView::default();
        self.aux = Aux::default();
        self.aux_gen += 1;
        // the reads that need the module id and its exclude rules wait for the first snapshot
        self.load_modes(false);
        // the scripts are read on open: a page's dot never claims what nobody looked at
        self.scripts_asked = false;
        if self.page == form::RELATIONS {
            self.load_installed();
        }
    }

    /// Follows the store: the first snapshot fills the form, a later one reaches every field the
    /// author has not touched. A frame only compares revisions.
    fn sync(&mut self) {
        let Some(session) = self.editor.project_opt() else { return };
        let root = session.root.to_string_lossy().into_owned();
        if root != self.root {
            self.reset(root);
            self.service_asked = false;
        }
        let Some(service) = ManifestService::get(&session) else {
            if !self.service_asked {
                self.service_asked = true;
                let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
                let _task = self.editor.exec.spawn(async move {
                    if let Ok(service) = ManifestService::of_session(&ed, &session).await {
                        let _ = service.sync().await;
                    }
                    inbox.lock().service_ready = true;
                    call::wake_ui(&ed);
                });
            }
            return;
        };
        let snap = service.snapshot();
        if self.base_rev == Some(snap.rev) {
            return;
        }
        let first = self.base_rev.is_none();
        self.base_rev = Some(snap.rev);
        self.exists = snap.exists();
        let fresh = snap.manifest().map(Form::of).unwrap_or_default();
        self.thumbnail = snap.manifest().map(|m| m.thumbnail.clone()).unwrap_or_default();
        if first {
            self.form = fresh.clone();
        } else {
            self.form.follow(&self.base, &fresh);
        }
        self.base = fresh;
        // a layout written meanwhile is on the PDA page's list
        self.load_tree();
        if first {
            self.load_commands();
        }
    }

    /// What tasks left: the data of the checks and the pickers, the picture, a save's answer.
    fn take_inbox(&mut self, ctx: Option<&egui::Context>) {
        let mut inbox = self.inbox.lock();
        if let Some(modes) = inbox.modes.take() {
            self.modes = Some(modes);
        }
        if let Some(list) = inbox.installed.take() {
            self.installed = Some(list);
        }
        if let Some((functions, taken)) = inbox.commands.take() {
            self.aux.functions_read = true;
            self.aux.functions = functions;
            self.aux.taken = taken;
            self.aux_gen += 1;
        }
        if let Some((pda, files, patches)) = inbox.tree.take() {
            self.aux.pda_files = pda;
            self.gated = (files, patches);
            self.aux_gen += 1;
        }
        if inbox.updates_yes {
            inbox.updates_yes = false;
            self.form.updates_off = true;
        }
        inbox.service_ready = false;
        let chosen = inbox.picture_chosen.take();
        let done = inbox.picture_done.take();
        let pending = inbox.picture_pending.take();
        let save_asked = std::mem::take(&mut inbox.save_asked);
        let saved = inbox.saved.take();
        let picture = if ctx.is_some() { inbox.picture.take() } else { None };
        drop(inbox);
        if let (Some((key, image)), Some(ctx)) = (picture, ctx)
            && key == self.picture.key
        {
            self.picture.loading = false;
            self.picture.unreadable = image.is_none();
            self.picture.texture =
                image.map(|img| ctx.load_texture("gz-ui-mod.thumbnail", img, TextureOptions::LINEAR));
        }
        if let Some(path) = chosen {
            self.set_picture(&path);
        }
        if let Some(result) = done {
            self.picture.made += 1;
            let took = self.picture.saving.take();
            if let Err(e) = &result {
                // the save did not carry the picture's change out: it waits again, and whoever
                // waits for the save is told
                if let Some((pending, remove)) = took {
                    if self.picture.pending.is_none() {
                        self.picture.pending = pending;
                    }
                    self.picture.remove |= remove;
                }
                self.save_error = Some(e.clone());
            }
            self.aux.picture_error = result.err().unwrap_or_default();
            self.aux_gen += 1;
        }
        if let Some(result) = pending {
            self.picture.making = false;
            self.picture.made += 1;
            match result {
                Ok(path) => {
                    self.picture.pending = Some(path);
                    self.picture.remove = false;
                    self.aux.picture_error.clear();
                }
                Err(e) if e.is_empty() => {}
                Err(e) => self.aux.picture_error = e,
            }
            self.aux_gen += 1;
        }
        if let Some(result) = saved {
            self.save_answered(result);
        }
        if save_asked && let Err(e) = self.save_now() {
            self.status = e;
        }
    }

    fn load_modes(&self, refresh: bool) {
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            let scan = match GameLink::of(&ed) {
                Ok(link) => {
                    if refresh {
                        let _ = link.refresh().await;
                    }
                    link.modes().await
                }
                Err(e) => Arc::new(ModeScan { modes: Vec::new(), error: Some(e.to_string()) }),
            };
            inbox.lock().modes = Some(scan);
            call::wake_ui(&ed);
        });
    }

    /// The modules of the linked game, read when Relations is picked.
    fn load_installed(&self) {
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            let files = match GameLink::of(&ed) {
                Ok(link) => link.files().await.ok(),
                Err(_) => None,
            };
            let list = match files {
                Some(files) => ed.exec.io(move || installed_modules(&files)).await,
                None => Vec::new(),
            };
            inbox.lock().installed = Some(list);
            call::wake_ui(&ed);
        });
    }

    /// The project's script functions and the linked game's other commands, read when the tab
    /// opens, when Commands is picked and when the scripts are read again.
    fn load_commands(&self) {
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let exclude = self.form.exclude_rules();
        let id = self.form.id.clone();
        let _task = self.editor.exec.spawn(async move {
            let Ok((session, project)) = Project::of(&ed) else { return };
            project.tree.refresh().await;
            let tree = project.tree.snapshot();
            let root = session.root.clone();
            let game = project.game_root();
            let data = ed
                .exec
                .io(move || {
                    let functions = gz_mod::script_functions(&root, &tree, &exclude);
                    let taken = if game.is_empty() {
                        Vec::new()
                    } else {
                        gz_mod::installed_commands(Path::new(&game), &id)
                    };
                    (functions, taken)
                })
                .await;
            inbox.lock().commands = Some(data);
            call::wake_ui(&ed);
        });
    }

    /// The project's PDA layouts and what mounts at engine start (`gamedata`, `patch`), from the
    /// file tree: taken on open and when the PDA page is picked, never per frame.
    fn load_tree(&self) {
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            let Ok((_, project)) = Project::of(&ed) else { return };
            project.tree.refresh().await;
            let tree = project.tree.snapshot();
            let data = ed
                .exec
                .io(move || {
                    let mut pda: Vec<String> = tree
                        .files_with_extension("pdaasset")
                        .iter()
                        .map(gz_editor::RelPath::to_backslash)
                        .collect();
                    pda.sort_by_key(|p| p.to_ascii_lowercase());
                    let count = |folder: &str, skip: Option<&str>| {
                        gz_editor::RelPath::new(folder).ok().map_or(0, |rel| {
                            tree.files_under(&rel)
                                .iter()
                                .filter(|(_, e)| skip.is_none_or(|s| !e.name.eq_ignore_ascii_case(s)))
                                .count()
                        })
                    };
                    (pda, count("gamedata", None), count("patch", Some("_readme.txt")))
                })
                .await;
            inbox.lock().tree = Some(data);
            call::wake_ui(&ed);
        });
    }

    /// Reads the thumbnail for the preview when what it is of changed.
    fn want_picture(&mut self) {
        let shown = match (&self.picture.pending, self.picture.remove) {
            (Some(p), _) => p.clone(),
            (None, false) if !self.thumbnail.is_empty() => format!("{}\\{}", self.root, self.thumbnail),
            _ => String::new(),
        };
        let key = if shown.is_empty() { String::new() } else { format!("{shown}|{}", self.picture.made) };
        if key == self.picture.key {
            return;
        }
        self.picture.key.clone_from(&key);
        self.picture.texture = None;
        self.picture.unreadable = false;
        if key.is_empty() {
            self.picture.loading = false;
            return;
        }
        self.picture.loading = true;
        let path = shown;
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            let image = ed.exec.io(move || read_picture(Path::new(&path))).await;
            inbox.lock().picture = Some((key, image));
            call::wake_ui(&ed);
        });
    }

    /// A picture the author picked is made into the scratch `temp\thumbnail_pending.dds`
    /// ; nothing reaches the module before the tab is saved.
    fn set_picture(&mut self, path: &str) {
        self.picture.making = true;
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let picture = std::path::PathBuf::from(path);
        let temp = Path::new(&self.root).join("temp");
        let _task = self.editor.exec.spawn(async move {
            let made = ed
                .exec
                .io(move || -> Result<String, String> {
                    let dds = gz_release::browser::make_thumbnail(&picture)?;
                    let _ = std::fs::create_dir_all(&temp);
                    let pending = temp.join(PENDING_NAME);
                    gz_editor::fsx::write_atomic(&pending, &dds)
                        .map_err(|_| format!("can't write {}", pending.display()))?;
                    Ok(pending.display().to_string())
                })
                .await;
            inbox.lock().picture_pending = Some(made);
            call::wake_ui(&ed);
        });
    }

    /// Remove: the picture waiting goes now (it is the editor's scratch); the module's is only
    /// marked, and goes when the tab is saved.
    fn remove_picture(&mut self) {
        if let Some(p) = self.picture.pending.take() {
            self.editor.exec.spawn_io(move || {
                let _ = std::fs::remove_file(p);
            });
        }
        self.picture.remove = !self.thumbnail.is_empty();
        self.picture.made += 1;
    }

    /// The picture's change waits for a save.
    fn picture_waits(&self) -> bool {
        self.picture.pending.is_some() || self.picture.remove
    }

    /// Saves the picture's change: the waiting picture moved to
    /// `thumbnail.dds` and named, or the removal carried out.
    fn save_picture(&mut self) {
        if !self.picture_waits() {
            return;
        }
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let pending = self.picture.pending.take();
        self.picture.saving = Some((pending.clone(), self.picture.remove));
        self.save_error = None;
        self.picture.remove = false;
        self.picture.making = true;
        let root = std::path::PathBuf::from(&self.root);
        let origin = self.editor.ui_origin();
        let _task = self.editor.exec.spawn(async move {
            let value = match pending {
                Some(p) => {
                    let target = root.join(gz_release::browser::THUMBNAIL_NAME);
                    let placed = ed.exec.io(move || place_thumbnail(Path::new(&p), &target)).await;
                    if let Err(e) = placed {
                        let mut i = inbox.lock();
                        i.picture_done = Some(Err(e));
                        i.picture_pending = Some(Err(String::new()));
                        drop(i);
                        call::wake_ui(&ed);
                        return;
                    }
                    gz_release::browser::THUMBNAIL_NAME.to_owned()
                }
                None => String::new(),
            };
            let a = if value.is_empty() {
                args([("remove", serde_json::Value::Bool(true))])
            } else {
                args([("from", s(&value))])
            };
            let reply = ed.call("mod.thumbnail", a, origin).await;
            let mut i = inbox.lock();
            i.picture_done = Some(if call::ok(&reply) { Ok(()) } else { Err(call::error_text(&reply)) });
            i.picture_pending = Some(Err(String::new()));
            drop(i);
            call::wake_ui(&ed);
        });
    }

    /// Forgets the manifest's own snapshot so the next frame follows the store again.
    pub(crate) fn refresh(&mut self) {
        self.take_inbox(None);
        self.sync();
        self.recheck();
    }

    fn recheck(&mut self) {
        if self.checked.as_ref().is_some_and(|(f, g)| *f == self.form && *g == self.aux_gen) {
            return;
        }
        self.problems = form::check(&self.form, &self.aux);
        self.checked = Some((self.form.clone(), self.aux_gen));
    }

    /// The tab holds edits `mod.ltx` does not have yet.
    #[must_use]
    pub fn dirty(&self) -> bool {
        // a save of the picture under way is not saved yet
        self.form != self.base || self.picture_waits() || self.picture.saving.is_some()
    }

    /// Opens a page; what its pickers and checks read is read now.
    pub fn set_page(&mut self, page: usize) {
        self.page = page.min(PAGES.len() - 1);
        match self.page {
            form::PDA => self.load_tree(),
            form::RELATIONS => self.load_installed(),
            form::COMMANDS => self.load_commands(),
            form::MODE => self.load_modes(false),
            _ => {}
        }
    }

    /// `mod.view show=`: opens the page the list is on and asks for the list; the page opens it
    /// on its next frame. `Err` says why it cannot be put up.
    pub fn show_list(&mut self, ctx: &egui::Context, name: &str) -> Result<(), String> {
        if name == SHOW_FUNCTION {
            if self.form.commands.is_empty() {
                return Err("the module has no command yet: add one (mod.set set={\"commands\": ...}), then its Function list can be shown".to_owned());
            }
            self.set_page(form::COMMANDS);
        } else if RELATION_SHOWS.contains(&name) {
            self.set_page(form::RELATIONS);
        } else {
            return Err(show_error());
        }
        gz_ui::menu::show_popup(ctx, &format!("mod_settings.{name}"));
        Ok(())
    }

    /// `mod.view ask_game_updates=true`: the question goes up on the tab's next frame.
    pub fn ask_game_updates_later(&mut self) {
        self.page = form::GENERAL;
        self.ask_updates = true;
    }

    // ---- saving ---------------------------------------------------------------------------------

    /// Sends the form's changes. The first problem that blocks is the error.
    fn save_now(&mut self) -> Result<(), String> {
        self.recheck();
        if let (Some(first), _) = form::blocking(&self.problems) {
            let label = PAGES.get(first.page).map_or("", |p| p.label);
            return Err(format!("{label}: {}", first.text));
        }
        // the picture goes with an explicit save only: the fields save as they are edited
        self.save_picture();
        if self.saving.is_some() {
            return Ok(());
        }
        let plan = form::plan(&self.base, &self.form);
        if plan.is_empty() {
            // what differs is spelling only (a blank line of Not Shipped): nothing to write
            self.form = self.base.clone();
            return Ok(());
        }
        self.saving = Some(self.form.clone());
        self.save_error = None;
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let origin = self.editor.ui_origin();
        let _task = self.editor.exec.spawn(async move {
            let reply = ed.call("mod.set", plan.args(), origin).await;
            let result = if call::ok(&reply) { Ok(()) } else { Err(call::error_text(&reply)) };
            inbox.lock().saved = Some(result);
            call::wake_ui(&ed);
        });
        Ok(())
    }

    /// A save answered: on success every field sent takes what `mod.ltx` says now (the
    /// repository as owner/repo, the description trimmed); a failure is the footer's status and
    /// is not sent again until the form changes.
    fn save_answered(&mut self, result: Result<(), String>) {
        let Some(sent) = self.saving.take() else { return };
        match result {
            Ok(()) => {
                self.status.clear();
                self.failed = None;
                if let Some(snap) =
                    self.editor.project_opt().and_then(|s| ManifestService::get(&s)).map(|svc| svc.snapshot())
                {
                    let fresh = snap.manifest().map(Form::of).unwrap_or_default();
                    self.thumbnail = snap.manifest().map(|m| m.thumbnail.clone()).unwrap_or_default();
                    self.form.follow(&sent, &fresh);
                    self.base = fresh;
                    self.base_rev = Some(snap.rev);
                    self.exists = snap.exists();
                }
            }
            Err(e) => {
                self.save_error = Some(e.clone());
                self.status = e;
                self.failed = Some(sent);
            }
        }
    }

    /// Saves once nothing is being typed in and nothing blocks.
    fn save_when_idle(&mut self, ctx: &egui::Context) {
        if self.form == self.base || self.saving.is_some() || self.failed.as_ref() == Some(&self.form) {
            return;
        }
        if ctx.egui_wants_keyboard_input()
            || ctx.dragged_id().is_some()
            || form::blocking(&self.problems).0.is_some()
        {
            return;
        }
        let _ = self.save_now();
    }

    // ---- MCP ------------------------------------------------------------------------------------

    /// The `mod.view` answer of the open tab.
    #[must_use]
    pub fn view_json(&self, active: bool) -> Value {
        view_json(true, active, self.page, self.dirty(), &self.problems)
    }

    // ---- drawing --------------------------------------------------------------------------------

    fn ask_game_updates(&self, ctx: &egui::Context, asker: Asker) {
        let spec = ConfirmSpec {
            id: "disable_game_updates".to_owned(),
            title: "Disable Game Updates".to_owned(),
            headline: "Everyone who installs this module stops getting updates of the game. Disable them only as a last resort.".to_owned(),
            items: vec![
                "The game no longer checks for updates, so its fixes never reach these players.".to_owned(),
                "Send Bug Report disappears from the main menu.".to_owned(),
                "The menu says in red that updates were disabled, and names this module.".to_owned(),
                "It lasts for as long as the module is installed, and other modules cannot turn it back on.".to_owned(),
            ],
            total: 0,
            consequence: "Right only for a module an update would break. If a game update merely might conflict with yours, keep updates on and release a fix instead.".to_owned(),
            action: "Disable Updates".to_owned(),
            destructive: true,
        };
        let (inbox, ed) = (self.inbox.clone(), self.editor.clone());
        let line = confirm(ctx, asker, spec, move |answer| {
            if answer == gz_ui::Answer::Yes {
                inbox.lock().updates_yes = true;
                call::wake_ui(&ed);
            }
        });
        call::log_lines(&self.editor, line);
    }

    fn draw_nav(&mut self, ui: &mut Ui, width: f32, height: f32) {
        let mut picked = None;
        let page = self.page;
        let problems = &self.problems;
        list(ui, "mod_settings_nav", Vec2::new(width, height), |l| {
            for (i, (info, icon)) in PAGES.iter().zip(PAGE_ICONS).enumerate() {
                let spec = NavRowSpec { icon, dot: form::page_tone(problems, i), ..NavRowSpec::default() };
                if nav_row(l.ui(), info.id, info.label, page == i, &spec).clicked {
                    picked = Some(i);
                }
            }
        });
        if let Some(i) = picked {
            self.set_page(i);
        }
    }

    fn draw_page(&mut self, ui: &mut Ui) {
        let p = palette(ui.ctx());
        let info = PAGES.get(self.page).copied().unwrap_or(PAGES[0]);
        let _ = text::label(ui, info.label, Font::Title, p.text, Clip::End);
        let _ = text::prose(ui, info.about);
        ui.add_space(8.0);
        // a page of the kit's cards: the page's fields in one raised box
        let tone = form::page_tone(&self.problems, self.page);
        let attention = tone.is_some_and(|t| matches!(t, Tone::Error | Tone::Required));
        gz_ui::card::card(ui, info.id, "", tone.unwrap_or(Tone::Muted), attention, |ui| match self.page {
            form::GENERAL => self.general_page(ui),
            form::BROWSER => self.browser_page(ui),
            form::MODE => self.mode_page(ui),
            form::RELATIONS => self.relations_page(ui),
            form::CONTENTS => self.contents_page(ui),
            form::PDA => self.pda_page(ui),
            _ => self.commands_page(ui),
        });
    }

    fn general_page(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut ask = false;
        let Self { form, problems, .. } = self;
        PropertyGrid::new(ROLE).label_frac(LABEL_FRAC).show(ui, |g| {
            g.row(
                "Name",
                "The mod's title, as the game's Mods menu prints it. Free text: it owes nothing to the module id or the project folder, and it is not the title of a game mode.",
                PropFlags::NONE,
                0,
                |ui| gz_ui::form::row_field(ui, ("gz-ui-mod.text", "name"), &mut form.name, ""),
            );
            g.row("Version", "1 to 4 numbers separated by dots. The game compares versions to offer an update.", flags(problems, "version", PropFlags::REQUIRED), 0, |ui| {
                gz_ui::form::row_field(ui, ("gz-ui-mod.text", "version"), &mut form.version, "1.0.0")
            });
            messages(g, problems, "version");
            g.row("Author", "Shown in the game's Mods menu.", PropFlags::NONE, 0, |ui| gz_ui::form::row_field(ui, ("gz-ui-mod.text", "author"), &mut form.author, ""));
            let _ = g.block(|ui| text_block(ui, "Description", "Shown in the game's Mods menu. Plain text or a string table id.", &mut form.description, 4));
            g.row("Website", "The mod's page on AP-PRO or ModDB. The game opens it from the Mods menu.", flags(problems, "website", PropFlags::NONE), 0, |ui| {
                gz_ui::form::row_field(ui, ("gz-ui-mod.text", "website"), &mut form.website, "https://ap-pro.ru/stuff/...")
            });
            messages(g, problems, "website");
            let repo = g.row(
                "GitHub Repository",
                "owner/repo. The game checks the latest release of this repository for updates. A pasted address is fine: it is kept as owner/repo.",
                flags(problems, "github", PropFlags::NONE),
                0,
                |ui| gz_ui::form::row_field(ui, ("gz-ui-mod.text", "github"), &mut form.github, "owner/repo"),
            );
            if repo.is_some_and(|r| r.lost_focus()) {
                form.github = normalize_github_repo(&form.github);
            }
            messages(g, problems, "github");
            let id_problem = problems.iter().any(|p| p.field == "id");
            g.section("More", id_problem, None, |g| {
                g.row(
                    "Module ID",
                    "Technical, never shown to the player: the folder the module installs into (<game>\\modules\\<id>) and the name other modules require it by. Seeded from the project folder - change it before the first release.",
                    flags(problems, "id", PropFlags::REQUIRED),
                    1,
                    |ui| gz_ui::form::row_field(ui, ("gz-ui-mod.text", "id"), &mut form.id, "my_mod"),
                );
                messages(g, problems, "id");
                let row = g.row(
                    "Disable Game Updates",
                    "Only for a module that replaces the game's own files so deeply that an update would break it. Saved with the module's other settings, and applied by the build.",
                    PropFlags::INLINE,
                    1,
                    |ui| {
                        let mut off = form.updates_off;
                        switch(ui, &mut off).changed().then_some(off)
                    },
                );
                match row.flatten() {
                    Some(false) => form.updates_off = false,
                    // a last resort, so turning it on asks; turning it off needs no question
                    Some(true) => ask = true,
                    None => {}
                }
                if form.updates_off {
                    g.message(Tone::Warning, "Players of this module get no game updates and cannot send bug reports.");
                }
            });
        });
        if ask {
            let asker = if self.editor.flags.unattended { Asker::Unattended } else { Asker::Ui };
            self.ask_game_updates(&ctx, asker);
        }
    }

    fn browser_page(&mut self, ui: &mut Ui) {
        self.want_picture();
        let picking = Picking::busy(&self.editor);
        let Self { form, problems, picture, thumbnail, actions, .. } = self;
        PropertyGrid::new(ROLE).label_frac(LABEL_FRAC).show(ui, |g| {
            g.row(
                "Picture",
                "PNG, JPG, TGA or BMP. The middle square is scaled to 256 x 256 and saved as thumbnail.dds at the root of the module.",
                flags(problems, "picture", PropFlags::NONE),
                0,
                |ui| {
                    local_value_column(ui, "picture", |ui| {
                        ui.horizontal_top(|ui| {
                            let _ = local_picture(ui, picture.texture.as_ref().map(TextureHandle::id), PICTURE_SIDE);
                            ui.add_space(12.0);
                            ui.vertical(|ui| {
                                let p = palette(ui.ctx());
                                let own = thumbnail.eq_ignore_ascii_case(gz_release::browser::THUMBNAIL_NAME);
                                if picture.making {
                                    gz_ui::state::busy_line(ui, "Making thumbnail.dds...");
                                } else if picture.pending.is_some() {
                                    let _ = text::prose(ui, if own || thumbnail.is_empty() {
                                        "A new picture: it becomes thumbnail.dds when you save."
                                    } else {
                                        "A new picture: it is saved as thumbnail.dds."
                                    });
                                } else if picture.remove {
                                    let _ = text::prose(ui, if own {
                                        "thumbnail.dds is deleted for good when you save."
                                    } else {
                                        "The manifest stops naming it when you save; the file stays."
                                    });
                                } else if thumbnail.is_empty() {
                                    let _ = text::prose(ui, "No picture: the Mods window shows the first letter of the name.");
                                } else {
                                    let _ = text::label(ui, thumbnail.as_str(), Font::Body, p.text_secondary, Clip::Start);
                                    if picture.unreadable {
                                        let _ = text::status(ui, Tone::Error, "The file does not read as a picture.");
                                    }
                                }
                                ui.add_space(2.0);
                                ui.horizontal(|ui| {
                                    let why = if picking { "The picture dialog is open" } else { "A thumbnail is being made" };
                                    if Button::new("Choose Picture...").enabled(!picking && !picture.making, why).show(ui).clicked() {
                                        actions.push(Action::ChoosePicture);
                                    }
                                    let has_picture = picture.pending.is_some() || (!picture.remove && !thumbnail.is_empty());
                                    if has_picture {
                                        if Button::new("Remove").kind(Kind::Ghost).enabled(!picture.making, why).show(ui).clicked() {
                                            actions.push(Action::RemovePicture);
                                        }
                                    } else if picture.remove && Button::new("Keep Picture").kind(Kind::Ghost).show(ui).clicked() {
                                        actions.push(Action::KeepPicture);
                                    }
                                });
                            });
                        });
                    });
                },
            );
            messages(g, problems, "picture");
            g.row("Tags", "Players filter the Mod Browser by these.", PropFlags::NONE, 0, |ui| {
                local_value_column(ui, "tags", |ui| {
                    let width = ui.available_width();
                    let columns = ((width / em(9.0)) as usize).clamp(1, 4);
                    let cell = (width / columns as f32).floor();
                    for row in TAG_VOCABULARY.chunks(columns) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 0.0;
                            for tag in row {
                                let mut on = form.tags.iter().any(|t| t == tag);
                                let id = Id::unique(("gz-ui-mod.check", tag));
                                if gz_ui::form::check(ui, id, tag_label(tag), &mut on, cell) {
                                    form.set_tag(TAG_VOCABULARY, tag, on);
                                }
                            }
                        });
                    }
                });
            });
        });
    }

    fn mode_page(&mut self, ui: &mut Ui) {
        let classic = self.base.kind() == Some(TargetKind::Any);
        let Self { form, problems, modes, gated, actions, .. } = self;
        let scan = modes.clone();
        let list: &[gz_game::GameMode] = scan.as_deref().map_or(&[], |m| m.modes.as_slice());
        let kind = form.kind();
        PropertyGrid::new(ROLE).label_frac(LABEL_FRAC).show(ui, |g| {
            g.row(
                "Target",
                "Which game the module's level work applies in: spawns, collision, AI map, visuals and the game graph are gated on the mode.",
                flags(problems, "target", PropFlags::REQUIRED),
                0,
                |ui| {
                    // a classic module (every mode) keeps its choice on show; it is made by
                    // tools/classic_mod, not picked here
                    let mut offered: Vec<TargetKind> = TargetKind::OFFERED.to_vec();
                    if classic {
                        offered.push(TargetKind::Any);
                    }
                    let labels: Vec<&str> = offered.iter().map(|k| k.label()).collect();
                    let mut pick = kind.and_then(|k| offered.iter().position(|o| *o == k)).unwrap_or(usize::MAX);
                    if segmented(ui, &mut pick, &labels, None)
                        && let Some(k) = offered.get(pick)
                    {
                        form.set_kind(*k, list.first().map(|m| m.id.as_str()));
                    }
                },
            );
            if let Some(k) = kind {
                g.message(Tone::Muted, k.about());
            }
            messages(g, problems, "target");
            // files, configs and scripts mount at engine start, before any mode is picked: a
            // weapon rebalance would leak into every campaign
            if kind.is_some_and(|k| k != TargetKind::Plain) && (gated.0 > 0 || gated.1 > 0) {
                let text = format!(
                    "{} under gamedata and {} are not gated: they mount at engine start, in every game.",
                    gz_mod::text::count_text(gated.0, "file", "files"),
                    gz_mod::text::count_text(gated.1, "config patch", "config patches")
                );
                g.message(Tone::Warning, &text);
            }
            match kind {
                Some(TargetKind::Existing) => {
                    let mut options: Vec<&str> = list.iter().map(|m| m.caption.as_str()).collect();
                    let mut current = list.iter().position(|m| m.id.eq_ignore_ascii_case(&form.mode));
                    if current.is_none() && !form.mode.is_empty() {
                        options.push(form.mode.as_str());
                        current = Some(options.len() - 1);
                    }
                    let picked = g
                        .row("Mode", "A mode of the linked game, by the name its new-game screen shows.", flags(problems, "mode", PropFlags::REQUIRED), 0, |ui| {
                            let mut i = current.unwrap_or(usize::MAX);
                            if options.is_empty() {
                                read_only(ui, "None", Clip::End);
                                return None;
                            }
                            combo(ui, "mod_settings.mode", &mut i, &options).then_some(i)
                        })
                        .flatten();
                    if g.trailing(Icon::Refresh, "Scan the linked game's modes again") {
                        actions.push(Action::RescanModes);
                    }
                    if let Some(m) = picked.and_then(|i| list.get(i)) {
                        m.id.clone_into(&mut form.mode);
                    }
                    if let Some(m) = list.iter().find(|m| m.id.eq_ignore_ascii_case(&form.mode)) {
                        let whose = if m.source == "game" { " - the linked game's".to_owned() } else { format!(" - module {}", m.source) };
                        g.message(Tone::Muted, &format!("{}{whose}", m.id));
                    }
                    if list.is_empty() && scan.is_some() {
                        let why = scan.as_ref().and_then(|s| s.error.clone()).unwrap_or_else(|| "The linked game declares no modes, only options.".to_owned());
                        g.message(Tone::Warning, &why);
                    }
                    messages(g, problems, "mode");
                }
                Some(TargetKind::New) => {
                    g.row(
                        "Mode ID",
                        "Technical id of the MODE, not of the mod: the module's level work is gated on it and the new-game screen offers it. It may match the module id or not.",
                        flags(problems, "pmode_id", PropFlags::REQUIRED),
                        0,
                        |ui| gz_ui::form::row_field(ui, ("gz-ui-mod.text", "pmode_id"), &mut form.pmode_id, "my_mode"),
                    );
                    messages(g, problems, "pmode_id");
                    g.row(
                        "Mode Title",
                        "What the new-game screen lists the mode as. It names the mode, not the mod: the Mods menu shows the Name on General instead.",
                        flags(problems, "pmode_title", PropFlags::NONE),
                        0,
                        |ui| gz_ui::form::row_field(ui, ("gz-ui-mod.text", "pmode_title"), &mut form.pmode_title, ""),
                    );
                    messages(g, problems, "pmode_title");
                }
                _ => {}
            }
        });
    }

    fn relations_page(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let linked = self
            .editor
            .project_opt()
            .and_then(|s| s.services.get::<Project>())
            .is_some_and(|p| !p.game_root().is_empty());
        const RELS: [(&str, &str, &str); 4] = [
            ("requires", "Requires", "The game refuses to load this module without these."),
            ("after", "Load After", "These load first; this module's changes go on top of theirs."),
            ("before", "Load Before", "This module loads first; their changes go on top of its."),
            ("conflicts", "Conflicts", "The game refuses to load this module together with these."),
        ];
        let mut anchors: [Option<Rect>; 4] = [None; 4];
        let mut opened = None;
        let Self { form, problems, installed, .. } = self;
        let installed_list: &[Installed] = installed.as_deref().unwrap_or(&[]);
        PropertyGrid::new(ROLE).label_frac(LABEL_FRAC).show(ui, |g| {
            for (i, (field, label, help)) in RELS.iter().enumerate() {
                let lines = relation_lines(form, i);
                let shown: Vec<String> = lines
                    .iter()
                    .map(|l| {
                        let label = module_label(installed_list, &form::relation_id(l));
                        let value = form::relation_value(l);
                        if value.is_empty() { label } else { format!("{label}  {value}") }
                    })
                    .collect();
                let request = format!("mod_settings.{}", RELATION_SHOWS[i]);
                let out = g.row(label, help, flags(problems, field, PropFlags::NONE), 0, |ui| {
                    local_value_column(ui, field, |ui| {
                        let refs: Vec<&str> = shown.iter().map(String::as_str).collect();
                        let removed = if refs.is_empty() { None } else { chips(ui, field, &refs, true) };
                        let resp = Button::new("Add Module...").show(ui);
                        // a click, or `mod.view show=` asking for the same list
                        let asked = gz_ui::menu::take_popup_request(&ctx, &request);
                        (removed, resp.clicked() || asked, resp.rect)
                    })
                });
                if let Some((removed, add, rect)) = out {
                    if let Some(r) = removed
                        && r < relation_lines(form, i).len()
                    {
                        relation_lines_mut(form, i).remove(r);
                    }
                    anchors[i] = Some(rect);
                    if add {
                        opened = Some(i);
                    }
                }
                // modules this computer does not have: kept, said
                if linked && installed.is_some() {
                    let missing: Vec<String> = relation_lines(form, i)
                        .iter()
                        .map(|l| form::relation_id(l))
                        .filter(|id| !installed_list.iter().any(|m| m.id == *id))
                        .collect();
                    if !missing.is_empty() {
                        g.message(
                            Tone::Muted,
                            &format!("Not installed in the linked game: {}.", missing.join(", ")),
                        );
                    }
                }
                messages(g, problems, field);
            }
        });
        let layer = ui.layer_id();
        if let Some(i) = opened {
            let id = picker_id(i);
            KitPickList::reset(&ctx, id);
            Popup::open_id(&ctx, id);
        }
        for (i, anchor) in anchors.iter().enumerate() {
            let Some(anchor) = *anchor else { continue };
            let popup = picker_id(i);
            let me = form::relation_id(&self.form.id);
            let taken: Vec<String> =
                relation_lines(&self.form, i).iter().map(|l| form::relation_id(l)).collect();
            let installed: &[Installed] = self.installed.as_deref().unwrap_or(&[]);
            let empty = if !installed.is_empty() {
                "No module matches. A module id typed here can be added as it is."
            } else if linked {
                "The linked game has no other module. Type a module id to add it as it is."
            } else {
                "No game is linked. Type a module id to add it as it is."
            };
            let is_taken = |id: &str| id == me || taken.iter().any(|t| t == id);
            let mut items: Vec<PickItem> = installed
                .iter()
                .filter(|m| !is_taken(&m.id))
                .map(|m| PickItem::new(m.id.as_str(), module_label(installed, &m.id)))
                .collect();
            // an id the game does not list is taken as typed
            let typed = form::relation_id(&KitPickList::query(&ctx, popup));
            if !typed.is_empty()
                && !installed.iter().any(|m| m.id == typed)
                && !is_taken(&typed)
                && validate_id(&typed)
            {
                items.push(
                    PickItem::new(typed.as_str(), format!("Add {typed}"))
                        .detail("Not installed in the linked game: the id is taken as typed."),
                );
            }
            let list = KitPickList::new(popup, &items)
                .hint("Search, or type a module id")
                .empty(empty)
                .width(em(24.0))
                .max_height(240.0);
            let picked = pick_popup(&ctx, popup, anchor, layer, list).map(|n| items[n].id.clone());
            if let Some(id) = picked {
                relation_lines_mut(&mut self.form, i).push(id);
            }
        }
    }

    fn contents_page(&mut self, ui: &mut Ui) {
        let Self { form, .. } = self;
        PropertyGrid::new(ROLE).label_frac(LABEL_FRAC).show(ui, |g| {
            let _ = g.block(|ui| {
                text_block(
                    ui,
                    "Not Shipped",
                    "Project paths or patterns, one per line:\ndocs - a folder, with everything in it\ndesign/story.md - one file\n*.psd - every file of that name, at any depth\ndocs/*.md - a pattern below one folder\nThe Content Browser sets them too: right-click > Exclude from Build.",
                    &mut form.exclude,
                    8,
                )
            });
            g.row(
                "Never Shipped",
                "Version control and tool files (.git, .gitignore, Thumbs.db, ...), the editor's own folders (releases, temp, backup, ...) and the sources under rawdata. No rule is needed for these.",
                PropFlags::READ_ONLY,
                0,
                |ui| read_only(ui, "Tooling, the editor's own folders, rawdata", Clip::End),
            );
        });
    }

    fn pda_page(&mut self, ui: &mut Ui) {
        let Self { form, problems, aux, actions, .. } = self;
        let files = &aux.pda_files;
        PropertyGrid::new(ROLE).label_frac(LABEL_FRAC).show(ui, |g| {
            let mut options: Vec<&str> = vec!["None"];
            options.extend(files.iter().map(String::as_str));
            let mut current = if form.pda.is_empty() { 0 } else { files.iter().position(|f| form::same_path(f, &form.pda)).map_or(usize::MAX, |i| i + 1) };
            if current == usize::MAX {
                options.push(form.pda.as_str());
                current = options.len() - 1;
            }
            let picked = g
                .row("Layout", "The .pdaasset of the project the game's PDA shows. None: the module leaves the PDA alone.", flags(problems, "pda", PropFlags::NONE), 0, |ui| {
                    let mut i = current;
                    combo(ui, "mod_settings.pda", &mut i, &options).then_some(i)
                })
                .flatten();
            if !form.pda.is_empty() && g.trailing(Icon::External, "Open it in the PDA Interface tab") {
                actions.push(Action::OpenPda(form.pda.clone()));
            }
            match picked {
                Some(0) => form.pda.clear(),
                Some(i) => {
                    if let Some(f) = files.get(i - 1) {
                        f.clone_into(&mut form.pda);
                    }
                }
                None => {}
            }
            if files.is_empty() {
                g.message(Tone::Muted, "The project has no PDA interface yet.");
            }
            messages(g, problems, "pda");
        });
        if self.aux.pda_files.is_empty() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                // made from the template and named in mod.ltx at once: the form follows that save
                if Button::new("Create PDA Interface").show(ui).clicked() {
                    self.actions.push(Action::CreatePda);
                }
            });
        }
    }

    fn commands_page(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let layer = ui.layer_id();
        let mut pick_at: Option<(usize, Rect)> = None;
        // a click on a row's chevron, or `mod.view show=function` asking for the first row's list
        let mut opened =
            gz_ui::menu::take_popup_request(&ctx, &format!("mod_settings.{SHOW_FUNCTION}")).then_some(0);
        let mut selected = self.command_sel.min(self.form.commands.len().saturating_sub(1));
        let columns = [
            Col::new("Name", em(10.0)),
            Col::new("Function", em(12.0)),
            Col::new("Arguments", em(9.0)),
            Col::new("Cheat", em(4.0)).centred(),
        ];
        let edit = {
            let Self { form, problems, .. } = self;
            let commands = &mut form.commands;
            let rows = commands.len();
            let mut cell_edit = |ui: &mut Ui, row: usize, col: usize| {
                let Some(c) = commands.get_mut(row) else { return };
                let f = |part: &str| format!("commands.{row}.{part}");
                let bad = |part: &str| problems.iter().any(|p| p.field == f(part) && p.blocks);
                let resp = match col {
                    0 => Some(gz_ui::form::row_field(
                        ui,
                        ("gz-ui-mod.text", &format!("name.{row}")),
                        &mut c.name,
                        "my_mod_status",
                    )),
                    1 => {
                        let w = ui.available_width() - metrics::ICON_BUTTON - 2.0;
                        let r = ui
                            .horizontal(|ui| {
                                ui.set_width(ui.available_width());
                                let r = ui.allocate_ui(Vec2::new(w, metrics::FRAME), |ui| {
                                    gz_ui::form::row_field(
                                        ui,
                                        ("gz-ui-mod.text", &format!("function.{row}")),
                                        &mut c.function,
                                        "my_mod.status",
                                    )
                                });
                                if icon_button(
                                    ui,
                                    Icon::ChevronDown,
                                    "Pick a function of the project's scripts",
                                    false,
                                    true,
                                )
                                .clicked()
                                {
                                    opened = Some(row);
                                }
                                r.inner
                            })
                            .inner;
                        if r.changed() {
                            c.declared = true;
                        }
                        // the list belongs to the row that asked for it, or has it open
                        if opened == Some(row) || Popup::is_id_open(&ctx, function_picker_id(row)) {
                            pick_at = Some((row, r.rect));
                        }
                        Some(r)
                    }
                    2 => Some(gz_ui::form::row_field(
                        ui,
                        ("gz-ui-mod.text", &format!("args.{row}")),
                        &mut c.args,
                        "none",
                    )),
                    _ => {
                        let r = ui
                            .vertical_centered(|ui| {
                                gz_ui::Check::new(&mut c.cheat, "").width(metrics::FRAME).show(ui)
                            })
                            .inner;
                        Some(r.on_hover_text(
                            "Runs only in Dead Air's debug mode (the game started with -dbg).",
                        ))
                    }
                };
                if let Some(r) = resp {
                    if r.has_focus() || r.clicked() {
                        selected = row;
                    }
                    let part = ["name", "function", "args", "cheat"].get(col).copied().unwrap_or("name");
                    if bad(part) {
                        ui.painter().rect_stroke(
                            r.rect,
                            egui::CornerRadius::same(4),
                            egui::Stroke::new(1.0, palette(ui.ctx()).tone(Tone::Error)),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            };
            ItemRows::new("mod_settings.commands", &columns, rows)
                .add_label("Add Command")
                .empty_text("No commands. A command is a line of [commands] in mod.ltx: the name the player types and the function it calls.")
                .show(ui, |ui, row, col| cell_edit(ui, row, col))
        };
        if let Some(i) = edit.removed
            && i < self.form.commands.len()
        {
            self.form.commands.remove(i);
        }
        if let Some((from, to)) = edit.moved {
            gz_ui::rows::apply_move(&mut self.form.commands, from, to);
        }
        if edit.added {
            self.form.commands.push(Command::default());
            selected = self.form.commands.len() - 1;
        }
        self.command_sel = selected;
        // the selected command's help and what is wrong with it, under the table
        if let Some(i) =
            (!self.form.commands.is_empty()).then_some(selected.min(self.form.commands.len() - 1))
        {
            ui.add_space(8.0);
            let Self { form, problems, .. } = self;
            if let Some(c) = form.commands.get_mut(i) {
                let title = if c.name.is_empty() { "New Command".to_owned() } else { c.name.clone() };
                PropertyGrid::new(ROLE).label_frac(LABEL_FRAC).show(ui, |g| {
                    g.section(&title, true, c.cheat.then_some("cheat"), |g| {
                        let f = |part: &str| format!("commands.{i}.{part}");
                        g.row(
                            "Help",
                            "The line the console's help shows for it. Plain text or a string table id.",
                            flags(problems, &f("help"), PropFlags::NONE),
                            0,
                            |ui| gz_ui::form::row_field(ui, ("gz-ui-mod.text", "help"), &mut c.help, ""),
                        );
                        for part in ["name", "function", "args", "help"] {
                            messages(g, problems, &f(part));
                        }
                    });
                });
            }
        }
        ui.add_space(4.0);
        if Button::new("Read the Scripts Again")
            .kind(Kind::Ghost)
            .tip("The functions the Function picker offers come from the project's gamedata\\scripts.")
            .show(ui)
            .clicked()
        {
            self.actions.push(Action::ReadScripts);
        }
        if let Some(i) = opened {
            let id = function_picker_id(i);
            KitPickList::reset(&ctx, id);
            Popup::open_id(&ctx, id);
        }
        if let Some((i, anchor)) = pick_at.or_else(|| opened.map(|i| (i, Rect::NOTHING))) {
            let popup = function_picker_id(i);
            let functions = &self.aux.functions;
            let empty = if functions.is_empty() {
                "The project's gamedata\\scripts has no function yet: write one, then pick it here."
            } else {
                "No function matches."
            };
            let items: Vec<PickItem> =
                functions.iter().map(|f| PickItem::new(f.as_str(), f.as_str())).collect();
            let list = KitPickList::new(popup, &items)
                .hint("Search the project's functions")
                .empty(empty)
                .width(em(24.0))
                .max_height(240.0);
            let picked = pick_popup(&ctx, popup, anchor, layer, list).map(|n| items[n].id.clone());
            if let Some(f) = picked
                && let Some(c) = self.form.commands.get_mut(i)
            {
                c.function = f;
                c.declared = true;
            }
        }
    }

    /// What the pages asked for, now that they are drawn.
    fn run_actions(&mut self, cx: &mut DocCx<'_>) {
        for action in std::mem::take(&mut self.actions) {
            match action {
                Action::OpenPda(rel) => cx.requests.open(OpenRequest::file("pda", &rel)),
                // the PDA tab opened on a module without a layout makes one from the template,
                // named after the module, and makes it the module's
                Action::CreatePda => call::open_tab(&self.editor, "pda"),
                Action::RescanModes => self.load_modes(true),
                Action::ReadScripts => self.load_commands(),
                Action::ChoosePicture => {
                    let inbox = self.inbox.clone();
                    let start = self.root.clone();
                    call::pick_path(
                        &self.editor,
                        Pick::Picture,
                        "Choose the module's picture",
                        &start,
                        move |path, _| {
                            inbox.lock().picture_chosen = Some(path);
                        },
                    );
                }
                Action::RemovePicture => self.remove_picture(),
                Action::KeepPicture => {
                    self.picture.remove = false;
                    self.picture.made += 1;
                }
            }
        }
    }

    /// What the footer says of the file: the first problem that stops a save, the last failure,
    /// or how the tab saves. The tab's empty toolbar row says it too.
    fn save_status(&self) -> (String, Tone) {
        match form::blocking(&self.problems).0 {
            Some(first) => {
                (format!("{}: {}", PAGES.get(first.page).map_or("", |p| p.label), first.text), Tone::Error)
            }
            None if !self.status.is_empty() => (self.status.clone(), Tone::Error),
            None if !self.exists => ("No mod.ltx yet: the first edit makes it.".to_owned(), Tone::Muted),
            None if self.picture_waits() => {
                ("The picture waits for Save; the rest is saved as you edit".to_owned(), Tone::Warning)
            }
            None => ("Saved to mod.ltx as you edit".to_owned(), Tone::Muted),
        }
    }
}

impl Document for ModSettingsDoc {
    fn id(&self) -> &str {
        TAB
    }
    fn kind(&self) -> &'static str {
        TAB
    }
    fn label(&self) -> String {
        "Mod Settings".to_owned()
    }
    fn icon(&self) -> Icon {
        // a module, not the editor's settings: Preferences is the gear
        Icon::Package
    }
    fn tooltip(&self) -> Option<String> {
        Some("mod.ltx: the module's name, version, game mode and what the build ships".to_owned())
    }
    fn dirty(&self) -> bool {
        ModSettingsDoc::dirty(self)
    }
    fn toolbar_status(&self) -> Option<String> {
        Some(self.save_status().0)
    }

    fn toolbar<'a>(&'a mut self, tb: &mut gz_ui::Toolbar<'_, 'a>, _cx: &mut DocCx<'_>) {
        // the fields save as they are edited; only a picture waits for a Save
        if !self.picture_waits() {
            return;
        }
        tb.group(2);
        if tb.action(
            "save",
            Icon::Save,
            "Save",
            "Save the picture into the module (Ctrl+S)",
            !self.picture.making,
            false,
            Kind::Primary,
        ) {
            self.inbox.lock().save_asked = true;
        }
        let (status, tone) = self.save_status();
        tb.group(1);
        tb.status(&status, tone);
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        crate::cards::ensure(cx.editor);
        let ctx = ui.ctx().clone();
        self.take_inbox(Some(&ctx));
        if self.editor.project_opt().is_none() {
            gz_ui::state::empty_state(
                ui,
                Icon::Settings,
                "No project",
                "Mod Settings edit the mod.ltx of the open project.",
                None,
            );
            return;
        }
        self.sync();
        // the scripts are read when the tab opens (on the pools), not when Commands is picked
        if !self.scripts_asked && self.exists && self.base_rev.is_some() {
            self.scripts_asked = true;
            self.load_commands();
        }
        if self.ask_updates {
            self.ask_updates = false;
            // put up over MCP to be looked at: it shows whatever -nodlg says
            self.ask_game_updates(&ctx, Asker::Ui);
        }
        self.recheck();

        let full = ui.max_rect();
        let nav_w = NAV_W.min(full.width() * 0.3).max(1.0);
        let body_bottom = full.max.y;
        let nav = Rect::from_min_max(full.min, pos2(full.min.x + nav_w, body_bottom));
        let mut left = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-mod.settings.nav")
                .max_rect(nav)
                .layout(Layout::top_down(Align::Min)),
        );
        self.draw_nav(&mut left, nav_w, nav.height());

        let page = Rect::from_min_max(
            pos2((nav.max.x + 8.0).min(full.max.x), full.min.y),
            pos2(full.max.x, body_bottom),
        );
        let mut right = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-mod.settings.page")
                .max_rect(page)
                .layout(Layout::top_down(Align::Min)),
        );
        ScrollArea::vertical()
            .id_salt(("gz-ui-mod.settings.scroll", self.page))
            .auto_shrink([false, false])
            .show(&mut right, |ui| {
                egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 8)).show(ui, |ui| {
                    let column = ui.available_width().clamp(1.0, gz_ui::metrics::FORM_W);
                    ui.set_max_width(column);
                    ui.set_min_width(column);
                    self.draw_page(ui);
                });
            });
        ui.allocate_rect(full, egui::Sense::hover());

        self.run_actions(cx);
        self.recheck();
        self.save_when_idle(&ctx);
    }

    fn save(&mut self, _cx: &mut DocCx<'_>) -> Result<(), String> {
        self.save_now()
    }

    fn save_failed(&mut self) -> Option<gz_editor::CmdError> {
        // the save's answer is taken here too: the tab may not be the one drawn
        self.take_inbox(None);
        self.save_error.take().map(|e| gz_editor::CmdError::new("failed", e))
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
        if let Some(p) = self.picture.pending.take() {
            self.editor.exec.spawn_io(move || {
                let _ = std::fs::remove_file(p);
            });
        }
        self.picture.remove = false;
        self.picture.made += 1;
        self.form = self.base.clone();
        self.failed = None;
        self.status.clear();
    }

    fn on_activate(&mut self, _cx: &mut DocCx<'_>) {
        self.set_page(self.page);
    }

    fn mcp_state(&self) -> Value {
        self.view_json(false)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The `mod.view` answer.
#[must_use]
pub fn view_json(open: bool, active: bool, page: usize, dirty: bool, problems: &[Problem]) -> Value {
    let pages: Vec<Value> = PAGES
        .iter()
        .enumerate()
        .map(|(i, p)| json!({ "id": p.id, "label": p.label, "state": form::page_tone(problems, i).map_or("ok", form::tone_name) }))
        .collect();
    let problems: Vec<Value> = problems
        .iter()
        .map(|p| {
            json!({
                "page": PAGES.get(p.page).map_or("", |x| x.id),
                "field": p.field,
                "tone": form::tone_name(p.tone),
                "blocks_save": p.blocks,
                "text": p.text,
            })
        })
        .collect();
    json!({
        "tab": TAB,
        "open": open,
        "active": active,
        "page": PAGES.get(page).map_or("general", |p| p.id),
        "dirty": dirty,
        "pages": pages,
        "problems": problems,
    })
}

/// What `mod.view show=` puts up: the four Add Module lists of the Relations page and the
/// Function list of the first command.
pub(crate) const SHOW_FUNCTION: &str = "function";
pub(crate) const RELATION_SHOWS: [&str; 4] =
    ["add_requires", "add_load_after", "add_load_before", "add_conflicts"];

fn picker_id(relation: usize) -> Id {
    Id::unique(("gz-ui-mod.pick_module", relation))
}

/// The refusal of a `show=` that names no list.
pub(crate) fn show_error() -> String {
    let all: Vec<&str> = RELATION_SHOWS.iter().copied().chain([SHOW_FUNCTION]).collect();
    format!("show must be one of: {}", all.join(", "))
}

fn function_picker_id(command: usize) -> Id {
    Id::unique(("gz-ui-mod.pick_function", command))
}

fn relation_lines(f: &Form, i: usize) -> &Vec<String> {
    match i {
        0 => &f.requires,
        1 => &f.after,
        2 => &f.before,
        _ => &f.conflicts,
    }
}

fn relation_lines_mut(f: &mut Form, i: usize) -> &mut Vec<String> {
    match i {
        0 => &mut f.requires,
        1 => &mut f.after,
        2 => &mut f.before,
        _ => &mut f.conflicts,
    }
}

/// "Name  (id)" for a module the linked game has, the id alone otherwise.
fn module_label(installed: &[Installed], id: &str) -> String {
    match installed.iter().find(|m| m.id == id) {
        Some(m) if !m.name.is_empty() && m.name != id => format!("{}  ({id})", m.name),
        _ => id.to_owned(),
    }
}

/// A row's flags with Invalid when one of its problems is an error.
fn flags(problems: &[Problem], field: &str, base: PropFlags) -> PropFlags {
    if problems.iter().any(|p| p.field == field && p.tone == Tone::Error) {
        base | PropFlags::INVALID
    } else {
        base
    }
}

/// The sentences of a row's problems under it.
fn messages(g: &mut Grid<'_>, problems: &[Problem], field: &str) {
    for p in problems.iter().filter(|p| p.field == field) {
        g.message(p.tone, &p.text);
    }
}

/// Every module of the linked game - `modules\`, then the JSGME `mods\` that still holds what was
/// installed before the split - by its manifest. Blocking.
#[must_use]
pub fn installed_modules(files: &gz_vfs::GameFiles) -> Vec<Installed> {
    let mut out: Vec<Installed> = Vec::new();
    for m in files.modules() {
        // the manifest reads are limited to 1 MB from the disk
        let Some(size) = m.get("mod.ltx").map(|f| f.size) else { continue };
        if size == 0 || size >= 1 << 20 {
            continue;
        }
        let Ok(bytes) = m.read("mod.ltx") else { continue };
        let text = gz_base::text::decode_guess(&bytes);
        let manifest = Manifest::parse(&ManifestDoc::parse(&text));
        let id = form::relation_id(if manifest.id.is_empty() { &m.id } else { &manifest.id });
        if !id.is_empty() && !out.iter().any(|o| o.id == id) {
            out.push(Installed { id, name: manifest.name });
        }
    }
    out.sort_by_key(|m| if m.name.is_empty() { m.id.to_lowercase() } else { m.name.to_lowercase() });
    out
}

/// The picture a thumbnail file holds, as egui takes it; `None` when it does not read. Blocking.
fn read_picture(path: &Path) -> Option<ColorImage> {
    let size = std::fs::metadata(path).ok()?.len();
    if size > PICTURE_READ_LIMIT {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let ext = path.extension().and_then(|e| e.to_str());
    let loaded = gz_fmt_tex::load_image(&bytes, ext).ok()?;
    let img = loaded.image;
    Some(ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.rgba))
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.document(TAB, |editor, req| {
        if editor.project_opt().is_none() {
            return Err("no project is open".to_owned());
        }
        let mut doc = ModSettingsDoc::new(editor);
        if let Some(page) = req.arg.as_deref().and_then(form::page_index) {
            doc.set_page(page);
        }
        Ok(Box::new(doc))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_names_every_page() {
        let v = view_json(false, false, 0, false, &[]);
        let ids: Vec<&str> =
            v["pages"].as_array().into_iter().flatten().filter_map(|p| p["id"].as_str()).collect();
        assert_eq!(
            ids,
            ["general", "mod_browser", "game_mode", "relations", "build_contents", "pda", "commands"]
        );
        assert!(v["pages"].as_array().into_iter().flatten().all(|p| p["state"] == "ok"));
        assert_eq!(v["tab"], "mod_settings");
    }

    #[test]
    fn a_page_takes_its_worst_tone() {
        let p = |tone| Problem {
            page: form::MODE,
            field: "mode".into(),
            tone,
            blocks: false,
            text: String::new(),
        };
        let v = view_json(true, true, form::MODE, false, &[p(Tone::Warning), p(Tone::Required)]);
        assert_eq!(v["pages"][form::MODE]["state"], "required");
        assert_eq!(v["problems"][0]["page"], "game_mode");
        assert_eq!(v["page"], "game_mode");
    }

    #[test]
    fn module_labels_name_what_the_game_has() {
        let list = [Installed { id: "od".into(), name: "Overnight Delivery".into() }];
        assert_eq!(module_label(&list, "od"), "Overnight Delivery  (od)");
        assert_eq!(module_label(&list, "x"), "x");
    }
}
