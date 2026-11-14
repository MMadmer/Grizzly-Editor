//! The Start page (`Start Page###start_page`): the project browser the work area shows while no
//! project is open - a page, not a modal (docs/UI_DESIGN.md, section 6, "Start page").
//!
//! Open Folder..., Open Project File..., a search (24 em at most) and the Last Opened / Name sort
//! (`_projects.ini [browser] sort`) head the page; then the cards, 232 x 184 pt, the first slot
//! New Project. That card turns into the form in its place (Name with its rule as the hint,
//! Location under More, Create the Primary) - nothing else moves - and is the form by itself
//! when no project is on the list. A card shows the project by its project file. Its `...` menu
//! (or a right click) offers Show in Explorer and Delete..., or Remove from List for a folder
//! that is no project any more; Delete opens `Delete Project###dlg.delete_project`, which names
//! what disappears. A project file or a project folder dropped on the page opens. The editor's
//! version stands in the page's corner.
//!
//! Everything the page shows comes from [`PageState`], filled off the UI thread: the list and
//! the folders' states ([`cards::load`]), the previews (a [`Query`] keyed by the folder and the
//! picture's write time), the delete check's count.

use std::sync::Arc;

use egui::epaint::RectShape;
use egui::{
    Align, Align2, Color32, ColorImage, CornerRadius, Id, Layout, Popup, Rect, ScrollArea, Sense, Stroke,
    StrokeKind, TextureHandle, TextureOptions, Ui, UiBuilder, Vec2, pos2,
};
use gz_app::{DocCx, ShellBuilder, contract};
use gz_base::Rev;
use gz_editor::{Editor, Value};
use gz_project::{DeleteContext, DeleteInfo, EntryState, Project, Projects, Sort, paths};
use gz_query::{Answer, Query};
use gz_ui::dialog::Footer;
use gz_ui::drop::{Payload, Zone};
use gz_ui::grid::{PathAction, path_field};
use gz_ui::section::Section;
use gz_ui::state::{badge, banner, busy_line, toast};
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{
    Button, Dialog, DialogSize, Font, Icon, Kind, MenuItem, SearchField, Tone, icon_button_sized, icons,
    menu_item, palette, segmented,
};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::cards::{self, Card, Picture, Snapshot};
use crate::folder::pick_folder;
use crate::local::local_text_field;
use crate::reply::{args, call_then, error_text, ok, post_ui, s};

/// pt, docs/UI_DESIGN.md section 6 (Start page).
pub const FORM_W: f32 = 320.0;
pub const CARD_W: f32 = 232.0;
pub const CARD_H: f32 = 184.0;
pub const CARD_GAP: f32 = 16.0;
pub const PREVIEW_H: f32 = 120.0;
/// The sort's labels and the keys `_projects.ini` and MCP know them by.
pub const SORT_LABELS: [&str; 2] = ["Last Opened", "Name"];

/// The longest path Windows takes without the long-path prefix.
const MAX_PATH: usize = 260;
/// The search field's widest, em.
const SEARCH_EM: f32 = 24.0;

/// What the New Project form's look at its target found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum NewState {
    #[default]
    New,
    NotFullPath,
    ProjectThere,
    FolderThere,
}

/// A card's menu: whose, and where it hangs.
#[derive(Clone, Debug)]
pub(crate) struct Menu {
    pub path: String,
    /// Under the card's `...` button, or at the pointer; `None` until a frame placed it.
    pub anchor: Option<Rect>,
    /// MCP asked for it: the card puts it up when it is in view (and scrolls it there first).
    pub request: bool,
}

/// The Delete Project confirmation.
#[derive(Clone, Debug)]
pub(crate) struct Confirm {
    pub path: String,
    pub name: String,
    /// The check and the count, taken once when it opened; `None` while they are being taken.
    pub info: Option<DeleteInfo>,
    /// A deletion ran and left something (or was refused): `info` is its result.
    pub done: bool,
    pub deleting: bool,
    /// The last frame drew it.
    pub shown: bool,
}

/// What the page shows and remembers. UI thread, MCP and the pools reach it under one lock.
#[derive(Default)]
pub(crate) struct PageState {
    pub cards: Vec<Card>,
    /// The list has been read once.
    pub loaded: bool,
    loading: bool,
    reload: bool,
    pub create_parent: String,
    pub search: String,
    pub sort: Sort,
    pub form_open: bool,
    /// The form just opened: the Name field takes the keys.
    form_focus: bool,
    pub name: String,
    pub parent: String,
    /// The author (or a browse) set the location: a reread list does not change it.
    parent_set: bool,
    checked_key: String,
    check_gen: u64,
    pub new_state: NewState,
    pub create_error: String,
    creating: bool,
    pub page_error: String,
    pub menu: Option<Menu>,
    /// MCP: take any card menu down on the next frame.
    pub menu_close: bool,
    /// The card whose menu the last frame drew.
    pub menu_shown: Option<String>,
    pub confirm: Option<Confirm>,
    textures: FxHashMap<String, (Rev, Option<TextureHandle>)>,
    /// Textures of cards that left: released on the next frame, the one being drawn may hold them.
    dead: Vec<TextureHandle>,
    last_frame: Option<u64>,
}

impl PageState {
    /// The New Project form is up: opened, or up by itself with no project on the list.
    #[must_use]
    pub fn form_shown(&self) -> bool {
        self.form_open || (self.loaded && self.cards.is_empty())
    }
}

/// The Start page's service: its state and the previews being decoded.
pub struct StartPage {
    pub(crate) state: Mutex<PageState>,
    pub(crate) previews: Query<String, Option<Picture>>,
}

impl StartPage {
    /// The page of this editor, made on first use.
    pub fn of(editor: &Editor) -> Arc<StartPage> {
        editor.services.get_or_init(|| StartPage {
            state: Mutex::new(PageState::default()),
            previews: Query::new(editor.exec.clone()),
        })
    }
}

