//! Mod Settings: the `mod_settings` singleton tab (docs/UI_DESIGN.md, section 6, "Mod Settings").
//!
//! A nav of 200 pt - one row per page with its icon and the dot of its worst problem - and the
//! page: its title, what it is for, then one label-left grid of up to 44 em with each field's
//! problem as a sentence under it. The tab saves as it is edited: once no field is being typed
//! in and nothing blocks, the form's changes go out as `mod_set_manifest` / `mod_commands` /
//! `pda_use` (the footer says so); a change of `mod.ltx` made elsewhere reaches every field the
//! author has not touched. The C++ specification is `EditorModManifest.cpp` from
//! `DrawGameUpdatesRow` to `ShowSettingsTab`, `EditorGameModes` (the Game Mode picker) and the
//! picture and tags rows.

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
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{
    Button, Font, Icon, Kind, NavRowSpec, PropFlags, PropertyGrid, Tone, nav_row, palette, segmented,
};
use parking_lot::Mutex;

use crate::call::{self, Pick, Picking, args, s};
use crate::form::{self, Aux, Form, Kind as TargetKind, PAGES, Problem};
use crate::local::{
    PickList, PickRow, local_check, local_pick_list, local_picture, local_text_field, local_value_column,
};

/// The tab's id and kind.
pub const TAB: &str = "mod_settings";

/// Each page's icon, in [`PAGES`]' order.
const PAGE_ICONS: [Icon; 7] =
    [Icon::Info, Icon::Globe, Icon::Mode, Icon::Relations, Icon::Exclude, Icon::PdaInterface, Icon::Terminal];

const ROLE: &str = "mod_settings";
const LABEL_FRAC: f32 = 0.28;
const PICTURE_SIDE: f32 = 96.0;
const FOOTER_H: f32 = 48.0;
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
    saved: Option<Result<(), String>>,
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
    /// Moves when a picture was made: the file of the same name is read again.
    made: u64,
}

