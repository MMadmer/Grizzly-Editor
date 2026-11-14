//! The Releases tab (document kind `releases`, a singleton): what the repository of `[update]
//! github` holds, drafts included, as a sortable table - Tag | Title | State | Published (local)
//! | Files - with the selected release's details beside it. Edit... opens `dlg.edit_release`,
//! Delete... asks through the kit's confirmation, which names the release, its files and its tag
//! and what players get instead. A failure is a banner sentence naming what failed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Rect, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2};
use gz_app::{Chord, DocCx, Document, EditAction, OpenRequest};
use gz_editor::{Editor, Rev};
use gz_github::ReleaseInfo;
use gz_mod::ManifestService;
use gz_publish::{ManageOp, ManageRequest, ManageState, ManageView, Publisher, ReleaseList, Setup};
use gz_ui::dialog::{self, Answer, ConfirmSpec};
use gz_ui::grid::read_only;
use gz_ui::list::{Column, list};
use gz_ui::menu::{MenuItem, menu_item, popup_body};
use gz_ui::splitter::splitter_tail;
use gz_ui::state::{badge, banner, busy_line, empty_state};
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{
    Button, Font, Icon, Kind, PropFlags, PropertyGrid, Section, Tone, Toolbar, ToolbarStyle, metrics, palette,
};
use parking_lot::Mutex;

use crate::local::cell;
use crate::{Hub, asker, open_page, sentence};

/// A tab that comes back after this long reads the list again.
const RELIST: Duration = Duration::from_secs(60);
const BUSY_WHY: &str = "A release job is running: wait for it to finish.";

// ---- the pure part ----------------------------------------------------------------------------

/// `15 KB`, `2.3 MB`, `512 B`.
#[must_use]
pub(crate) fn size_text(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes >= 1 << 20 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

/// `3 files · 15 KB`.
#[must_use]
pub(crate) fn files_text(r: &ReleaseInfo) -> String {
    let n = r.assets.len();
    format!("{n} {} · {}", if n == 1 { "file" } else { "files" }, size_text(r.bytes))
}

/// GitHub gives the time in UTC (`2026-09-19 12:34`); the author reads it in their own.
#[must_use]
pub(crate) fn local_time(utc: &str) -> String {
    let read = || -> Option<String> {
        let (date, hm) = utc.trim().split_once(' ')?;
        let mut d = date.split('-');
        let year: i32 = d.next()?.parse().ok()?;
        let month = time::Month::try_from(d.next()?.parse::<u8>().ok()?).ok()?;
        let day: u8 = d.next()?.parse().ok()?;
        let (h, m) = hm.split_once(':')?;
        let at = time::PrimitiveDateTime::new(
            time::Date::from_calendar_date(year, month, day).ok()?,
            time::Time::from_hms(h.parse().ok()?, m.parse().ok()?, 0).ok()?,
        )
        .assume_utc();
        let local = at.to_offset(time::UtcOffset::local_offset_at(at).ok()?);
        Some(format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            local.year(),
            u8::from(local.month()),
            local.day(),
            local.hour(),
            local.minute()
        ))
    };
    read().unwrap_or_else(|| utc.to_owned())
}

/// The order of the State column: what players get first.
#[must_use]
pub(crate) fn state_rank(r: &ReleaseInfo) -> u8 {
    if r.latest {
        0
    } else if r.draft {
        3
    } else if r.prerelease {
        2
    } else {
        1
    }
}

/// The column a table is sorted by and which way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sort {
    pub column: usize,
    pub ascending: bool,
}

impl Default for Sort {
    /// The newest first, as GitHub lists them.
    fn default() -> Self {
        Self { column: 3, ascending: false }
    }
}

/// The rows in the order the header asks for; a draft is not published yet: it is the newest.
#[must_use]
pub(crate) fn sorted_order(releases: &[ReleaseInfo], sort: Sort) -> Vec<usize> {
    let mut order: Vec<usize> = (0..releases.len()).collect();
    order.sort_by(|&a, &b| {
        let (Some(x), Some(y)) = (releases.get(a), releases.get(b)) else { return std::cmp::Ordering::Equal };
        let c = match sort.column {
            0 => x.tag.to_lowercase().cmp(&y.tag.to_lowercase()),
            1 => x.title.to_lowercase().cmp(&y.title.to_lowercase()),
            2 => state_rank(x).cmp(&state_rank(y)),
            3 => {
                let key = |r: &ReleaseInfo| {
                    if r.published.is_empty() { "~".to_owned() } else { r.published.clone() }
                };
                key(x).cmp(&key(y))
            }
            _ => x.bytes.cmp(&y.bytes),
        };
        if sort.ascending { c } else { c.reverse() }
    });
    order
}