/// Puts a fresh read of the list in place. The first read brings the sort `_projects.ini`
/// keeps; a card that left hands its texture to the next frame.
pub(crate) fn apply_snapshot(st: &mut PageState, snap: Snapshot) {
    if !st.loaded {
        st.sort = snap.sort;
    }
    st.loaded = true;
    if !st.parent_set {
        st.parent.clone_from(&snap.create_parent);
    }
    st.create_parent = snap.create_parent;
    let keep: Vec<&str> = snap.cards.iter().map(|c| c.path.as_str()).collect();
    let gone: Vec<String> =
        st.textures.keys().filter(|k| !keep.iter().any(|p| paths::same_path(p, k))).cloned().collect();
    for k in gone {
        if let Some((_, Some(tex))) = st.textures.remove(&k) {
            st.dead.push(tex);
        }
    }
    st.cards = snap.cards;
}

/// Reads the list again off the UI thread; what it finds comes back to the start of a frame.
pub(crate) fn refresh(st: &mut PageState, editor: &Arc<Editor>, page: &Arc<StartPage>) {
    if st.loading {
        st.reload = true;
        return;
    }
    st.loading = true;
    let (ed, page) = (editor.clone(), page.clone());
    let _task = editor.exec.spawn(async move {
        let snap = cards::load(&ed).await;
        let ed2 = ed.clone();
        post_ui(&ed, move |_| {
            let mut st = page.state.lock();
            apply_snapshot(&mut st, snap);
            st.loading = false;
            if std::mem::take(&mut st.reload) {
                refresh(&mut st, &ed2, &page);
            }
        });
    });
}

/// The check in front of a deletion and the count of what it takes, as the confirmation opens:
/// `gz-project`'s one set of checks, over the list as the file has it now. Off the UI thread.
pub(crate) async fn delete_plan(editor: &Arc<Editor>, path: String) -> DeleteInfo {
    let listed = Projects::of(editor).list(editor).await.entries();
    let open = Project::of(editor).ok().map(|(_, p)| (p.root().to_owned(), p.game_root()));
    let (exe, root, home) =
        (editor.paths.exe_dir.clone(), editor.paths.root.clone(), editor.paths.home.clone());
    editor
        .exec
        .io(move || {
            let guards =
                gz_project::system_guards(&exe, &root, &home, open.as_ref().map(|o| o.1.as_str()), &listed);
            let cx = DeleteContext { listed, open: open.map(|o| o.0), guards };
            gz_project::check_delete(&cx, &path)
        })
        .await
}

/// The New Project form's rule for a name: Latin letters, digits, `_` and `-`.
#[must_use]
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// What a card offers in its state: (label, enabled), as the page draws it.
#[must_use]
pub fn menu_items(state: EntryState) -> [(&'static str, bool); 2] {
    [
        (contract::CARD_SHOW_IN_EXPLORER, state != EntryState::Missing),
        (
            if state == EntryState::Project {
                contract::CARD_DELETE
            } else {
                contract::CARD_REMOVE_FROM_LIST
            },
            true,
        ),
    ]
}

/// The tooltip of a card that is no project.
fn state_help(state: EntryState) -> &'static str {
    if state == EntryState::Missing {
        "The folder is not there: moved, deleted, or on a drive that is not plugged in. Its menu takes it off the list."
    } else {
        "The folder holds no project file or mod.ltx any more. Its menu takes it off the list."
    }
}

/// The confirmation's id, as `screenshot_editor` names the modal: `dlg.delete_project`.
#[must_use]
pub fn delete_dialog_name() -> &'static str {
    contract::DELETE_PROJECT_DIALOG.split_once("###").map_or(contract::DELETE_PROJECT_DIALOG, |(_, n)| n)
}

fn delete_dialog_title() -> &'static str {
    contract::DELETE_PROJECT_DIALOG.split_once("###").map_or(contract::DELETE_PROJECT_DIALOG, |(t, _)| t)
}

fn menu_id() -> Id {
    Id::unique("gz-ui-app.card_menu")
}

/// What a frame of the page asked for; carried out once the page is drawn.
enum Action {
    Open(String),
    OpenFolder,
    OpenFile,
    Dropped(String),
    BrowseLocation,
    Create,
    SaveSort(Sort),
    Reveal(String),
    AskDelete(String, String),
    Forget(String, String),
    DeleteNow(String, String),
    Recheck(String),
}

pub(crate) fn install(b: &mut ShellBuilder) {
    let mut page: Option<Arc<StartPage>> = None;
    b.start_page(move |ui, cx| {
        let page = page.get_or_insert_with(|| StartPage::of(cx.editor)).clone();
        draw(&page, ui, cx);
    });
}

fn draw(page: &Arc<StartPage>, ui: &mut Ui, cx: &mut DocCx<'_>) {
    let ctx = ui.ctx().clone();
    let editor = cx.editor.clone();
    let frame = ctx.cumulative_frame_nr();
    let mut st = page.state.lock();
    st.dead.clear();
    let appearing = st.last_frame.is_none_or(|f| f + 1 < frame);
    st.last_frame = Some(frame);
    if appearing {
        refresh(&mut st, &editor, page);
    }
    // a project opened under the page (over MCP): the confirmation and the menu go with it
    if editor.project_opt().is_some() {
        st.menu = None;
        st.confirm = None;
        st.menu_shown = None;
        return;
    }
    if std::mem::take(&mut st.menu_close) {
        Popup::close_id(&ctx, menu_id());
        st.menu = None;
    }
    check_new_folder(&mut st, &editor, page);

    let mut actions = Vec::new();
    let mut menu_shown = None;
    let page_rect = ui.max_rect();
    gz_ui::card::page(ui, contract::WINDOW_START_PAGE, |ui| {
        let p = palette(ui.ctx());
        text::label(ui, "Projects", Font::Page, p.text, Clip::End);
        ui.add_space(8.0);
        if !st.page_error.is_empty() {
            let error = st.page_error.clone();
            if banner(ui, Tone::Error, &error, Some("Dismiss"), None).is_some() {
                st.page_error.clear();
            }
            ui.add_space(8.0);
        }
        draw_projects(ui, &mut st, page, &mut actions);
        menu_shown = draw_menu(ui, &mut st, &mut actions);
    });
    st.menu_shown = menu_shown;
    draw_version(ui, page_rect);
    // a project file or a project folder dropped anywhere on the page opens
    let takes = |p: &Payload| !p.files().is_empty();
    if let Some(dropped) = Zone::new("Start Page", page_rect, &takes).catch_all().take(ui)
        && let Some(path) = dropped.files().into_iter().next()
    {
        actions.push(Action::Dropped(path.display().to_string()));
    }
    draw_confirm(ui, &mut st, &mut actions);

    for action in actions {
        run(action, &mut st, &editor, page);
    }
}

