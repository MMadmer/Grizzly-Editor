//! World Rules: the `world_rules` singleton tab (docs/UI_DESIGN.md, section 6, "World Rules").
//!
//! What the module's mode keeps of the original game, where the player starts and with what -
//! on the surface; where the rules apply, the playable area and the transitions that start closed
//! - under one folded More. The tab is saved as it is edited: every edit is a `world_set` call,
//!   sent one after another; `mod.ltx` is the one truth and the tab follows the manifest's store.
//!   The Player Starts and the problems are read again a second apart, on the pools. A problems bar
//!   (and the drawer it opens) shows only when there are problems. The C++ specification is the
//!   World Rules tab of `EditorWorld.cpp` (`DrawRules` .. `DrawWorldBody`, `McpView`).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use egui::{Align, Id, Layout, Rect, ScrollArea, Ui, UiBuilder, Vec2, pos2};
use gz_app::{DocCx, Document, OpenRequest, ShellBuilder};
use gz_base::Rev;
use gz_editor::{Args, Editor, Value, json};
use gz_game::{GameLink, ModeScan};
use gz_mod::{Manifest, ManifestService};
use gz_project::Project;
use gz_ui::dialog::{Answer, ConfirmSpec, confirm};
use gz_ui::grid::chips;
use gz_ui::list::{Column, table_headers};
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{
    Button, Font, Icon, Kind, PropFlags, PropertyGrid, SearchField, Tone, Toolbar, icon_button, metrics,
    palette, segmented,
};
use gz_world::{
    GameWorld, KitItem, MAX_PLAYER_CHOICES, RULES, SceneStarts, Start, StartKit, World, WorldService,
};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::call::{self, args, s};
use crate::lists::{Lists, ListsView};
use crate::local::{
    drawer_height, local_cell, local_cell_check, local_check, local_level_picker, local_number_text,
    local_problems_drawer, local_section, open_level_picker, set_section_open,
};
use crate::picker::{item_picker, local_combo};

/// The tab's id and kind.
pub const TAB: &str = "world_rules";
/// What the toolbar says while every edit is in mod.ltx.
pub const SAVED: &str = "Saved to mod.ltx as you edit";
/// The More section's key.
const MORE: &str = "world.more";
const HOST: &str = crate::characters::HOST;
const COLUMN_W: f32 = 900.0;
const TABLE_ROW: f32 = 26.0;

/// The five rules as the author reads them (`kRules`).
const RULE_TEXT: [(&str, &str); 5] = [
    (
        "Original characters",
        "Traders, story characters and every squad the game placed by name (a story id). Off: they are not in the world at all.",
    ),
    (
        "Random stalkers",
        "The simulation's stalker squads: the ones placed at the start and the ones smart terrains bring back.",
    ),
    ("Mutants", "The simulation's monster squads, placed at the start and respawned."),
    (
        "Jobs from NPCs",
        "\"Got any work?\" - the jobs squad leaders hand out: fetch, kill, escort. Off: the offer is not in their dialogs.",
    ),
    (
        "Story quests",
        "Tasks of the named characters and the storylines. Off: their offers are not in the dialogs.",
    ),
];