/// The one question before a release goes: the release, its files and its tag by name, and what
/// players get instead when it is the latest one. GitHub keeps no copy.
#[must_use]
pub(crate) fn delete_spec(view: &ManageView, target: &ReleaseInfo) -> ConfirmSpec {
    // the list is newest first: the first other release players may receive is the fallback
    let next =
        view.releases.iter().find(|r| r.id != target.id && !r.draft && !r.prerelease).map(|r| r.tag.clone());
    let tag = if target.tag.is_empty() { "(no tag)" } else { &target.tag };
    let headline = format!(
        "Delete {} {tag} \"{}\" from {}?",
        if target.draft { "draft release" } else { "release" },
        target.title,
        view.repo
    );
    let mut consequence = String::new();
    if target.latest {
        match &next {
            None => consequence.push_str(
                "It is the latest release: players are offered no update until another one is published. ",
            ),
            Some(next) => consequence
                .push_str(&format!("It is the latest release: GitHub then serves {next} to players. ")),
        }
    }
    if !target.draft && !target.prerelease {
        consequence.push_str(
            "Players who have this version keep it; published again, the game offers it to them as a fix. ",
        );
    }
    consequence.push_str("Deleted from GitHub for good - this cannot be undone.");
    let mut items = target.assets.clone();
    if !target.draft && !target.tag.is_empty() {
        items.push(format!("the git tag {}", target.tag));
    }
    ConfirmSpec {
        id: "delete_release".to_owned(),
        title: "Delete Release".to_owned(),
        headline,
        total: items.len(),
        items,
        consequence,
        action: "Delete Release".to_owned(),
        destructive: true,
    }
}

/// The status bar: the repository and what it holds.
#[must_use]
pub(crate) fn status_text(view: &ManageView) -> String {
    let mut text_ = view.repo.clone();
    if view.listed {
        let drafts = view.releases.iter().filter(|r| r.draft).count();
        text_.push_str(&format!(" · {} published", view.releases.len() - drafts));
        if drafts > 0 {
            text_.push_str(&format!(" · {drafts} {}", if drafts == 1 { "draft" } else { "drafts" }));
        }
    }
    text_
}

/// Asks before a release goes; Yes deletes it, the release first, its tag second.
pub(crate) fn ask_delete(
    ctx: &egui::Context,
    editor: &Arc<Editor>,
    view: &ManageView,
    target: &ReleaseInfo,
    start_error: &Arc<Mutex<String>>,
) {
    let spec = delete_spec(view, target);
    let (ed, slot, id) = (editor.clone(), start_error.clone(), target.id);
    let line = dialog::confirm(ctx, asker(editor), spec, move |answer| {
        if answer != Answer::Yes {
            return;
        }
        let list = ReleaseList::of(&ed);
        if list.selected() == id {
            list.select(0);
        }
        let e2 = ed.clone();
        ed.exec.spawn(async move {
            let rq = ManageRequest { op: ManageOp::Delete, id, ..ManageRequest::default() };
            let started = ReleaseList::of(&e2).start(&e2, rq).await;
            *slot.lock() = started.err().map(|(e, _)| e).unwrap_or_default();
            e2.exec.wake().ping();
        });
    });
    if let Some(line) = line {
        editor.log.finding(&line);
    }
}

// ---- the tab ----------------------------------------------------------------------------------

/// A row's texts, made once per list.
struct Row {
    index: usize,
    id: u64,
    tag: String,
    title: String,
    published: String,
    files: String,
}

/// Why the tab cannot list, as the last start said.
struct Slot {
    setup: Setup,
}