/// The editor's version in the page's bottom right corner.
fn draw_version(ui: &Ui, page: Rect) {
    let p = palette(ui.ctx());
    let line = format!("Grizzly Editor {}", env!("CARGO_PKG_VERSION"));
    let at = pos2(page.max.x - 16.0, page.max.y - 10.0);
    text::paint(
        ui.painter(),
        &Font::Small.id(),
        at,
        Align2::RIGHT_BOTTOM,
        p.text_muted,
        &line,
        None,
        Clip::End,
    );
}

/// The form's look at its target, taken again only when a field changes: every keystroke would
/// otherwise be a round of file operations.
fn check_new_folder(st: &mut PageState, editor: &Arc<Editor>, page: &Arc<StartPage>) {
    let key = format!("{}|{}", st.parent, st.name);
    if key == st.checked_key {
        return;
    }
    st.checked_key = key;
    // the fields changed: the last failure spoke of something else
    st.create_error.clear();
    st.new_state = NewState::New;
    st.check_gen += 1;
    let parent = paths::trim_trailing(paths::to_backslash(&st.parent));
    if !parent.is_empty() && !paths::is_absolute(&parent) {
        st.new_state = NewState::NotFullPath;
        return;
    }
    if parent.is_empty() || !valid_name(&st.name) || parent.len() + st.name.len() + 2 > MAX_PATH {
        return;
    }
    let folder = paths::join(&parent, &st.name);
    let generation = st.check_gen;
    let (ed, page) = (editor.clone(), page.clone());
    let _task = editor.exec.spawn(async move {
        let found = ed
            .exec
            .io(move || {
                let dir = std::path::Path::new(&folder);
                if !dir.is_dir() {
                    NewState::New
                } else if gz_project::list::has_manifest(dir) {
                    NewState::ProjectThere
                } else {
                    NewState::FolderThere
                }
            })
            .await;
        post_ui(&ed, move |_| {
            let mut st = page.state.lock();
            if st.check_gen == generation {
                st.new_state = found;
            }
        });
    });
}

/// The New Project card turned into the form, in its slot: Name, Location under More, Cancel |
/// Create. It never grows past the card.
fn draw_form(ui: &mut Ui, st: &mut PageState, rect: Rect, actions: &mut Vec<Action>) {
    let p = palette(ui.ctx());
    ui.painter().rect_filled(rect, CornerRadius::same(6), p.raised);
    ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, p.accent), StrokeKind::Inside);
    let inner = rect.shrink(12.0);
    let mut ui = ui.new_child(
        UiBuilder::new().id_salt("gz-ui-app.start.form").max_rect(inner).layout(Layout::top_down(Align::Min)),
    );
    let ui = &mut ui;
    ui.spacing_mut().item_spacing.y = 4.0;
    let width = inner.width();
    text::label(ui, "New Project", Font::Strong, p.text, Clip::End);
    let resp =
        local_text_field(ui, "new_project.name", &mut st.name, "Latin letters, digits, _ and -", width);
    if st.form_focus {
        resp.request_focus();
        st.form_focus = false;
    }
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

    // the worst first, one line
    let name_ok = valid_name(&st.name);
    let message = if !st.create_error.is_empty() {
        Some((Tone::Error, st.create_error.clone()))
    } else if !st.name.is_empty() && !name_ok {
        Some((Tone::Error, "Use Latin letters, digits, _ and - only.".to_owned()))
    } else if st.new_state == NewState::NotFullPath {
        Some((Tone::Error, "The location must be a full path.".to_owned()))
    } else if st.new_state == NewState::ProjectThere {
        Some((Tone::Info, "A project with this name is there: Create opens it.".to_owned()))
    } else if st.new_state == NewState::FolderThere {
        Some((Tone::Warning, "The folder exists: the project is made inside it.".to_owned()))
    } else {
        None
    };
    // one line: what is wrong, else where the folder is made (its start cut, its end kept)
    match message {
        Some((tone, m)) => {
            text::label(ui, &m, Font::Small, p.tone(tone), Clip::End).on_hover_text(m.as_str());
        }
        None => {
            let parent =
                if st.parent.is_empty() { "Pick a location under More." } else { st.parent.as_str() };
            text::label(ui, parent, Font::Small, p.text_muted, Clip::Start)
                .on_hover_text(format!("The project's folder is made in {parent}"));
        }
    }
    let more = Section::new("start.new_project.more", "More")
        .default_open(st.parent.is_empty() || st.new_state == NewState::NotFullPath)
        .show(ui);
    if more.open {
        let r = Rect::from_min_size(ui.cursor().min, Vec2::new(width, gz_ui::metrics::FRAME));
        let mut row = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-app.start.location")
                .max_rect(r)
                .layout(Layout::left_to_right(Align::Center)),
        );
        match path_field(&mut row, "new_project.location", &mut st.parent, true) {
            PathAction::Edited => st.parent_set = true,
            PathAction::Browse => actions.push(Action::BrowseLocation),
            PathAction::None => {}
        }
        ui.allocate_rect(r, Sense::hover());
    }

    let why = if st.parent.is_empty() {
        Some("Pick a location first.")
    } else if st.new_state == NewState::NotFullPath {
        Some("The location must be a full path.")
    } else if st.name.is_empty() {
        Some("Enter a name first.")
    } else if !name_ok {
        Some("The name may hold Latin letters, digits, _ and - only.")
    } else if st.creating {
        Some("The project is being created.")
    } else {
        None
    };
    // the footer stands at the card's foot
    let foot = Rect::from_min_max(pos2(inner.min.x, inner.max.y - gz_ui::metrics::FRAME), inner.max);
    let mut row = ui.new_child(
        UiBuilder::new()
            .id_salt("gz-ui-app.start.form_foot")
            .max_rect(foot)
            .layout(Layout::right_to_left(Align::Center)),
    );
    row.spacing_mut().item_spacing.x = 8.0;
    if Button::new("Create").primary().enabled(why.is_none(), why.unwrap_or("")).show(&mut row).clicked()
        || (enter && why.is_none())
    {
        actions.push(Action::Create);
    }
    let lone = st.loaded && st.cards.is_empty();
    if Button::new("Cancel")
        .enabled(!lone, "With no project on the list the form stays up.")
        .show(&mut row)
        .clicked()
    {
        st.form_open = false;
        st.create_error.clear();
    }
}