/// "W203 world: locked names 'x', ..." as the sentence the author reads (`ProblemLine`).
#[must_use]
pub fn problem_line(problem: &str) -> String {
    let mut text_ = match problem.find(": ") {
        Some(at) => problem.get(at + 2..).unwrap_or("").to_owned(),
        None => problem.to_owned(),
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

/// What the rules shape, in one sentence (`ModeText`).
#[must_use]
pub fn mode_text(m: &Manifest, modes: Option<&ModeScan>) -> String {
    if let Some(p) = m.provides_modes.first() {
        return format!(
            "These rules shape the module's own mode '{}'.",
            if p.title.is_empty() { &p.id } else { &p.title }
        );
    }
    if !m.mode.is_empty() && m.mode != "*" {
        let caption = modes
            .and_then(|s| s.modes.iter().find(|x| x.id == m.mode))
            .map_or(m.mode.as_str(), |x| x.caption.as_str());
        return format!("These rules shape the mode '{caption}'.");
    }
    if m.target.eq_ignore_ascii_case("default") {
        return "These rules shape every new game: the module targets the plain game.".to_owned();
    }
    if m.mode == "*" {
        return "These rules shape every new game: the module targets every mode.".to_owned();
    }
    "No game mode is chosen yet: pick one in Mod Settings, Game Mode.".to_owned()
}

/// A kit's items as `world_set start_kit=` takes them: `section` or `section=N`.
#[must_use]
pub fn kit_list(items: &[KitItem]) -> String {
    items
        .iter()
        .filter(|i| !i.section.is_empty())
        .map(|i| if i.count == 1 { i.section.clone() } else { format!("{}={}", i.section, i.count) })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The calls of the tab, sent one after another so that they land in the order made.
#[derive(Default)]
struct Outbox {
    queue: Mutex<VecDeque<Args>>,
    running: AtomicBool,
}

impl Outbox {
    fn busy(&self) -> bool {
        self.running.load(Ordering::Acquire) || !self.queue.lock().is_empty()
    }
}

/// The linked game's world as read at a revision: the world, the level captions, the modes.
type GameRead = (Rev, Arc<GameWorld>, Arc<FxHashMap<String, String>>, Option<Arc<ModeScan>>);

/// What tasks leave for the tab; taken at the start of its next frame.
#[derive(Default)]
struct Inbox {
    /// A save's answer: empty when it went in.
    status: Option<String>,
    checked: Option<(Vec<Start>, Vec<String>, String)>,
    game: Option<GameRead>,
    empty_yes: bool,
}

/// What the drawing asked for, done once it is drawn.
enum Action {
    Send(Args),
    SetKit,
    BaseKit,
    PlaceStart,
    Focus(String),
    OpenSettings,
    AskEmpty,
}

/// The World Rules tab.
pub struct WorldDoc {
    editor: Arc<Editor>,
    root: String,
    rev: Option<Rev>,
    have_manifest: bool,
    manifest: Option<Manifest>,
    world: World,
    kit: StartKit,
    area_limited: bool,
    levels_limited: bool,
    status: String,
    trans_level: String,
    trans_filter: String,
    drawer_open: bool,
    drawer_errors: usize,
    starts: Vec<Start>,
    problems: Vec<String>,
    lines: Vec<(Tone, String)>,
    scene_level: String,
    checked_at: Option<Instant>,
    checking: bool,
    game: Option<Arc<GameWorld>>,
    captions: Arc<FxHashMap<String, String>>,
    modes: Option<Arc<ModeScan>>,
    game_rev: Option<Rev>,
    loading_game: bool,
    game_gen: u64,
    /// The linked game's levels with their captions, for the level pickers.
    levels: Vec<(String, String)>,
    /// The levels that have transitions: (name, "label  (n)").
    level_rows: Vec<(String, String)>,
    /// The rows of the transitions table: (level, filter, game generation, rows).
    trans_memo: (String, String, u64, Vec<usize>),
    kit_dirty: bool,
    service_asked: bool,
    outbox: Arc<Outbox>,
    inbox: Arc<Mutex<Inbox>>,
    lists: ListsView,
    actions: Vec<Action>,
    /// How far the page scrolled at most, the last frame: where `ui_scroll to=bottom` goes.
    scroll_max: f32,
}

impl WorldDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>) -> Self {
        Self {
            editor: editor.clone(),
            root: String::new(),
            rev: None,
            have_manifest: false,
            manifest: None,
            world: World::default(),
            kit: StartKit::default(),
            area_limited: false,
            levels_limited: false,
            status: String::new(),
            trans_level: String::new(),
            trans_filter: String::new(),
            drawer_open: false,
            drawer_errors: 0,
            starts: Vec::new(),
            problems: Vec::new(),
            lines: Vec::new(),
            scene_level: String::new(),
            checked_at: None,
            checking: false,
            game: None,
            captions: Arc::new(FxHashMap::default()),
            modes: None,
            game_rev: None,
            loading_game: false,
            game_gen: 0,
            levels: Vec::new(),
            level_rows: Vec::new(),
            trans_memo: (String::new(), String::new(), u64::MAX, Vec::new()),
            kit_dirty: false,
            service_asked: false,
            outbox: Arc::new(Outbox::default()),
            inbox: Arc::new(Mutex::new(Inbox::default())),
            lists: ListsView::default(),
            actions: Vec::new(),
            scroll_max: 0.0,
        }
    }

    // ---- the data -------------------------------------------------------------------------------

    /// mod.ltx is the one truth: the tab follows the manifest's store whenever nothing of its own
    /// is on the way there (`RefreshWorld`). A frame only compares revisions.
    fn sync(&mut self) {
        let Some(session) = self.editor.project_opt() else { return };
        let root = session.root.to_string_lossy().into_owned();
        let new_project = root != self.root;
        if new_project {
            self.root = root;
            self.rev = None;
            self.trans_level.clear();
            self.trans_filter.clear();
            self.drawer_errors = 0;
            self.game_rev = None;
            self.service_asked = false;
        }
        let Some(service) = ManifestService::get(&session) else {
            if !self.service_asked {
                self.service_asked = true;
                let ed = self.editor.clone();
                let _task = self.editor.exec.spawn(async move {
                    if let Ok(service) = ManifestService::of_session(&ed, &session).await {
                        let _ = service.sync().await;
                    }
                    call::wake_ui(&ed);
                });
            }
            return;
        };
        let snap = service.snapshot();
        if self.rev == Some(snap.rev) || self.outbox.busy() || self.kit_dirty {
            return;
        }
        let first = self.rev.is_none();
        self.rev = Some(snap.rev);
        self.have_manifest = snap.exists() && snap.doc().is_some();
        self.manifest = snap.manifest().cloned();
        match snap.doc() {
            Some(doc) => {
                self.world = World::parse(doc);
                self.kit = StartKit::parse(doc);
            }
            None => {
                self.world = World::default();
                self.kit = StartKit::default();
            }
        }
        if first {
            self.area_limited = !self.world.area.is_empty();
            self.levels_limited = !self.world.levels.is_empty();
        } else {
            self.area_limited |= !self.world.area.is_empty();
            self.levels_limited |= !self.world.levels.is_empty();
        }
        self.checked_at = None;
        self.trans_memo.2 = u64::MAX;
    }

    /// The linked game's world, its level captions and its modes, read again when the game moved.
    fn want_game(&mut self) {
        let Ok((_, project)) = Project::of(&self.editor) else { return };
        let rev = project.game_rev();
        if self.loading_game || self.game_rev == Some(rev) {
            return;
        }
        self.loading_game = true;
        self.game_rev = Some(rev);
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            let (game, captions) = match WorldService::of(&ed).await {
                Ok(svc) => (svc.game().await, svc.captions(&ed).await),
                Err(e) => (Arc::new(GameWorld::failed(e.message)), FxHashMap::default()),
            };
            let modes = match GameLink::of(&ed) {
                Ok(link) => Some(link.modes().await),
                Err(_) => None,
            };
            inbox.lock().game = Some((rev, game, Arc::new(captions), modes));
            call::wake_ui(&ed);
        });
    }

    /// The Player Starts and the problems, a second apart: the spawn layer is files, and a tab
    /// must not walk them every frame.
    fn want_check(&mut self) {
        if self.checking || self.checked_at.is_some_and(|t| t.elapsed() < Duration::from_secs(1)) {
            return;
        }
        self.checking = true;
        self.checked_at = Some(Instant::now());
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let (w, k) = (self.world.clone(), self.kit.clone());
        let _task = self.editor.exec.spawn(async move {
            let scene = SceneStarts::of(&ed).get(&ed).map(|(level, _)| level).unwrap_or_default();
            let (starts, problems) = match WorldService::of(&ed).await {
                Ok(svc) => svc.problems(&ed, &w, &k).await,
                Err(_) => (Vec::new(), Vec::new()),
            };
            inbox.lock().checked = Some((starts, problems, scene));
            call::wake_ui(&ed);
        });
    }

    fn take_inbox(&mut self) {
        let mut inbox = self.inbox.lock();
        let status = inbox.status.take();
        let checked = inbox.checked.take();
        let game = inbox.game.take();
        let empty_yes = std::mem::take(&mut inbox.empty_yes);
        drop(inbox);
        if let Some(status) = status {
            self.status = status;
            self.checked_at = None;
            self.relines();
        }
        if let Some((starts, problems, scene)) = checked {
            self.checking = false;
            self.starts = starts;
            self.problems = problems;
            self.scene_level = scene;
            self.relines();
            // a new error opens the drawer: the author should not have to go looking for it
            let errors = self.lines.iter().filter(|(t, _)| *t == Tone::Error).count();
            if errors > self.drawer_errors {
                self.drawer_open = true;
            }
            self.drawer_errors = errors;
        }
        if let Some((rev, game, captions, modes)) = game {
            self.loading_game = false;
            if self.game_rev == Some(rev) {
                self.set_game(game, captions, modes);
            }
        }
        if empty_yes {
            self.world.keep = [false; 5];
            self.send(args([("preset", s("empty"))]));
        }
    }

    fn set_game(
        &mut self,
        game: Arc<GameWorld>,
        captions: Arc<FxHashMap<String, String>>,
        modes: Option<Arc<ModeScan>>,
    ) {
        let caption = |l: &str| captions.get(l).cloned().unwrap_or_default();
        self.levels = game.levels.iter().map(|l| (l.clone(), caption(l))).collect();
        self.level_rows = game
            .levels
            .iter()
            .filter_map(|l| {
                let n = game.transitions_on(l);
                (n > 0).then(|| (l.clone(), format!("{}  ({n})", level_label(&captions, l))))
            })
            .collect();
        self.game = Some(game);
        self.captions = captions;
        self.modes = modes;
        self.game_gen += 1;
    }

    /// The problems bar's lines: the last save's failure, then the problems.
    fn relines(&mut self) {
        let mut lines = Vec::new();
        if !self.status.is_empty() {
            lines.push((Tone::Error, self.status.clone()));
        }
        lines.extend(
            self.problems
                .iter()
                .map(|p| (if p.starts_with('E') { Tone::Error } else { Tone::Warning }, problem_line(p))),
        );
        self.lines = lines;
    }

    // ---- saving ---------------------------------------------------------------------------------

    /// An edit goes to mod.ltx (`Commit`): `world_set` with what changed, after the ones before.
    fn send(&self, a: Args) {
        self.outbox.queue.lock().push_back(a);
        run_outbox(&self.editor, &self.outbox, &self.inbox);
    }

    /// The kit's rows and money as they stand, once nothing is being typed in or dragged.
    fn flush_kit(&mut self, ctx: &egui::Context) {
        if !self.kit_dirty || ctx.egui_wants_keyboard_input() || ctx.dragged_id().is_some() {
            return;
        }
        self.kit_dirty = false;
        let money = if self.kit.money >= 0 { Value::from(self.kit.money) } else { Value::from(-1) };
        self.send(args([
            ("start_kit", s(&kit_list(&self.kit.items))),
            ("start_money", money),
            ("start_perks", Value::Bool(self.kit.perks)),
        ]));
    }

    fn run_actions(&mut self, cx: &mut DocCx<'_>) {
        for action in std::mem::take(&mut self.actions) {
            match action {
                Action::Send(a) => self.send(a),
                Action::SetKit => {
                    // the game's own base kit is where a mode's kit usually starts from
                    let ed = self.editor.clone();
                    let (outbox, inbox) = (self.outbox.clone(), self.inbox.clone());
                    call::call_then(
                        &self.editor,
                        "world_set",
                        args([("start_kit_base", Value::Bool(true))]),
                        move |reply, _| {
                            if !call::ok(reply) {
                                outbox.queue.lock().push_back(args([("start_money", Value::from(0))]));
                                run_outbox(&ed, &outbox, &inbox);
                            }
                        },
                    );
                }
                Action::BaseKit => {
                    let inbox = self.inbox.clone();
                    call::call_then(
                        &self.editor,
                        "world_set",
                        args([("start_kit_base", Value::Bool(true))]),
                        move |reply, ctx| {
                            if call::ok(reply) {
                                inbox.lock().status = Some(String::new());
                            } else {
                                gz_ui::state::toast(
                                    ctx,
                                    Tone::Warning,
                                    "The linked game has no plugins\\faction_loadouts.ltx to read its base kit from.",
                                );
                            }
                        },
                    );
                }
                Action::PlaceStart => {
                    let inbox = self.inbox.clone();
                    call::call_then(
                        &self.editor,
                        "place_spawn",
                        args([("section", s("player_start"))]),
                        move |reply, _| {
                            inbox.lock().status = Some(if call::ok(reply) {
                                String::new()
                            } else {
                                format!("Not placed: {}", call::error_text(reply))
                            });
                        },
                    );
                }
                Action::Focus(name) => {
                    call::call_then(
                        &self.editor,
                        "focus_object",
                        args([("name", s(&name)), ("select", Value::Bool(true))]),
                        |reply, ctx| {
                            if !call::ok(reply) {
                                gz_ui::state::toast(ctx, Tone::Warning, &call::error_text(reply));
                            }
                        },
                    );
                }
                Action::OpenSettings => cx.requests.open(OpenRequest::kind("mod_settings")),
                Action::AskEmpty => self.ask_empty_world(cx.ctx),
            }
        }
    }

    /// Takes everything of the original game out: reversible, but it is five decisions at once,
    /// so it asks (`AskEmptyWorld`).
    fn ask_empty_world(&mut self, ctx: &egui::Context) {
        let kept: Vec<String> = RULE_TEXT
            .iter()
            .zip(self.world.keep)
            .filter(|(_, keep)| *keep)
            .map(|((caption, _), _)| (*caption).to_owned())
            .collect();
        if kept.is_empty() {
            gz_ui::state::toast(
                ctx,
                Tone::Info,
                "The world is empty already: the mode keeps nothing of the original game.",
            );
            return;
        }
        let spec = ConfirmSpec {
            id: "empty_world".to_owned(),
            title: "Empty World".to_owned(),
            headline: "Take everything of the original game out of this mode?".to_owned(),
            items: kept,
            total: 0,
            consequence: "Your own characters and quests stay. It is saved to mod.ltx at once; Keep Everything brings it back.".to_owned(),
            action: "Empty World".to_owned(),
            destructive: false,
        };
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let line = confirm(ctx, call::asker(&self.editor), spec, move |answer| {
            if answer == Answer::Yes {
                inbox.lock().empty_yes = true;
                call::wake_ui(&ed);
            }
        });
        call::log_line(&self.editor, line);
    }

    // ---- MCP ------------------------------------------------------------------------------------

    pub(crate) fn set_drawer(&mut self, open: bool) {
        self.drawer_open = open;
    }

    pub(crate) fn set_level(&mut self, level: &str) {
        level.clone_into(&mut self.trans_level);
    }

    pub(crate) fn set_filter(&mut self, filter: &str) {
        filter.clone_into(&mut self.trans_filter);
    }

    /// The `world_view` answer of the open tab, the problems as the manifest has them.
    #[must_use]
    pub(crate) fn view_json(&self, active: bool, problems: &[String]) -> Value {
        json!({
            "tab": TAB,
            "open": true,
            "active": active,
            "drawer": if self.drawer_open { "open" } else { "close" },
            "level": self.trans_level,
            "filter": self.trans_filter,
            "status": if self.status.is_empty() { SAVED } else { self.status.as_str() },
            "problems": problems,
        })
    }

    // ---- drawing --------------------------------------------------------------------------------

    fn level_label(&self, level: &str) -> String {
        level_label(&self.captions, level)
    }

    fn draw_rules(&mut self, ui: &mut Ui) {
        let choices = self.world.choice_count();
        let total = ui.available_width();
        let (keep_w, choice_w) = (em(5.0), em(8.0));
        let rule_w = (total - keep_w - choice_w).max(em(8.0));
        let columns = [
            Column::new("Rule", rule_w),
            Column { centred: true, ..Column::new("Keep", keep_w) },
            Column { centred: true, ..Column::new("Player Decides", choice_w) },
        ];
        let _ = table_headers(ui, &columns);
        let p = palette(ui.ctx());
        for (i, (caption, hint)) in RULE_TEXT.iter().enumerate() {
            let key = RULES.get(i).copied().unwrap_or("");
            ui.push_id(i, |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(TABLE_ROW);
                    ui.spacing_mut().item_spacing.x = 0.0;
                    local_cell(
                        ui,
                        Vec2::new(rule_w, metrics::FRAME),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.add_space(6.0);
                            let _ = text::label(ui, caption, Font::Body, p.text, Clip::End)
                                .on_hover_text(format!("{hint}\nmod.ltx: {key}"));
                        },
                    );
                    let mut keep = self.world.keep.get(i).copied().unwrap_or(true);
                    let tip = if keep { "Kept in the mode" } else { "Taken out of the mode" };
                    if local_cell_check(ui, &format!("world.keep.{i}"), &mut keep, keep_w, true, tip) {
                        if let Some(k) = self.world.keep.get_mut(i) {
                            *k = keep;
                        }
                        self.actions.push(Action::Send(args([(key, Value::Bool(keep))])));
                    }
                    let mut choice = self.world.player_choice.get(i).copied().unwrap_or(false);
                    let full = !choice && choices >= MAX_PLAYER_CHOICES;
                    let tip = if full {
                        "The new-game screen has room for three: clear another one first."
                    } else {
                        "A checkbox on the new-game screen, set to what Keep says: the player may change it."
                    };
                    if local_cell_check(ui, &format!("world.choice.{i}"), &mut choice, choice_w, !full, tip) {
                        if let Some(c) = self.world.player_choice.get_mut(i) {
                            *c = choice;
                        }
                        self.actions.push(Action::Send(args([(
                            "player_choice",
                            s(&self.world.choices().join(",")),
                        )])));
                    }
                });
            });
        }
        ui.add_space(2.0);
        let _ = text::prose(
            ui,
            &format!("{choices} of {MAX_PLAYER_CHOICES} rules offered on the new-game screen."),
        );
        let all = self.world.keep.iter().all(|k| *k);
        if Button::new("Keep Everything").enabled(!all, "Everything is kept already").show(ui).clicked() {
            self.world.keep = [true; 5];
            let a = args([
                (RULES[0], Value::Bool(true)),
                (RULES[1], Value::Bool(true)),
                (RULES[2], Value::Bool(true)),
                (RULES[3], Value::Bool(true)),
                (RULES[4], Value::Bool(true)),
            ]);
            self.actions.push(Action::Send(a));
        }
    }

    fn draw_start(&mut self, ui: &mut Ui) {
        if self.starts.is_empty() {
            let _ = text::prose(ui, "None: a new game starts where the faction's start puts the player.");
        }
        let p = palette(ui.ctx());
        for (i, st) in self.starts.iter().enumerate() {
            ui.push_id(i, |ui| {
                ui.horizontal(|ui| {
                    if st.source == "scene"
                        && icon_button(
                            ui,
                            Icon::FrameSelection,
                            "Select it in the scene and frame it",
                            false,
                            true,
                        )
                        .clicked()
                    {
                        self.actions.push(Action::Focus(st.name.clone()));
                    }
                    let line = format!(
                        "{}  at {:.1}, {:.1}, {:.1}",
                        level_label(&self.captions, &st.level),
                        st.position[0],
                        st.position[1],
                        st.position[2]
                    );
                    let _ = text::label(ui, &line, Font::Body, p.text, Clip::End)
                        .on_hover_text(format!("{}: {}", st.source, st.name));
                });
            });
        }
        if self.starts.len() > 1 {
            let _ = text::status(
                ui,
                Tone::Error,
                &format!("A module has one Player Start: the build refuses {}.", self.starts.len()),
            );
        }
        ui.add_space(2.0);
        let resp = Button::new("Place in Scene")
            .enabled(!self.scene_level.is_empty(), "Open the level's scene first")
            .tip("In front of the camera; the arrow is where the player faces. The Spawn tool has it too: Player Start.")
            .show(ui);
        if resp.clicked() {
            self.actions.push(Action::PlaceStart);
        }
    }

    /// What a new game of the mode begins with, in place of the new-game screen's loadout
    /// (`DrawStartKit`).
    fn draw_kit(&mut self, ui: &mut Ui) {
        if !self.kit.declared {
            let _ = text::prose(
                ui,
                "None: a new game begins with the loadout picked on the new-game screen - the faction's kit and the extra kits.",
            );
            ui.add_space(2.0);
            if Button::new("Set a Starting Kit").tip("It starts as the linked game's base kit, the one every faction gets, and changes from there.").show(ui).clicked() {
                self.kit.declared = true;
                self.actions.push(Action::SetKit);
            }
            return;
        }
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let _ = text::prose(
            ui,
            "A new game begins with exactly this: what the new-game screen's loadout and extra kits gave is taken away. The PDA and the bolt always stay.",
        );
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let _ = text::label(ui, "Money", Font::Body, p.text_secondary, Clip::End).on_hover_text(
                "The money a new game begins with. Empty: what the new-game screen gives.\nmod.ltx: money",
            );
            ui.add_space(8.0);
            let money = (self.kit.money >= 0).then_some(i64::from(self.kit.money));
            if let Some(v) = local_number_text(ui, "world.money", money, "the screen's", em(8.0)) {
                self.kit.money = v.map_or(-1, |n| i32::try_from(n).unwrap_or(i32::MAX));
                self.kit_dirty = true;
            }
            ui.add_space(16.0);
            let label = "Keep the perks picked on the new-game screen";
            let w = metrics::FRAME + 8.0 + text::width(&ctx, &Font::Body.id(), label);
            let mut perks = self.kit.perks;
            if local_check(ui, "world.perks", label, &mut perks, w) {
                self.kit.perks = perks;
                self.kit_dirty = true;
            }
        });
        ui.add_space(2.0);
        if self.kit.items.is_empty() {
            let _ =
                text::status(ui, Tone::Muted, "No items: the player begins with the PDA and nothing else.");
        } else {
            let total = ui.available_width();
            let (count_w, action_w) = (em(5.0), metrics::ICON_BUTTON);
            let item_w = (total - count_w - action_w - 16.0).max(em(6.0));
            let _ = table_headers(
                ui,
                &[
                    Column::new("Item", item_w + 8.0),
                    Column::new("Count", count_w + 8.0),
                    Column::new("", action_w),
                ],
            );
            let mut remove = None;
            let lists = &self.lists;
            for (i, it) in self.kit.items.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_height(TABLE_ROW);
                        let mut section = it.section.clone();
                        let picked = local_cell(
                            ui,
                            Vec2::new(item_w, metrics::FRAME),
                            Layout::left_to_right(Align::Center),
                            |ui| item_picker(ui, &format!("world.kit.{i}"), &mut section, lists, None),
                        )
                        .inner;
                        if picked && !section.is_empty() {
                            it.section = section;
                            self.kit_dirty = true;
                        }
                        let changed = ui
                            .allocate_ui_with_layout(
                                Vec2::new(count_w, metrics::FRAME),
                                Layout::left_to_right(Align::Center),
                                |ui| gz_ui::grid::integer(ui, &mut it.count, 0.1, "").changed(),
                            )
                            .inner;
                        if changed {
                            it.count = it.count.max(1);
                            self.kit_dirty = true;
                        }
                        if icon_button(ui, Icon::Delete, "Take the row out", false, true).clicked() {
                            remove = Some(i);
                        }
                    })
                    .response
                    .on_hover_text("How many of it; ammo counts in boxes.");
                });
            }
            if let Some(i) = remove
                && i < self.kit.items.len()
            {
                self.kit.items.remove(i);
                self.kit_dirty = true;
            }
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            // a row is born with its item: the tab saves at once, and a row without one would not
            // survive the save
            let mut add = String::new();
            let picked = local_cell(ui, Vec2::new(em(18.0), metrics::FRAME), Layout::left_to_right(Align::Center), |ui| item_picker(ui, "world.kit.add", &mut add, &self.lists, Some("Add an item..."))).inner;
            if picked && !add.is_empty() {
                match self.kit.items.iter_mut().find(|i| i.section == add) {
                    Some(it) => it.count += 1,
                    None => self.kit.items.push(KitItem { section: add, count: 1 }),
                }
                self.kit_dirty = true;
            }
            let resp = Button::new("Game's Base Kit")
                .enabled(self.lists.linked, "Link a game to read its kit")
                .tip("In place of these rows and the money: what every faction starts with in the linked game ([shared] of plugins\\faction_loadouts.ltx).")
                .show(ui);
            if resp.clicked() {
                self.actions.push(Action::BaseKit);
            }
            if Button::new("Remove Kit").kind(Kind::GhostDanger).tip("No kit of the mode's own: the new-game screen's loadout decides again.").show(ui).clicked() {
                self.kit = StartKit::default();
                self.kit_dirty = false;
                self.actions.push(Action::Send(args([("start_kit_remove", Value::Bool(true))])));
            }
        });
    }

    /// Two ways, one list: the first segment empties it (everywhere), the second lets levels be
    /// picked (`Scope`). Returns the list when it changed.
    fn scope(&mut self, ui: &mut Ui, key: &str, labels: [&str; 2], area: bool) {
        let ctx = ui.ctx().clone();
        let mut limited = if area { self.area_limited } else { self.levels_limited };
        let mut pick = usize::from(limited);
        let field = if area { "area" } else { "levels" };
        if ui.push_id(key, |ui| segmented(ui, &mut pick, &labels, None)).inner {
            limited = pick == 1;
            let list = if area { &mut self.world.area } else { &mut self.world.levels };
            if !limited && !list.is_empty() {
                list.clear();
                self.actions.push(Action::Send(args([(field, s(""))])));
            }
        }
        if area {
            self.area_limited = limited;
        } else {
            self.levels_limited = limited;
        }
        if !limited {
            return;
        }
        ui.add_space(2.0);
        let game_ok = self.game.as_ref().is_some_and(|g| g.ok);
        let shown: Vec<String> = {
            let list = if area { &self.world.area } else { &self.world.levels };
            list.iter().map(|l| self.level_label(l)).collect()
        };
        let mut changed = false;
        if !shown.is_empty() {
            let refs: Vec<&str> = shown.iter().map(String::as_str).collect();
            if let Some(r) = chips(ui, key, &refs, true) {
                let list = if area { &mut self.world.area } else { &mut self.world.levels };
                if r < list.len() {
                    list.remove(r);
                    changed = true;
                }
            }
        }
        // names the game does not have: kept, said
        if let Some(g) = self.game.as_ref().filter(|g| g.ok) {
            let list = if area { &self.world.area } else { &self.world.levels };
            let unknown: Vec<&str> = list.iter().filter(|l| !g.level_exists(l)).map(String::as_str).collect();
            if !unknown.is_empty() {
                let _ = text::status(
                    ui,
                    Tone::Warning,
                    &format!("Not in the linked game: {}.", unknown.join(", ")),
                );
            }
        }
        let popup = Id::unique(("gz-ui-world.pick_level", key));
        let resp = Button::new("Add Level...").enabled(game_ok, "Link a game to pick its levels").show(ui);
        if resp.clicked() {
            open_level_picker(&ctx, popup);
        }
        let taken = if area { self.world.area.clone() } else { self.world.levels.clone() };
        if let Some(level) = local_level_picker(&ctx, popup, resp.rect, ui.layer_id(), &self.levels, &taken) {
            let list = if area { &mut self.world.area } else { &mut self.world.levels };
            list.push(level);
            changed = true;
        }
        if changed {
            let list = if area { &self.world.area } else { &self.world.levels };
            self.actions.push(Action::Send(args([(field, s(&list.join(",")))])));
        }
    }

    fn draw_transitions(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let Some(game) = self.game.clone() else {
            gz_ui::state::busy_line(ui, "Reading the linked game's transitions...");
            return;
        };
        if !game.ok {
            let text_ = if game.error.is_empty() {
                "Link a game to list its transitions."
            } else {
                game.error.as_str()
            };
            let _ = text::status(ui, Tone::Muted, text_);
            return;
        }
        // the scene's level first, then the first of the area, then the first with any
        if self.trans_level.is_empty() {
            self.trans_level = if !self.scene_level.is_empty() {
                self.scene_level.clone()
            } else if let Some(first) = self.world.area.first() {
                first.clone()
            } else {
                game.transitions.first().map(|t| t.level.clone()).unwrap_or_default()
            };
        }
        let mut lock_level: Option<bool> = None;
        let mut picked_level: Option<String> = None;
        let preview = self.level_label(&self.trans_level);
        let rows: Vec<String> = self.level_rows.iter().map(|(_, label)| label.clone()).collect();
        let current = self.level_rows.iter().position(|(name, _)| *name == self.trans_level);
        let level_locked = self.world.locked_levels.contains(&self.trans_level);
        PropertyGrid::new("world").label_frac(0.25).show(ui, |g| {
            let picked = g.row(
                "Level",
                "The level whose transitions are listed: the linked game's levels that have any.",
                PropFlags::NONE,
                0,
                |ui| {
                    let w = ui.available_width().min(em(24.0));
                    local_cell(ui, Vec2::new(w, metrics::FRAME), Layout::left_to_right(Align::Center), |ui| {
                        local_combo(ui, "world.trans_level", &preview, false, &rows, current)
                    })
                    .inner
                },
            );
            if let Some(Some(i)) = picked {
                picked_level = self.level_rows.get(i).map(|(name, _)| name.clone());
            }
            let flipped = g.row(
                "Whole Level",
                "Every transition on this level starts closed; quests open them one by one.",
                PropFlags::NONE,
                0,
                |ui| {
                    let label = "Close every transition on this level";
                    let w = metrics::FRAME + 8.0 + text::width(ui.ctx(), &Font::Body.id(), label);
                    let mut on = level_locked;
                    local_check(ui, "world.lock_level", label, &mut on, w).then_some(on)
                },
            );
            if let Some(Some(on)) = flipped {
                lock_level = Some(on);
            }
        });
        if let Some(level) = picked_level {
            self.trans_level = level;
        }
        if let Some(on) = lock_level {
            gz_world::model::toggle(&mut self.world.locked_levels, &self.trans_level, on);
            let key = if on { "lock_level" } else { "unlock_level" };
            self.actions.push(Action::Send(args([(key, s(&self.trans_level))])));
        }
        let w = ui.available_width().min(em(18.0));
        let _ = SearchField::new(&mut self.trans_filter)
            .hint("Filter transitions")
            .width(w)
            .id_salt("gz-ui-world.trans_filter")
            .show(ui);
        ui.add_space(4.0);

        if self.trans_memo.0 != self.trans_level
            || self.trans_memo.1 != self.trans_filter
            || self.trans_memo.2 != self.game_gen
        {
            let rows: Vec<usize> = game
                .transitions
                .iter()
                .enumerate()
                .filter(|(_, t)| t.level == self.trans_level)
                .filter(|(_, t)| {
                    gz_ui::search::matches(
                        &self.trans_filter,
                        &format!("{} {} {}", t.name, level_caption(&self.captions, &t.to_level), t.to_level),
                    )
                })
                .map(|(i, _)| i)
                .collect();
            self.trans_memo = (self.trans_level.clone(), self.trans_filter.clone(), self.game_gen, rows);
        }
        let total = ui.available_width();
        let closed_w = em(5.0);
        let half = ((total - closed_w) * 0.5).max(em(6.0));
        let columns = [
            Column::new("Transition", half),
            Column::new("Leads To", half),
            Column { centred: true, ..Column::new("Closed", closed_w) },
        ];
        let _ = table_headers(ui, &columns);
        let mut toggled: Option<(String, bool)> = None;
        for &i in &self.trans_memo.3 {
            let Some(t) = game.transitions.get(i) else { continue };
            ui.push_id(t.name.as_str(), |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(TABLE_ROW);
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let to = level_caption(&self.captions, &t.to_level);
                    local_cell(
                        ui,
                        Vec2::new(half, metrics::FRAME),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.add_space(6.0);
                            let tip = format!(
                                "{}\nat {:.1}, {:.1}, {:.1}",
                                t.name, t.position[0], t.position[1], t.position[2]
                            );
                            let _ =
                                text::label(ui, &t.name, Font::Body, p.text, Clip::End).on_hover_text(tip);
                        },
                    );
                    local_cell(
                        ui,
                        Vec2::new(half, metrics::FRAME),
                        Layout::left_to_right(Align::Center),
                        |ui| {
                            ui.add_space(6.0);
                            let _ = text::label(ui, &to, Font::Body, p.text, Clip::End)
                                .on_hover_text(t.to_level.as_str());
                        },
                    );
                    let by = self.world.closed_by(&t.name, &t.level, &t.to_level);
                    let mut closed = !by.is_empty();
                    let own = by == "locked" || by.is_empty();
                    let tip = match by {
                        "level" => "Closed with the whole level",
                        "area" => "Closed: it leads out of the playable area",
                        _ if closed => "Starts closed: a quest opens it",
                        _ => "Open at the start of a game",
                    };
                    if local_cell_check(
                        ui,
                        &format!("world.closed.{}", t.name),
                        &mut closed,
                        closed_w,
                        own,
                        tip,
                    ) {
                        toggled = Some((t.name.clone(), closed));
                    }
                });
            });
        }
        if self.trans_memo.3.is_empty() {
            let _ = text::status(
                ui,
                Tone::Muted,
                if self.trans_filter.is_empty() {
                    "This level has no transitions."
                } else {
                    "No transition matches the filter."
                },
            );
        }
        if let Some((name, closed)) = toggled {
            gz_world::model::toggle(&mut self.world.locked, &name, closed);
            self.actions.push(Action::Send(args([(if closed { "lock" } else { "unlock" }, s(&name))])));
        }
        // names the game does not have: kept, shown, removable
        let mut forget: Option<String> = None;
        for name in self.world.locked.iter().filter(|n| game.transition(n).is_none()) {
            ui.push_id(name.as_str(), |ui| {
                ui.horizontal(|ui| {
                    if icon_button(ui, Icon::Close, "Take it off the closed list", false, true).clicked() {
                        forget = Some(name.clone());
                    }
                    let _ = text::status(
                        ui,
                        Tone::Warning,
                        &format!("Closed, but not in the linked game: {name}"),
                    );
                });
            });
        }
        if let Some(name) = forget {
            gz_world::model::toggle(&mut self.world.locked, &name, false);
            self.actions.push(Action::Send(args([("unlock", s(&name))])));
        }
    }

    fn draw_page(&mut self, ui: &mut Ui) {
        let text_ = self.manifest.as_ref().map_or_else(String::new, |m| mode_text(m, self.modes.as_deref()));
        let _ = text::prose(ui, &text_).on_hover_text("A new game reads these rules once and keeps them in its saves. Quests change them while the game runs.");
        ui.add_space(6.0);
        if local_section(ui, "world.original", "Original Game", true, None) {
            self.draw_rules(ui);
            ui.add_space(8.0);
        }
        if local_section(ui, "world.start", "Player Start", true, None) {
            self.draw_start(ui);
            ui.add_space(8.0);
        }
        let kit_right = if !self.kit.declared {
            "The new-game screen's".to_owned()
        } else {
            format!("{} item{}", self.kit.items.len(), if self.kit.items.len() == 1 { "" } else { "s" })
        };
        if local_section(ui, "world.kit", "Starting Kit", true, Some(&kit_right)) {
            self.draw_kit(ui);
            ui.add_space(8.0);
        }
        if local_section(ui, MORE, "More", false, None) {
            ui.indent("gz-ui-world.more", |ui| {
                let levels = if self.world.levels.is_empty() {
                    "Every level".to_owned()
                } else {
                    format!("{} levels", self.world.levels.len())
                };
                if local_section(ui, "world.applies", "Applies On", true, Some(&levels)) {
                    let _ = text::prose(
                        ui,
                        "Where the five rules hold. The rest of the map keeps the original game.",
                    );
                    ui.add_space(2.0);
                    self.scope(ui, "world.levels", ["Every Level", "These Levels"], false);
                    ui.add_space(8.0);
                }
                let area = if self.world.area.is_empty() {
                    "Anywhere".to_owned()
                } else {
                    format!("{} levels", self.world.area.len())
                };
                if local_section(ui, "world.area", "Playable Area", true, Some(&area)) {
                    let _ = text::prose(
                        ui,
                        "A transition out of the area stays closed. A quest can open it or grow the area.",
                    );
                    ui.add_space(2.0);
                    self.scope(ui, "world.area_levels", ["Anywhere", "Only These Levels"], true);
                    ui.add_space(8.0);
                }
                if local_section(ui, "world.transitions", "Transitions", true, None) {
                    self.draw_transitions(ui);
                }
            });
        }
    }

    fn draw_body(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let any_problem = !self.lines.is_empty();
        let drawer = if any_problem { drawer_height(self.drawer_open, full.height()) } else { 0.0 };
        let body = Rect::from_min_max(
            full.min,
            pos2(
                full.max.x,
                (full.max.y - drawer - if any_problem { 4.0 } else { 0.0 }).max(full.min.y + 1.0),
            ),
        );
        let mut page = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.world.page")
                .max_rect(body)
                .layout(Layout::top_down(Align::Min)),
        );
        let mut scroll =
            ScrollArea::vertical().id_salt("gz-ui-world.world.scroll").auto_shrink([false, false]);
        match gz_app::take_scroll(&ctx, HOST) {
            Some(gz_app::ScrollTo::Top) => scroll = scroll.vertical_scroll_offset(0.0),
            Some(gz_app::ScrollTo::Bottom) => scroll = scroll.vertical_scroll_offset(self.scroll_max),
            Some(gz_app::ScrollTo::Offset(y)) => scroll = scroll.vertical_scroll_offset(y),
            _ => {}
        }
        let out = scroll.show(&mut page, |ui| {
            // one column of up to 900 pt in the middle, the scrollbar at the right edge of the tab
            let avail = ui.available_width();
            let column = avail.clamp(1.0, COLUMN_W);
            let x = ui.cursor().min.x + ((avail - column) * 0.5).max(0.0);
            let top = ui.cursor().min.y;
            let mut col = ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-ui-world.world.column")
                    .max_rect(Rect::from_min_max(pos2(x, top), pos2(x + column, f32::INFINITY)))
                    .layout(Layout::top_down(Align::Min)),
            );
            col.set_width(column);
            self.draw_page(&mut col);
            let used = col.min_rect();
            ui.allocate_rect(
                Rect::from_min_max(
                    pos2(ui.cursor().min.x, top),
                    pos2(ui.cursor().min.x + avail, used.max.y + 12.0),
                ),
                egui::Sense::hover(),
            );
        });
        let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
        self.scroll_max = max;
        gz_app::report_scroll(&ctx, HOST, out.state.clamped_offset().y.min(max), max);
        if any_problem {
            let r = Rect::from_min_max(pos2(full.min.x, full.max.y - drawer), full.max);
            let mut bottom = ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-ui-world.world.drawer")
                    .max_rect(r)
                    .layout(Layout::top_down(Align::Min)),
            );
            local_problems_drawer(
                &mut bottom,
                &mut self.drawer_open,
                drawer,
                &self.lines,
                "No problems: the build takes these rules as they are.",
            );
        }
        ui.allocate_rect(full, egui::Sense::hover());
    }
}