pub(crate) struct ReleasesDoc {
    editor: Arc<Editor>,
    hub: Hub,
    slot: Arc<Mutex<Slot>>,
    listed_at: Option<Instant>,
    setup_rev: Option<Rev>,
    sort: Sort,
    /// The rows of the list of this generation, and their order for `sorted`.
    rows: Vec<Row>,
    rows_for: Option<u64>,
    order: Vec<usize>,
    sorted: Option<(u64, Sort)>,
    tag_w: f32,
}

fn manifest_rev(editor: &Editor) -> Option<Rev> {
    let session = editor.project_opt()?;
    Some(ManifestService::get(&session)?.rev())
}

impl ReleasesDoc {
    pub(crate) fn new(editor: &Arc<Editor>, hub: Hub) -> Self {
        let mut doc = Self {
            editor: editor.clone(),
            hub,
            slot: Arc::new(Mutex::new(Slot { setup: Setup::Ok })),
            listed_at: None,
            setup_rev: None,
            sort: Sort::default(),
            rows: Vec::new(),
            rows_for: None,
            order: Vec::new(),
            sorted: None,
            tag_w: 0.0,
        };
        ReleaseList::of(editor).set_tab_open(true);
        doc.hub.borrow_mut().refresh_wanted = false;
        doc.refresh();
        doc
    }

    fn busy(&self, view: &ManageView) -> bool {
        view.state == ManageState::Running || Publisher::of(&self.editor).running()
    }

    /// Reads the list again. A failure is what the tab shows - never a window of its own.
    fn refresh(&mut self) {
        self.listed_at = Some(Instant::now());
        self.setup_rev = manifest_rev(&self.editor);
        let list = ReleaseList::of(&self.editor);
        if list.running() || Publisher::of(&self.editor).running() {
            return;
        }
        let (ed, slot, error) =
            (self.editor.clone(), self.slot.clone(), self.hub.borrow().start_error.clone());
        self.editor.exec.spawn(async move {
            let started = ReleaseList::of(&ed).start(&ed, ManageRequest::default()).await;
            match started {
                Ok(_) => {
                    slot.lock().setup = Setup::Ok;
                    error.lock().clear();
                }
                Err((e, setup)) => {
                    slot.lock().setup = setup;
                    if setup == Setup::Ok {
                        *error.lock() = e;
                    }
                }
            }
            ed.exec.wake().ping();
        });
    }

    /// A release went out from the window, or the menu asked: the list is read again when it can.
    fn take_refresh(&mut self) {
        let wanted = self.hub.borrow().refresh_wanted;
        if wanted && !ReleaseList::of(&self.editor).running() && !Publisher::of(&self.editor).running() {
            self.hub.borrow_mut().refresh_wanted = false;
            self.refresh();
        }
    }