/// The first slot while the form is down: New Project as a card.
fn draw_new_card(ui: &mut Ui, st: &mut PageState, rect: Rect) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let id = ui.make_persistent_id("gz-ui-app.start.new_card");
    let resp = ui.interact(rect, id, Sense::click());
    let hovered = resp.hovered();
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(6), if hovered { p.hover } else { p.raised });
    painter.rect_stroke(
        rect,
        CornerRadius::same(6),
        Stroke::new(1.0, if hovered { p.border } else { p.border_subtle }),
        StrokeKind::Inside,
    );
    let side = 32.0;
    let icon = Rect::from_center_size(rect.center() - Vec2::new(0.0, 14.0), Vec2::splat(side));
    icons::draw(painter, Icon::Plus, icon, if hovered { p.text } else { p.text_secondary });
    text::paint(
        painter,
        &Font::Strong.id(),
        pos2(rect.center().x, icon.max.y + 18.0),
        Align2::CENTER_CENTER,
        p.text,
        "New Project",
        None,
        Clip::End,
    );
    gz_ui::audit::control(
        &ctx,
        "New Project",
        true,
        None,
        Some("Makes a new project folder with its project file"),
        rect,
    );
    if resp.on_hover_text("Makes a new project folder with its project file").clicked() {
        st.form_open = true;
        st.form_focus = true;
    }
}

/// Open Folder..., Open Project File..., the search and the sort, then the cards.
fn draw_projects(ui: &mut Ui, st: &mut PageState, page: &Arc<StartPage>, actions: &mut Vec<Action>) {
    let ctx = ui.ctx().clone();
    let body = Font::Body.id();
    let sort_w: f32 = SORT_LABELS.iter().map(|l| 20.0 + text::width(&ctx, &body, l)).sum();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        if Button::new("Open Folder...").tip("Opens a folder that holds a project file (.grizzly) or mod.ltx, such as a fresh clone of a mod's repository.").show(ui).clicked() {
            actions.push(Action::OpenFolder);
        }
        if Button::new("Open Project File...").tip("Opens a project by its project file (.grizzly).").show(ui).clicked() {
            actions.push(Action::OpenFile);
        }
        let w = (ui.available_width() - sort_w - 16.0).clamp(em(8.0), em(SEARCH_EM));
        SearchField::new(&mut st.search).hint("Search projects").width(w).show(ui);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut sort = usize::from(st.sort == Sort::Name);
            if segmented(ui, &mut sort, &SORT_LABELS, None) {
                st.sort = if sort == 1 { Sort::Name } else { Sort::Recent };
                actions.push(Action::SaveSort(st.sort));
            }
        });
    });
    ui.add_space(16.0);
    let order = cards::card_order(&st.cards, &st.search, st.sort);
    // the room under the toolbar scrolls; the version line keeps the page's corner
    let room = Rect::from_min_max(ui.cursor().min, pos2(ui.max_rect().max.x, ui.clip_rect().max.y - 28.0));
    let mut area = ui.new_child(UiBuilder::new().id_salt("gz-ui-app.start.cards_room").max_rect(room));
    ScrollArea::vertical().id_salt("gz-ui-app.start.cards").auto_shrink([false, false]).show(
        &mut area,
        |ui| {
            if !st.loaded {
                busy_line(ui, "Reading the project list...");
                return;
            }
            draw_grid(ui, st, page, &order, actions);
            if !st.cards.is_empty() && order.is_empty() {
                ui.add_space(16.0);
                text::status(ui, Tone::Muted, "No project name or folder holds every word of the search.");
            }
        },
    );
    ui.allocate_rect(room, Sense::hover());
}

/// The cards in rows as wide as the room allows, New Project in the first slot.
fn draw_grid(
    ui: &mut Ui,
    st: &mut PageState,
    page: &Arc<StartPage>,
    order: &[usize],
    actions: &mut Vec<Action>,
) {
    let avail = ui.available_width();
    let per_row = (((avail + CARD_GAP) / (CARD_W + CARD_GAP)) as usize).max(1);
    let origin = ui.cursor().min;
    let slot = |n: usize| {
        pos2(
            origin.x + (n % per_row) as f32 * (CARD_W + CARD_GAP),
            origin.y + (n / per_row) as f32 * (CARD_H + CARD_GAP),
        )
    };
    let first = Rect::from_min_size(slot(0), Vec2::new(CARD_W, CARD_H));
    // with no project on the list the form is up by itself
    if st.form_shown() {
        draw_form(ui, st, first, actions);
    } else {
        draw_new_card(ui, st, first);
    }
    for (n, &i) in order.iter().enumerate() {
        draw_card(ui, st, page, i, slot(n + 1), actions);
    }
    // the grid as one block, so the scroll knows how far it reaches
    let count = order.len() + 1;
    let rows = count.div_ceil(per_row);
    let w = avail.min(per_row as f32 * (CARD_W + CARD_GAP) - CARD_GAP);
    let h = rows as f32 * (CARD_H + CARD_GAP) - CARD_GAP;
    ui.allocate_rect(Rect::from_min_size(origin, Vec2::new(w, h)), Sense::hover());
}