/// The caption of a level, the name itself when it has none (`EditorWorld::Caption`).
fn level_caption(captions: &FxHashMap<String, String>, level: &str) -> String {
    captions.get(level).filter(|c| !c.is_empty()).cloned().unwrap_or_else(|| level.to_owned())
}

/// "Caption  (name)", the name alone when it has no caption (`LevelLabel`).
fn level_label(captions: &FxHashMap<String, String>, level: &str) -> String {
    let caption = level_caption(captions, level);
    if caption == level { level.to_owned() } else { format!("{caption}  ({level})") }
}

/// Sends what the outbox holds, one call after another, on one task at a time; each answer is the
/// tab's status.
fn run_outbox(editor: &Arc<Editor>, outbox: &Arc<Outbox>, inbox: &Arc<Mutex<Inbox>>) {
    if outbox.running.swap(true, Ordering::AcqRel) {
        return;
    }
    let (ed, outbox, inbox) = (editor.clone(), outbox.clone(), inbox.clone());
    let origin = editor.ui_origin();
    let _task = editor.exec.spawn(async move {
        loop {
            let next = outbox.queue.lock().pop_front();
            let Some(a) = next else {
                outbox.running.store(false, Ordering::Release);
                // a call queued while this one was finding the queue empty
                if outbox.queue.lock().is_empty() || outbox.running.swap(true, Ordering::AcqRel) {
                    break;
                }
                continue;
            };
            let reply = ed.call("world_set", a, origin).await;
            inbox.lock().status = Some(if call::ok(&reply) {
                String::new()
            } else {
                format!("Not saved: {}", call::error_text(&reply))
            });
            call::wake_ui(&ed);
        }
        call::wake_ui(&ed);
    });
}