    fn selected<'v>(&self, view: &'v ManageView) -> Option<&'v ReleaseInfo> {
        let id = ReleaseList::of(&self.editor).selected();
        view.releases.iter().find(|r| r.id == id)
    }

    fn open_edit(&self, target: &ReleaseInfo) {
        self.hub.borrow_mut().open_edit(target.clone());
    }

    fn ask(&self, ctx: &egui::Context, view: &ManageView, target: &ReleaseInfo) {
        let error = self.hub.borrow().start_error.clone();
        ask_delete(ctx, &self.editor, view, target, &error);
    }

    /// The rows' texts and the Tag column's width, once per list.
    fn make_rows(&mut self, ctx: &egui::Context, view: &ManageView) {
        if self.rows_for == Some(view.generation) {
            return;
        }
        self.rows_for = Some(view.generation);
        self.sorted = None;
        self.rows = view
            .releases
            .iter()
            .enumerate()
            .map(|(index, r)| Row {
                index,
                id: r.id,
                tag: if r.tag.is_empty() { "(no tag)".to_owned() } else { r.tag.clone() },
                title: r.title.clone(),
                published: if r.published.is_empty() {
                    "Not published".to_owned()
                } else {
                    local_time(&r.published)
                },
                files: files_text(r),
            })
            .collect();
        // Tag is as wide as the widest tag (a long one ellipsizes); Title takes every change of the width
        let mut w = text::width(ctx, &Font::Small.id(), "Tag") + em(1.25);
        for row in &self.rows {
            w = w.max(text::width(ctx, &Font::Body.id(), &row.tag) + 14.0);
        }
        self.tag_w = w.min(em(14.0));
    }

    fn table(
        &mut self,
        ui: &mut Ui,
        ctx: &egui::Context,
        view: &ManageView,
        busy: bool,
        size: Vec2,
        web: &str,
    ) {
        self.make_rows(ctx, view);
        if self.sorted != Some((view.generation, self.sort)) {
            self.order = sorted_order(&view.releases, self.sort);
            self.sorted = Some((view.generation, self.sort));
        }
        let fixed = [em(6.0), em(9.0), em(8.5)];
        let inner = size.x - 10.0;
        let title_w = (inner - self.tag_w - fixed.iter().sum::<f32>()).max(em(6.0));
        let widths = [self.tag_w, title_w, fixed[0], fixed[1], fixed[2]];
        let names = ["Tag", "Title", "State", "Published (local)", "Files"];
        let columns: Vec<Column<'_>> = names
            .iter()
            .zip(widths)
            .enumerate()
            .map(|(i, (name, width))| Column {
                name,
                width,
                centred: false,
                sorted: (self.sort.column == i).then_some(self.sort.ascending),
                sortable: true,
            })
            .collect();
        let selected = ReleaseList::of(&self.editor).selected();
        let mut picked = None;
        let mut menu_act: Option<(RowAct, usize)> = None;
        let p = palette(ctx);
        ui.horizontal(|ui| {
            ui.add_space(5.0);
            if let Some(c) = gz_ui::list::table_headers_ruled(ui, &columns) {
                // a click on the sorted column turns it round; another column sorts its own way
                self.sort = if self.sort.column == c {
                    Sort { column: c, ascending: !self.sort.ascending }
                } else {
                    Sort { column: c, ascending: c != 3 }
                };
            }
        });
        let height = (size.y - 28.0).max(metrics::ROW * 2.0);
        list(ui, "releases", Vec2::new(size.x, height), |l| {
            for &k in &self.order {
                let (Some(row), Some(r)) = (self.rows.get(k), view.releases.get(k)) else { continue };
                let resp = l.row(&row.id.to_string(), row.id == selected, |ui| {
                    let rect = ui.max_rect();
                    let mut x = rect.min.x - 6.0;
                    for (i, w) in widths.iter().enumerate() {
                        let cell_rect = Rect::from_min_max(
                            pos2(x + 6.0, rect.min.y),
                            pos2((x + w - 6.0).max(x + 7.0), rect.max.y),
                        );
                        x += w;
                        let mut c = cell(ui, ("cell", row.id, i), cell_rect);
                        match i {
                            0 => {
                                text::label(&mut c, &row.tag, Font::Body, p.text, Clip::End);
                            }
                            1 => {
                                text::label(&mut c, &row.title, Font::Body, p.text, Clip::End);
                            }
                            2 => state_cell(&mut c, r),
                            3 => {
                                text::label(&mut c, &row.published, Font::Body, p.text, Clip::End);
                            }
                            _ => {
                                text::label(&mut c, &row.files, Font::Body, p.text, Clip::End);
                            }
                        }
                    }
                });
                if resp.clicked() || resp.secondary_clicked() {
                    picked = Some(row.id);
                }
                let index = row.index;
                let _ = resp.context_menu(|ui| {
                    popup_body(ui, |ui| {
                        if menu_item(
                            ui,
                            "Edit Title and Notes...",
                            MenuItem { enabled: !busy, ..MenuItem::default() },
                        ) {
                            menu_act = Some((RowAct::Edit, index));
                        }
                        if menu_item(
                            ui,
                            "Open Release Page",
                            MenuItem { enabled: crate::page_ok(&r.url, web), ..MenuItem::default() },
                        ) {
                            menu_act = Some((RowAct::Page, index));
                        }
                        if menu_item(
                            ui,
                            "Copy Tag",
                            MenuItem { enabled: !r.tag.is_empty(), ..MenuItem::default() },
                        ) {
                            menu_act = Some((RowAct::CopyTag, index));
                        }
                        ui.separator();
                        if menu_item(
                            ui,
                            "Delete Release...",
                            MenuItem {
                                icon: Icon::Delete,
                                enabled: !busy,
                                danger: true,
                                ..MenuItem::default()
                            },
                        ) {
                            menu_act = Some((RowAct::Delete, index));
                        }
                    });
                });
            }
        });
        if let Some(id) = picked {
            ReleaseList::of(&self.editor).select(id);
        }
        if let Some((act, index)) = menu_act
            && let Some(r) = view.releases.get(index)
        {
            ReleaseList::of(&self.editor).select(r.id);
            match act {
                RowAct::Edit => self.open_edit(r),
                RowAct::Page => open_page(ctx, &r.url, web),
                RowAct::CopyTag => ctx.copy_text(r.tag.clone()),
                RowAct::Delete => self.ask(ctx, view, r),
            }
        }
    }

    fn details(&self, ui: &mut Ui, ctx: &egui::Context, view: &ManageView, busy: bool, web: &str) {
        let Some(r) = self.selected(view) else {
            empty_state(
                ui,
                Icon::Releases,
                "No release selected",
                "Select a release to see its files and notes.",
                None,
            );
            return;
        };
        let p = palette(ctx);
        let (edit, page, delete) = {
            let mut tb = Toolbar::begin(ui, "release_actions", ToolbarStyle::Panel);
            tb.group(2);
            let edit_tip = if busy {
                BUSY_WHY
            } else {
                "Edit the title and the notes. The files, the tag and the latest mark stay as they are."
            };
            let edit = tb.action("edit", Icon::Rename, "Edit...", edit_tip, !busy, false, Kind::Ghost);
            tb.group(1);
            let page = tb.action(
                "page",
                Icon::External,
                "Open Page",
                "The release's page on GitHub.",
                crate::page_ok(&r.url, web),
                false,
                Kind::Ghost,
            );
            tb.spring();
            tb.group(3);
            let delete_tip = if busy {
                BUSY_WHY
            } else {
                "Delete the release and its git tag from GitHub. Asks first, naming what goes."
            };
            let delete =
                tb.action("delete", Icon::Delete, "Delete...", delete_tip, !busy, false, Kind::GhostDanger);
            tb.end();
            (edit, page, delete)
        };
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let tag = if r.tag.is_empty() { "(no tag)" } else { &r.tag };
            ui.scope(|ui| {
                ui.set_max_width((ui.available_width() - em(6.0)).max(em(4.0)));
                text::label(ui, tag, Font::Title, p.text, Clip::End);
            });
            state_badge(ui, r);
        });
        if !r.title.is_empty() {
            text::prose(ui, &r.title);
        }
        ui.add_space(4.0);
        PropertyGrid::new("release").label_frac(gz_ui::units::em(8.0) / ui.available_width().max(1.0)).show(
            ui,
            |g| {
                let published = if r.published.is_empty() {
                    "Not published (draft)".to_owned()
                } else {
                    local_time(&r.published)
                };
                g.row(
                    "Published",
                    "In your local time. A draft is not published: players never see it.",
                    PropFlags::READ_ONLY,
                    0,
                    |ui| {
                        read_only(ui, &published, Clip::End);
                    },
                );
                let files = files_text(r);
                g.row(
                    "Files",
                    "What the release holds: the packages, the file index and the descriptor.",
                    PropFlags::READ_ONLY,
                    0,
                    |ui| {
                        read_only(ui, &files, Clip::End);
                    },
                );
            },
        );
        ui.indent("release_files", |ui| {
            for name in &r.assets {
                text::label(ui, name, Font::Body, p.text_secondary, Clip::Middle);
            }
        });
        ui.add_space(4.0);
        if Section::new("release_notes", "Notes").show(ui).open {
            ui.add_space(2.0);
            if r.notes.is_empty() {
                text::label(ui, "No notes.", Font::Body, p.text_muted, Clip::End);
            } else {
                text::prose(ui, &r.notes);
            }
            ui.add_space(4.0);
        }
        if edit {
            self.open_edit(r);
        }
        if page {
            open_page(ctx, &r.url, web);
        }
        if delete {
            self.ask(ctx, view, r);
        }
    }
}