/// The picture of a card, uploaded once its decode is in; the old one while a new one comes.
fn texture_for(
    st: &mut PageState,
    page: &StartPage,
    ctx: &egui::Context,
    path: &str,
    rev: Rev,
) -> Option<TextureHandle> {
    if let Some((r, tex)) = st.textures.get(path)
        && *r == rev
    {
        return tex.clone();
    }
    let folder = path.to_owned();
    match page.previews.get(&path.to_owned(), rev, move || cards::read_preview(&folder)) {
        Answer::Fresh(value) => {
            let tex = value.as_ref().as_ref().map(|pic| {
                ctx.load_texture(
                    format!("gz-ui-app.preview:{path}"),
                    ColorImage::from_rgb([pic.w, pic.h], &pic.rgb),
                    TextureOptions::LINEAR,
                )
            });
            if let Some((_, Some(old))) = st.textures.insert(path.to_owned(), (rev, tex.clone())) {
                st.dead.push(old);
            }
            tex
        }
        Answer::Stale(_) | Answer::Pending => st.textures.get(path).and_then(|(_, t)| t.clone()),
    }
}

/// One card, 232 x 184 pt: the preview, the name and the `...` menu, the path with a start
/// ellipsis (a twin's differing parent kept whole in it).
fn draw_card(
    ui: &mut Ui,
    st: &mut PageState,
    page: &Arc<StartPage>,
    i: usize,
    at: egui::Pos2,
    actions: &mut Vec<Action>,
) {
    let Some(card) = st.cards.get(i).cloned() else { return };
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let (w, h, pad) = (CARD_W, CARD_H, 8.0);
    let here = card.state == EntryState::Project;
    let rect = Rect::from_min_size(at, Vec2::new(w, h));
    // by folder, not by slot: a card leaving the list must not hand its menu on
    let id = ui.make_persistent_id(("gz-ui-app.card", card.path.as_str()));
    let resp = ui.interact(rect, id, Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    // a folder that holds no project any more is veiled and opens nothing
    let veil = if here { 1.0 } else { 0.5 };
    let visible = ui.is_rect_visible(rect);

    let pmin = pos2(rect.min.x + pad, rect.min.y + pad);
    let prect = Rect::from_min_max(pmin, pos2(rect.max.x - pad, rect.min.y + pad + PREVIEW_H));
    let row_y = prect.max.y + 6.0;
    let (row_h, more) = (24.0, 24.0);
    let more_rect = Rect::from_min_size(pos2(rect.max.x - pad - more, row_y), Vec2::splat(more));
    let mut path_cut = false;
    if visible {
        let painter = ui.painter().clone();
        painter.rect_filled(rect, CornerRadius::same(6), if hovered { p.hover } else { p.raised });
        painter.rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0, if hovered { p.border } else { p.border_subtle }),
            StrokeKind::Inside,
        );

        // the preview is cut to the card's shape, never stretched
        let tex = match (here, card.preview) {
            (true, Some(rev)) => texture_for(st, page, &ctx, &card.path, rev),
            _ => None,
        };
        match tex {
            Some(tex) => {
                let [tw, th] = tex.size();
                let uv = cards::crop_uv(tw as f32, th as f32, prect.width(), prect.height());
                painter.add(
                    RectShape::filled(prect, CornerRadius::same(4), Color32::WHITE.gamma_multiply(veil))
                        .with_texture(tex.id(), uv),
                );
            }
            None => {
                painter.rect_filled(prect, CornerRadius::same(4), p.canvas.gamma_multiply(veil));
                let side = 32.0;
                let icon_rect = Rect::from_min_size(
                    pos2((prect.center().x - side * 0.5).round(), (prect.center().y - side * 0.5).round()),
                    Vec2::splat(side),
                );
                icons::draw(
                    &painter,
                    if here { Icon::Level } else { Icon::Folder },
                    icon_rect,
                    p.text_muted.gamma_multiply(veil),
                );
            }
        }
        if !here {
            let r = Rect::from_min_size(
                pos2(prect.min.x + 6.0, prect.min.y + 6.0),
                Vec2::new(prect.width() - 12.0, 18.0),
            );
            let mut b = ui.new_child(
                UiBuilder::new()
                    .id_salt(id.with("badge"))
                    .max_rect(r)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            badge(
                &mut b,
                if card.state == EntryState::Missing { "Missing" } else { "Not a project" },
                Tone::Warning,
            );
        }

        // the name has its row to itself, the ... menu at its end; the path under it
        let name_room = w - pad * 2.0 - more - 4.0;
        text::paint(
            &painter,
            &Font::Strong.id(),
            pos2(prect.min.x, row_y + row_h * 0.5),
            Align2::LEFT_CENTER,
            p.text.gamma_multiply(veil),
            &card.name,
            Some(name_room),
            Clip::End,
        );
        path_cut = draw_card_path(
            &painter,
            &ctx,
            &card,
            pos2(prect.min.x, row_y + row_h + 2.0),
            w - pad * 2.0,
            veil,
        );
    }

    let mut child = ui.new_child(UiBuilder::new().id_salt(id.with("more")).max_rect(more_rect));
    let open_here =
        st.menu.as_ref().is_some_and(|m| paths::same_path(&m.path, &card.path) && m.anchor.is_some());
    let more_clicked =
        icon_button_sized(&mut child, Icon::More, "Actions", open_here, true, more, gz_ui::metrics::GLYPH)
            .clicked();
    if more_clicked {
        st.menu = Some(Menu { path: card.path.clone(), anchor: Some(more_rect), request: false });
        Popup::open_id(&ctx, menu_id());
        actions.push(Action::Recheck(card.path.clone()));
    } else if resp.secondary_clicked()
        && let Some(at) = ctx.pointer_interact_pos()
    {
        st.menu = Some(Menu {
            path: card.path.clone(),
            anchor: Some(Rect::from_min_size(at, Vec2::ZERO)),
            request: false,
        });
        Popup::open_id(&ctx, menu_id());
        actions.push(Action::Recheck(card.path.clone()));
    }
    // MCP asks the way the ... button does; a card scrolled away comes into view first
    if st.menu.as_ref().is_some_and(|m| m.request && paths::same_path(&m.path, &card.path)) {
        if ui.clip_rect().contains_rect(rect) {
            st.menu = Some(Menu { path: card.path.clone(), anchor: Some(more_rect), request: false });
            Popup::open_id(&ctx, menu_id());
        } else {
            ui.scroll_to_rect(rect, Some(Align::Center));
        }
    }

    if resp.clicked() && here {
        actions.push(Action::Open(card.path.clone()));
    }
    if !here {
        resp.on_hover_text(state_help(card.state));
    } else if path_cut {
        resp.on_hover_text(card.path.as_str());
    }
}