impl Document for WorldDoc {
    fn id(&self) -> &str {
        TAB
    }
    fn kind(&self) -> &'static str {
        TAB
    }
    fn label(&self) -> String {
        "World Rules".to_owned()
    }
    fn icon(&self) -> Icon {
        Icon::World
    }
    fn tooltip(&self) -> Option<String> {
        Some("What the mode keeps of the original game - saved to mod.ltx as you edit".to_owned())
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        tb.group(2);
        if tb.action(
            "reload",
            Icon::Refresh,
            "Reload Game Data",
            "Read the linked game's levels, transitions and their names again",
            true,
            false,
            Kind::Ghost,
        ) {
            self.game_rev = None;
            self.checked_at = None;
            let ed = cx.editor.clone();
            if let Some(lists) = Lists::of(cx.editor) {
                lists.force(cx.editor);
            }
            let _task = cx.editor.exec.spawn(async move {
                if let Ok(link) = GameLink::of(&ed) {
                    let _ = link.refresh().await;
                }
                call::wake_ui(&ed);
            });
        }
        let tip = if self.have_manifest {
            "Take everything of the original game out of this mode"
        } else {
            "The project has no mod.ltx yet"
        };
        if tb.action("empty", Icon::Clear, "Empty World...", tip, self.have_manifest, false, Kind::Ghost) {
            self.actions.push(Action::AskEmpty);
        }
        tb.group(0);
        tb.separator();
        if self.status.is_empty() {
            tb.status(SAVED, Tone::Muted);
        } else {
            tb.status(&self.status, Tone::Error);
        }
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        self.take_inbox();
        if cx.editor.project_opt().is_none() {
            gz_ui::state::empty_state(
                ui,
                Icon::World,
                "No project",
                "World rules live in the mod.ltx of the open project.",
                None,
            );
            return;
        }
        self.sync();
        self.want_game();
        if let Some(lists) = Lists::of(cx.editor) {
            lists.want(cx.editor);
            self.lists = lists.view();
        }
        if self.rev.is_none() {
            gz_ui::state::busy_line(ui, "Reading mod.ltx...");
            self.run_actions(cx);
            return;
        }
        if !self.have_manifest {
            if gz_ui::state::empty_state(
                ui,
                Icon::World,
                "No mod.ltx yet",
                "World rules live in the module's mod.ltx. Mod Settings makes it.",
                Some("Open Mod Settings"),
            ) {
                self.actions.push(Action::OpenSettings);
            }
            self.run_actions(cx);
            return;
        }
        self.want_check();
        self.draw_body(ui);
        self.run_actions(cx);
        self.flush_kit(&ctx);
    }

    fn on_activate(&mut self, _cx: &mut DocCx<'_>) {
        self.checked_at = None;
    }

    fn mcp_state(&self) -> Value {
        self.view_json(false, &self.problems)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Opens More from outside: what MCP asks for (a level, a filter) is under it.
pub(crate) fn open_more(ctx: &egui::Context) {
    set_section_open(ctx, MORE, true);
}

/// The `world_view` answer of a tab that is not open.
#[must_use]
pub fn closed_json(problems: &[String]) -> Value {
    json!({ "tab": TAB, "open": false, "active": false, "drawer": "close", "level": "", "filter": "", "status": SAVED, "problems": problems })
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.document(TAB, |editor, _req| {
        if editor.project_opt().is_none() {
            return Err("no project is open".to_owned());
        }
        Ok(Box::new(WorldDoc::new(editor)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_problem_reads_as_a_sentence() {
        assert_eq!(
            problem_line("W203 world: levels names 'x', a level the linked game does not have"),
            "Levels names 'x', a level the linked game does not have."
        );
        assert_eq!(
            problem_line("E201 world: 2 Player Starts - a module has one: a"),
            "2 Player Starts - a module has one: a."
        );
    }

    #[test]
    fn a_kit_goes_out_as_one_list() {
        let items = vec![
            KitItem { section: "bandage".into(), count: 2 },
            KitItem { section: "wpn_knife".into(), count: 1 },
            KitItem { section: String::new(), count: 1 },
        ];
        assert_eq!(kit_list(&items), "bandage=2, wpn_knife");
        assert_eq!(kit_list(&[]), "");
    }

    #[test]
    fn the_mode_text_names_what_the_rules_shape() {
        let mut m = Manifest::default();
        assert_eq!(mode_text(&m, None), "No game mode is chosen yet: pick one in Mod Settings, Game Mode.");
        m.target = "default".into();
        assert_eq!(
            mode_text(&m, None),
            "These rules shape every new game: the module targets the plain game."
        );
        m.mode = "metro".into();
        assert_eq!(mode_text(&m, None), "These rules shape the mode 'metro'.");
        m.provides_modes.push(gz_mod::ProvidesMode { id: "mine".into(), title: "Mine".into() });
        assert_eq!(mode_text(&m, None), "These rules shape the module's own mode 'Mine'.");
    }

    #[test]
    fn a_level_reads_by_its_caption() {
        let mut captions = FxHashMap::default();
        captions.insert("l01_escape".to_owned(), "Cordon".to_owned());
        assert_eq!(level_label(&captions, "l01_escape"), "Cordon  (l01_escape)");
        assert_eq!(level_label(&captions, "l02"), "l02");
        let v = closed_json(&[]);
        assert_eq!(v["status"], SAVED);
        assert_eq!(v["drawer"], "close");
    }
}