#[derive(Clone, Copy)]
enum RowAct {
    Edit,
    Page,
    CopyTag,
    Delete,
}

fn state_badge(ui: &mut Ui, r: &ReleaseInfo) {
    if r.latest {
        badge(ui, "Latest", Tone::Success);
    } else if r.draft {
        badge(ui, "Draft", Tone::Muted);
    } else if r.prerelease {
        badge(ui, "Pre-release", Tone::Info);
    }
}

/// A badge for what stands out, plain secondary text for a release that is simply out.
fn state_cell(ui: &mut Ui, r: &ReleaseInfo) {
    if r.latest || r.draft || r.prerelease {
        state_badge(ui, r);
    } else {
        text::secondary(ui, "Published");
    }
}

fn web_of(editor: &Editor) -> String {
    gz_github::Github::of(editor).endpoints().web.clone()
}

/// An empty state whose action is a Secondary button: the tab's row holds Build Mod, the one
/// Primary of every tab.
fn empty_with(ui: &mut Ui, icon: Icon, title: &str, text_: &str, action: Option<&str>) -> bool {
    empty_state(ui, icon, title, text_, None);
    let Some(action) = action else { return false };
    ui.vertical_centered(|ui| Button::new(action).show(ui).clicked()).inner
}

impl Document for ReleasesDoc {
    fn id(&self) -> &str {
        crate::RELEASES_KIND
    }