/// The card's path in Small type, cut at its start. For a twin the folder that tells it apart
/// stays whole and reads in the text colour: when the path from it on is too long, the cut moves
/// between it and the leaf too (`…\tour3a\…\Name`); only a folder wider than the card is cut
/// itself. Returns whether the path was cut (its tooltip is then the whole path).
fn draw_card_path(
    painter: &egui::Painter,
    ctx: &egui::Context,
    card: &Card,
    at: egui::Pos2,
    room: f32,
    veil: f32,
) -> bool {
    let p = palette(ctx);
    let font = Font::Small.id();
    let secondary = p.text_secondary.gamma_multiply(veil);
    let strong = p.text.gamma_multiply(veil);
    let y = at.y + Font::Small.line() * 0.5;
    let path = card.path.as_str();
    let Some(seg) = card.seg.filter(|s| s.end > s.begin) else {
        return text::paint(
            painter,
            &font,
            pos2(at.x, y),
            Align2::LEFT_CENTER,
            secondary,
            path,
            Some(room),
            Clip::Start,
        );
    };
    let (Some(head), Some(folder), Some(tail)) =
        (path.get(..seg.begin), path.get(seg.begin..seg.end), path.get(seg.end..))
    else {
        return text::paint(
            painter,
            &font,
            pos2(at.x, y),
            Align2::LEFT_CENTER,
            secondary,
            path,
            Some(room),
            Clip::Start,
        );
    };
    // a cut before the folder only where something stands before it, after it only where folders
    // stand between it and the leaf
    let lead = if seg.begin > 0 { "…\\" } else { "" };
    let past = if seg.leaf > seg.end + 1 {
        format!("\\…\\{}", path.get(seg.leaf..).unwrap_or(""))
    } else {
        tail.to_owned()
    };
    let width = |t: &str| text::width(ctx, &font, t);
    let folder_w = width(folder);
    let tries = [(head, tail.to_owned()), (lead, tail.to_owned()), (lead, past)];
    let pick = tries.iter().position(|(pre, post)| width(pre) + folder_w + width(post) <= room);
    match pick.and_then(|k| tries.get(k).map(|t| (k, t))) {
        Some((k, (pre, post))) => {
            let mut x = at.x;
            for (part, color) in [(*pre, secondary), (folder, strong), (post.as_str(), secondary)] {
                if !part.is_empty() {
                    let r = painter.text(pos2(x, y), Align2::LEFT_CENTER, part, font.clone(), color);
                    x = r.max.x;
                }
            }
            k != 0
        }
        None => {
            text::paint(
                painter,
                &font,
                pos2(at.x, y),
                Align2::LEFT_CENTER,
                strong,
                &card.parent,
                Some(room),
                Clip::End,
            );
            true
        }
    }
}

/// The card menu: Show in Explorer, then Delete... or Remove from List. Returns the card whose
/// menu this frame drew.
fn draw_menu(ui: &mut Ui, st: &mut PageState, actions: &mut Vec<Action>) -> Option<String> {
    let menu = st.menu.clone()?;
    let anchor = menu.anchor?;
    let ctx = ui.ctx().clone();
    let Some(card) = st.cards.iter().find(|c| paths::same_path(&c.path, &menu.path)).cloned() else {
        st.menu = None;
        Popup::close_id(&ctx, menu_id());
        return None;
    };
    let shown = gz_ui::menu::menu_popup(&ctx, menu_id(), anchor, ui.layer_id(), |ui| {
        let missing = card.state == EntryState::Missing;
        if menu_item(
            ui,
            contract::CARD_SHOW_IN_EXPLORER,
            MenuItem { icon: Icon::Reveal, enabled: !missing, ..MenuItem::default() },
        ) {
            actions.push(Action::Reveal(card.path.clone()));
        }
        ui.separator();
        if card.state == EntryState::Project {
            if menu_item(
                ui,
                contract::CARD_DELETE,
                MenuItem { icon: Icon::Delete, danger: true, ..MenuItem::default() },
            ) {
                actions.push(Action::AskDelete(card.path.clone(), card.name.clone()));
            }
        } else if menu_item(
            ui,
            contract::CARD_REMOVE_FROM_LIST,
            MenuItem { icon: Icon::Clear, ..MenuItem::default() },
        ) {
            actions.push(Action::Forget(card.path.clone(), card.name.clone()));
        }
    });
    match shown {
        Some(()) => Some(card.path),
        None => {
            st.menu = None;
            None
        }
    }
}

/// One line of the list of what goes.
fn what_goes(ui: &mut Ui, line: &str) {
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let color = palette(ui.ctx()).text;
        text::label(ui, line, Font::Body, color, Clip::Middle);
    });
}

/// A dialog of its own: the page under it is no modal, so nothing closes it by opening next to
/// it. Cancel, Esc and MCP `show=none` take it down.
fn draw_confirm(ui: &mut Ui, st: &mut PageState, actions: &mut Vec<Action>) {
    let Some(c) = st.confirm.as_mut() else { return };
    let mut open = true;
    let mut pressed: Option<bool> = None;
    let question = !c.done && c.info.as_ref().is_none_or(|d| d.refused.is_none() && !d.missing);
    let counting = c.info.is_none();
    let deleting = c.deleting;
    let shown = Dialog::new(delete_dialog_title(), delete_dialog_name(), DialogSize::M).show(
        ui,
        &mut open,
        |ui| confirm_body(ui, c),
        |f: &mut Footer<'_>| {
            if deleting {
                f.busy("Deleting the project...");
            }
            if question {
                if f.button(
                    "Delete",
                    Kind::Danger,
                    false,
                    !counting && !deleting,
                    if counting { "The folder is still being counted." } else { "" },
                ) {
                    pressed = Some(true);
                }
                if f.button("Cancel", Kind::Secondary, true, !deleting, "") {
                    pressed = Some(false);
                }
            } else if f.button("Close", Kind::Secondary, true, true, "") {
                pressed = Some(false);
            }
        },
    );
    c.shown = shown.is_some();
    match pressed {
        Some(true) => {
            c.deleting = true;
            actions.push(Action::DeleteNow(c.path.clone(), c.name.clone()));
        }
        Some(false) => st.confirm = None,
        None if !open => st.confirm = None,
        None => {}
    }
}