/// What a page asks of the shell once it is drawn.
enum Action {
    OpenPda(String),
    CreatePda,
    RescanModes,
    ReadScripts,
    ChoosePicture,
    RemovePicture,
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
    inbox: Arc<Mutex<Inbox>>,
    modes: Option<Arc<ModeScan>>,
    installed: Option<Vec<Installed>>,
    gated: (usize, usize),
    search: String,
    pick_focus: Option<Id>,
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
            inbox: Arc::new(Mutex::new(Inbox::default())),
            modes: None,
            installed: None,
            gated: (0, 0),
            search: String::new(),
            pick_focus: None,
            picture: PictureView::default(),
            ask_updates: false,
            service_asked: false,
            actions: Vec::new(),
        };
        doc.sync();
        doc
    }

    // ---- the data ------------------------------------------------------------------------------

    /// The form as `mod.ltx` says it, the reads of the checks started (`LoadSettingsForm`).
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
        if self.page == form::RELATIONS {
            self.load_installed();
        }
    }

    /// Follows the store: the first snapshot fills the form, a later one reaches every field the
    /// author has not touched (`SyncSettingsForm`). A frame only compares revisions.
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
            self.picture.making = false;
            self.picture.made += 1;
            self.aux.picture_error = result.err().unwrap_or_default();
            self.aux_gen += 1;
        }
        if let Some(result) = saved {
            self.save_answered(result);
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

    /// The project's script functions and the linked game's other commands, read when Commands
    /// is picked or Refresh is pressed.
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
        let key = if self.thumbnail.is_empty() {
            String::new()
        } else {
            format!("{}\\{}|{}", self.root, self.thumbnail, self.picture.made)
        };
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
        let path = format!("{}\\{}", self.root, self.thumbnail);
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            let image = ed.exec.io(move || read_picture(Path::new(&path))).await;
            inbox.lock().picture = Some((key, image));
            call::wake_ui(&ed);
        });
    }

    /// A picture the author picked becomes `thumbnail.dds` (`mod_set_manifest thumbnail=`).
    fn set_picture(&mut self, path: &str) {
        self.picture.making = true;
        let inbox = self.inbox.clone();
        call::call_then(&self.editor, "mod_set_manifest", args([("thumbnail", s(path))]), move |reply, _| {
            inbox.lock().picture_done =
                Some(if call::ok(reply) { Ok(()) } else { Err(call::error_text(reply)) });
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
        self.form != self.base
    }

    /// Opens a page; what its pickers and checks read is read now (`kSettingsPages` click).
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

    /// `mod_settings_view ask_game_updates=true`: the question goes up on the tab's next frame.
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
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let origin = self.editor.ui_origin();
        let _task = self.editor.exec.spawn(async move {
            let mut calls: Vec<(&'static str, gz_editor::Args)> = Vec::new();
            if let Some(a) = plan.manifest {
                calls.push(("mod_set_manifest", a));
            }
            calls.extend(plan.commands.into_iter().map(|a| ("mod_commands", a)));
            if let Some(pda) = plan.pda {
                calls.push(("pda_use", args([("path", s(&pda))])));
            }
            let mut result = Ok(());
            for (name, a) in calls {
                let reply = ed.call(name, a, origin).await;
                if !call::ok(&reply) {
                    result = Err(call::error_text(&reply));
                    break;
                }
            }
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
                self.status = e;
                self.failed = Some(sent);
            }
        }
    }

    /// Saves once nothing is being typed in and nothing blocks (`DrawSettingsBody`).
    fn save_when_idle(&mut self, ctx: &egui::Context) {
        if !self.dirty() || self.saving.is_some() || self.failed.as_ref() == Some(&self.form) {
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

    /// The `mod_settings_view` answer of the open tab.
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
        match self.page {
            form::GENERAL => self.general_page(ui),
            form::BROWSER => self.browser_page(ui),
            form::MODE => self.mode_page(ui),
            form::RELATIONS => self.relations_page(ui),
            form::CONTENTS => self.contents_page(ui),
            form::PDA => self.pda_page(ui),
            _ => self.commands_page(ui),
        }
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
                |ui| local_text_field(ui, "name", &mut form.name, ""),
            );
            g.row("Version", "1 to 4 numbers separated by dots. The game compares versions to offer an update.", flags(problems, "version", PropFlags::REQUIRED), 0, |ui| {
                local_text_field(ui, "version", &mut form.version, "1.0.0")
            });
            messages(g, problems, "version");
            g.row("Author", "Shown in the game's Mods menu.", PropFlags::NONE, 0, |ui| local_text_field(ui, "author", &mut form.author, ""));
            let _ = g.block(|ui| text_block(ui, "Description", "Shown in the game's Mods menu. Plain text or a string table id.", &mut form.description, 4));
            g.row("Website", "The mod's page on AP-PRO or ModDB. The game opens it from the Mods menu.", flags(problems, "website", PropFlags::NONE), 0, |ui| {
                local_text_field(ui, "website", &mut form.website, "https://ap-pro.ru/stuff/...")
            });
            messages(g, problems, "website");
            let repo = g.row(
                "GitHub Repository",
                "owner/repo. The game checks the latest release of this repository for updates. A pasted address is fine: it is kept as owner/repo.",
                flags(problems, "github", PropFlags::NONE),
                0,
                |ui| local_text_field(ui, "github", &mut form.github, "owner/repo"),
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
                    |ui| local_text_field(ui, "id", &mut form.id, "my_mod"),
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
                                if picture.making {
                                    gz_ui::state::busy_line(ui, "Making thumbnail.dds...");
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
                                    if !thumbnail.is_empty() && Button::new("Remove").kind(Kind::Ghost).enabled(!picture.making, why).show(ui).clicked() {
                                        actions.push(Action::RemovePicture);
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
                                if local_check(ui, tag, tag_label(tag), &mut on, cell) {
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
                        |ui| local_text_field(ui, "pmode_id", &mut form.pmode_id, "my_mode"),
                    );
                    messages(g, problems, "pmode_id");
                    g.row(
                        "Mode Title",
                        "What the new-game screen lists the mode as. It names the mode, not the mod: the Mods menu shows the Name on General instead.",
                        flags(problems, "pmode_title", PropFlags::NONE),
                        0,
                        |ui| local_text_field(ui, "pmode_title", &mut form.pmode_title, ""),
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
                let out = g.row(label, help, flags(problems, field, PropFlags::NONE), 0, |ui| {
                    local_value_column(ui, field, |ui| {
                        let refs: Vec<&str> = shown.iter().map(String::as_str).collect();
                        let removed = if refs.is_empty() { None } else { chips(ui, field, &refs, true) };
                        let resp = Button::new("Add Module...").show(ui);
                        (removed, resp.clicked(), resp.rect)
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
            self.search.clear();
            Popup::open_id(&ctx, id);
            self.pick_focus = Some(id);
        }
        for (i, anchor) in anchors.iter().enumerate() {
            let Some(anchor) = *anchor else { continue };
            let popup = picker_id(i);
            let focus = self.pick_focus.take_if(|f| *f == popup).is_some();
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
            let spec = PickList { popup, anchor, layer, hint: "Search, or type a module id", empty, focus };
            let picked = local_pick_list(&ctx, &spec, &mut self.search, |search| {
                let is_taken = |id: &str| id == me || taken.iter().any(|t| t == id);
                let mut rows: Vec<PickRow> = installed
                    .iter()
                    .filter(|m| !is_taken(&m.id))
                    .map(|m| (m, module_label(installed, &m.id)))
                    .filter(|(_, label)| gz_ui::search::matches(search, label))
                    .map(|(m, label)| PickRow { value: m.id.clone(), label, tip: String::new() })
                    .collect();
                let typed = form::relation_id(search);
                if !typed.is_empty()
                    && !installed.iter().any(|m| m.id == typed)
                    && !is_taken(&typed)
                    && validate_id(&typed)
                {
                    rows.push(PickRow {
                        value: typed.clone(),
                        label: format!("Add {typed}"),
                        tip: "Not installed in the linked game: the id is taken as typed.".to_owned(),
                    });
                }
                rows
            });
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
        let mut remove = None;
        let mut pick_at: Vec<(usize, Rect)> = Vec::new();
        let mut opened = None;
        let mut rescan = false;
        {
            let Self { form, problems, .. } = self;
            for (i, c) in form.commands.iter_mut().enumerate() {
                ui.push_id(("command", i), |ui| {
                    let title = if c.name.is_empty() { "New Command".to_owned() } else { c.name.clone() };
                    PropertyGrid::new(ROLE).label_frac(LABEL_FRAC).show(ui, |g| {
                        g.section(&title, true, c.cheat.then_some("cheat"), |g| {
                            let f = |part: &str| format!("commands.{i}.{part}");
                            g.row(
                                "Name",
                                "What the player types in the console: a-z, 0-9 and _. A name the game has, or that a module loading earlier took, is refused by the game.",
                                flags(problems, &f("name"), PropFlags::REQUIRED),
                                1,
                                |ui| local_text_field(ui, "name", &mut c.name, "my_mod_status"),
                            );
                            if g.trailing(Icon::Delete, "Take the command out") {
                                remove = Some(i);
                            }
                            messages(g, problems, &f("name"));
                            let field = g.row(
                                "Function",
                                "<script>.<function> of the project's gamedata\\scripts. The game calls it with the text typed after the name, the module id and the command's name, and prints the string it returns.",
                                flags(problems, &f("function"), PropFlags::REQUIRED),
                                1,
                                |ui| local_text_field(ui, "function", &mut c.function, "my_mod.status"),
                            );
                            if let Some(resp) = &field {
                                if resp.changed() {
                                    c.declared = true;
                                }
                                pick_at.push((i, resp.rect));
                            }
                            if g.trailing(Icon::ChevronDown, "Pick a function of the project's scripts") {
                                opened = Some(i);
                            }
                            if g.trailing(Icon::Refresh, "Read the project's scripts again") {
                                rescan = true;
                            }
                            messages(g, problems, &f("function"));
                            g.row("Help", "The line the console's help shows for it. Plain text or a string table id.", flags(problems, &f("help"), PropFlags::NONE), 1, |ui| {
                                local_text_field(ui, "help", &mut c.help, "")
                            });
                            messages(g, problems, &f("help"));
                            g.row(
                                "Arguments",
                                "What it takes after the name, e.g. <night 1-7>: shown by help and as the hint while typing. The function reads them from its first argument (xms.command_args splits them into words).",
                                flags(problems, &f("args"), PropFlags::NONE),
                                1,
                                |ui| local_text_field(ui, "args", &mut c.args, "none"),
                            );
                            messages(g, problems, &f("args"));
                            g.row("Cheat", "Runs only in Dead Air's debug mode (the game started with -dbg); otherwise the console says so and nothing runs.", PropFlags::INLINE, 1, |ui| {
                                let _ = switch(ui, &mut c.cheat);
                            });
                        });
                    });
                });
            }
        }
        if let Some(i) = remove
            && i < self.form.commands.len()
        {
            self.form.commands.remove(i);
        }
        if rescan {
            self.actions.push(Action::ReadScripts);
        }
        if self.form.commands.is_empty() {
            let _ = text::prose(
                ui,
                "No commands. A command is a line of [commands] in mod.ltx: the name the player types and the function it calls.",
            );
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add_space(6.0);
            if Button::new("Add Command").show(ui).clicked() {
                self.form.commands.push(Command::default());
            }
        });
        if let Some(i) = opened {
            let id = function_picker_id(i);
            self.search.clear();
            Popup::open_id(&ctx, id);
            self.pick_focus = Some(id);
        }
        for (i, anchor) in pick_at {
            let popup = function_picker_id(i);
            let focus = self.pick_focus.take_if(|f| *f == popup).is_some();
            let functions = &self.aux.functions;
            let empty = if functions.is_empty() {
                "The project's gamedata\\scripts has no function yet: write one, then pick it here."
            } else {
                "No function matches."
            };
            let spec =
                PickList { popup, anchor, layer, hint: "Search the project's functions", empty, focus };
            let picked = local_pick_list(&ctx, &spec, &mut self.search, |search| {
                functions
                    .iter()
                    .filter(|f| gz_ui::search::matches(search, f))
                    .map(|f| PickRow { value: f.clone(), label: f.clone(), tip: String::new() })
                    .collect()
            });
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
                Action::CreatePda => {
                    let safe: String = if self.form.id.is_empty() {
                        "app".to_owned()
                    } else {
                        self.form
                            .id
                            .chars()
                            .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
                            .collect()
                    };
                    let title = if self.form.name.is_empty() { safe.clone() } else { self.form.name.clone() };
                    let ed = self.editor.clone();
                    call::call_then(
                        &self.editor,
                        "pda_write",
                        args([("id", s(&safe)), ("title", s(&title)), ("use", Value::Bool(true))]),
                        move |reply, ctx| {
                            if call::ok(reply) {
                                call::open_tab(&ed, "pda");
                            } else {
                                gz_ui::state::toast(ctx, Tone::Error, &call::error_text(reply));
                            }
                        },
                    );
                }
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
                Action::RemovePicture => self.set_picture(""),
            }
        }
    }

    fn draw_footer(&self, ui: &mut Ui, rect: Rect) {
        let p = palette(ui.ctx());
        ui.painter()
            .line_segment([rect.left_top(), rect.right_top()], egui::Stroke::new(1.0, p.border_subtle));
        let (status, tone) = match form::blocking(&self.problems).0 {
            Some(first) => {
                (format!("{}: {}", PAGES.get(first.page).map_or("", |p| p.label), first.text), Tone::Error)
            }
            None if !self.status.is_empty() => (self.status.clone(), Tone::Error),
            None if !self.exists => ("No mod.ltx yet: the first edit makes it.".to_owned(), Tone::Muted),
            None => ("Saved to mod.ltx as you edit".to_owned(), Tone::Muted),
        };
        let r = Rect::from_min_max(pos2(rect.min.x, rect.min.y + 4.0), rect.max);
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-mod.settings.footer")
                .max_rect(r)
                .layout(Layout::top_down(Align::Min)),
        );
        let mut tb = gz_ui::Toolbar::begin(&mut child, "mod_settings_footer", gz_ui::ToolbarStyle::Row);
        tb.group(1);
        tb.status(&status, tone);
        tb.end();
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
        Icon::Settings
    }
    fn tooltip(&self) -> Option<String> {
        Some("mod.ltx: the module's name, version, game mode and what the build ships".to_owned())
    }
    fn dirty(&self) -> bool {
        ModSettingsDoc::dirty(self)
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
        if self.ask_updates {
            self.ask_updates = false;
            // put up over MCP to be looked at: it shows whatever -nodlg says
            self.ask_game_updates(&ctx, Asker::Ui);
        }
        self.recheck();

        let full = ui.max_rect();
        let nav_w = NAV_W.min(full.width() * 0.3).max(1.0);
        let body_bottom = (full.max.y - FOOTER_H).max(full.min.y + 1.0);
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
                    let column = ui.available_width().min(em(44.0)).max(1.0);
                    ui.set_max_width(column);
                    ui.set_min_width(column);
                    self.draw_page(ui);
                });
            });
        let footer = Rect::from_min_max(pos2(full.min.x, body_bottom), full.max);
        self.draw_footer(ui, footer);
        ui.allocate_rect(full, egui::Sense::hover());

        self.run_actions(cx);
        self.recheck();
        self.save_when_idle(&ctx);
    }

    fn save(&mut self, _cx: &mut DocCx<'_>) -> Result<(), String> {
        self.save_now()
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
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

/// The `mod_settings_view` answer (`McpSettingsView`).
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

fn picker_id(relation: usize) -> Id {
    Id::unique(("gz-ui-mod.pick_module", relation))
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

/// "Name  (id)" for a module the linked game has, the id alone otherwise (`ModuleLabel`).
fn module_label(installed: &[Installed], id: &str) -> String {
    match installed.iter().find(|m| m.id == id) {
        Some(m) if !m.name.is_empty() && m.name != id => format!("{}  ({id})", m.name),
        _ => id.to_owned(),
    }
}

/// A row's flags with Invalid when one of its problems is an error (`RowFlags`).
fn flags(problems: &[Problem], field: &str, base: PropFlags) -> PropFlags {
    if problems.iter().any(|p| p.field == field && p.tone == Tone::Error) {
        base | PropFlags::INVALID
    } else {
        base
    }
}

/// The sentences of a row's problems under it (`RowMessages`).
fn messages(g: &mut Grid<'_>, problems: &[Problem], field: &str) {
    for p in problems.iter().filter(|p| p.field == field) {
        g.message(p.tone, &p.text);
    }
}

/// Every module of the linked game - `modules\`, then the JSGME `mods\` that still holds what was
/// installed before the split - by its manifest (`ScanInstalledModules`). Blocking.
#[must_use]
pub fn installed_modules(files: &gz_vfs::GameFiles) -> Vec<Installed> {
    let mut out: Vec<Installed> = Vec::new();
    for m in files.modules() {
        // the C++ read manifests up to 1 MB from the disk
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