    fn kind(&self) -> &'static str {
        crate::RELEASES_KIND
    }

    fn label(&self) -> String {
        "Releases".to_owned()
    }

    fn icon(&self) -> Icon {
        Icon::Releases
    }

    fn tooltip(&self) -> Option<String> {
        let repo = ReleaseList::of(&self.editor).view().repo;
        Some(if repo.is_empty() {
            "The module's releases on GitHub".to_owned()
        } else {
            format!("The releases of {repo} on GitHub")
        })
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        let view = ReleaseList::of(cx.editor).view();
        let busy = self.busy(&view);
        tb.group(2);
        if tb.action(
            "refresh",
            Icon::Refresh,
            "Refresh",
            if busy { BUSY_WHY } else { "Read the releases from GitHub again (F5)." },
            !busy,
            false,
            Kind::Ghost,
        ) {
            self.refresh();
        }
        tb.group(1);
        let web = web_of(cx.editor);
        let page = if view.repo.is_empty() { String::new() } else { format!("{web}/{}/releases", view.repo) };
        if tb.action(
            "github",
            Icon::External,
            "Open on GitHub",
            "The repository's releases page in the browser.",
            crate::page_ok(&page, &web),
            false,
            Kind::Ghost,
        ) {
            open_page(cx.ctx, &page, &web);
        }
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        self.take_refresh();
        let ctx = cx.ctx.clone();
        let p = palette(&ctx);
        if cx.editor.project_opt().is_none() {
            empty_state(
                ui,
                Icon::Releases,
                "No project",
                "Open a project to see the releases of its module.",
                None,
            );
            return;
        }
        let setup = self.slot.lock().setup;
        // Mod Settings set the repository meanwhile: the list follows
        if setup == Setup::NoRepo && self.setup_rev != manifest_rev(cx.editor) {
            self.refresh();
        }
        match setup {
            Setup::NoProject => {
                empty_state(
                    ui,
                    Icon::Releases,
                    "No project",
                    "Open a project to see the releases of its module.",
                    None,
                );
                return;
            }
            Setup::NoRepo => {
                if empty_with(
                    ui,
                    Icon::Releases,
                    "No GitHub repository",
                    "The game updates this module from the latest release of the GitHub repository Mod Settings names.",
                    Some("Open Mod Settings"),
                ) {
                    cx.requests.open(OpenRequest::kind("mod_settings"));
                }
                return;
            }
            Setup::NoAccount => {
                if empty_with(
                    ui,
                    Icon::Link,
                    "Not signed in to GitHub",
                    "Sign in from Publish Release, sign in to the GitHub CLI (gh auth login) or set GH_TOKEN, then refresh.",
                    Some("Sign In..."),
                ) {
                    cx.requests.dialog(crate::PUBLISH_ID);
                }
                return;
            }
            Setup::Ok => {}
        }

        let view = ReleaseList::of(cx.editor).view();
        let busy = self.busy(&view);
        let start_error = self.hub.borrow().start_error.lock().clone();
        if Publisher::of(cx.editor).running() {
            banner(
                ui,
                Tone::Info,
                "A release is being published: the list is read again when it is out.",
                None,
                None,
            );
        }
        if view.state == ManageState::Running {
            busy_line(
                ui,
                match view.op {
                    ManageOp::Delete => "Deleting the release...",
                    ManageOp::Edit => "Saving the release...",
                    ManageOp::List => "Reading the releases...",
                },
            );
        } else if !view.error.is_empty() || !start_error.is_empty() {
            let text_ = if view.error.is_empty() { sentence(&start_error) } else { view.error_text.clone() };
            if banner(ui, Tone::Error, &text_, Some("Retry"), None) == Some(0) {
                self.refresh();
            }
        }
        if !view.note.is_empty() {
            banner(ui, if view.tag_kept { Tone::Warning } else { Tone::Success }, &view.note, None, None);
        }
        if !view.listed {
            return;
        }
        if view.releases.is_empty() {
            if empty_with(
                ui,
                Icon::Releases,
                "No releases yet",
                "Publish Release packages the module and publishes it here.",
                Some("Publish Release..."),
            ) {
                cx.requests.dialog(crate::PUBLISH_ID);
            }
            return;
        }

        // the table takes what the details leave; too narrow for both, the table alone
        let web = web_of(cx.editor);
        let avail = ui.available_size();
        let origin = ui.cursor().min;
        let gap = ui.spacing().item_spacing.x;
        if avail.x < 400.0 + 260.0 + gap {
            let mut t = ui.new_child(
                UiBuilder::new().id_salt("releases_table").max_rect(Rect::from_min_size(origin, avail)),
            );
            self.table(&mut t, &ctx, &view, busy, avail, &web);
            ui.allocate_rect(Rect::from_min_size(origin, avail), Sense::hover());
            return;
        }
        let side = splitter_tail(ui, "releases.details", true, 400.0, 260.0, 340.0).size;
        let table_w = avail.x - side - gap;
        let table_rect = Rect::from_min_size(origin, Vec2::new(table_w, avail.y));
        let mut t = ui.new_child(UiBuilder::new().id_salt("releases_table").max_rect(table_rect));
        self.table(&mut t, &ctx, &view, busy, table_rect.size(), &web);

        let side_rect =
            Rect::from_min_size(pos2(origin.x + table_w + gap, origin.y), Vec2::new(side, avail.y));
        ui.painter().rect_stroke(
            side_rect,
            egui::CornerRadius::same(metrics::RADIUS),
            Stroke::new(1.0, p.border_subtle),
            StrokeKind::Inside,
        );
        let inner = side_rect.shrink2(Vec2::new(12.0, 8.0));
        let mut d = ui.new_child(UiBuilder::new().id_salt("release_details").max_rect(inner));
        d.set_clip_rect(inner.intersect(ui.clip_rect()));
        ScrollArea::vertical().id_salt("release_details_scroll").auto_shrink([false, false]).show(
            &mut d,
            |ui| {
                self.details(ui, &ctx, &view, busy, &web);
            },
        );
        ui.allocate_rect(Rect::from_min_size(origin, avail), Sense::hover());
    }

    fn status(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let view = ReleaseList::of(cx.editor).view();
        if !view.repo.is_empty() {
            text::secondary(ui, &status_text(&view));
        }
    }

    fn shortcut(&mut self, cx: &mut DocCx<'_>, chord: &Chord) -> bool {
        if chord.ctrl || chord.shift || chord.alt {
            return false;
        }
        if chord.key == egui::Key::F5 {
            self.refresh();
            return true;
        }
        if chord.key != egui::Key::Delete {
            return false;
        }
        let view = ReleaseList::of(cx.editor).view();
        let Some(r) = self.selected(&view).cloned() else { return false };
        if self.busy(&view) {
            return false;
        }
        self.ask(cx.ctx, &view, &r);
        true
    }

    fn edit(&mut self, cx: &mut DocCx<'_>, action: EditAction, run: bool) -> bool {
        if action != EditAction::Delete {
            return false;
        }
        let view = ReleaseList::of(cx.editor).view();
        let target = self.selected(&view).cloned();
        let can = target.is_some() && !self.busy(&view);
        if run
            && can
            && let Some(r) = target
        {
            self.ask(cx.ctx, &view, &r);
        }
        can
    }

    fn on_activate(&mut self, _cx: &mut DocCx<'_>) {
        ReleaseList::of(&self.editor).set_tab_open(true);
        if !ReleaseList::of(&self.editor).running() && self.listed_at.is_none_or(|at| at.elapsed() > RELIST) {
            self.refresh();
        }
    }

    fn inactive(&mut self, _cx: &mut DocCx<'_>) {
        self.take_refresh();
    }

    fn on_close(&mut self, _cx: &mut DocCx<'_>) {
        ReleaseList::of(&self.editor).set_tab_open(false);
    }

    fn mcp_state(&self) -> serde_json::Value {
        serde_json::json!({ "selected": ReleaseList::of(&self.editor).selected() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(id: u64, tag: &str, published: &str) -> ReleaseInfo {
        ReleaseInfo {
            id,
            tag: tag.to_owned(),
            title: format!("OD {tag}"),
            published: published.to_owned(),
            ..ReleaseInfo::default()
        }
    }

    #[test]
    fn sizes_and_files() {
        assert_eq!(size_text(512), "512 B");
        assert_eq!(size_text(15 * 1024), "15 KB");
        assert_eq!(size_text(3 * 1_048_576 + 400_000), "3.4 MB");
        let r = ReleaseInfo {
            assets: vec!["a".into(), "b".into(), "c".into()],
            bytes: 15 * 1024,
            ..ReleaseInfo::default()
        };
        assert_eq!(files_text(&r), "3 files · 15 KB");
        let one = ReleaseInfo { assets: vec!["a".into()], bytes: 10, ..ReleaseInfo::default() };
        assert_eq!(files_text(&one), "1 file · 10 B");
    }

    #[test]
    fn a_time_that_does_not_read_stays_as_it_is() {
        assert_eq!(local_time("soon"), "soon");
        assert_eq!(local_time("2026-13-01 10:00"), "2026-13-01 10:00");
        assert_eq!(local_time("2026-09-19 12:34").len(), 16);
    }

    #[test]
    fn drafts_are_the_newest() {
        let mut draft = rel(3, "v1.2.0", "");
        draft.draft = true;
        let releases =
            vec![rel(1, "v1.0.0", "2026-01-01 10:00"), draft, rel(2, "v1.1.0", "2026-02-01 10:00")];
        assert_eq!(sorted_order(&releases, Sort::default()), vec![1, 2, 0]);
        assert_eq!(sorted_order(&releases, Sort { column: 0, ascending: true }), vec![0, 2, 1]);
        assert_eq!(sorted_order(&releases, Sort { column: 2, ascending: true }), vec![0, 2, 1]);
    }

    #[test]
    fn the_delete_question_names_what_goes() {
        let mut latest = rel(2, "v1.1.0", "2026-02-01 10:00");
        latest.latest = true;
        latest.assets = vec!["od-1.1.0.zip".into(), "od.update.ltx".into()];
        let view = ManageView {
            repo: "me/od".into(),
            listed: true,
            releases: vec![latest.clone(), rel(1, "v1.0.0", "2026-01-01 10:00")],
            ..ManageView::default()
        };
        let spec = delete_spec(&view, &latest);
        assert_eq!(spec.headline, "Delete release v1.1.0 \"OD v1.1.0\" from me/od?");
        assert_eq!(
            spec.items,
            vec!["od-1.1.0.zip".to_owned(), "od.update.ltx".to_owned(), "the git tag v1.1.0".to_owned()]
        );
        assert_eq!(spec.total, 3);
        assert!(
            spec.consequence.starts_with("It is the latest release: GitHub then serves v1.0.0 to players.")
        );
        assert!(spec.consequence.ends_with("Deleted from GitHub for good - this cannot be undone."));
        assert!(spec.destructive);
        let mut draft = rel(3, "v1.2.0", "");
        draft.draft = true;
        let spec = delete_spec(&view, &draft);
        assert!(spec.headline.starts_with("Delete draft release v1.2.0"));
        assert!(spec.items.is_empty());
    }

    #[test]
    fn the_status_counts_drafts() {
        let mut draft = rel(3, "v1.2.0", "");
        draft.draft = true;
        let view = ManageView {
            repo: "me/od".into(),
            listed: true,
            releases: vec![draft, rel(1, "v1.0.0", "x")],
            ..ManageView::default()
        };
        assert_eq!(status_text(&view), "me/od · 1 published · 1 draft");
    }
}