fn confirm_body(ui: &mut Ui, c: &Confirm) {
    let p = palette(ui.ctx());
    let Some(d) = c.info.as_ref() else {
        text::label(
            ui,
            &format!("Delete the project \"{}\" from disk?", c.name),
            Font::Strong,
            p.text,
            Clip::Middle,
        );
        text::label(ui, &c.path, Font::Body, p.text_secondary, Clip::Start);
        ui.add_space(8.0);
        busy_line(ui, "Counting what the folder holds...");
        return;
    };
    let name = if d.name.is_empty() { c.name.as_str() } else { d.name.as_str() };
    let path = if d.path.is_empty() { c.path.as_str() } else { d.path.as_str() };
    let held = format!(
        "{} in {}, {}",
        cards::count_text(d.files, "file", "files"),
        cards::count_text(d.folders, "folder", "folders"),
        cards::size_text(d.bytes)
    );
    let question = !c.done && d.refused.is_none() && !d.missing;
    let partial = c.done && d.refused.is_none();
    let headline = if question {
        format!("Delete the project \"{name}\" from disk?")
    } else if partial {
        format!("The project \"{name}\" was not deleted completely.")
    } else if d.missing {
        format!("The folder of the project \"{name}\" does not exist any more.")
    } else {
        format!("The project \"{name}\" cannot be deleted from here.")
    };
    text::label(ui, &headline, Font::Strong, p.text, Clip::Middle);
    text::label(ui, path, Font::Body, p.text_secondary, Clip::Start);
    ui.add_space(8.0);
    if question {
        text::secondary(ui, "What goes:");
        what_goes(ui, &held);
        if d.git {
            what_goes(ui, ".git - the git repository, with the project's whole history");
        }
        if d.releases {
            what_goes(
                ui,
                &format!(
                    "releases\\ - the files of published releases: {}, {}",
                    cards::count_text(d.release_files, "file", "files"),
                    cards::size_text(d.release_bytes)
                ),
            );
        }
        if d.links > 0 {
            what_goes(
                ui,
                &format!(
                    "{} - removed as links; what they point at stays",
                    cards::count_text(d.links, "link", "links")
                ),
            );
        }
        ui.add_space(8.0);
        text::status(ui, Tone::Warning, "Deleted for good - this cannot be undone.");
    } else if partial {
        text::secondary(ui, &format!("Deleted: {held}. What stayed:"));
        for f in d.failed.iter().take(10) {
            what_goes(ui, &format!("{} - {}", f.path, f.reason));
        }
        let listed = d.failed.len().min(10) as u32;
        if d.failed_total > listed {
            what_goes(ui, &format!("and {} more", d.failed_total - listed));
        }
        ui.add_space(8.0);
        text::status(
            ui,
            Tone::Info,
            "The project stays in the list. Close whatever holds these files, then delete it again.",
        );
    } else if d.missing {
        text::status(
            ui,
            Tone::Muted,
            "Nothing was deleted. Remove from List in its menu takes it off the list.",
        );
    } else {
        text::status(ui, Tone::Error, d.refused.as_deref().unwrap_or(""));
        for n in d.nested.iter().skip(1) {
            what_goes(ui, n);
        }
        ui.add_space(8.0);
        text::status(ui, Tone::Muted, "Nothing was deleted.");
    }
}

/// Opens the confirmation and takes the check and the count off the UI thread.
pub(crate) fn open_confirm(
    st: &mut PageState,
    editor: &Arc<Editor>,
    page: &Arc<StartPage>,
    path: String,
    name: String,
) {
    st.menu = None;
    st.confirm =
        Some(Confirm { path: path.clone(), name, info: None, done: false, deleting: false, shown: false });
    let (ed, page) = (editor.clone(), page.clone());
    let _task = editor.exec.spawn(async move {
        let info = delete_plan(&ed, path.clone()).await;
        post_ui(&ed, move |_| {
            let mut st = page.state.lock();
            if let Some(c) = st.confirm.as_mut()
                && c.path == path
                && c.info.is_none()
            {
                c.info = Some(info);
            }
        });
    });
}

fn set_page_error(page: &StartPage, text_: String) {
    page.state.lock().page_error = text_;
}

fn run(action: Action, st: &mut PageState, editor: &Arc<Editor>, page: &Arc<StartPage>) {
    let (ed, pg) = (editor.clone(), page.clone());
    match action {
        Action::Open(path) => {
            call_then(editor, "project.open", args([("path", s(&path))]), move |reply, _| {
                if !ok(reply) {
                    let mut st = pg.state.lock();
                    st.page_error = format!("The project cannot be opened: {}", error_text(reply));
                    refresh(&mut st, &ed, &pg);
                }
            })
        }
        Action::OpenFolder => {
            pick_folder(editor, "Open Project Folder", "", move |folder, _| open_folder(&ed, &pg, folder))
        }
        Action::OpenFile => {
            let request = gz_ui::file_dialog::FileRequest::open("Open Project File")
                .filter("Grizzly Editor project", &["grizzly"]);
            crate::folder::pick_file(editor, request, move |file, _| open_folder(&ed, &pg, file))
        }
        Action::Dropped(path) => open_folder(editor, page, path),
        Action::BrowseLocation => {
            let start = st.parent.clone();
            pick_folder(editor, "Project Location", &start, move |folder, _| {
                let mut st = pg.state.lock();
                st.parent = folder;
                st.parent_set = true;
            });
        }
        Action::Create => {
            st.creating = true;
            let a = args([("name", s(&st.name)), ("parent", s(&st.parent))]);
            call_then(editor, "project.create", a, move |reply, _| {
                let mut st = pg.state.lock();
                st.creating = false;
                if ok(reply) {
                    st.name.clear();
                    st.checked_key.clear();
                } else {
                    st.create_error = error_text(reply);
                }
            });
        }
        Action::SaveSort(sort) => {
            let _task = editor.exec.spawn(async move {
                if let Err(e) = Projects::of(&ed).set_sort(sort).await {
                    ed.log.warn(e.message);
                }
            });
        }
        Action::Reveal(path) => {
            call_then(editor, "project.reveal", args([("path", s(&path))]), move |reply, _| {
                if !ok(reply) {
                    set_page_error(&pg, format!("Show in Explorer: {}", error_text(reply)));
                }
            })
        }
        Action::AskDelete(path, name) => open_confirm(st, editor, page, path, name),
        Action::Forget(path, name) => {
            call_then(editor, "project.forget", args([("path", s(&path))]), move |reply, ctx| {
                let mut st = pg.state.lock();
                if ok(reply) {
                    toast(ctx, Tone::Info, &format!("{name} was taken off the list."));
                } else {
                    st.page_error = error_text(reply);
                }
                refresh(&mut st, &ed, &pg);
            })
        }
        Action::DeleteNow(path, name) => {
            call_then(editor, "project.delete", args([("path", s(&path))]), move |reply, ctx| {
                let mut st = pg.state.lock();
                if ok(reply) {
                    st.confirm = None;
                    let info = cards::info_from_reply(reply);
                    let line = if info.missing {
                        format!("Project \"{name}\" removed from the list: its folder was gone already.")
                    } else {
                        format!(
                            "Project \"{name}\" deleted: {}, {}.",
                            cards::count_text(info.files, "file", "files"),
                            cards::count_text(info.folders, "folder", "folders")
                        )
                    };
                    toast(ctx, Tone::Success, &line);
                } else if let Some(c) = st.confirm.as_mut().filter(|c| c.path == path) {
                    c.deleting = false;
                    c.done = true;
                    c.info = Some(cards::info_from_reply(reply));
                }
                refresh(&mut st, &ed, &pg);
            })
        }
        Action::Recheck(path) => {
            let _task = editor.exec.spawn(async move {
                let probe = path.clone();
                let state = ed.exec.io(move || gz_project::list::entry_state(&probe)).await;
                post_ui(&ed, move |_| {
                    let mut st = pg.state.lock();
                    if let Some(card) = st.cards.iter_mut().find(|c| c.path == path) {
                        card.state = state;
                    }
                });
            });
        }
    }
}

/// Open Folder..., Open Project File..., a drop: a project file, or a folder that holds one or
/// `mod.ltx`, opens; anything else says why.
fn open_folder(editor: &Arc<Editor>, page: &Arc<StartPage>, folder: String) {
    let (ed, pg) = (editor.clone(), page.clone());
    let _task = editor.exec.spawn(async move {
        let probe = folder.clone();
        let is_project = ed
            .exec
            .io(move || {
                gz_project::project::project_folder_of(std::path::Path::new(&probe))
                    .is_some_and(|dir| gz_project::list::has_manifest(&dir))
            })
            .await;
        if !is_project {
            let text_ = format!("Not a project - no project file and no mod.ltx: {folder}");
            post_ui(&ed, move |_| set_page_error(&pg, text_));
            return;
        }
        let reply = ed.call("project.open", args([("path", s(&folder))]), ed.ui_origin()).await;
        if !ok(&reply) {
            let text_ = format!("The project cannot be opened: {}", error_text(&reply));
            post_ui(&ed, move |_| set_page_error(&pg, text_));
        }
    });
}

/// What `project.page` answers of a card's picture: `picture` when the card shows its
/// `preview.png`, `placeholder` when it shows the icon instead.
pub(crate) async fn preview_state(page: &StartPage, card: &Card) -> &'static str {
    let (EntryState::Project, Some(rev)) = (card.state, card.preview) else { return "placeholder" };
    let folder = card.path.clone();
    let value = page.previews.fresh(&card.path, rev, move || cards::read_preview(&folder)).await;
    if value.is_some() { "picture" } else { "placeholder" }
}

/// The JSON of a card as `project.page` lists it.
pub(crate) fn card_json(card: &Card, preview: &str) -> Value {
    serde_json::json!({
        "path": paths::wire(&card.path),
        "name": paths::wire(&card.name),
        "state": cards::state_name(card.state),
        "parent": card.parent,
        "preview": preview,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_rule() {
        assert!(valid_name("Overnight_Delivery-2"));
        assert!(!valid_name(""));
        assert!(!valid_name("Qa Created!"));
        assert!(!valid_name("Проект"));
    }

    #[test]
    fn the_menu_follows_the_state() {
        assert_eq!(menu_items(EntryState::Project), [("Show in Explorer", true), ("Delete...", true)]);
        assert_eq!(
            menu_items(EntryState::Missing),
            [("Show in Explorer", false), ("Remove from List", true)]
        );
        assert_eq!(
            menu_items(EntryState::NotAProject),
            [("Show in Explorer", true), ("Remove from List", true)]
        );
    }

    #[test]
    fn the_dialog_is_named_as_the_contract_says() {
        assert_eq!(delete_dialog_name(), "dlg.delete_project");
        assert_eq!(delete_dialog_title(), "Delete Project");
    }

    #[test]
    fn a_new_read_releases_the_pictures_of_cards_that_left() {
        let mut st = PageState::default();
        let card = |p: &str| Card {
            path: p.to_owned(),
            name: p.to_owned(),
            state: EntryState::Project,
            parent: String::new(),
            seg: None,
            preview: None,
        };
        apply_snapshot(
            &mut st,
            Snapshot {
                cards: vec![card("D:\\a"), card("D:\\b")],
                create_parent: "D:\\".to_owned(),
                sort: Sort::Name,
            },
        );
        assert_eq!(st.sort, Sort::Name);
        assert_eq!(st.parent, "D:\\");
        st.sort = Sort::Recent;
        apply_snapshot(
            &mut st,
            Snapshot { cards: vec![card("D:\\b")], create_parent: "E:\\".to_owned(), sort: Sort::Name },
        );
        // a later read keeps the sort the page shows
        assert_eq!(st.sort, Sort::Recent);
        assert_eq!(st.cards.len(), 1);
    }
}
